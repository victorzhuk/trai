use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::capture::pw_record;
use crate::capture::tee_pcm_to_wav;
use crate::capture::wav_writer::WavWriter;
use crate::domain::SpeakerTag;
use crate::pipeline::{Pipeline, SegmentInput, TranscriptUpdate};
use crate::segmenter::{self, SegmentEvent, Segmenter, SpeechSpan};
use crate::store::Store;
use crate::transcriber::Transcriber;
use crate::translator::Translator;

pub struct RecordingParams {
    pub store_dir: PathBuf,
    pub title: String,
    pub mic_source: String,
    pub monitor_source: String,
    pub mic_language: Option<String>,
    pub monitor_language: Option<String>,
    pub target_language: String,
    pub vad_threshold: f32,
    pub silence_hold_ms: u64,
    pub duration_cap_ms: u64,
    pub live_chunk_ms: u64,
    pub confidence_floor: f32,
}

impl RecordingParams {
    /// Pipeline knobs come from configuration; title, store dir, and
    /// the two sources come from the wizard (the sources may override
    /// the configured defaults for this one Recording).
    pub fn from_config(
        config: &crate::config::Config,
        store_dir: PathBuf,
        title: String,
        mic_source: String,
        monitor_source: String,
    ) -> Self {
        Self {
            store_dir,
            title,
            mic_source,
            monitor_source,
            mic_language: config.mic_language.clone(),
            monitor_language: config.monitor_language.clone(),
            target_language: config.target_language.clone(),
            vad_threshold: config.vad_threshold,
            silence_hold_ms: config.silence_hold_ms,
            duration_cap_ms: config.duration_cap_ms,
            live_chunk_ms: config.live_chunk_ms,
            confidence_floor: config.confidence_floor,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RecordingMeta {
    title: String,
    mic_source: String,
    monitor_source: String,
    start_time: u64,
    target_language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_secs: Option<u64>,
    // Written at stop so history listing need not re-read the whole
    // transcript just to count rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    segment_count: Option<u64>,
}

// Transcripts hold meeting content: owner-only from creation.
fn write_meta(path: &Path, meta: &RecordingMeta) -> io::Result<()> {
    let json = serde_json::to_string_pretty(meta).map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| io::Error::new(e.kind(), format!("open {}: {e}", path.display())))?;
    file.write_all(json.as_bytes())
        .map_err(|e| io::Error::new(e.kind(), format!("write {}: {e}", path.display())))
}

// Segment submissions block on the transcription backend, so a small
// fixed worker set drains the queue: a slow or dead backend piles up
// queue entries (bounded by the configured whisper timeout) instead of
// one 8MB-stack thread per Segment.
const SUBMISSION_WORKERS: usize = 4;

// Multi-consumer queue: std mpsc Receiver is not Sync, and locking it
// would let only one worker wait on recv at a time.
struct SubmissionQueue {
    inner: Mutex<SubmissionQueueInner>,
    cvar: Condvar,
}

#[derive(Default)]
struct SubmissionQueueInner {
    queue: VecDeque<SegmentInput>,
    closed: bool,
}

impl SubmissionQueue {
    fn push(&self, input: SegmentInput) {
        self.inner
            .lock()
            .expect("submission queue mutex poisoned")
            .queue
            .push_back(input);
        self.cvar.notify_one();
    }

    // Buffered jobs still drain after close; None means closed AND empty.
    fn pop(&self) -> Option<SegmentInput> {
        let mut inner = self.inner.lock().expect("submission queue mutex poisoned");
        loop {
            if let Some(input) = inner.queue.pop_front() {
                return Some(input);
            }
            if inner.closed {
                return None;
            }
            inner = self
                .cvar
                .wait(inner)
                .expect("submission queue mutex poisoned");
        }
    }

    fn close(&self) {
        self.inner
            .lock()
            .expect("submission queue mutex poisoned")
            .closed = true;
        self.cvar.notify_all();
    }

    fn pending(&self) -> usize {
        self.inner
            .lock()
            .expect("submission queue mutex poisoned")
            .queue
            .len()
    }
}

/// Owns the two pw-record child processes feeding a Recording, so the
/// capture lifecycle (spawn both, kill both) lives at the process edge
/// and the library core works with plain readers.
struct Capture {
    mic_child: Child,
    monitor_child: Child,
}

impl Capture {
    fn spawn(
        mic_source: &str,
        monitor_source: &str,
    ) -> io::Result<(Self, std::process::ChildStdout, std::process::ChildStdout)> {
        let mut mic_child = pw_record::spawn(mic_source).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("spawn pw-record for mic source '{mic_source}': {e}"),
            )
        })?;
        let mic_stdout = mic_child
            .stdout
            .take()
            .expect("pw-record spawned with piped stdout");

