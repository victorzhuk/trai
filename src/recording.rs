use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::capture::pw_record;
use crate::capture::tee_pcm_to_wav;
use crate::capture::wav_writer::WavWriter;
use crate::domain::{Segment, SpeakerTag};
use crate::pipeline::{Pipeline, SegmentInput};
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

pub struct Recording {
    mic_child: Option<Child>,
    monitor_child: Option<Child>,
    mic_reader: JoinHandle<io::Result<()>>,
    monitor_reader: JoinHandle<io::Result<()>>,
    submissions: Arc<Mutex<Vec<JoinHandle<()>>>>,
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
        on_transcript_update: impl Fn(Vec<Segment>) + Send + Sync + 'static,
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

        let mut mic_child = pw_record::spawn(&params.mic_source).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "spawn pw-record for mic source '{}': {e}",
                    params.mic_source
                ),
            )
        })?;
        let mic_stdout = mic_child
            .stdout
            .take()
            .expect("pw-record spawned with piped stdout");

        let mut monitor_child = match pw_record::spawn(&params.monitor_source) {
            Ok(child) => child,
            Err(e) => {
                if let Err(stop_err) = pw_record::stop(&mut mic_child) {
                    crate::debug!(
                        "start: stopping mic pw-record after monitor spawn failed: {stop_err}"
                    );
                }
                return Err(io::Error::new(
                    e.kind(),
                    format!(
                        "spawn pw-record for monitor source '{}': {e}",
                        params.monitor_source
                    ),
                ));
            }
        };
        let monitor_stdout = monitor_child
            .stdout
            .take()
            .expect("pw-record spawned with piped stdout");

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
                if let Err(stop_err) = pw_record::stop(&mut mic_child) {
                    crate::debug!("start: stopping mic pw-record after failure: {stop_err}");
                }
                if let Err(stop_err) = pw_record::stop(&mut monitor_child) {
                    crate::debug!("start: stopping monitor pw-record after failure: {stop_err}");
                }
                return Err(e);
            }
        };

        recording.mic_child = Some(mic_child);
        recording.monitor_child = Some(monitor_child);
        Ok(recording)
    }

    pub fn start_with_sources<R1, R2>(
        mic_source: R1,
        monitor_source: R2,
        params: RecordingParams,
        transcriber: Arc<dyn Transcriber>,
        translator: Arc<dyn Translator>,
        on_transcript_update: impl Fn(Vec<Segment>) + Send + Sync + 'static,
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

        let submissions: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));

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
            mic_child: None,
            monitor_child: None,
            mic_reader,
            monitor_reader,
            submissions,
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
        if let Some(mut child) = self.mic_child.take() {
            pw_record::stop(&mut child)?;
        }
        if let Some(mut child) = self.monitor_child.take() {
            pw_record::stop(&mut child)?;
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
        let handles =
            std::mem::take(&mut *self.submissions.lock().expect("submissions mutex poisoned"));
        let pending = handles.len();
        for handle in handles {
            handle
                .join()
                .map_err(|_| io::Error::other("segment submission thread panicked"))?;
        }
        crate::debug!(
            "stop: {pending} submissions joined in {:.2}s",
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
    submissions: Arc<Mutex<Vec<JoinHandle<()>>>>,
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
                    pipeline.begin(segmenter::to_ms(start_sample));
                }
                SegmentEvent::Closed(span) => {
                    segments_closed += 1;
                    crate::debug!(
                        "segment closed {:?} {}..{}ms ({} samples)",
                        speaker_tag,
                        segmenter::to_ms(span.start_sample),
                        segmenter::to_ms(span.end_sample),
                        span.end_sample - span.start_sample,
                    );
                    spawn_submission(
                        &pipeline,
                        &submissions,
                        speaker_tag,
                        language.clone(),
                        &buffer,
                        span,
                    );
                }
            }
        }
    })?;

    if let Some(span) = segmenter.finish() {
        // Its Opened event already fired earlier, when this trailing
        // span first opened mid-stream -- no begin() call needed here.
        segments_closed += 1;
        spawn_submission(
            &pipeline,
            &submissions,
            speaker_tag,
            language,
            &buffer,
            span,
        );
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

fn spawn_submission(
    pipeline: &Arc<Pipeline>,
    submissions: &Arc<Mutex<Vec<JoinHandle<()>>>>,
    speaker_tag: SpeakerTag,
    language: Option<String>,
    buffer: &[i16],
    span: SpeechSpan,
) {
    let samples = buffer[span.start_sample as usize..span.end_sample as usize].to_vec();
    let start_ms = segmenter::to_ms(span.start_sample);
    let end_ms = segmenter::to_ms(span.end_sample);
    let pipeline = pipeline.clone();

    let handle = thread::spawn(move || {
        let input = SegmentInput {
            speaker_tag,
            start_ms,
            end_ms,
            samples,
            language,
        };
        if let Err(e) = pipeline.finish_pending(input) {
            crate::debug!("segment submission failed: {e}");
        }
    });

    submissions
        .lock()
        .expect("submissions mutex poisoned")
        .push(handle);
}
