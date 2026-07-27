use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

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
    pub mic_source: String,
    pub monitor_source: String,
    pub mic_language: Option<String>,
    pub monitor_language: Option<String>,
    pub vad_threshold: f32,
    pub silence_hold_ms: u64,
    pub duration_cap_ms: u64,
    pub confidence_floor: f32,
}

pub struct Recording {
    mic_child: Option<Child>,
    monitor_child: Option<Child>,
    mic_reader: JoinHandle<io::Result<()>>,
    monitor_reader: JoinHandle<io::Result<()>>,
    submissions: Arc<Mutex<Vec<JoinHandle<()>>>>,
    pipeline: Arc<Pipeline>,
}

impl Recording {
    pub fn start(
        params: RecordingParams,
        transcriber: Arc<dyn Transcriber>,
        translator: Arc<dyn Translator>,
        on_transcript_update: impl Fn(Vec<Segment>) + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let mut mic_child = pw_record::spawn(&params.mic_source)?;
        let mic_stdout = mic_child
            .stdout
            .take()
            .expect("pw-record spawned with piped stdout");

        let mut monitor_child = match pw_record::spawn(&params.monitor_source) {
            Ok(child) => child,
            Err(e) => {
                let _ = pw_record::stop(&mut mic_child);
                return Err(e);
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
                let _ = pw_record::stop(&mut mic_child);
                let _ = pw_record::stop(&mut monitor_child);
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
        fs::create_dir_all(&params.store_dir)?;

        let store = Store::open(&params.store_dir.join("segments.jsonl"))?;
        let pipeline = Arc::new(Pipeline::new(
            transcriber,
            translator,
            store,
            params.confidence_floor,
        ));
        // Register the live-view callback before either reader thread
        // is spawned: a segment can arrive the instant a reader
        // starts, so the callback must already be in place or early
        // rows would silently never reach the UI.
        pipeline.set_on_update(on_transcript_update);

        let mic_wav = WavWriter::create(&params.store_dir.join("mic.wav"))?;
        let monitor_wav = WavWriter::create(&params.store_dir.join("monitor.wav"))?;

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
        })
    }

    pub fn stop(mut self) -> io::Result<()> {
        if let Some(mut child) = self.mic_child.take() {
            pw_record::stop(&mut child)?;
        }
        if let Some(mut child) = self.monitor_child.take() {
            pw_record::stop(&mut child)?;
        }

        let mic_result = self.mic_reader.join().expect("mic reader thread panicked");
        let monitor_result = self
            .monitor_reader
            .join()
            .expect("monitor reader thread panicked");

        let handles =
            std::mem::take(&mut *self.submissions.lock().expect("submissions mutex poisoned"));
        for handle in handles {
            handle.join().expect("segment submission thread panicked");
        }
        self.pipeline.drain_translations();

        mic_result?;
        monitor_result?;
        Ok(())
    }
}

struct ReaderParams {
    speaker_tag: SpeakerTag,
    language: Option<String>,
    vad_threshold: f32,
    silence_hold_ms: u64,
    duration_cap_ms: u64,
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
    } = params;

    let mut segmenter = Segmenter::new(vad_threshold, silence_hold_ms, duration_cap_ms);
    let mut buffer: Vec<i16> = Vec::new();

    tee_pcm_to_wav(source, &mut wav, |chunk| {
        buffer.extend_from_slice(chunk);
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
        spawn_submission(
            &pipeline,
            &submissions,
            speaker_tag,
            language,
            &buffer,
            span,
        );
    }

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
            eprintln!("segment submission failed: {e}");
        }
    });

    submissions
        .lock()
        .expect("submissions mutex poisoned")
        .push(handle);
}