        let mut monitor_child = match pw_record::spawn(monitor_source) {
            Ok(child) => child,
            Err(e) => {
                if let Err(stop_err) = pw_record::stop(&mut mic_child) {
                    crate::debug!(
                        "start: stopping mic pw-record after monitor spawn failed: {stop_err}"
                    );
                }
                return Err(io::Error::new(
                    e.kind(),
                    format!("spawn pw-record for monitor source '{monitor_source}': {e}"),
                ));
            }
        };
        let monitor_stdout = monitor_child
            .stdout
            .take()
            .expect("pw-record spawned with piped stdout");

        Ok((
            Self {
                mic_child,
                monitor_child,
            },
            mic_stdout,
            monitor_stdout,
        ))
    }

    fn stop(&mut self) -> io::Result<()> {
        pw_record::stop(&mut self.mic_child)?;
        pw_record::stop(&mut self.monitor_child)?;
        Ok(())
    }

    fn stop_quiet(&mut self) {
        if let Err(e) = self.stop() {
            crate::debug!("start: stopping pw-record children after failure: {e}");
        }
    }
}

pub struct Recording {
    capture: Option<Capture>,
    mic_reader: JoinHandle<io::Result<()>>,
    monitor_reader: JoinHandle<io::Result<()>>,
    submissions: Arc<SubmissionQueue>,
    submission_workers: Vec<JoinHandle<()>>,
    pipeline: Arc<Pipeline>,
    store_dir: PathBuf,
    start: Instant,
    meta: RecordingMeta,
}

impl Recording {
    pub fn start(
        params: RecordingParams,
        transcriber: Arc<dyn Transcriber>,
        translator: Arc<dyn Translator>,
        on_transcript_update: impl Fn(TranscriptUpdate) + Send + Sync + 'static,
    ) -> io::Result<Self> {
        crate::debug!(
            "recording: start mic={} monitor={} dir={} langs={:?}/{:?} vad={} hold={}ms cap={}ms chunk={}ms floor={}",
            params.mic_source,
            params.monitor_source,
            params.store_dir.display(),
            params.mic_language,
            params.monitor_language,
            params.vad_threshold,
            params.silence_hold_ms,
            params.duration_cap_ms,
            params.live_chunk_ms,
            params.confidence_floor,
        );

        let (mut capture, mic_stdout, monitor_stdout) =
            Capture::spawn(&params.mic_source, &params.monitor_source)?;

        let mut recording = match Self::start_with_sources(
            mic_stdout,
            monitor_stdout,
            params,
            transcriber,
            translator,
            on_transcript_update,
        ) {
            Ok(recording) => recording,
            Err(e) => {
                capture.stop_quiet();
                return Err(e);
            }
        };

        recording.capture = Some(capture);
        Ok(recording)
    }

