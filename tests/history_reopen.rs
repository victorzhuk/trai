use std::io::Cursor;
use std::sync::Arc;

use trai::domain::SpeakerTag;
use trai::history;
use trai::recording::{Recording, RecordingParams};
use trai::segmenter::{SegmentEvent, Segmenter, SpeechSpan, FRAME_LEN, SAMPLE_RATE_HZ};
use trai::transcriber::{FakeTranscriber, Transcription};
use trai::translator::FakeTranslator;

const VAD_THRESHOLD: f32 = 0.5;
const SILENCE_HOLD_MS: u64 = 200;
const DURATION_CAP_MS: u64 = 20_000;

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
    let mut segmenter = Segmenter::new(VAD_THRESHOLD, SILENCE_HOLD_MS, DURATION_CAP_MS);
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
fn record_stop_reopen_transcript_matches_captured_lines() {
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

    let root = tempfile::tempdir().unwrap();
    let store_dir = root.path().join("session");

    let params = RecordingParams {
        store_dir: store_dir.clone(),
        title: "Reopen Me".to_string(),
        mic_source: "unused".to_string(),
        monitor_source: "unused".to_string(),
        mic_language: Some("en".to_string()),
        monitor_language: None,
        target_language: "en".to_string(),
        vad_threshold: VAD_THRESHOLD,
        silence_hold_ms: SILENCE_HOLD_MS,
        duration_cap_ms: DURATION_CAP_MS,
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
        text: "mic said something".to_string(),
        mean_confidence: 0.9,
    });
    monitor_call.respond(Transcription {
        text: "monitor said something".to_string(),
        mean_confidence: 0.85,
    });

    recording.stop().unwrap();

    let segments = history::read_transcript(&store_dir).unwrap();
    assert_eq!(segments.len(), 2, "reopened transcript has both lines");
    assert!(
        segments.windows(2).all(|w| w[0].start_ms <= w[1].start_ms),
        "reopened transcript is ordered by start_ms"
    );

    let mic_segment = segments
        .iter()
        .find(|s| s.speaker_tag == SpeakerTag::Me)
        .expect("mic segment present and tagged Me");
    assert_eq!(mic_segment.text, "mic said something");

    let monitor_segment = segments
        .iter()
        .find(|s| s.speaker_tag == SpeakerTag::Them)
        .expect("monitor segment present and tagged Them");
    assert_eq!(monitor_segment.text, "monitor said something");

    let entries = history::list_recordings(root.path()).unwrap();
    assert_eq!(entries.len(), 1, "recording is listable right after stop");
    assert_eq!(entries[0].title, "Reopen Me");
    assert_eq!(entries[0].line_count, 2);
    assert!(
        entries[0].duration_secs.is_some(),
        "listing carries duration_secs after stop"
    );
}
