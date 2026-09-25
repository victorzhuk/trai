use std::io::Cursor;
use std::sync::mpsc;
use std::sync::Arc;

use trai::domain::SpeakerTag;
use trai::recording::{Recording, RecordingParams};
use trai::segmenter::{SegmentEvent, Segmenter, SpeechSpan, FRAME_LEN, SAMPLE_RATE_HZ};
use trai::store;
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

// Poll instead of sleeping a fixed duration: an in-memory Cursor is
// drained as fast as the scheduler allows, so a fixed margin either
// wastes time or expires early under CI load. The deadline is generous
// because the 75s-audio drain test churns real CPU before the
// condition can become true.
fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the expected condition"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn expected_spans(samples: &[i16]) -> Vec<SpeechSpan> {
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
    spans
}

// Runs the same Segmenter algorithm the Recording under test will run,
// against the full known PCM, to learn the exact sample range it will
// carve out as a Segment before we register that range with the fake.
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
fn recording_lifecycle_transcribes_both_streams_and_finalizes_wav_and_transcript() {
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
        title: "test".to_string(),
        mic_source: "unused".to_string(),
        monitor_source: "unused".to_string(),
        mic_language: Some("en".to_string()),
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

    // Both in-memory Cursors get drained in microseconds with no
    // real-time pacing (unlike a live pw-record stream), so wait until
    // both reader threads have reached their blocked transcribe() call
    // before releasing either response. Without this, one stream's
    // segment could finish and flush before the other stream's reader
    // thread has even submitted its own earlier segment, which this
    // test isn't set up to arbitrate.
    wait_until(|| fake.pending_count() == 0);

    mic_call.respond(Transcription {
        text: "mic said something".to_string(),
        mean_confidence: 0.9,
        language: None,
    });
    monitor_call.respond(Transcription {
        text: "monitor said something".to_string(),
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

    let mic_segment = segments
        .iter()
        .find(|s| s.speaker_tag == SpeakerTag::Me)
        .expect("mic segment must be present and tagged Me");
    assert_eq!(mic_segment.text, "mic said something");

    let monitor_segment = segments
        .iter()
        .find(|s| s.speaker_tag == SpeakerTag::Them)
        .expect("monitor segment must be present and tagged Them");
    assert_eq!(monitor_segment.text, "monitor said something");

    let mic_wav_path = store_dir.join("mic.wav");
    let monitor_wav_path = store_dir.join("monitor.wav");
    assert!(mic_wav_path.exists());
    assert!(monitor_wav_path.exists());

    let mic_reader = hound::WavReader::open(&mic_wav_path).unwrap();
    assert_eq!(mic_reader.duration() as usize, mic_samples.len());

    let monitor_reader = hound::WavReader::open(&monitor_wav_path).unwrap();
    assert_eq!(monitor_reader.duration() as usize, monitor_samples.len());
}

// A source the test can feed on its own schedule: `read()` blocks on
// the channel whenever its internal buffer is drained, standing in
// for a live stream whose next bytes (e.g. the silence that would
// close a still-open span) simply haven't arrived yet. Dropping the
// sender closes the channel, which surfaces as ordinary EOF.
struct ChunkedReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl ChunkedReader {
    fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
        }
    }
}