    pub fn start_with_sources<R1, R2>(
        mic_source: R1,
        monitor_source: R2,
        params: RecordingParams,
        transcriber: Arc<dyn Transcriber>,
        translator: Arc<dyn Translator>,
        on_transcript_update: impl Fn(TranscriptUpdate) + Send + Sync + 'static,
    ) -> io::Result<Self>
    where
        R1: io::Read + Send + 'static,
        R2: io::Read + Send + 'static,
    {
        fs::create_dir_all(&params.store_dir).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("create store dir '{}': {e}", params.store_dir.display()),
            )
        })?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&params.store_dir, fs::Permissions::from_mode(0o700))?;

        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let start = Instant::now();

        // Written at start, not stop, so a mid-meeting crash still
        // leaves the recording self-describing on disk.
        let meta = RecordingMeta {
            title: params.title.clone(),
            mic_source: params.mic_source.clone(),
            monitor_source: params.monitor_source.clone(),
            start_time,
            target_language: params.target_language.clone(),
            duration_secs: None,
            segment_count: None,
        };
        write_meta(&params.store_dir.join("meta.json"), &meta)?;

        let store = Store::open(&params.store_dir.join("segments.jsonl"))?;
        let pipeline = Arc::new(Pipeline::new(
            transcriber,
            translator,
            store,
            params.confidence_floor,
            params.target_language.clone(),
        ));
        // Register the live-view callback before either reader thread
        // is spawned: a segment can arrive the instant a reader
        // starts, so the callback must already be in place or early
        // rows would silently never reach the UI.
        pipeline.set_on_update(on_transcript_update);

        let mic_wav = WavWriter::create(&params.store_dir.join("mic.wav"))
            .map_err(|e| io::Error::new(e.kind(), format!("create mic.wav: {e}")))?;
        let monitor_wav = WavWriter::create(&params.store_dir.join("monitor.wav"))
            .map_err(|e| io::Error::new(e.kind(), format!("create monitor.wav: {e}")))?;

        let submissions = Arc::new(SubmissionQueue {
            inner: Mutex::new(SubmissionQueueInner::default()),
            cvar: Condvar::new(),
        });
        let submission_workers = (0..SUBMISSION_WORKERS)
            .map(|_| {
                let pipeline = pipeline.clone();
                let submissions = submissions.clone();
                thread::spawn(move || {
                    while let Some(input) = submissions.pop() {
                        if let Err(e) = pipeline.finish_pending(input) {
                            crate::debug!("segment submission failed: {e}");
                        }
                    }
                })
            })
            .collect();

        let mic_reader = {
            let pipeline = pipeline.clone();
            let submissions = submissions.clone();
            let reader_params = ReaderParams {
                speaker_tag: SpeakerTag::Me,
                language: params.mic_language.clone(),
                vad_threshold: params.vad_threshold,
                silence_hold_ms: params.silence_hold_ms,
                duration_cap_ms: params.duration_cap_ms,
                live_chunk_ms: params.live_chunk_ms,
            };
            thread::spawn(move || {
                run_reader(mic_source, mic_wav, reader_params, pipeline, submissions)
            })
        };

        let monitor_reader = {
            let pipeline = pipeline.clone();
            let submissions = submissions.clone();
            let reader_params = ReaderParams {
                speaker_tag: SpeakerTag::Them,
                language: params.monitor_language.clone(),
                vad_threshold: params.vad_threshold,
                silence_hold_ms: params.silence_hold_ms,
                duration_cap_ms: params.duration_cap_ms,
                live_chunk_ms: params.live_chunk_ms,
            };
            thread::spawn(move || {
                run_reader(
                    monitor_source,
                    monitor_wav,
                    reader_params,
                    pipeline,
                    submissions,
                )
            })
        };

        Ok(Self {
            capture: None,
            mic_reader,
            monitor_reader,
            submissions,
            submission_workers,
            pipeline,
            store_dir: params.store_dir,
            start,
            meta,
        })
    }

    /// Returns the shared Pipeline so callers (e.g., a force-stop
    /// button) can signal it without owning the Recording.
    pub fn pipeline(&self) -> &Arc<Pipeline> {
        &self.pipeline
    }

    pub fn stop(mut self) -> io::Result<()> {
        // Each stage is timed separately: the UI stays in its "stopping"
        // state for the sum of these, so a slow stop needs to name which
        // stage is slow.
        let began = Instant::now();
        if let Some(mut capture) = self.capture.take() {
            capture.stop()?;
        }
        crate::debug!(
            "stop: capture killed in {:.2}s",
            began.elapsed().as_secs_f64()
        );

        let at_readers = Instant::now();
        let mic_result = self
            .mic_reader
            .join()
            .map_err(|_| io::Error::other("mic reader thread panicked"))?;
        let monitor_result = self
            .monitor_reader
            .join()
            .map_err(|_| io::Error::other("monitor reader thread panicked"))?;
        crate::debug!(
            "stop: readers joined in {:.2}s",
            at_readers.elapsed().as_secs_f64()
        );

        let at_submissions = Instant::now();
        let pending = self.submissions.pending();
        self.submissions.close();
        for handle in self.submission_workers {
            handle
                .join()
                .map_err(|_| io::Error::other("segment submission worker panicked"))?;
        }
        crate::debug!(
            "stop: {pending} queued submissions drained in {:.2}s",
            at_submissions.elapsed().as_secs_f64()
        );

        let at_translations = Instant::now();
        self.pipeline.drain_translations();
        crate::debug!(
            "stop: translations drained in {:.2}s (total {:.2}s)",
            at_translations.elapsed().as_secs_f64(),
            began.elapsed().as_secs_f64()
        );

        mic_result?;
        monitor_result?;

        self.meta.duration_secs = Some(self.start.elapsed().as_secs());
        self.meta.segment_count = Some(
            crate::store::count_segment_lines(&self.store_dir.join("segments.jsonl")).unwrap_or(0)
                as u64,
        );
        write_meta(&self.store_dir.join("meta.json"), &self.meta)?;

        Ok(())
    }
}

struct ReaderParams {
    speaker_tag: SpeakerTag,
    language: Option<String>,
    vad_threshold: f32,
    silence_hold_ms: u64,
    duration_cap_ms: u64,
    live_chunk_ms: u64,
}

