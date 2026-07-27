use crate::confidence::passes_floor;
use crate::domain::{Segment, SpeakerTag};
use crate::store::{Record, Store};
use crate::transcriber::{TranscribeError, Transcriber};
use crate::transcript::insert_ordered;
use crate::translator::Translator;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub struct SegmentInput {
    pub speaker_tag: SpeakerTag,
    pub start_ms: u64,
    pub end_ms: u64,
    pub samples: Vec<i16>,
    pub language: Option<String>,
}

#[derive(Debug)]
pub enum PipelineError {
    Transcribe(TranscribeError),
    Store(std::io::Error),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineError::Transcribe(e) => write!(f, "transcription failed: {e}"),
            PipelineError::Store(e) => write!(f, "store append failed: {e}"),
        }
    }
}

impl std::error::Error for PipelineError {}

impl From<TranscribeError> for PipelineError {
    fn from(e: TranscribeError) -> Self {
        PipelineError::Transcribe(e)
    }
}

impl From<std::io::Error> for PipelineError {
    fn from(e: std::io::Error) -> Self {
        PipelineError::Store(e)
    }
}

struct Persisted {
    // Holds only segments not yet written to store, kept sorted by
    // start_ms via insert_ordered. A segment that completes out of
    // order sits here until every still-in-flight submission with a
    // smaller start_ms has landed ahead of it, then gets removed as
    // it's written. Segments can only be appended to Store in
    // start_ms order (Store only supports appending at EOF, it can't
    // insert), so a flushed entry must never stay in this Vec: an
    // index-based watermark would misfire once a later insert_ordered
    // call shifts positions behind it.
    transcript: Vec<Segment>,
    // Every Segment that ever passes the confidence floor, sorted by
    // start_ms, never removed. Unlike `transcript`, this is the UI's
    // data source: it monotonically grows for the life of the
    // Pipeline, so a live panel built on it keeps rows even after
    // they've been flushed to disk and dropped from `transcript`.
    live_view: Vec<Segment>,
    store: Store,
    // start_ms -> count of segments currently between opening (or, for
    // callers with no separate open phase, submitting) and completing
    // their transcribe() call, keyed by start_ms (not id) so
    // flush_ready_prefix can compute "smallest start_ms that could
    // still land earlier than what's next to flush".
    in_flight: BTreeMap<u64, usize>,
}

impl Persisted {
    fn register_in_flight(&mut self, start_ms: u64) {
        *self.in_flight.entry(start_ms).or_insert(0) += 1;
    }

    fn retire_in_flight(&mut self, start_ms: u64) {
        if let Some(count) = self.in_flight.get_mut(&start_ms) {
            *count -= 1;
            if *count == 0 {
                self.in_flight.remove(&start_ms);
            }
        }
    }

    fn flush_ready_prefix(&mut self) -> Result<(), PipelineError> {
        let watermark = self.in_flight.keys().next().copied();
        while let Some(candidate) = self.transcript.first() {
            let safe = match watermark {
                Some(min_in_flight) => candidate.start_ms <= min_in_flight,
                None => true,
            };
            if !safe {
                break;
            }
            self.store.append(candidate)?;
            self.transcript.remove(0);
        }
        Ok(())
    }

    fn translation_context(&self, segment_id: u64) -> Vec<String> {
        let Some(position) = self
            .live_view
            .iter()
            .position(|segment| segment.id == segment_id)
        else {
            return Vec::new();
        };

        let mut context: Vec<String> = self.live_view[..position]
            .iter()
            .rev()
            .filter_map(|segment| segment.translation.clone())
            .take(3)
            .collect();
        context.reverse();
        context
    }
}

// Snapshot callback fired with a full, sorted copy of `live_view` each
// time a Segment passes the confidence floor or gains a translation.
type TranscriptUpdateCallback = Box<dyn Fn(Vec<Segment>) + Send + Sync>;

