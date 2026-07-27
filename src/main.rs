use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use slint::{Model, ModelRc, VecModel};

use trai::capture::source_list::{enumerate, resolve_defaults, SourceKind};
use trai::config::{Config, TranslateBackendConfig};
use trai::domain::Segment;
use trai::recording::{Recording, RecordingParams};
use trai::transcriber::{Transcriber, WhisperClient};
use trai::transcript::{build_rows, TranscriptRow as UiTranscriptRow};
use trai::translator::{FallbackTranslator, OpenAITranslator, Translator};

slint::include_modules!();

fn main() {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".to_string());
    let config = match Config::load(Path::new(&config_path)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to load config from {config_path}: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = run(config) {
        eprintln!("trai: {e}");
        std::process::exit(1);
    }
}

/// Format `secs` (seconds since the Unix epoch) as a UTC
/// `YYYY-MM-DD HH:MM` string using the civil-from-days algorithm,
/// without pulling in chrono.
fn format_timestamp(secs: u64) -> String {
    const SECS_PER_DAY: u64 = 86_400;
    let days = (secs / SECS_PER_DAY) as i64;
    let rem = secs % SECS_PER_DAY;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;

    // Howard Hinnant's civil-from-days: days since 1970-01-01 -> y/m/d.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02} {:02}:{:02}", year, m, d, hour, minute)
}

fn make_transcript_updater(
    window_weak: slint::Weak<AppWindow>,
) -> impl Fn(Vec<Segment>) + Send + Sync + 'static {
    move |segments: Vec<Segment>| {
        let rows: Vec<TranscriptRow> = build_rows(&segments)
            .into_iter()
            .map(|row: UiTranscriptRow| TranscriptRow {
                speaker: row.speaker.into(),
                timestamp: row.timestamp.into(),
                text: row.text.into(),
                translation: row.translation.into(),
                degraded: row.degraded,
                error: row.error,
            })
            .collect();
        let window_weak = window_weak.clone();
        // Marshals each live-view snapshot onto the UI thread.
        // Captures only the Send-safe weak handle; the row model
        // is rebuilt off-thread and handed to invoke_from_event_loop.
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = window_weak.upgrade() {
                w.set_transcript_rows(ModelRc::new(VecModel::from(rows)));
            }
        });
    }
}

