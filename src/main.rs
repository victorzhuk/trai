use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use slint::{ModelRc, VecModel};

use trai::config::Config;
use trai::domain::{Segment, SpeakerTag};
use trai::recording::{Recording, RecordingParams};
use trai::transcriber::{Transcriber, WhisperClient};

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

fn run(config: Config) -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;

    let recording: Rc<RefCell<Option<Recording>>> = Rc::new(RefCell::new(None));

    // Captured by value so the start handler stays `FnMut` and can be
    // invoked for each new Recording without borrowing `config`.
    let cfg_store_root = config.store_root.clone();
    let cfg_mic_source = config.mic_source.clone();
    let cfg_monitor_source = config.monitor_source.clone();
    let cfg_mic_language = config.mic_language.clone();
    let cfg_monitor_language = config.monitor_language.clone();
    let cfg_whisper_url = config.whisper_url.clone();
    let cfg_vad_threshold = config.vad_threshold;
    let cfg_silence_hold_ms = config.silence_hold_ms;
    let cfg_duration_cap_ms = config.duration_cap_ms;
    let cfg_confidence_floor = config.confidence_floor;

    {
        let recording_slot = recording.clone();
        let window_weak = window.as_weak();
        window.on_start_clicked(move || {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let store_dir = cfg_store_root.join(secs.to_string());
            let params = RecordingParams {
                store_dir,
                mic_source: cfg_mic_source.clone(),
                monitor_source: cfg_monitor_source.clone(),
                mic_language: cfg_mic_language.clone(),
                monitor_language: cfg_monitor_language.clone(),
                vad_threshold: cfg_vad_threshold,
                silence_hold_ms: cfg_silence_hold_ms,
                duration_cap_ms: cfg_duration_cap_ms,
                confidence_floor: cfg_confidence_floor,
            };
            let transcriber: Arc<dyn Transcriber> = Arc::new(WhisperClient::new(&cfg_whisper_url));

            // Marshals each live-view snapshot onto the UI thread.
            // Captures only the Send-safe weak handle; the row model
            // is rebuilt off-thread and handed to invoke_from_event_loop.
            let on_update_weak = window_weak.clone();
            let on_transcript_update = move |segments: Vec<Segment>| {
                let rows: Vec<TranscriptRow> = segments
                    .iter()
                    .map(|s| TranscriptRow {
                        speaker: speaker_label(s.speaker_tag).into(),
                        timestamp: format_timestamp(s.start_ms).into(),
                        text: s.text.clone().into(),
                    })
                    .collect();
                let on_update_weak = on_update_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(w) = on_update_weak.upgrade() {
                        w.set_transcript_rows(ModelRc::new(VecModel::from(rows)));
                    }
                });
            };

            match Recording::start(params, transcriber, on_transcript_update) {
                Ok(recording) => {
                    *recording_slot.borrow_mut() = Some(recording);
                    if let Some(w) = window_weak.upgrade() {
                        w.set_recording_active(true);
                        w.set_status_text("".into());
                    }
                }
                Err(e) => {
                    if let Some(w) = window_weak.upgrade() {
                        w.set_status_text(format!("start failed: {e}").into());
                    }
                }
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

fn speaker_label(tag: SpeakerTag) -> &'static str {
    match tag {
        SpeakerTag::Me => "me",
        SpeakerTag::Them => "them",
    }
}

fn format_timestamp(start_ms: u64) -> String {
    let total_seconds = start_ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}