impl std::io::Read for ChunkedReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[test]
fn an_open_earlier_span_blocks_flush_of_a_later_span_that_transcribes_first() {
    let mic_formants = [180.0, 420.0, 900.0, 1800.0, 2600.0];
    let monitor_formants = [220.0, 500.0, 1100.0, 2000.0, 3000.0];

    // mic opens early and, unlike every other stream in this file, is
    // fed through a channel instead of a finite Cursor: we send its
    // lead-silence-plus-speech onset and then deliberately withhold
    // the trailing silence that would close it, so its span stays
    // open on purpose while the rest of this scenario plays out.
    let mic_lead_silence_len = FRAME_LEN * 10;
    let mic_speech_len = FRAME_LEN * 20;
    let mic_trail_silence_len = FRAME_LEN * 40;

    let mut mic_onset_samples = vec![0i16; mic_lead_silence_len];
    mic_onset_samples.extend(synthetic_speech_frame(
        mic_speech_len,
        mic_lead_silence_len,
        &mic_formants,
    ));
    let mic_trail_samples = vec![0i16; mic_trail_silence_len];

    let mut mic_full_samples = mic_onset_samples.clone();
    mic_full_samples.extend(mic_trail_samples.iter().copied());

    // monitor opens later (bigger lead silence -> bigger start_ms)
    // but is a plain, finite Cursor that closes and submits within
    // microseconds of the recording starting.
    let monitor_lead_silence_len = FRAME_LEN * 40;
    let monitor_speech_len = FRAME_LEN * 20;
    let monitor_trail_silence_len = FRAME_LEN * 40;

    let mut monitor_samples = vec![0i16; monitor_lead_silence_len];
    monitor_samples.extend(synthetic_speech_frame(
        monitor_speech_len,
        monitor_lead_silence_len,
        &monitor_formants,
    ));
    monitor_samples.extend(vec![0i16; monitor_trail_silence_len]);

    let mic_span = expected_span(&mic_full_samples);
    let monitor_span = expected_span(&monitor_samples);
    assert!(
        mic_span.start_sample < monitor_span.start_sample,
        "mic must open before monitor for this scenario to be meaningful"
    );

    let mic_segment_samples =
        mic_full_samples[mic_span.start_sample as usize..mic_span.end_sample as usize].to_vec();
    let monitor_segment_samples = monitor_samples
        [monitor_span.start_sample as usize..monitor_span.end_sample as usize]
        .to_vec();

    let fake = Arc::new(FakeTranscriber::new());
    let mic_call = fake.expect_call(mic_segment_samples);
    let monitor_call = fake.expect_call(monitor_segment_samples);
    let translator = Arc::new(FakeTranslator::new());

    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("session");
    let segments_path = store_dir.join("segments.jsonl");

    let params = RecordingParams {
        store_dir: store_dir.clone(),
        title: "test".to_string(),
        mic_source: "unused".to_string(),
        monitor_source: "unused".to_string(),
        mic_language: None,
        monitor_language: None,
        target_language: "en".to_string(),
        vad_threshold: VAD_THRESHOLD,
        silence_hold_ms: SILENCE_HOLD_MS,
        duration_cap_ms: DURATION_CAP_MS,
        live_chunk_ms: DURATION_CAP_MS,
        confidence_floor: 0.0,
    };

    let (mic_tx, mic_rx) = mpsc::channel::<Vec<u8>>();
    let mic_reader = ChunkedReader::new(mic_rx);

    let recording = Recording::start_with_sources(
        mic_reader,
        Cursor::new(pcm_bytes(&monitor_samples)),
        params,
        fake.clone(),
        translator,
        |_segments| {},
    )
    .unwrap();

    mic_tx.send(pcm_bytes(&mic_onset_samples)).unwrap();
    // Generous margin for the mic reader thread to process the onset
    // chunk and call pipeline.begin() before we touch monitor's
    // response; without it we couldn't guarantee mic's span is
    // registered as open by the time monitor's segment tries to flush.
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Monitor's short segment (later start_ms) was a plain, finite
    // Cursor, so it has already closed and submitted by now. Release
    // its transcription while mic's span is still open.
    monitor_call.respond(Transcription {
        text: "monitor said something".to_string(),
        mean_confidence: 0.85,
        language: None,
    });

    // Give the monitor submission thread time to retire from
    // in_flight, insert its segment, and attempt a flush. This is
    // exactly the window in which the bug this test targets would
    // write monitor's segment to disk ahead of mic's, before mic has
    // even closed: if an open span isn't registered until it closes,
    // nothing here is holding the watermark back.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let mid_flight_segments = store::read_all(&segments_path).unwrap();
    assert!(
        mid_flight_segments.is_empty(),
        "monitor's later segment must not be flushed while mic's earlier-starting span is still open, got {mid_flight_segments:?}"
    );

    // Now let mic's monologue actually end.
    mic_tx.send(pcm_bytes(&mic_trail_samples)).unwrap();
    drop(mic_tx);

    mic_call.respond(Transcription {
        text: "mic said something".to_string(),
        mean_confidence: 0.9,
        language: None,
    });

    recording.stop().unwrap();

    let segments = store::read_all(&segments_path).unwrap();
    assert_eq!(segments.len(), 2, "both segments must be persisted");
    assert!(
        segments.windows(2).all(|w| w[0].start_ms <= w[1].start_ms),
        "segments must be ordered by start_ms on disk, not by transcription completion order"
    );
    assert_eq!(
        segments[0].speaker_tag,
        SpeakerTag::Me,
        "mic's earlier-starting segment must land first on disk even though monitor transcribed first"
    );
    assert_eq!(segments[0].text, "mic said something");
    assert_eq!(segments[1].speaker_tag, SpeakerTag::Them);
    assert_eq!(segments[1].text, "monitor said something");
}