/// Turns Segments into transcribed, ordered, persisted ones. Callers
/// with a distinct open-span phase should call `begin` the instant a
/// span opens and `finish_pending` once it closes and is ready to
/// transcribe; `submit` is `begin` + `finish_pending` combined for
/// callers with no such phase. `finish_pending`/`submit` block the
/// calling thread on the transcription call; two Streams are expected
/// to each drive this from their own thread so one Stream's in-flight
/// transcription never stalls the other.
pub struct Pipeline {
    transcriber: Arc<dyn Transcriber>,
    translator: Arc<dyn Translator>,
    confidence_floor: f32,
    next_id: AtomicU64,
    persisted: Arc<Mutex<Persisted>>,
    on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
    translation_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    translation_chain_tail: Arc<Mutex<Option<Receiver<()>>>>,
}

impl Pipeline {
    pub fn new(
        transcriber: Arc<dyn Transcriber>,
        translator: Arc<dyn Translator>,
        store: Store,
        confidence_floor: f32,
    ) -> Self {
        Self {
            transcriber,
            translator,
            confidence_floor,
            next_id: AtomicU64::new(1),
            persisted: Arc::new(Mutex::new(Persisted {
                transcript: Vec::new(),
                live_view: Vec::new(),
                store,
                in_flight: BTreeMap::new(),
            })),
            on_update: Arc::new(Mutex::new(None)),
            translation_handles: Arc::new(Mutex::new(Vec::new())),
            translation_chain_tail: Arc::new(Mutex::new(None)),
        }
    }

    /// Registers a callback fired (off the Pipeline's internal lock)
    /// with a full, sorted snapshot of `live_view` every time a
    /// Segment passes the confidence floor. No-op if never called.
    /// The callback receives `Vec<Segment>` by value and must be
    /// `Send + Sync` because it is invoked from the submission and
    /// translation threads, not the UI thread.
    pub fn set_on_update(&self, callback: impl Fn(Vec<Segment>) + Send + Sync + 'static) {
        *self.on_update.lock().expect("on_update mutex poisoned") = Some(Box::new(callback));
    }

    /// Registers a segment as pending (open or transcribing) the moment
    /// it starts, so flush_ready_prefix knows not to write anything past
    /// this start_ms until it's accounted for. Safe to call from the
    /// Stream's own reader thread, synchronously, before the segment has
    /// even closed.
    pub fn begin(&self, start_ms: u64) {
        self.persisted
            .lock()
            .expect("pipeline mutex poisoned")
            .register_in_flight(start_ms);
    }

    /// Transcribes, retires the in_flight entry registered by a prior
    /// `begin(input.start_ms)` call, and applies the confidence
    /// floor/ordering/flush exactly as `submit` did before. Does NOT
    /// call `begin` itself -- the caller must have already called it.
    pub fn finish_pending(&self, input: SegmentInput) -> Result<(), PipelineError> {
        let outcome = self
            .transcriber
            .transcribe(&input.samples, input.language.as_deref());

        let mut translation_job = None;
        let snapshot: Option<Vec<Segment>> = {
            let mut persisted = self.persisted.lock().expect("pipeline mutex poisoned");
            persisted.retire_in_flight(input.start_ms);

            let transcription = match outcome {
                Ok(transcription) => transcription,
                Err(e) => {
                    persisted.flush_ready_prefix()?;
                    return Err(e.into());
                }
            };

            let segment = Segment {
                id: self.next_id.fetch_add(1, Ordering::SeqCst),
                speaker_tag: input.speaker_tag,
                start_ms: input.start_ms,
                end_ms: input.end_ms,
                text: transcription.text,
                mean_confidence: transcription.mean_confidence,
                translation: None,
            };

            let mut snapshot = None;
            if passes_floor(&segment, self.confidence_floor) {
                let segment_id = segment.id;
                let text = segment.text.clone();
                insert_ordered(&mut persisted.transcript, segment.clone());
                insert_ordered(&mut persisted.live_view, segment);
                translation_job = Some((segment_id, text));
                snapshot = Some(persisted.live_view.clone());
            }

            persisted.flush_ready_prefix()?;
            snapshot
        };

        if let Some(snapshot) = snapshot {
            self.fire_update(snapshot);
        }

        if let Some((segment_id, text)) = translation_job {
            self.dispatch_translation(segment_id, text);
        }

        Ok(())
    }

