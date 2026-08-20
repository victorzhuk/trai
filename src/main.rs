use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use slint::{Model, ModelRc, VecModel};

use trai::capture::source_list::{enumerate, resolve_defaults, SourceKind};
use trai::config::{Config, TranslateBackendConfig};
use trai::domain::Segment;
use trai::history;
use trai::pipeline::{Pipeline, TranscriptUpdate};
use trai::recording::{Recording, RecordingParams};
use trai::transcriber::{Transcriber, WhisperClient};
use trai::transcript::{build_row, summarize, TranscriptRow as UiTranscriptRow};
use trai::translator::{FallbackTranslator, OpenAITranslator, Translator};

slint::include_modules!();

fn main() {
    trai::log::init();

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

thread_local! {
    // Owned by the UI thread. Live updates mutate this model in place
    // (insert/remove/set_row_data), so one row's change costs O(1)
    // instead of rebuilding and re-rendering the whole list per event.
    static TRANSCRIPT_MODEL: RefCell<Option<Rc<VecModel<TranscriptRow>>>> =
        const { RefCell::new(None) };
}

fn transcript_model() -> Rc<VecModel<TranscriptRow>> {
    TRANSCRIPT_MODEL.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(Rc::new(VecModel::from(Vec::<TranscriptRow>::new())));
        }
        slot.as_ref().expect("model just initialized").clone()
    })
}

fn to_ui_row(segment: &Segment, target_language: &str) -> TranscriptRow {
    let row: UiTranscriptRow = build_row(segment, target_language);
    TranscriptRow {
        speaker: row.speaker.into(),
        mine: row.mine,
        timestamp: row.timestamp.into(),
        text: row.text.into(),
        translation: row.translation.into(),
        transcribing: row.transcribing,
        text_failed: row.text_failed,
        pending: row.pending,
        verbatim: row.verbatim,
        degraded: row.degraded,
        error: row.error,
    }
}

fn to_ui_rows(segments: &[Segment], target_language: &str) -> Vec<TranscriptRow> {
    segments
        .iter()
        .map(|segment| to_ui_row(segment, target_language))
        .collect()
}

fn make_transcript_updater(
    window_weak: slint::Weak<AppWindow>,
    target_language: String,
) -> impl Fn(TranscriptUpdate) + Send + Sync + 'static {
    // Mirrors the live view on the callback's side so the status chips
    // can be recomputed without the pipeline shipping a full snapshot
    // per event. Fresh per Recording: rows accumulate from an empty
    // live view, matching the model reset in on_begin_clicked.
    let mirror: Arc<Mutex<Vec<Segment>>> = Arc::new(Mutex::new(Vec::new()));
    move |update: TranscriptUpdate| {
        let status = {
            let mut mirror = mirror.lock().expect("transcript mirror mutex poisoned");
            match &update {
                TranscriptUpdate::Insert { index, segment } => {
                    mirror.insert(*index, segment.clone())
                }
                TranscriptUpdate::Replace { index, segment } => mirror[*index] = segment.clone(),
                TranscriptUpdate::Remove { index } => {
                    mirror.remove(*index);
                }
            }
            summarize(&mirror, &target_language)
        };
        let window_weak = window_weak.clone();
        let target_language = target_language.clone();
        // Marshals each update onto the UI thread, which owns the model.
        let _ = slint::invoke_from_event_loop(move || {
            let model = transcript_model();
            match update {
                TranscriptUpdate::Insert { index, segment } => {
                    model.insert(index, to_ui_row(&segment, &target_language));
                }
                TranscriptUpdate::Replace { index, segment } => {
                    model.set_row_data(index, to_ui_row(&segment, &target_language));
                }
                TranscriptUpdate::Remove { index } => {
                    model.remove(index);
                }
            }
            if let Some(w) = window_weak.upgrade() {
                w.set_pending_transcriptions(status.transcribing as i32);
                w.set_pending_translations(status.translating as i32);
                w.set_failed_transcriptions(status.failed as i32);
                w.set_mic_language(status.mic_language.unwrap_or_default().into());
                w.set_monitor_language(status.monitor_language.unwrap_or_default().into());
                w.set_translation_degraded(status.degraded);
            }
        });
    }
}

// Rebuilds the history list model from the store root and hands it to
// the window on the UI thread. Called on startup, after a recording
// stops, and whenever the list is returned to. `list_recordings` does
// filesystem I/O; for a small store root this is acceptable inline,
// matching the enumerate() call in on_start_clicked.
fn refresh_history_rows(store_root: &Path, window_weak: slint::Weak<AppWindow>) {
    let rows: Vec<HistoryRow> = match history::list_recordings(store_root) {
        Ok(entries) => entries
            .iter()
            .map(|entry| HistoryRow {
                title: entry.title.clone().into(),
                date: format_timestamp(entry.start_time).into(),
                duration: match entry.duration_secs {
                    Some(d) => format!("{}:{:02}", d / 60, d % 60).into(),
                    None => "—".into(),
                },
                line_count: entry.line_count as i32,
            })
            .collect(),
        Err(e) => {
            let msg = format!("history list failed: {e}");
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = window_weak.upgrade() {
                    w.set_status_text(msg.into());
                }
            });
            return;
        }
    };
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(w) = window_weak.upgrade() {
            w.set_history_rows(ModelRc::new(VecModel::from(rows)));
        }
    });
}