#[test]
fn meta_json_is_written_at_start_with_title_and_source_names() {
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
        title: "Standup".to_string(),
        mic_source: "alsa_input.usb-mic".to_string(),
        monitor_source: "alsa_output.stereo.monitor".to_string(),
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

    // meta.json is written at start, before stop, so a crash still
    // leaves it on disk. Verify it is already present mid-recording.
    let mid_meta = std::fs::read_to_string(store_dir.join("meta.json")).unwrap();
    assert!(mid_meta.contains("Standup"));
    assert!(mid_meta.contains("alsa_input.usb-mic"));
    assert!(mid_meta.contains("alsa_output.stereo.monitor"));

    mic_call.respond(Transcription {
        text: "mic".to_string(),
        mean_confidence: 0.9,
        language: None,
    });
    monitor_call.respond(Transcription {
        text: "monitor".to_string(),
        mean_confidence: 0.85,
        language: None,
    });

    recording.stop().unwrap();

    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store_dir.join("meta.json")).unwrap())
            .unwrap();
    assert_eq!(meta["title"], "Standup");
    assert_eq!(meta["mic_source"], "alsa_input.usb-mic");
    assert_eq!(meta["monitor_source"], "alsa_output.stereo.monitor");
    assert!(
        meta["start_time"].as_u64().is_some(),
        "stop-written meta must carry start_time"
    );
    assert_eq!(meta["target_language"], "en");
    assert!(
        meta["duration_secs"].as_u64().is_some(),
        "stop-written meta must carry duration_secs"
    );
}