fn run_reader<R: io::Read>(
    source: R,
    mut wav: WavWriter,
    params: ReaderParams,
    pipeline: Arc<Pipeline>,
    submissions: Arc<SubmissionQueue>,
) -> io::Result<()> {
    let ReaderParams {
        speaker_tag,
        language,
        vad_threshold,
        silence_hold_ms,
        duration_cap_ms,
        live_chunk_ms,
    } = params;

    let mut segmenter = Segmenter::new(
        vad_threshold,
        silence_hold_ms,
        duration_cap_ms,
        live_chunk_ms,
    );
    let mut buffer: Vec<i16> = Vec::new();
    // The Segmenter's spans are absolute sample offsets, so `drained`
    // tracks how many leading samples have been dropped from `buffer`;
    // slicing subtracts it. Without draining, the buffer grows for the
    // whole recording (~115 MB/hr per stream at 16kHz i16).
    let mut drained: u64 = 0;
    let mut open_span_start: Option<u64> = None;
    let mut last_closed_end: u64 = 0;
    // Counted so a stream that captures nothing — the signature of a
    // mis-targeted source — is visible at stop instead of silent.
    let mut samples_read: u64 = 0;
    let mut segments_closed: u32 = 0;

    tee_pcm_to_wav(source, &mut wav, |chunk| {
        buffer.extend_from_slice(chunk);
        samples_read += chunk.len() as u64;
        for event in segmenter.push_samples(chunk) {
            match event {
                // Registered synchronously, right here on the reader
                // thread, before this closure returns: this is what
                // lets flush_ready_prefix see a still-open monologue
                // and hold back a later stream's segment that would
                // otherwise finish and flush first.
                SegmentEvent::Opened { start_sample } => {
                    open_span_start = Some(start_sample);
                    pipeline.begin(segmenter::to_ms(start_sample));
                }
                SegmentEvent::Closed(span) => {
                    // A duration-cap force-close is immediately followed
                    // by a new Opened in the same batch, which re-sets
                    // open_span_start below.
                    open_span_start = None;
                    last_closed_end = span.end_sample;
                    segments_closed += 1;
                    crate::debug!(
                        "segment closed {:?} {}..{}ms ({} samples)",
                        speaker_tag,
                        segmenter::to_ms(span.start_sample),
                        segmenter::to_ms(span.end_sample),
                        span.end_sample - span.start_sample,
                    );
                    spawn_submission(
                        &submissions,
                        speaker_tag,
                        language.clone(),
                        &buffer,
                        span,
                        drained,
                    );
                }
            }
        }
        drain_dead_prefix(
            &mut buffer,
            &mut drained,
            open_span_start.unwrap_or(last_closed_end),
        );
    })?;

    if let Some(span) = segmenter.finish() {
        // Its Opened event already fired earlier, when this trailing
        // span first opened mid-stream -- no begin() call needed here.
        segments_closed += 1;
        spawn_submission(&submissions, speaker_tag, language, &buffer, span, drained);
    }

    crate::debug!(
        "reader {:?} ended: {} samples ({:.1}s), {} segments",
        speaker_tag,
        samples_read,
        samples_read as f64 / segmenter::SAMPLE_RATE_HZ as f64,
        segments_closed,
    );

    wav.finalize()
}

// Drops samples no span can still reference: anything before the
// currently open span's start (or the last closed span's end when no
// span is open) was already copied into a submission thread. Runs per
// chunk; the threshold keeps the drain amortized instead of per-frame.
const DRAIN_THRESHOLD_SAMPLES: u64 = segmenter::SAMPLE_RATE_HZ * 60;

fn drain_dead_prefix(buffer: &mut Vec<i16>, drained: &mut u64, floor: u64) {
    if floor > *drained && floor - *drained > DRAIN_THRESHOLD_SAMPLES {
        buffer.drain(..(floor - *drained) as usize);
        *drained = floor;
    }
}

fn spawn_submission(
    submissions: &Arc<SubmissionQueue>,
    speaker_tag: SpeakerTag,
    language: Option<String>,
    buffer: &[i16],
    span: SpeechSpan,
    drained: u64,
) {
    let start = (span.start_sample - drained) as usize;
    let end = (span.end_sample - drained) as usize;
    let samples = buffer[start..end].to_vec();

    submissions.push(SegmentInput {
        speaker_tag,
        start_ms: segmenter::to_ms(span.start_sample),
        end_ms: segmenter::to_ms(span.end_sample),
        samples,
        language,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_dead_prefix_drops_only_below_the_floor_and_only_past_threshold() {
        let mut buffer: Vec<i16> = (0..100).collect();
        let mut drained = 0;

        drain_dead_prefix(&mut buffer, &mut drained, 50);
        assert_eq!(buffer.len(), 100, "below the threshold nothing drains");
        assert_eq!(drained, 0);

        let floor = DRAIN_THRESHOLD_SAMPLES + 50;
        let mut big: Vec<i16> = vec![0; floor as usize + 10];
        big[0] = 7;
        drain_dead_prefix(&mut big, &mut drained, floor);
        assert_eq!(big.len(), 10);
        assert_eq!(drained, floor);

        let before = big.len();
        drain_dead_prefix(&mut big, &mut drained, floor);
        assert_eq!(big.len(), before, "floor at the offset drains nothing");
    }
}