fn run(config: Config) -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;

    let recording: Rc<RefCell<Option<Recording>>> = Rc::new(RefCell::new(None));

    // Captured by value so the handlers stay `FnMut` and can be invoked
    // for each new Recording without borrowing `config`.
    let cfg_store_root = config.store_root.clone();
    let cfg_mic_source = config.mic_source.clone();
    let cfg_monitor_source = config.monitor_source.clone();
    let cfg_mic_language = config.mic_language.clone();
    let cfg_monitor_language = config.monitor_language.clone();
    let cfg_whisper_url = config.whisper_url.clone();
    let cfg_translate_backends = config.translate.backends.clone();
    let cfg_translate_request_timeout = Duration::from_millis(config.translate.request_timeout_ms);
    let cfg_translate_reprobe_interval =
        Duration::from_millis(config.translate.reprobe_interval_ms);
    let cfg_target_language = config.target_language.clone();
    let cfg_vad_threshold = config.vad_threshold;
    let cfg_silence_hold_ms = config.silence_hold_ms;
    let cfg_duration_cap_ms = config.duration_cap_ms;
    let cfg_confidence_floor = config.confidence_floor;

    {
        let window_weak = window.as_weak();
        let cfg_store_root = cfg_store_root.clone();
        let cfg_mic_source = cfg_mic_source.clone();
        let cfg_monitor_source = cfg_monitor_source.clone();
        window.on_start_clicked(move || {
            // start-clicked now opens the source-selection wizard rather
            // than starting a recording immediately. Enumerate PipeWire
            // sources, resolve the configured defaults within them, and
            // hand the filtered lists to the wizard's combo boxes.
            let sources = match enumerate() {
                Ok(s) => s,
                Err(e) => {
                    if let Some(w) = window_weak.upgrade() {
                        w.set_status_text(format!("enumerate failed: {e}").into());
                    }
                    return;
                }
            };
            let (mic_resolved, monitor_resolved) =
                resolve_defaults(&sources, &cfg_mic_source, &cfg_monitor_source);

            let mic_names: Vec<String> = sources
                .iter()
                .filter(|s| s.kind == SourceKind::Input)
                .map(|s| s.name.clone())
                .collect();
            let monitor_names: Vec<String> = sources
                .iter()
                .filter(|s| s.kind == SourceKind::Monitor)
                .map(|s| s.name.clone())
                .collect();

            // A monitor is never offered as a microphone and vice versa.
            // Map the resolved source index to its position within the
            // filtered list; an absent configured default leaves that
            // selection empty (-1) rather than preselecting something
            // the user did not ask for.
            let mic_selected = mic_resolved
                .and_then(|i| mic_names.iter().position(|n| n == &sources[i].name))
                .map(|p| p as i32)
                .unwrap_or(-1);
            let monitor_selected = monitor_resolved
                .and_then(|i| monitor_names.iter().position(|n| n == &sources[i].name))
                .map(|p| p as i32)
                .unwrap_or(-1);

            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            if let Some(w) = window_weak.upgrade() {
                w.set_wizard_title(format_timestamp(secs).into());
                let mic_model: Vec<slint::SharedString> =
                    mic_names.iter().map(|n| n.as_str().into()).collect();
                let monitor_model: Vec<slint::SharedString> =
                    monitor_names.iter().map(|n| n.as_str().into()).collect();
                w.set_mic_source_names(ModelRc::new(VecModel::from(mic_model)));
                w.set_monitor_source_names(ModelRc::new(VecModel::from(monitor_model)));
                w.set_mic_selected_index(mic_selected);
                w.set_monitor_selected_index(monitor_selected);
                w.set_store_root_display(cfg_store_root.display().to_string().into());
                w.set_wizard_open(true);
            }
        });
    }

    {
        let recording_slot = recording.clone();
        let window_weak = window.as_weak();
        let cfg_store_root = cfg_store_root.clone();
        let cfg_mic_language = cfg_mic_language.clone();
        let cfg_monitor_language = cfg_monitor_language.clone();
        let cfg_whisper_url = cfg_whisper_url.clone();
        let cfg_translate_backends = cfg_translate_backends.clone();
        let cfg_target_language = cfg_target_language.clone();
        window.on_begin_clicked(move || {
            let w = match window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };

            let mic_names: ModelRc<slint::SharedString> = w.get_mic_source_names();
            let monitor_names: ModelRc<slint::SharedString> = w.get_monitor_source_names();
            let mic_index = w.get_mic_selected_index();
            let monitor_index = w.get_monitor_selected_index();

            let mic_source = match usize::try_from(mic_index)
                .ok()
                .and_then(|i| mic_names.row_data(i))
            {
                Some(name) => name.to_string(),
                None => {
                    w.set_status_text("select a microphone source".into());
                    return;
                }
            };
            let monitor_source = match usize::try_from(monitor_index)
                .ok()
                .and_then(|i| monitor_names.row_data(i))
            {
                Some(name) => name.to_string(),
                None => {
                    w.set_status_text("select a monitor source".into());
                    return;
                }
            };

            let title = {
                let t = w.get_wizard_title().to_string();
                if t.trim().is_empty() {
                    let secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    format_timestamp(secs)
                } else {
                    t
                }
            };

            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let store_dir = cfg_store_root.join(secs.to_string());

            let params = RecordingParams {
                store_dir,
                title,
                mic_source,
                monitor_source,
                mic_language: cfg_mic_language.clone(),
                monitor_language: cfg_monitor_language.clone(),
                vad_threshold: cfg_vad_threshold,
                silence_hold_ms: cfg_silence_hold_ms,
                duration_cap_ms: cfg_duration_cap_ms,
                confidence_floor: cfg_confidence_floor,
            };

            let transcriber: Arc<dyn Transcriber> = Arc::new(WhisperClient::new(&cfg_whisper_url));
            let translate_backends: Vec<Arc<dyn Translator>> = cfg_translate_backends
                .iter()
                .map(|backend: &TranslateBackendConfig| {
                    Arc::new(OpenAITranslator::new(
                        &backend.base_url,
                        &backend.model,
                        &cfg_target_language,
                        backend.api_key.clone(),
                        cfg_translate_request_timeout,
                    )) as Arc<dyn Translator>
                })
                .collect();
            let translator: Arc<dyn Translator> = Arc::new(FallbackTranslator::new(
                translate_backends,
                cfg_translate_reprobe_interval,
            ));

            let on_transcript_update = make_transcript_updater(window_weak.clone());

            match Recording::start(params, transcriber, translator, on_transcript_update) {
                Ok(recording) => {
                    *recording_slot.borrow_mut() = Some(recording);
                    w.set_wizard_open(false);
                    w.set_recording_active(true);
                    w.set_status_text("".into());
                }
                Err(e) => {
                    // Leave the wizard open so the user can retry or
                    // pick a different source.
                    w.set_status_text(format!("start failed: {e}").into());
                }
            }
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_cancel_wizard_clicked(move || {
            if let Some(w) = window_weak.upgrade() {
                w.set_wizard_open(false);
            }
        });
    }

    {
        let recording_slot = recording.clone();
        let window_weak = window.as_weak();
        window.on_stop_clicked(move || {
            let Some(recording) = recording_slot.borrow_mut().take() else {
                return;
            };
            // stop() joins both reader threads and every outstanding
            // submission thread, so it can block for the tail of an
            // in-flight transcription; run it off the UI thread and
            // flip the active flag back from the event loop.
            let window_weak = window_weak.clone();
            std::thread::spawn(move || {
                if let Err(e) = recording.stop() {
                    eprintln!("recording stop failed: {e}");
                }
                let window_weak = window_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(w) = window_weak.upgrade() {
                        w.set_recording_active(false);
                    }
                });
            });
        });
    }

    window.run()
}