#[test]
fn recording_drains_dead_audio_without_corrupting_later_segments() {
    let mic_formants = [180.0, 420.0, 900.0, 1800.0, 2600.0];
    let monitor_formants = [220.0, 500.0, 1100.0, 2000.0, 3000.0];

    // 75s of continuous mic speech: three 20s duration-cap force-closes
    // plus a trailing span at stream end. The dead-prefix drain fires at
    // 60s, so the spans after it slice the reader buffer through the
    // drained offset. The fake matches expectations by exact samples, so
    // a wrong offset fails the transcription instead of passing silently.
    let mic_samples = synthetic_speech_frame(SAMPLE_RATE_HZ as usize * 75, 0, &mic_formants);
    let monitor_samples = build_stream_samples(&monitor_formants);

    let mic_spans = expected_spans(&mic_samples);
    assert!(
        mic_spans.len() >= 3,
        "75s at a 20s cap must produce several spans, got {}",
        mic_spans.len()
    );
    let monitor_span = expected_span(&monitor_samples);

    let fake = Arc::new(FakeTranscriber::new());
    let mic_calls: Vec<_> = mic_spans
        .iter()
        .map(|span| {
            fake.expect_call(
                mic_samples[span.start_sample as usize..span.end_sample as usize].to_vec(),
            )
        })
        .collect();
    let monitor_call = fake.expect_call(
        monitor_samples[monitor_span.start_sample as usize..monitor_span.end_sample as usize]
            .to_vec(),
    );
    let translator = Arc::new(FakeTranslator::new());

    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("session");

    let params = RecordingParams {
        store_dir: store_dir.clone(),
        title: "Long meeting".to_string(),
        mic_source: "alsa_input.usb-mic".to_string(),
        monitor_source: "alsa_output.stereo.monitor".to_string(),
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

    // More expectations than submission workers: the queued job's
    // transcribe call only starts once a worker frees, so responses
    // must flow while we wait. Each responder thread's rendezvous send
    // blocks until its matching transcribe call arrives.
    let responders: Vec<_> = mic_calls
        .into_iter()
        .enumerate()
        .map(|(i, call)| {
            std::thread::spawn(move || {
                call.respond(Transcription {
                    text: format!("mic span {i}"),
                    mean_confidence: 0.9,
                    language: None,
                });
            })
        })
        .chain(std::iter::once(std::thread::spawn(move || {
            monitor_call.respond(Transcription {
                text: "monitor said something".to_string(),
                mean_confidence: 0.85,
                language: None,
            });
        })))
        .collect();

    wait_until(|| fake.pending_count() == 0);
    for responder in responders {
        responder.join().unwrap();
    }

    recording.stop().unwrap();

    let segments = store::read_all(&store_dir.join("segments.jsonl")).unwrap();
    let mic_segments: Vec<_> = segments
        .iter()
        .filter(|s| s.speaker_tag == SpeakerTag::Me)
        .collect();
    assert_eq!(
        mic_segments.len(),
        mic_spans.len(),
        "every span past the drain must still transcribe and persist"
    );
    for (i, segment) in mic_segments.iter().enumerate() {
        assert_eq!(segment.text, format!("mic span {i}"));
        assert_eq!(segment.start_ms, segmenter_to_ms(mic_spans[i].start_sample));
    }
}

// A source that streams its bytes normally, then fails hard: stands in
// for pw-record dying mid-meeting with a span still open.
struct FailingReader {
    data: Vec<u8>,
    pos: usize,
}

impl std::io::Read for FailingReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos < self.data.len() {
            let n = out.len().min(self.data.len() - self.pos);
            out[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            return Ok(n);
        }
        Err(std::io::Error::other("mic pcm stream died"))
    }
}

// The mic's span is still open when its stream errors; the monitor's
// later-starting segment completed but was gated behind the orphan
// watermark. The reader's cancel_pending must retire the watermark and
// unblock the flush, stop() must surface the original reader error, the
// partial wav must still be finalized, and meta.json must still gain
// duration_secs — none of those may be skipped by the error path.
#[test]
fn reader_error_cancels_orphan_span_unblocks_flush_and_preserves_error() {
    let mic_formants = [180.0, 420.0, 900.0, 1800.0, 2600.0];
    let monitor_formants = [220.0, 500.0, 1100.0, 2000.0, 3000.0];

    let mic_lead_silence_len = FRAME_LEN * 10;
    let mic_speech_len = FRAME_LEN * 20;
    let mic_trail_silence_len = FRAME_LEN * 40;

    let mut mic_onset_samples = vec![0i16; mic_lead_silence_len];
    mic_onset_samples.extend(synthetic_speech_frame(
        mic_speech_len,
        mic_lead_silence_len,
        &mic_formants,
    ));

    let mut mic_full_samples = mic_onset_samples.clone();
    mic_full_samples.extend(vec![0i16; mic_trail_silence_len]);

    let mut monitor_samples = vec![0i16; FRAME_LEN * 40];
    monitor_samples.extend(synthetic_speech_frame(
        FRAME_LEN * 20,
        FRAME_LEN * 40,
        &monitor_formants,
    ));
    monitor_samples.extend(vec![0i16; FRAME_LEN * 40]);

    // Only the monitor segment ever completes; the mic's open span has
    // no expectation registered.
    let monitor_span = expected_span(&monitor_samples);
    let monitor_segment_samples = monitor_samples
        [monitor_span.start_sample as usize..monitor_span.end_sample as usize]
        .to_vec();

    let fake = Arc::new(FakeTranscriber::new());
    let monitor_call = fake.expect_call(monitor_segment_samples);
    let translator = Arc::new(FakeTranslator::new());

    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("session");
    let segments_path = store_dir.join("segments.jsonl");

    let params = RecordingParams {
        store_dir: store_dir.clone(),
        title: "test".to_string(),
        mic_source: "unused".to_string(),
        monitor_source: "unused".to_string(),
        mic_language: None,
        monitor_language: None,
        target_language: "en".to_string(),
        vad_threshold: VAD_THRESHOLD,
        silence_hold_ms: SILENCE_HOLD_MS,
        duration_cap_ms: DURATION_CAP_MS,
        live_chunk_ms: DURATION_CAP_MS,
        confidence_floor: 0.0,
    };

    let mic_reader = FailingReader {
        data: pcm_bytes(&mic_onset_samples),
        pos: 0,
    };

    let recording = Recording::start_with_sources(
        mic_reader,
        Cursor::new(pcm_bytes(&monitor_samples)),
        params,
        fake.clone(),
        translator,
        |_segments| {},
    )
    .unwrap();

    wait_until(|| fake.pending_count() == 0);
    monitor_call.respond(Transcription {
        text: "monitor said something".to_string(),
        mean_confidence: 0.85,
        language: None,
    });
    wait_until(|| !store::read_all(&segments_path).unwrap().is_empty());

    let stop_err = recording.stop().unwrap_err();
    assert!(
        stop_err.to_string().contains("mic pcm stream died"),
        "stop must surface the original reader error, got: {stop_err}"
    );

    let segments = store::read_all(&segments_path).unwrap();
    assert_eq!(
        segments.len(),
        1,
        "the completed monitor segment must be flushed once the orphan watermark is retired, got {segments:?}"
    );
    assert_eq!(segments[0].speaker_tag, SpeakerTag::Them);
    assert_eq!(segments[0].text, "monitor said something");

    let mic_wav = hound::WavReader::open(store_dir.join("mic.wav")).unwrap();
    assert_eq!(
        mic_wav.duration() as usize,
        mic_onset_samples.len(),
        "the partial mic wav must be finalized with the samples that arrived"
    );

    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store_dir.join("meta.json")).unwrap())
            .unwrap();
    assert!(
        meta["duration_secs"].as_u64().is_some(),
        "stop must write duration_secs into meta.json even when it returns an error"
    );
}