fn run(config: Config) -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    window.set_transcript_rows(ModelRc::from(transcript_model()));

    let recording: Rc<RefCell<Option<Recording>>> = Rc::new(RefCell::new(None));
    // The active recording's Pipeline, kept so the force-stop button
    // can signal it while stop() is draining in a background thread.
    // Arc<Mutex> (not Rc<RefCell>) because the stop-completion callback
    // runs via invoke_from_event_loop which requires Send.
    let force_stop_pipeline: Arc<Mutex<Option<Arc<Pipeline>>>> = Arc::new(Mutex::new(None));
    // Dir of the recording currently open in the replay view, so the
    // delete handler can wipe it without re-resolving from the index.
    let replay_target: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));

    // Captured by value so the handlers stay `FnMut` and can be invoked
    // for each new Recording without borrowing `config`.
    let cfg_store_root = config.store_root.clone();
    let cfg_mic_source = config.mic_source.clone();
    let cfg_monitor_source = config.monitor_source.clone();
    let cfg_for_params = config.clone();
    let cfg_whisper_url = config.whisper_url.clone();
    let cfg_whisper_timeout = Duration::from_millis(config.whisper_timeout_ms);
    let cfg_translate_backends = config.translate.backends.clone();
    let cfg_translate_request_timeout = Duration::from_millis(config.translate.request_timeout_ms);
    let cfg_translate_reprobe_interval =
        Duration::from_millis(config.translate.reprobe_interval_ms);
    let cfg_target_language = config.target_language.clone();

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
                w.set_wizard_error("".into());
                w.set_wizard_open(true);
            }
        });
    }

    {
        let recording_slot = recording.clone();
        let window_weak = window.as_weak();
        let cfg_store_root = cfg_store_root.clone();
        let cfg_for_params = cfg_for_params.clone();
        let cfg_whisper_url = cfg_whisper_url.clone();
        let cfg_translate_backends = cfg_translate_backends.clone();
        let cfg_target_language = cfg_target_language.clone();
        window.on_begin_clicked(move || {
            let w = match window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };

            // Reentrancy guard: a second begin while a Recording is
            // active (or still starting) is refused outright.
            if recording_slot.borrow().is_some() {
                return;
            }

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
                    w.set_wizard_error("select a microphone source".into());
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
                    w.set_wizard_error("select a monitor source".into());
                    w.set_status_text("select a monitor source".into());
                    return;
                }
            };
            w.set_wizard_error("".into());
            // Live rows accumulate incrementally from here; drop whatever
            // a replay or a previous Recording left in the model.
            transcript_model().set_vec(Vec::new());

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

            let params = RecordingParams::from_config(
                &cfg_for_params,
                store_dir,
                title,
                mic_source,
                monitor_source,
            );

            let transcriber: Arc<dyn Transcriber> =
                match WhisperClient::new(&cfg_whisper_url, cfg_whisper_timeout) {
                    Ok(client) => Arc::new(client),
                    Err(e) => {
                        w.set_status_text(format!("start failed: {e}").into());
                        return;
                    }
                };
            let translate_backends: Vec<Arc<dyn Translator>> = cfg_translate_backends
                .iter()
                .filter_map(|backend: &TranslateBackendConfig| {
                    match OpenAITranslator::new(
                        &backend.base_url,
                        &backend.model,
                        &cfg_target_language,
                        backend.api_key.clone(),
                        cfg_translate_request_timeout,
                    ) {
                        Ok(translator) => Some(Arc::new(translator) as Arc<dyn Translator>),
                        Err(e) => {
                            w.set_status_text(format!("start failed: {e}").into());
                            None
                        }
                    }
                })
                .collect();
            if translate_backends.len() != cfg_translate_backends.len() {
                return;
            }
            let translator: Arc<dyn Translator> = Arc::new(FallbackTranslator::new(
                translate_backends,
                cfg_translate_reprobe_interval,
            ));

            let on_transcript_update =
                make_transcript_updater(window_weak.clone(), cfg_target_language.clone());

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
        let force_stop_pipeline_slot = force_stop_pipeline.clone();
        let window_weak = window.as_weak();
        let cfg_store_root = cfg_store_root.clone();
        window.on_stop_clicked(move || {
            let Some(recording) = recording_slot.borrow_mut().take() else {
                return;
            };
            // Keep a handle to the Pipeline so the force-stop button
            // can signal it while stop() drains in the background.
            let pipeline = recording.pipeline().clone();
            *force_stop_pipeline_slot
                .lock()
                .expect("force-stop slot mutex poisoned") = Some(pipeline);

            // stop() joins both reader threads and every outstanding
            // submission thread, so it can block for the tail of an
            // in-flight transcription. Flip the controls before that
            // drain rather than after it, or the button stays lit for
            // as long as the last transcription takes. `stopping` keeps
            // the transcript on screen meanwhile, since late Segments
            // still arrive during the drain.
            if let Some(w) = window_weak.upgrade() {
                w.set_recording_active(false);
                w.set_stopping(true);
            }

            let window_weak = window_weak.clone();
            let cfg_store_root = cfg_store_root.clone();
            let force_stop_pipeline_slot = force_stop_pipeline_slot.clone();
            std::thread::spawn(move || {
                if let Err(e) = recording.stop() {
                    eprintln!("recording stop failed: {e}");
                }
                // Refresh the history list in the same callback so the
                // recording that just stopped lands at the top of it.
                let window_weak = window_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(w) = window_weak.upgrade() {
                        w.set_stopping(false);
                    }
                    // The Pipeline is no longer draining, so drop the
                    // force-stop handle.
                    *force_stop_pipeline_slot
                        .lock()
                        .expect("force-stop slot mutex poisoned") = None;
                    refresh_history_rows(&cfg_store_root, window_weak);
                });
            });
        });
    }

    {
        let force_stop_pipeline_slot = force_stop_pipeline.clone();
        window.on_force_stop_clicked(move || {
            if let Some(p) = force_stop_pipeline_slot
                .lock()
                .expect("force-stop slot mutex poisoned")
                .as_ref()
            {
                p.signal_force_stop();
            }
        });
    }

    {
        let replay_target = replay_target.clone();
        let window_weak = window.as_weak();
        let cfg_store_root = cfg_store_root.clone();
        let cfg_target_language = cfg_target_language.clone();
        window.on_history_clicked(move |index| {
            let w = match window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            // Resolve the clicked row's directory from the current
            // listing; the index is only valid against a fresh read
            // since the list could have changed between renders.
            let entries = match history::list_recordings(&cfg_store_root) {
                Ok(e) => e,
                Err(e) => {
                    w.set_status_text(format!("history list failed: {e}").into());
                    return;
                }
            };
            let entry = match usize::try_from(index).ok().and_then(|i| entries.get(i)) {
                Some(e) => e,
                None => return,
            };
            let dir = entry.dir.clone();
            let segments = match history::read_transcript(&dir) {
                Ok(s) => s,
                Err(e) => {
                    w.set_status_text(format!("read transcript failed: {e}").into());
                    return;
                }
            };
            // Same row mapping as make_transcript_updater: replay
            // reuses build_rows so degraded/error and same-language
            // marks match the live view. The Recording's own target
            // language decides which of its Segments were
            // same-language, not whatever is configured now.
            let target_language = entry
                .target_language
                .clone()
                .unwrap_or_else(|| cfg_target_language.clone());
            let rows = to_ui_rows(&segments, &target_language);
            transcript_model().set_vec(rows);
            w.set_target_language(target_language.as_str().into());
            w.set_replay_title(entry.title.clone().into());
            w.set_replay_open(true);
            *replay_target.borrow_mut() = Some(dir);
        });
    }

    {
        let replay_target = replay_target.clone();
        let window_weak = window.as_weak();
        let cfg_store_root = cfg_store_root.clone();
        let cfg_target_language = cfg_target_language.clone();
        window.on_replay_back_clicked(move || {
            if let Some(w) = window_weak.upgrade() {
                w.set_replay_open(false);
                w.set_replay_title("".into());
                transcript_model().set_vec(Vec::new());
                // A replayed Recording may have used a different target
                // language; put the configured one back.
                w.set_target_language(cfg_target_language.as_str().into());
            }
            *replay_target.borrow_mut() = None;
            refresh_history_rows(&cfg_store_root, window_weak.clone());
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_delete_clicked(move || {
            if let Some(w) = window_weak.upgrade() {
                w.set_delete_target_name(w.get_replay_title());
                w.set_delete_confirmation_open(true);
            }
        });
    }

    {
        let replay_target = replay_target.clone();
        let window_weak = window.as_weak();
        let cfg_store_root = cfg_store_root.clone();
        window.on_confirm_delete_clicked(move || {
            let w = match window_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let dir = replay_target.borrow_mut().take();
            if let Some(dir) = dir {
                if let Err(e) = history::delete_recording(&dir) {
                    w.set_status_text(format!("delete failed: {e}").into());
                }
            }
            w.set_replay_open(false);
            w.set_delete_confirmation_open(false);
            w.set_replay_title("".into());
            transcript_model().set_vec(Vec::new());
            refresh_history_rows(&cfg_store_root, window_weak.clone());
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_cancel_delete_clicked(move || {
            if let Some(w) = window_weak.upgrade() {
                w.set_delete_confirmation_open(false);
            }
        });
    }

    window.set_target_language(cfg_target_language.as_str().into());

    // Seed the list so it shows on startup.
    refresh_history_rows(&cfg_store_root, window.as_weak());

    window.run()
}