    /// Convenience for callers with no separate open-span phase (existing
    /// tests use this): begin + finish_pending combined, net-identical
    /// behavior to before this change.
    pub fn submit(&self, input: SegmentInput) -> Result<(), PipelineError> {
        self.begin(input.start_ms);
        self.finish_pending(input)
    }

    pub fn drain_translations(&self) {
        let handles = std::mem::take(
            &mut *self
                .translation_handles
                .lock()
                .expect("translation handles mutex poisoned"),
        );
        for handle in handles {
            handle.join().expect("translation thread panicked");
        }
    }

    fn dispatch_translation(&self, segment_id: u64, text: String) {
        let translator = self.translator.clone();
        let persisted = self.persisted.clone();
        let on_update = self.on_update.clone();

        let (wait_for_previous, notify_settled) = {
            let mut chain_tail = self
                .translation_chain_tail
                .lock()
                .expect("translation chain mutex poisoned");
            (chain_tail.take(), {
                let (send, recv) = channel();
                *chain_tail = Some(recv);
                send
            })
        };

        let handle = thread::spawn(move || {
            if let Some(previous) = wait_for_previous {
                let _ = previous.recv();
            }

            let context = {
                let persisted = persisted.lock().expect("pipeline mutex poisoned");
                persisted.translation_context(segment_id)
            };

            match translator.translate(&text, &context) {
                Ok(translation) => {
                    if let Err(e) =
                        Self::record_translation(persisted, on_update, segment_id, translation)
                    {
                        eprintln!("translation persist failed: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("translation failed: {e}");
                }
            }

            let _ = notify_settled.send(());
        });

        self.translation_handles
            .lock()
            .expect("translation handles mutex poisoned")
            .push(handle);
    }

    fn record_translation(
        persisted: Arc<Mutex<Persisted>>,
        on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
        segment_id: u64,
        text: String,
    ) -> Result<(), PipelineError> {
        let snapshot = {
            let mut persisted = persisted.lock().expect("pipeline mutex poisoned");
            let Some(position) = persisted
                .live_view
                .iter()
                .position(|segment| segment.id == segment_id)
            else {
                return Ok(());
            };

            persisted.store.append_record(&Record::Translation {
                segment_id,
                text: text.clone(),
            })?;
            persisted.live_view[position].translation = Some(text);
            Some(persisted.live_view.clone())
        };

        if let Some(snapshot) = snapshot {
            if let Some(callback) = on_update.lock().expect("on_update mutex poisoned").as_ref() {
                callback(snapshot);
            }
        }

        Ok(())
    }

    fn fire_update(&self, snapshot: Vec<Segment>) {
        if let Some(callback) = self
            .on_update
            .lock()
            .expect("on_update mutex poisoned")
            .as_ref()
        {
            callback(snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SpeakerTag;
    use crate::transcriber::{FakeTranscriber, Transcription};
    use crate::translator::FakeTranslator;
    use std::fs;
    #[cfg(target_os = "linux")]
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn translation_context_uses_last_three_prior_translations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let store = Store::open(&path).unwrap();

        let transcriber = Arc::new(FakeTranscriber::new());
        let translator = Arc::new(FakeTranslator::new());
        let pipeline = Arc::new(Pipeline::new(
            transcriber.clone(),
            translator.clone(),
            store,
            0.0,
        ));

        for id in 1..=5 {
            submit_and_translate(&pipeline, &transcriber, &translator, id);
        }

        let calls = translator.recorded_calls();
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[0], ("segment 1".to_string(), vec![]));
        assert_eq!(calls[3].1, vec!["t1", "t2", "t3"]);
        assert_eq!(calls[4].1, vec!["t2", "t3", "t4"]);
        assert!(!calls[4].1.iter().any(|text| text == "t1"));
    }
    #[test]
    fn translation_workers_wait_for_previous_to_settle_before_next_call() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("transcript.jsonl")).unwrap();
        let transcriber = Arc::new(FakeTranscriber::new());
        let translator = Arc::new(FakeTranslator::new());
        let pipeline = Arc::new(Pipeline::new(
            transcriber.clone(),
            translator.clone(),
            store,
            0.0,
        ));

        let first_samples = vec![1i16; 8];
        let first_transcribe = transcriber.expect_call(first_samples.clone());
        let first_translate = translator.expect_call("segment 1".to_string());
        let first_input = SegmentInput {
            speaker_tag: SpeakerTag::Me,
            start_ms: 0,
            end_ms: 400,
            samples: first_samples,
            language: None,
        };

        let pipeline_first = pipeline.clone();
        let first_handle = thread::spawn(move || pipeline_first.submit(first_input).unwrap());
        first_transcribe.respond(Transcription {
            text: "segment 1".to_string(),
            mean_confidence: 0.95,
        });
        first_handle.join().unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            translator.recorded_calls(),
            vec![("segment 1".to_string(), vec![])]
        );

        let second_samples = vec![2i16; 8];
        let second_transcribe = transcriber.expect_call(second_samples.clone());
        let second_translate = translator.expect_call("segment 2".to_string());
        let second_input = SegmentInput {
            speaker_tag: SpeakerTag::Them,
            start_ms: 1_000,
            end_ms: 1_400,
            samples: second_samples,
            language: None,
        };

        let pipeline_second = pipeline.clone();
        let second_handle = thread::spawn(move || pipeline_second.submit(second_input).unwrap());
        second_transcribe.respond(Transcription {
            text: "segment 2".to_string(),
            mean_confidence: 0.95,
        });
        second_handle.join().unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            translator.recorded_calls(),
            vec![("segment 1".to_string(), vec![])]
        );