fn segmenter_to_ms(sample: u64) -> u64 {
    sample * 1000 / SAMPLE_RATE_HZ
}

fn bare_params(store_dir: std::path::PathBuf, title: &str) -> RecordingParams {
    RecordingParams {
        store_dir,
        title: title.to_string(),
        mic_source: "unused".to_string(),
        monitor_source: "unused".to_string(),
        mic_language: Some("en".to_string()),
        monitor_language: None,
        target_language: "en".to_string(),
        vad_threshold: VAD_THRESHOLD,
        silence_hold_ms: SILENCE_HOLD_MS,
        duration_cap_ms: DURATION_CAP_MS,
        live_chunk_ms: DURATION_CAP_MS,
        confidence_floor: 0.0,
    }
}

fn start_recording(params: RecordingParams) -> std::io::Result<Recording> {
    Recording::start_with_sources(
        Cursor::new(Vec::new()),
        Cursor::new(Vec::new()),
        params,
        Arc::new(FakeTranscriber::new()),
        Arc::new(FakeTranslator::new()),
        |_segments| {},
    )
}

// Two processes handed the same dir: the loser must fail without
// truncating the winner's meta.json or wav files.
#[test]
fn collided_store_dir_fails_without_truncating_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("session");
    std::fs::create_dir(&store_dir).unwrap();
    std::fs::write(store_dir.join("meta.json"), b"sentinel-meta").unwrap();
    std::fs::write(store_dir.join("mic.wav"), b"sentinel-wav").unwrap();

    let err = match start_recording(bare_params(store_dir.clone(), "test")) {
        Err(e) => e,
        Ok(_) => panic!("start must refuse an already-populated store dir"),
    };
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(
        std::fs::read(store_dir.join("meta.json")).unwrap(),
        b"sentinel-meta"
    );
    assert_eq!(
        std::fs::read(store_dir.join("mic.wav")).unwrap(),
        b"sentinel-wav"
    );
}

