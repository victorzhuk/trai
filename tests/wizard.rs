use std::io::Cursor;
use std::sync::Arc;

use trai::capture::source_list::{parse_listing, resolve_defaults};
use trai::domain::SpeakerTag;
use trai::recording::{Recording, RecordingParams};
use trai::segmenter::{SegmentEvent, Segmenter, SpeechSpan, FRAME_LEN, SAMPLE_RATE_HZ};
use trai::store;
use trai::transcriber::{FakeTranscriber, Transcription};
use trai::translator::FakeTranslator;

const VAD_THRESHOLD: f32 = 0.5;
const SILENCE_HOLD_MS: u64 = 200;
const DURATION_CAP_MS: u64 = 20_000;

// Fixture listing mirroring `pactl list sources` shape: one input and
// one monitor, the same names the wizard will resolve as defaults.
const LISTING: &str = "\
Source #42
\tState: SUSPENDED
\tName: alsa_input.usb-mic.mono
\tDescription: USB Microphone Mono
\tDriver: PipeWire

Source #62
\tState: SUSPENDED
\tName: alsa_output.stereo.monitor
\tDescription: Monitor of Stereo Output
\tDriver: PipeWire
";

fn synthetic_speech_frame(len: usize, phase_start: usize, formants: &[f32]) -> Vec<i16> {
    (phase_start..phase_start + len)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE_HZ as f32;
            let mut sample = 0.0f32;
            for &f in formants {
                sample += (2.0 * std::f32::consts::PI * f * t).sin();
            }
            (sample / formants.len() as f32 * 22000.0) as i16
        })
        .collect()
}

fn build_stream_samples(formants: &[f32]) -> Vec<i16> {
    let lead_silence_len = FRAME_LEN * 10;
    let speech_len = FRAME_LEN * 20;
    let trail_silence_len = FRAME_LEN * 40;

    let mut samples = vec![0i16; lead_silence_len];
    samples.extend(synthetic_speech_frame(
        speech_len,
        lead_silence_len,
        formants,
    ));
    samples.extend(vec![0i16; trail_silence_len]);
    samples
}

fn pcm_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn expected_span(samples: &[i16]) -> SpeechSpan {
    let mut segmenter = Segmenter::new(
        VAD_THRESHOLD,
        SILENCE_HOLD_MS,
        DURATION_CAP_MS,
        DURATION_CAP_MS,
    );
    let mut spans: Vec<SpeechSpan> = segmenter
        .push_samples(samples)
        .into_iter()
        .filter_map(|event| match event {
            SegmentEvent::Closed(span) => Some(span),
            _ => None,
        })
        .collect();
    if let Some(span) = segmenter.finish() {
        spans.push(span);
    }
    assert_eq!(
        spans.len(),
        1,
        "expected exactly one span, got {}",
        spans.len()
    );
    spans[0]
}

#[test]
fn wizard_resolution_path_starts_recording_and_persists_meta_and_segments() {
    let sources = parse_listing(LISTING);
    let (mic_idx, monitor_idx) = resolve_defaults(
        &sources,
        "alsa_input.usb-mic.mono",
        "alsa_output.stereo.monitor",
    );
    let mic_idx = mic_idx.expect("mic default must resolve to an input");
    let monitor_idx = monitor_idx.expect("monitor default must resolve to a monitor");

    let mic_source = sources[mic_idx].name.clone();
    let monitor_source = sources[monitor_idx].name.clone();
    assert!(mic_source.starts_with("alsa_input."));
    assert!(monitor_source.ends_with(".monitor"));

    let mic_formants = [180.0, 420.0, 900.0, 1800.0, 2600.0];
    let monitor_formants = [220.0, 500.0, 1100.0, 2000.0, 3000.0];
    let mic_samples = build_stream_samples(&mic_formants);
    let monitor_samples = build_stream_samples(&monitor_formants);

    let mic_span = expected_span(&mic_samples);
    let monitor_span = expected_span(&monitor_samples);
    let mic_segment_samples =
        mic_samples[mic_span.start_sample as usize..mic_span.end_sample as usize].to_vec();
    let monitor_segment_samples = monitor_samples
        [monitor_span.start_sample as usize..monitor_span.end_sample as usize]
        .to_vec();

    let fake = Arc::new(FakeTranscriber::new());
    let mic_call = fake.expect_call(mic_segment_samples);
    let monitor_call = fake.expect_call(monitor_segment_samples);
    let translator = Arc::new(FakeTranslator::new());

    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("session");

    let params = RecordingParams {
        store_dir: store_dir.clone(),
        title: "Wizard Standup".to_string(),
        mic_source,
        monitor_source,
        mic_language: None,
        monitor_language: None,
        target_language: "en".to_string(),
        vad_threshold: VAD_THRESHOLD,
        silence_hold_ms: SILENCE_HOLD_MS,
        duration_cap_ms: DURATION_CAP_MS,
        live_chunk_ms: DURATION_CAP_MS,
        confidence_floor: 0.0,
    };

    let recording = Recording::start_with_sources(
        Cursor::new(pcm_bytes(&mic_samples)),
        Cursor::new(pcm_bytes(&monitor_samples)),
        params,
        fake.clone(),
        translator,
        |_segments| {},
    )
    .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(200));

    mic_call.respond(Transcription {
        text: "mic said".to_string(),
        mean_confidence: 0.9,
        language: None,
    });
    monitor_call.respond(Transcription {
        text: "monitor said".to_string(),
        mean_confidence: 0.85,
        language: None,
    });

    recording.stop().unwrap();

    let segments = store::read_all(&store_dir.join("segments.jsonl")).unwrap();
    assert_eq!(segments.len(), 2, "both segments must be persisted");
    assert!(
        segments.windows(2).all(|w| w[0].start_ms <= w[1].start_ms),
        "segments must be ordered by start_ms"
    );
    assert!(segments
        .iter()
        .any(|s| s.speaker_tag == SpeakerTag::Me && s.text == "mic said"));
    assert!(segments
        .iter()
        .any(|s| s.speaker_tag == SpeakerTag::Them && s.text == "monitor said"));

    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store_dir.join("meta.json")).unwrap())
            .unwrap();
    assert_eq!(meta["title"], "Wizard Standup");
    assert_eq!(meta["mic_source"], "alsa_input.usb-mic.mono");
    assert_eq!(meta["monitor_source"], "alsa_output.stereo.monitor");
}