        first_translate.respond("t1".to_string());

        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            translator.recorded_calls(),
            vec![
                ("segment 1".to_string(), vec![]),
                ("segment 2".to_string(), vec!["t1".to_string()])
            ]
        );

        second_translate.respond("t2".to_string());
        pipeline.drain_translations();
    }

    #[test]
    fn translation_workers_wait_for_failed_settlement_before_next_call() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("transcript.jsonl")).unwrap();
        let transcriber = Arc::new(FakeTranscriber::new());
        let translator = Arc::new(FakeTranslator::new());
        let pipeline = Arc::new(Pipeline::new(
            transcriber.clone(),
            translator.clone(),
            store,
            0.0,
        ));

        let first_samples = vec![1i16; 8];
        let first_transcribe = transcriber.expect_call(first_samples.clone());
        let first_translate = translator.expect_call("segment 1".to_string());
        let first_input = SegmentInput {
            speaker_tag: SpeakerTag::Me,
            start_ms: 0,
            end_ms: 400,
            samples: first_samples,
            language: None,
        };

        let pipeline_first = pipeline.clone();
        let first_handle = thread::spawn(move || pipeline_first.submit(first_input).unwrap());
        first_transcribe.respond(Transcription {
            text: "segment 1".to_string(),
            mean_confidence: 0.95,
        });
        first_handle.join().unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            translator.recorded_calls(),
            vec![("segment 1".to_string(), vec![])]
        );

        let second_samples = vec![2i16; 8];
        let second_transcribe = transcriber.expect_call(second_samples.clone());
        let second_translate = translator.expect_call("segment 2".to_string());
        let second_input = SegmentInput {
            speaker_tag: SpeakerTag::Them,
            start_ms: 1_000,
            end_ms: 1_400,
            samples: second_samples,
            language: None,
        };

        let pipeline_second = pipeline.clone();
        let second_handle = thread::spawn(move || pipeline_second.submit(second_input).unwrap());
        second_transcribe.respond(Transcription {
            text: "segment 2".to_string(),
            mean_confidence: 0.95,
        });
        second_handle.join().unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            translator.recorded_calls(),
            vec![("segment 1".to_string(), vec![])]
        );

        first_translate.fail(crate::translator::TranslateError::new("forced failure"));

        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            translator.recorded_calls(),
            vec![
                ("segment 1".to_string(), vec![]),
                ("segment 2".to_string(), vec![])
            ]
        );

        second_translate.respond("t2".to_string());
        pipeline.drain_translations();

        let live_view = pipeline
            .persisted
            .lock()
            .expect("pipeline mutex poisoned")
            .live_view
            .clone();
        assert_eq!(live_view[0].translation, None);
        assert!(live_view[0].text == "segment 1");
    }

    #[test]
    fn record_translation_unknown_segment_id_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let store = Store::open(&path).unwrap();

        let transcriber = Arc::new(FakeTranscriber::new());
        let translator = Arc::new(FakeTranslator::new());
        let pipeline = Arc::new(Pipeline::new(transcriber, translator, store, 0.0));

        let snapshots: Arc<Mutex<Vec<Vec<Segment>>>> = Arc::new(Mutex::new(Vec::new()));
        let snapshots_for_cb = snapshots.clone();
        let on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>> =
            Arc::new(Mutex::new(Some(Box::new(move |snapshot: Vec<Segment>| {
                snapshots_for_cb.lock().unwrap().push(snapshot);
            }))));

        Pipeline::record_translation(
            pipeline.persisted.clone(),
            on_update.clone(),
            123,
            "ignored".to_string(),
        )
        .unwrap();

        assert!(snapshots.lock().unwrap().is_empty());
        assert!(fs::read_to_string(path).unwrap().is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn record_translation_leaves_live_view_pending_when_append_fails() {
        let store = Store::open(Path::new("/dev/full")).unwrap();
        let transcriber = Arc::new(FakeTranscriber::new());
        let translator = Arc::new(FakeTranslator::new());
        let pipeline = Arc::new(Pipeline::new(transcriber, translator, store, 0.0));

        pipeline
            .persisted
            .lock()
            .expect("pipeline mutex poisoned")
            .live_view
            .push(Segment {
                id: 1,
                speaker_tag: SpeakerTag::Me,
                start_ms: 0,
                end_ms: 400,
                text: "segment 1".to_string(),
                mean_confidence: 0.95,
                translation: None,
            });

        let snapshots: Arc<Mutex<Vec<Vec<Segment>>>> = Arc::new(Mutex::new(Vec::new()));
        let snapshots_for_cb = snapshots.clone();
        let on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>> =
            Arc::new(Mutex::new(Some(Box::new(move |snapshot: Vec<Segment>| {
                snapshots_for_cb.lock().unwrap().push(snapshot);
            }))));

        let err = Pipeline::record_translation(
            pipeline.persisted.clone(),
            on_update,
            1,
            "t1".to_string(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("store append failed"));
        assert_eq!(
            pipeline
                .persisted
                .lock()
                .expect("pipeline mutex poisoned")
                .live_view[0]
                .translation,
            None
        );
        assert!(snapshots.lock().unwrap().is_empty());
    }

    // The live-view feed delivered to `set_on_update` must stay sorted
    // by start_ms at every callback even when segments complete out of
    // order, and must keep holding rows after they've been flushed to
    // disk and removed from the disk-bookkeeping `transcript` Vec.
    #[test]
    fn live_view_stays_sorted_and_outlives_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let store = Store::open(&path).unwrap();

        let fake = Arc::new(FakeTranscriber::new());
        let translator = Arc::new(FakeTranslator::new());
        let pipeline = Arc::new(Pipeline::new(fake.clone(), translator, store, 0.0));

        let snapshots: Arc<Mutex<Vec<Vec<Segment>>>> = Arc::new(Mutex::new(Vec::new()));
        let snapshots_for_cb = snapshots.clone();
        pipeline.set_on_update(move |segments| {
            snapshots_for_cb.lock().unwrap().push(segments);
        });

        let earlier_samples = vec![11i16; 8];
        let later_samples = vec![22i16; 8];

        let earlier_call = fake.expect_call(earlier_samples.clone());
        let later_call = fake.expect_call(later_samples.clone());

        let earlier_input = SegmentInput {
            speaker_tag: SpeakerTag::Them,
            start_ms: 0,
            end_ms: 400,
            samples: earlier_samples,
            language: None,
        };
        let later_input = SegmentInput {
            speaker_tag: SpeakerTag::Me,
            start_ms: 1_000,
            end_ms: 1_400,
            samples: later_samples,
            language: None,
        };

        let pipeline_earlier = pipeline.clone();
        let earlier_handle = thread::spawn(move || pipeline_earlier.submit(earlier_input).unwrap());
        let pipeline_later = pipeline.clone();
        let later_handle = thread::spawn(move || pipeline_later.submit(later_input).unwrap());

        // Give both reader threads time to call `begin` before either
        // transcription completes, so the later segment can't flush
        // past the still-open earlier span.
        thread::sleep(Duration::from_millis(50));

        // Release the segment with the LATER start_ms first: its
        // callback fires before the earlier segment's, but the
        // live-view snapshot it delivers must still be sorted.
        later_call.respond(Transcription {
            text: "later reply, arrives first".to_string(),
            mean_confidence: 0.95,
        });
        earlier_call.respond(Transcription {
            text: "earlier reply, arrives last".to_string(),
            mean_confidence: 0.9,
        });

        earlier_handle.join().unwrap();
        later_handle.join().unwrap();

        let snaps = snapshots.lock().unwrap().clone();
        assert!(
            !snaps.is_empty(),
            "set_on_update must fire when a segment passes the floor"
        );
        for snap in &snaps {
            assert!(
                snap.windows(2).all(|w| w[0].start_ms <= w[1].start_ms),
                "every live-view snapshot must be sorted by start_ms, got {snap:?}"
            );
        }

        // After both segments are flushed to disk (and thus removed
        // from the disk-bookkeeping `transcript`), the live-view must
        // still report both -- proving it doesn't shrink on flush.
        let on_disk = crate::store::read_all(&path).unwrap();
        assert_eq!(
            on_disk.len(),
            2,
            "both segments must have been flushed to disk for this test to mean anything"
        );

        let final_snap = snaps.last().unwrap();
        assert_eq!(final_snap.len(), 2);
        assert_eq!(final_snap[0].start_ms, 0);
        assert_eq!(final_snap[0].text, "earlier reply, arrives last");
        assert_eq!(final_snap[1].start_ms, 1_000);
        assert_eq!(final_snap[1].text, "later reply, arrives first");
    }
    #[test]
    fn segment_below_confidence_floor_is_not_translated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let store = Store::open(&path).unwrap();

        let transcriber = Arc::new(FakeTranscriber::new());
        let translator = Arc::new(FakeTranslator::new());
        let pipeline = Arc::new(Pipeline::new(
            transcriber.clone(),
            translator.clone(),
            store,
            0.6,
        ));

        let samples = vec![9i16; 8];
        let transcribe_call = transcriber.expect_call(samples.clone());
        let input = SegmentInput {
            speaker_tag: SpeakerTag::Me,
            start_ms: 0,
            end_ms: 400,
            samples,
            language: None,
        };

        let pipeline_for_thread = pipeline.clone();
        let handle = thread::spawn(move || pipeline_for_thread.submit(input).unwrap());
        transcribe_call.respond(Transcription {
            text: "quiet segment".to_string(),
            mean_confidence: 0.4,
        });
        handle.join().unwrap();
        pipeline.drain_translations();

        assert_eq!(translator.recorded_calls(), vec![]);
        assert_eq!(
            pipeline
                .persisted
                .lock()
                .expect("pipeline mutex poisoned")
                .live_view,
            Vec::new()
        );
        assert_eq!(crate::store::read_all(&path).unwrap(), vec![]);
    }

    fn submit_and_translate(
        pipeline: &Arc<Pipeline>,
        transcriber: &Arc<FakeTranscriber>,
        translator: &Arc<FakeTranslator>,
        id: u64,
    ) {
        let samples = vec![id as i16; 8];
        let transcribe_call = transcriber.expect_call(samples.clone());
        let translate_call = translator.expect_call(format!("segment {id}"));

        let input = SegmentInput {
            speaker_tag: SpeakerTag::Me,
            start_ms: id * 1_000,
            end_ms: id * 1_000 + 400,
            samples,
            language: None,
        };

        let pipeline_for_thread = pipeline.clone();
        let handle = thread::spawn(move || pipeline_for_thread.submit(input).unwrap());

        transcribe_call.respond(Transcription {
            text: format!("segment {id}"),
            mean_confidence: 0.95,
        });
        handle.join().unwrap();
        translate_call.respond(format!("t{id}"));
        pipeline.drain_translations();
    }
}