// The start-time meta write and the stop-time replacement both go
// through temp+rename: the final file must be complete JSON with the
// stop-only fields, no temp file may survive, and the owner-only mode
// must hold on the renamed file.
#[test]
fn stop_replaces_meta_with_duration_and_leaves_no_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("session");

    let recording = start_recording(bare_params(store_dir.clone(), "Replace Me")).unwrap();
    recording.stop().unwrap();

    use std::os::unix::fs::PermissionsExt;
    let meta_path = store_dir.join("meta.json");
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
    assert_eq!(meta["title"], "Replace Me");
    assert!(meta["duration_secs"].as_u64().is_some());
    assert_eq!(meta["segment_count"].as_u64(), Some(0));
    let mode = meta_path.metadata().unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);

    let leftovers: Vec<_> = std::fs::read_dir(&store_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n.to_string_lossy().contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "temp files survived: {leftovers:?}");
}

#[test]
fn reserve_store_dir_hands_out_distinct_dirs_under_one_root() {
    let root = tempfile::tempdir().unwrap();
    let first = trai::recording::reserve_store_dir(root.path()).unwrap();
    let second = trai::recording::reserve_store_dir(root.path()).unwrap();
    assert_ne!(first, second);
    assert!(first.is_dir() && second.is_dir());
    let dirs: Vec<_> = std::fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(dirs.len(), 2, "each reservation got its own dir: {dirs:?}");
}

#[test]
fn stop_reports_reader_failure_even_when_meta_write_also_fails() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("session");

    let recording = Recording::start_with_sources(
        FailingReader {
            data: Vec::new(),
            pos: 0,
        },
        Cursor::new(Vec::new()),
        bare_params(store_dir.clone(), "test"),
        Arc::new(FakeTranscriber::new()),
        Arc::new(FakeTranslator::new()),
        |_segments| {},
    )
    .unwrap();

    std::fs::remove_file(store_dir.join("meta.json")).unwrap();
    std::fs::create_dir(store_dir.join("meta.json")).unwrap();

    let err = recording.stop().expect_err("stop should fail");
    assert!(
        err.to_string().contains("mic pcm stream died"),
        "first stage error must win over the later meta write: {err}"
    );
}

#[test]
fn failed_start_rolls_back_its_files_and_keeps_user_files() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("session");
    std::fs::create_dir(&store_dir).unwrap();
    // A directory named segments.jsonl makes Store::open fail after the
    // recording's own files were created.
    std::fs::create_dir(store_dir.join("segments.jsonl")).unwrap();

    let err = match start_recording(bare_params(store_dir.clone(), "test")) {
        Ok(_) => panic!("start should fail on segments.jsonl"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), std::io::ErrorKind::IsADirectory);

    for name in ["meta.json", "mic.wav", "monitor.wav"] {
        assert!(
            !store_dir.join(name).exists(),
            "{name} survived a failed start"
        );
    }
    assert!(
        store_dir.join("segments.jsonl").is_dir(),
        "user file must survive a failed start"
    );
}

#[test]
fn failed_start_leaves_no_partially_created_files_behind() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("session");
    std::fs::create_dir(&store_dir).unwrap();
    std::fs::write(store_dir.join("mic.wav"), b"sentinel").unwrap();

    let err = start_recording(bare_params(store_dir.clone(), "test"));
    assert!(err.is_err());

    assert!(
        !store_dir.join("meta.json").exists(),
        "meta.json created before the collision must be rolled back"
    );
    assert_eq!(
        std::fs::read(store_dir.join("mic.wav")).unwrap(),
        b"sentinel",
        "the pre-existing file must not be touched"
    );
}

#[test]
fn reserve_store_dir_cleans_up_when_root_fsync_fails() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();

    // write+execute without read: candidate mkdir succeeds, opening the
    // root for fsync fails. Root cannot do this, so skip there.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o311)).unwrap();
    let can_still_create = {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(root.join("probe"))
            .is_ok()
    };
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    if can_still_create {
        return;
    }
    let _ = std::fs::remove_dir(root.join("probe"));

    assert!(trai::recording::reserve_store_dir(&root).is_err());
    let leftover: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e: std::io::Result<std::fs::DirEntry>| e.unwrap().file_name())
        .collect();
    assert!(
        leftover.is_empty(),
        "failed reservation left a dir behind: {leftover:?}"
    );
}
