use crate::confidence::passes_floor;
use crate::domain::{insert_ordered, Segment, SegmentState, SpeakerTag};
use crate::language;
use crate::store::{Record, Store};
use crate::transcriber::{TranscribeError, Transcriber};
use crate::translator::Translator;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct SegmentInput {
    pub speaker_tag: SpeakerTag,
    pub start_ms: u64,
    pub end_ms: u64,
    pub samples: Vec<i16>,
    pub language: Option<String>,
}

// How often drain_translations wakes to check force-stop while the
// watcher thread joins settled translation handles.
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
    // The UI's data source, sorted by start_ms. A row is added the
    // moment a Segment closes, before it has any text, and filled in
    // place when its transcription lands -- so unlike `transcript`
    // this holds rows that are not yet, or never will be, Segments on
    // disk. It only ever loses a row to the confidence floor;
    // otherwise it grows for the life of the Pipeline, so a live panel
    // built on it keeps rows after they've been flushed to disk and
    // dropped from `transcript`.
    live_view: Vec<Segment>,
    // live_view indices by Segment id: every lookup below used to be a
    // linear scan over a Vec that grows for the whole meeting.
    row_index: HashMap<u64, usize>,
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

    // Drains the still-unflushed prefix that is safe to write, without
    // doing any I/O: the caller appends the returned Segments under the
    // store lock, after releasing this one, so an fsync never blocks
    // reader or translation threads touching live state.
    fn take_ready_prefix(&mut self) -> Vec<Segment> {
        let watermark = self.in_flight.keys().next().copied();
        let mut ready = 0;
        while let Some(candidate) = self.transcript.get(ready) {
            let safe = match watermark {
                Some(min_in_flight) => candidate.start_ms <= min_in_flight,
                None => true,
            };
            if !safe {
                break;
            }
            ready += 1;
        }
        self.transcript.drain(..ready).collect()
    }

    // The row was inserted at this Segment's start_ms when it closed and
    // start_ms cannot have changed since, so filling it in place keeps
    // live_view sorted without re-inserting.
    fn replace_live_row(&mut self, segment: Segment) {
        if let Some(&position) = self.row_index.get(&segment.id) {
            self.live_view[position] = segment;
            return;
        }
        self.insert_live_row(segment);
    }

    // Sorted insert that keeps row_index valid: everything at or past
    // the insertion point shifts by one.
    fn insert_live_row(&mut self, segment: Segment) {
        let position = self
            .live_view
            .partition_point(|existing| existing.start_ms <= segment.start_ms);
        for index in self.row_index.values_mut() {
            if *index >= position {
                *index += 1;
            }
        }
        self.row_index.insert(segment.id, position);
        self.live_view.insert(position, segment);
    }

    fn fail_live_row(&mut self, segment_id: u64, message: String) {
        if let Some(&position) = self.row_index.get(&segment_id) {
            self.live_view[position].state = SegmentState::Failed(message);
        }
    }

    fn drop_live_row(&mut self, segment_id: u64) {
        if let Some(position) = self.row_index.remove(&segment_id) {
            self.live_view.remove(position);
            for index in self.row_index.values_mut() {
                if *index > position {
                    *index -= 1;
                }
            }
        }
    }

    fn translation_context(&self, segment_id: u64, target_language: &str) -> Vec<String> {
        let Some(&position) = self.row_index.get(&segment_id) else {
            return Vec::new();
        };

        let mut context: Vec<String> = self.live_view[..position]
            .iter()
            .rev()
            .filter_map(|segment| target_language_text(segment, target_language))
            .take(3)
            .collect();
        context.reverse();
        context
    }
}

// A same-language Segment already reads as target-language text, so it
// carries context for the next translation exactly like a translated
// line does.
fn target_language_text(segment: &Segment, target_language: &str) -> Option<String> {
    if let Some(translation) = &segment.translation {
        return Some(translation.clone());
    }
    segment
        .is_target_language(target_language)
        .then(|| segment.text.clone())
}

// A configured source language wins over whatever the server reports
// detecting: the request forced that language, so the text really is in
// it. Kept in the Stream's own spelling when it isn't a language we can
// canonicalise, so nothing silently renames it.
fn configured_language(configured: Option<&str>) -> Option<String> {
    let configured = configured?.trim();
    if configured.is_empty() {
        return None;
    }
    Some(
        language::canonical(configured)
            .unwrap_or(configured)
            .to_string(),
    )
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
    target_language: String,
    next_id: AtomicU64,
    persisted: Arc<Mutex<Persisted>>,
    // Separate from `persisted` so write+flush+sync_data (an fsync per
    // append, milliseconds on a busy disk) never runs while the live
    // state lock is held. Lock order is always store -> persisted.
    store: Arc<Mutex<Store>>,
    on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
    translation_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    translation_chain_tail: Arc<Mutex<Option<Receiver<()>>>>,
    // Set by `signal_force_stop` so `drain_translations` stops waiting
    // for translation threads and detaches them instead. The threads
    // keep running in the background and finish on their own; the
    // caller just stops blocking on them.
    force_stop: AtomicBool,
}

impl Pipeline {
    pub fn new(
        transcriber: Arc<dyn Transcriber>,
        translator: Arc<dyn Translator>,
        store: Store,
        confidence_floor: f32,
        target_language: impl Into<String>,
    ) -> Self {
        Self {
            transcriber,
            translator,
            confidence_floor,
            target_language: target_language.into(),
            next_id: AtomicU64::new(1),
            persisted: Arc::new(Mutex::new(Persisted {
                transcript: Vec::new(),
                live_view: Vec::new(),
                in_flight: BTreeMap::new(),
                row_index: HashMap::new(),
            })),
            store: Arc::new(Mutex::new(store)),
            on_update: Arc::new(Mutex::new(None)),
            translation_handles: Arc::new(Mutex::new(Vec::new())),
            translation_chain_tail: Arc::new(Mutex::new(None)),
            force_stop: AtomicBool::new(false),
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
    /// A row for the Segment reaches the UI before the transcription
    /// call is even made, and is filled, dropped or marked failed
    /// according to how that call ends.
    pub fn finish_pending(&self, input: SegmentInput) -> Result<(), PipelineError> {
        let segment_id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let configured_language = configured_language(input.language.as_deref());

        let snapshot = {
            let mut persisted = self.persisted.lock().expect("pipeline mutex poisoned");
            persisted.insert_live_row(Segment {
                id: segment_id,
                speaker_tag: input.speaker_tag,
                start_ms: input.start_ms,
                end_ms: input.end_ms,
                text: String::new(),
                mean_confidence: 0.0,
                source_language: configured_language.clone(),
                translation: None,
                degraded: false,
                state: SegmentState::Transcribing,
                translation_error: None,
            });
            persisted.live_view.clone()
        };
        self.fire_update(snapshot);

        let outcome = self
            .transcriber
            .transcribe(&input.samples, input.language.as_deref());

        let transcription = match outcome {
            Ok(transcription) => transcription,
            Err(e) => {
                let snapshot = {
                    let mut persisted = self.persisted.lock().expect("pipeline mutex poisoned");
                    persisted.retire_in_flight(input.start_ms);
                    persisted.fail_live_row(segment_id, e.to_string());
                    persisted.live_view.clone()
                };
                // The failed row is worth more on screen than the flush
                // result is, so it goes out before the error propagates.
                self.fire_update(snapshot);
                self.flush_ready()?;
                return Err(e.into());
            }
        };

        let mut translation_job = None;
        let snapshot = {
            let mut persisted = self.persisted.lock().expect("pipeline mutex poisoned");
            persisted.retire_in_flight(input.start_ms);

            let segment = Segment {
                id: segment_id,
                speaker_tag: input.speaker_tag,
                start_ms: input.start_ms,
                end_ms: input.end_ms,
                text: transcription.text,
                mean_confidence: transcription.mean_confidence,
                source_language: configured_language.or(transcription.language),
                translation: None,
                degraded: false,
                state: SegmentState::Ready,
                translation_error: None,
            };

            if passes_floor(&segment, self.confidence_floor) {
                crate::debug!(
                    "segment {} {:?} {}ms conf={:.2} lang={} \"{}\"",
                    segment.id,
                    segment.speaker_tag,
                    segment.start_ms,
                    segment.mean_confidence,
                    segment.source_language.as_deref().unwrap_or("?"),
                    crate::log::preview(&segment.text, 60),
                );
                if segment.is_target_language(&self.target_language) {
                    crate::debug!(
                        "segment {} already in {}, not translated",
                        segment.id,
                        self.target_language,
                    );
                } else {
                    translation_job = Some((segment_id, segment.text.clone()));
                }
                insert_ordered(&mut persisted.transcript, segment.clone());
                persisted.replace_live_row(segment);
            } else {
                // Dropped here and nowhere else: without this line the
                // segment leaves no trace at all, in the UI or on disk.
                crate::debug!(
                    "segment {} {:?} {}ms DROPPED conf={:.2} < floor={:.2} \"{}\"",
                    segment.id,
                    segment.speaker_tag,
                    segment.start_ms,
                    segment.mean_confidence,
                    self.confidence_floor,
                    crate::log::preview(&segment.text, 60),
                );
                persisted.drop_live_row(segment_id);
            }

            persisted.live_view.clone()
        };

        self.flush_ready()?;
        self.fire_update(snapshot);

        if let Some((segment_id, text)) = translation_job {
            self.dispatch_translation(segment_id, text);
        }

        Ok(())
    }

    /// Convenience for tests with no separate open-span phase:
    /// begin + finish_pending combined. Production uses the two-phase
    /// contract (recording.rs registers open spans via begin).
    #[cfg(test)]
    pub fn submit(&self, input: SegmentInput) -> Result<(), PipelineError> {
        self.begin(input.start_ms);
        self.finish_pending(input)
    }

    // Appends every Segment whose watermark gate has opened, in
    // transcript order. The batch is taken under the persisted lock
    // (no I/O) and written under the store lock only.
    fn flush_ready(&self) -> Result<(), PipelineError> {
        let mut store = self.store.lock().expect("store mutex poisoned");
        let batch = {
            let mut persisted = self.persisted.lock().expect("pipeline mutex poisoned");
            persisted.take_ready_prefix()
        };
        for segment in &batch {
            store.append(segment)?;
        }
        Ok(())
    }

    /// Signals `drain_translations` (or a future caller) to stop
    /// waiting for outstanding translation threads. The threads are
    /// detached, not killed: they keep running and will finish on
    /// their own once their backend calls time out or succeed. Late
    /// writes to the store and `on_update` callbacks are safe because
    /// both go through `Arc`-shared, mutex-guarded state.
    pub fn signal_force_stop(&self) {
        self.force_stop.store(true, Ordering::Relaxed);
    }

    pub fn drain_translations(&self) {
        let handles = std::mem::take(
            &mut *self
                .translation_handles
                .lock()
                .expect("translation handles mutex poisoned"),
        );
        if handles.is_empty() {
            return;
        }

        // A watcher thread joins each handle in order and notifies
        // the channel as each one settles. This lets `drain_translations`
        // poll with a timeout rather than blocking on a single
        // `handle.join()` that could be stuck on a dead backend's
        // request timeout for tens of seconds.
        let (settled_tx, settled_rx) = channel();
        thread::spawn(move || {
            for handle in handles {
                let _ = handle.join();
                let _ = settled_tx.send(());
            }
            // settled_tx drops here, so settled_rx gets Disconnected
            // once every handle has been joined.
        });

        loop {
            if self.force_stop.load(Ordering::Relaxed) {
                crate::debug!("drain_translations: force-stop signaled, detaching remaining translation threads");
                return;
            }
            match settled_rx.recv_timeout(DRAIN_POLL_INTERVAL) {
                Ok(()) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn dispatch_translation(&self, segment_id: u64, text: String) {
        let translator = self.translator.clone();
        let persisted = self.persisted.clone();
        let store = self.store.clone();
        let on_update = self.on_update.clone();
        let target_language = self.target_language.clone();

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
                persisted.translation_context(segment_id, &target_language)
            };

            let began = Instant::now();
            match translator.translate(&text, &context) {
                Ok(translation) => {
                    crate::debug!(
                        "translate: segment {segment_id} ok in {:.2}s{} \"{}\"",
                        began.elapsed().as_secs_f64(),
                        if translation.degraded {
                            " (degraded)"
                        } else {
                            ""
                        },
                        crate::log::preview(&translation.text, 60),
                    );
                    if let Err(e) = Self::record_translation(
                        persisted,
                        store,
                        on_update,
                        segment_id,
                        translation.text,
                        translation.degraded,
                    ) {
                        crate::debug!("translation persist failed: {e}");
                    }
                }
                Err(e) => {
                    crate::debug!(
                        "translate: segment {segment_id} failed after {:.2}s: {e}",
                        began.elapsed().as_secs_f64()
                    );
                    if e.is_misconfigured() {
                        if let Err(store_err) = Self::record_translation_error(
                            persisted,
                            on_update,
                            segment_id,
                            e.to_string(),
                        ) {
                            crate::debug!("translation error persist failed: {store_err}");
                        }
                    }
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
        store: Arc<Mutex<Store>>,
        on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
        segment_id: u64,
        text: String,
        degraded: bool,
    ) -> Result<(), PipelineError> {
        {
            let persisted = persisted.lock().expect("pipeline mutex poisoned");
            if !persisted.row_index.contains_key(&segment_id) {
                return Ok(());
            }
        }

        // Persisted first, like before, but the fsync now runs under
        // the store lock only (store -> persisted lock order).
        let mut store = store.lock().expect("store mutex poisoned");
        store.append_record(&Record::Translation {
            segment_id,
            text: text.clone(),
            degraded,
        })?;
        drop(store);

        Self::mutate_row(persisted, on_update, segment_id, |row| {
            row.translation = Some(text);
            row.degraded = degraded;
            row.translation_error = None;
        })
    }

    fn record_translation_error(
        persisted: Arc<Mutex<Persisted>>,
        on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
        segment_id: u64,
        message: String,
    ) -> Result<(), PipelineError> {
        Self::mutate_row(persisted, on_update, segment_id, |row| {
            row.translation_error = Some(message);
        })
    }

    // Shared shape of the per-row updates: lock, find by id (a row that
    // left the live view is a no-op), mutate, then notify with a fresh
    // snapshot outside the lock.
    fn mutate_row(
        persisted: Arc<Mutex<Persisted>>,
        on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
        segment_id: u64,
        mutate: impl FnOnce(&mut Segment),
    ) -> Result<(), PipelineError> {
        let snapshot = {
            let mut persisted = persisted.lock().expect("pipeline mutex poisoned");
            let Some(&position) = persisted.row_index.get(&segment_id) else {
                return Ok(());
            };

            mutate(&mut persisted.live_view[position]);
            persisted.live_view.clone()
        };

        if let Some(callback) = on_update.lock().expect("on_update mutex poisoned").as_ref() {
            callback(snapshot);
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

    // Poll a condition instead of sleeping a fixed duration: a 50ms
    // sleep expires before a delayed thread runs under CI load, and the
    // failure then looks like a real regression.
    fn wait_until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the expected condition"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
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
            "en",
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
            "en",
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
            language: None,
        });
        first_handle.join().unwrap();
        wait_until(|| translator.recorded_calls().len() == 1);
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
            language: None,
        });
        second_handle.join().unwrap();
        wait_until(|| translator.recorded_calls().len() == 1);
        assert_eq!(
            translator.recorded_calls(),
            vec![("segment 1".to_string(), vec![])]
        );

        first_translate.respond("t1".to_string());

        wait_until(|| translator.recorded_calls().len() == 2);
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
            "en",
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
            language: None,
        });
        first_handle.join().unwrap();
        wait_until(|| translator.recorded_calls().len() == 1);
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
            language: None,
        });
        second_handle.join().unwrap();
        wait_until(|| translator.recorded_calls().len() == 1);
        assert_eq!(
            translator.recorded_calls(),
            vec![("segment 1".to_string(), vec![])]
        );

        first_translate.fail(crate::translator::TranslateError::unavailable(
            "forced failure",
        ));

        wait_until(|| translator.recorded_calls().len() == 2);
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
        let pipeline = Arc::new(Pipeline::new(transcriber, translator, store, 0.0, "en"));

        let snapshots: Arc<Mutex<Vec<Vec<Segment>>>> = Arc::new(Mutex::new(Vec::new()));
        let snapshots_for_cb = snapshots.clone();
        let on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>> =
            Arc::new(Mutex::new(Some(Box::new(move |snapshot: Vec<Segment>| {
                snapshots_for_cb.lock().unwrap().push(snapshot);
            }))));

        Pipeline::record_translation(
            pipeline.persisted.clone(),
            pipeline.store.clone(),
            on_update.clone(),
            123,
            "ignored".to_string(),
            false,
        )
        .unwrap();

        assert!(snapshots.lock().unwrap().is_empty());
        assert!(fs::read_to_string(path).unwrap().is_empty());
    }

    #[test]
    fn record_translation_error_unknown_segment_id_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let store = Store::open(&path).unwrap();

        let transcriber = Arc::new(FakeTranscriber::new());
        let translator = Arc::new(FakeTranslator::new());
        let pipeline = Arc::new(Pipeline::new(transcriber, translator, store, 0.0, "en"));

        let snapshots: Arc<Mutex<Vec<Vec<Segment>>>> = Arc::new(Mutex::new(Vec::new()));
        let snapshots_for_cb = snapshots.clone();
        let on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>> =
            Arc::new(Mutex::new(Some(Box::new(move |snapshot: Vec<Segment>| {
                snapshots_for_cb.lock().unwrap().push(snapshot);
            }))));

        Pipeline::record_translation_error(
            pipeline.persisted.clone(),
            on_update.clone(),
            123,
            "ignored".to_string(),
        )
        .unwrap();

        assert!(snapshots.lock().unwrap().is_empty());
        assert!(fs::read_to_string(path).unwrap().is_empty());
    }

    #[test]
    fn misconfigured_translation_error_flows_through_to_live_view_and_on_update() {
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
            "en",
        ));

        let snapshots: Arc<Mutex<Vec<Vec<Segment>>>> = Arc::new(Mutex::new(Vec::new()));
        let snapshots_for_cb = snapshots.clone();
        pipeline.set_on_update(move |segments| {
            snapshots_for_cb.lock().unwrap().push(segments);
        });

        let samples = vec![9i16; 8];
        let transcribe_call = transcriber.expect_call(samples.clone());
        let translate_call = translator.expect_call("segment 1");
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
            text: "segment 1".to_string(),
            mean_confidence: 0.95,
            language: None,
        });
        handle.join().unwrap();

        translate_call.fail(crate::translator::TranslateError::misconfigured(
            "bad model name",
        ));
        pipeline.drain_translations();

        let final_snapshot = snapshots.lock().unwrap().last().unwrap().clone();
        assert_eq!(final_snapshot.len(), 1);
        assert_eq!(final_snapshot[0].translation, None);
        assert_eq!(
            final_snapshot[0].translation_error.as_deref(),
            Some("bad model name")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn record_translation_leaves_live_view_pending_when_append_fails() {
        let store = Store::open(Path::new("/dev/full")).unwrap();
        let transcriber = Arc::new(FakeTranscriber::new());
        let translator = Arc::new(FakeTranslator::new());
        let pipeline = Arc::new(Pipeline::new(transcriber, translator, store, 0.0, "en"));

        pipeline
            .persisted
            .lock()
            .expect("pipeline mutex poisoned")
            .insert_live_row(Segment {
                id: 1,
                speaker_tag: SpeakerTag::Me,
                start_ms: 0,
                end_ms: 400,
                text: "segment 1".to_string(),
                mean_confidence: 0.95,
                source_language: None,
                translation: None,
                degraded: false,
                state: SegmentState::Ready,
                translation_error: None,
            });

        let snapshots: Arc<Mutex<Vec<Vec<Segment>>>> = Arc::new(Mutex::new(Vec::new()));
        let snapshots_for_cb = snapshots.clone();
        let on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>> =
            Arc::new(Mutex::new(Some(Box::new(move |snapshot: Vec<Segment>| {
                snapshots_for_cb.lock().unwrap().push(snapshot);
            }))));

        let err = Pipeline::record_translation(
            pipeline.persisted.clone(),
            pipeline.store.clone(),
            on_update,
            1,
            "t1".to_string(),
            false,
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
        let pipeline = Arc::new(Pipeline::new(fake.clone(), translator, store, 0.0, "en"));

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

        // Wait until both reader threads have called `begin` before
        // either transcription completes (an expectation leaves the
        // pending map only once its transcribe call starts, which is
        // after begin), so the later segment can't flush past the
        // still-open earlier span.
        wait_until(|| fake.pending_count() == 0);

        // Release the segment with the LATER start_ms first: its
        // callback fires before the earlier segment's, but the
        // live-view snapshot it delivers must still be sorted.
        later_call.respond(Transcription {
            text: "later reply, arrives first".to_string(),
            mean_confidence: 0.95,
            language: None,
        });
        earlier_call.respond(Transcription {
            text: "earlier reply, arrives last".to_string(),
            mean_confidence: 0.9,
            language: None,
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
            "en",
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
            language: None,
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

    #[test]
    fn segment_with_all_translate_backends_exhausted_keeps_original_text_untranslated() {
        use crate::translator::FallbackTranslator;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let store = Store::open(&path).unwrap();

        let transcriber = Arc::new(FakeTranscriber::new());
        let backend0 = Arc::new(FakeTranslator::new());
        let backend1 = Arc::new(FakeTranslator::new());
        let translator = Arc::new(FallbackTranslator::new(
            vec![
                backend0.clone() as Arc<dyn crate::translator::Translator>,
                backend1.clone() as Arc<dyn crate::translator::Translator>,
            ],
            Duration::from_secs(60),
        ));
        let pipeline = Arc::new(Pipeline::new(
            transcriber.clone(),
            translator.clone(),
            store,
            0.0,
            "en",
        ));

        let samples = vec![9i16; 8];
        let transcribe_call = transcriber.expect_call(samples.clone());
        let call0 = backend0.expect_call("segment 1");
        let call1 = backend1.expect_call("segment 1");
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
            text: "segment 1".to_string(),
            mean_confidence: 0.95,
            language: None,
        });
        handle.join().unwrap();

        call0.fail(crate::translator::TranslateError::unavailable(
            "backend0 down",
        ));
        call1.fail(crate::translator::TranslateError::unavailable(
            "backend1 down",
        ));
        pipeline.drain_translations();

        let live_view = pipeline
            .persisted
            .lock()
            .expect("pipeline mutex poisoned")
            .live_view
            .clone();
        assert_eq!(live_view.len(), 1);
        assert_eq!(live_view[0].text, "segment 1");
        assert_eq!(live_view[0].translation, None);
        assert!(!live_view[0].degraded);

        let on_disk = crate::store::read_all(&path).unwrap();
        assert_eq!(on_disk.len(), 1);
        assert_eq!(on_disk[0].text, "segment 1");
        assert_eq!(on_disk[0].translation, None);
    }

    // Everything a language/in-flight test needs: a store on disk, a
    // pipeline translating into `target_language`, and the snapshots
    // its callback has fired so far.
    struct Harness {
        path: std::path::PathBuf,
        pipeline: Arc<Pipeline>,
        transcriber: Arc<FakeTranscriber>,
        translator: Arc<FakeTranslator>,
        snapshots: Arc<Mutex<Vec<Vec<Segment>>>>,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        fn new(target_language: &str, confidence_floor: f32) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("transcript.jsonl");
            let store = Store::open(&path).unwrap();
            let transcriber = Arc::new(FakeTranscriber::new());
            let translator = Arc::new(FakeTranslator::new());
            let pipeline = Arc::new(Pipeline::new(
                transcriber.clone(),
                translator.clone(),
                store,
                confidence_floor,
                target_language,
            ));

            let snapshots: Arc<Mutex<Vec<Vec<Segment>>>> = Arc::new(Mutex::new(Vec::new()));
            let snapshots_for_cb = snapshots.clone();
            pipeline.set_on_update(move |segments| {
                snapshots_for_cb.lock().unwrap().push(segments);
            });

            Self {
                path,
                pipeline,
                transcriber,
                translator,
                snapshots,
                _dir: dir,
            }
        }

        fn snapshots(&self) -> Vec<Vec<Segment>> {
            self.snapshots.lock().unwrap().clone()
        }

        fn last_snapshot(&self) -> Vec<Segment> {
            self.snapshots().last().cloned().unwrap_or_default()
        }

        fn raw_store(&self) -> String {
            fs::read_to_string(&self.path).unwrap()
        }
    }

    fn spawn_submission(
        pipeline: &Arc<Pipeline>,
        speaker_tag: SpeakerTag,
        language: Option<&str>,
        samples: Vec<i16>,
    ) -> thread::JoinHandle<Result<(), PipelineError>> {
        let input = SegmentInput {
            speaker_tag,
            start_ms: 0,
            end_ms: 400,
            samples,
            language: language.map(str::to_string),
        };
        let pipeline = pipeline.clone();
        thread::spawn(move || pipeline.submit(input))
    }

    #[test]
    fn a_row_reaches_the_ui_before_its_transcription_and_is_filled_in_place() {
        let harness = Harness::new("ru", 0.0);
        let samples = vec![7i16; 8];
        let transcribe = harness.transcriber.expect_call(samples.clone());
        let translate = harness.translator.expect_call("hello");

        let handle = spawn_submission(&harness.pipeline, SpeakerTag::Me, None, samples);
        wait_until(|| !harness.snapshots().is_empty());

        let in_flight = harness.snapshots();
        assert_eq!(
            in_flight.len(),
            1,
            "the closed Segment must reach the UI before its transcription returns"
        );
        assert_eq!(in_flight[0].len(), 1);
        assert_eq!(in_flight[0][0].state, SegmentState::Transcribing);
        assert_eq!(in_flight[0][0].speaker_tag, SpeakerTag::Me);
        assert_eq!(in_flight[0][0].start_ms, 0);
        assert!(in_flight[0][0].text.is_empty());
        assert!(
            harness.raw_store().is_empty(),
            "an in-flight row must not be written to the transcript file"
        );

        transcribe.respond(Transcription {
            text: "hello".to_string(),
            mean_confidence: 0.95,
            language: Some("en".to_string()),
        });
        handle.join().unwrap().unwrap();
        translate.respond("привет".to_string());
        harness.pipeline.drain_translations();

        let final_snapshot = harness.last_snapshot();
        assert_eq!(
            final_snapshot.len(),
            1,
            "the row is filled, not appended to"
        );
        assert_eq!(final_snapshot[0].state, SegmentState::Ready);
        assert_eq!(final_snapshot[0].text, "hello");
        assert_eq!(final_snapshot[0].source_language.as_deref(), Some("en"));
        assert_eq!(final_snapshot[0].translation.as_deref(), Some("привет"));
    }

    #[test]
    fn a_row_below_the_confidence_floor_is_removed_from_the_ui() {
        let harness = Harness::new("ru", 0.6);
        let samples = vec![8i16; 8];
        let transcribe = harness.transcriber.expect_call(samples.clone());

        let handle = spawn_submission(&harness.pipeline, SpeakerTag::Them, None, samples);
        wait_until(|| !harness.snapshots().is_empty());
        assert_eq!(harness.last_snapshot().len(), 1);

        transcribe.respond(Transcription {
            text: "mumble".to_string(),
            mean_confidence: 0.2,
            language: Some("en".to_string()),
        });
        handle.join().unwrap().unwrap();

        assert!(
            harness.last_snapshot().is_empty(),
            "a discarded Segment must take its in-flight row with it"
        );
        assert!(harness.translator.recorded_calls().is_empty());
    }

    #[test]
    fn a_failed_transcription_leaves_its_row_marked_rather_than_vanishing() {
        let harness = Harness::new("ru", 0.0);
        let samples = vec![9i16; 8];
        let transcribe = harness.transcriber.expect_call(samples.clone());

        let handle = spawn_submission(&harness.pipeline, SpeakerTag::Me, None, samples);
        wait_until(|| !harness.snapshots().is_empty());
        transcribe.fail(TranscribeError::new("whisper returned 503"));

        let error = handle.join().unwrap().expect_err("transcription failed");
        assert!(error.to_string().contains("503"));

        let final_snapshot = harness.last_snapshot();
        assert_eq!(final_snapshot.len(), 1);
        assert_eq!(
            final_snapshot[0].state,
            SegmentState::Failed("whisper returned 503".to_string())
        );
        assert!(
            harness.raw_store().is_empty(),
            "a failed row must not be written to the transcript file"
        );
    }

    #[test]
    fn a_segment_already_in_the_target_language_is_not_sent_to_a_backend() {
        let harness = Harness::new("ru", 0.0);
        let samples = vec![10i16; 8];
        let transcribe = harness.transcriber.expect_call(samples.clone());

        let handle = spawn_submission(&harness.pipeline, SpeakerTag::Them, None, samples);
        transcribe.respond(Transcription {
            text: "уже по-русски".to_string(),
            mean_confidence: 0.95,
            language: Some("ru".to_string()),
        });
        handle.join().unwrap().unwrap();
        harness.pipeline.drain_translations();

        assert!(
            harness.translator.recorded_calls().is_empty(),
            "a same-language Segment must not reach a Translate backend"
        );

        let final_snapshot = harness.last_snapshot();
        assert_eq!(final_snapshot.len(), 1);
        assert_eq!(final_snapshot[0].text, "уже по-русски");
        assert_eq!(final_snapshot[0].source_language.as_deref(), Some("ru"));
        assert_eq!(final_snapshot[0].translation, None);

        let raw = harness.raw_store();
        assert!(raw.contains(r#""source_language":"ru""#));
        assert!(
            !raw.contains(r#""kind":"translation""#),
            "no translation record may be written for a same-language Segment, got {raw}"
        );
    }

    #[test]
    fn a_configured_source_language_overrides_what_the_server_detected() {
        let harness = Harness::new("ru", 0.0);
        let samples = vec![11i16; 8];
        let transcribe = harness.transcriber.expect_call(samples.clone());
        let translate = harness.translator.expect_call("guten tag");

        let handle = spawn_submission(&harness.pipeline, SpeakerTag::Me, Some("german"), samples);
        transcribe.respond(Transcription {
            text: "guten tag".to_string(),
            mean_confidence: 0.95,
            language: Some("en".to_string()),
        });
        handle.join().unwrap().unwrap();
        translate.respond("добрый день".to_string());
        harness.pipeline.drain_translations();

        let final_snapshot = harness.last_snapshot();
        assert_eq!(
            final_snapshot[0].source_language.as_deref(),
            Some("de"),
            "the forced language decides what the text is in, not the detection"
        );
        assert_eq!(harness.translator.recorded_calls().len(), 1);
    }

    #[test]
    fn a_segment_whose_language_is_unrecognisable_is_translated_anyway() {
        let harness = Harness::new("klingon", 0.0);
        let samples = vec![12i16; 8];
        let transcribe = harness.transcriber.expect_call(samples.clone());
        let translate = harness.translator.expect_call("Qapla'");

        let handle = spawn_submission(&harness.pipeline, SpeakerTag::Me, Some("klingon"), samples);
        transcribe.respond(Transcription {
            text: "Qapla'".to_string(),
            mean_confidence: 0.95,
            language: None,
        });
        handle.join().unwrap().unwrap();
        translate.respond("success".to_string());
        harness.pipeline.drain_translations();

        assert_eq!(
            harness.translator.recorded_calls().len(),
            1,
            "an unknown language must not be taken for the target language"
        );
        assert_eq!(
            harness.last_snapshot()[0].source_language.as_deref(),
            Some("klingon")
        );
    }

    #[test]
    fn a_same_language_line_is_context_for_the_next_translation() {
        let harness = Harness::new("ru", 0.0);

        let russian_samples = vec![13i16; 8];
        let russian = harness.transcriber.expect_call(russian_samples.clone());
        let input = SegmentInput {
            speaker_tag: SpeakerTag::Them,
            start_ms: 0,
            end_ms: 400,
            samples: russian_samples,
            language: None,
        };
        let pipeline = harness.pipeline.clone();
        let handle = thread::spawn(move || pipeline.submit(input));
        russian.respond(Transcription {
            text: "уже по-русски".to_string(),
            mean_confidence: 0.95,
            language: Some("ru".to_string()),
        });
        handle.join().unwrap().unwrap();

        let english_samples = vec![14i16; 8];
        let english = harness.transcriber.expect_call(english_samples.clone());
        let translate = harness.translator.expect_call("and now in english");
        let input = SegmentInput {
            speaker_tag: SpeakerTag::Me,
            start_ms: 1_000,
            end_ms: 1_400,
            samples: english_samples,
            language: None,
        };
        let pipeline = harness.pipeline.clone();
        let handle = thread::spawn(move || pipeline.submit(input));
        english.respond(Transcription {
            text: "and now in english".to_string(),
            mean_confidence: 0.95,
            language: Some("en".to_string()),
        });
        handle.join().unwrap().unwrap();
        translate.respond("а теперь по-английски".to_string());
        harness.pipeline.drain_translations();

        assert_eq!(
            harness.translator.recorded_calls(),
            vec![(
                "and now in english".to_string(),
                vec!["уже по-русски".to_string()]
            )],
            "target-language text already on screen is context, translated or not"
        );
    }

    #[test]
    fn force_stop_detaches_translation_threads_and_drain_returns_immediately() {
        let harness = Harness::new("ru", 0.0);

        // Submit a segment whose transcription completes but whose
        // translation is deliberately stuck (the handle is never
        // released).
        let samples = vec![15i16; 8];
        let transcribe = harness.transcriber.expect_call(samples.clone());
        let _stuck_translate = harness.translator.expect_call("stuck");

        let handle = spawn_submission(&harness.pipeline, SpeakerTag::Me, None, samples);
        transcribe.respond(Transcription {
            text: "stuck".to_string(),
            mean_confidence: 0.95,
            language: Some("en".to_string()),
        });
        handle.join().unwrap().unwrap();

        // At this point one translation thread is blocked waiting for
        // the stuck handle. Signal force-stop and drain: it must
        // return quickly rather than blocking on the stuck thread.
        harness.pipeline.signal_force_stop();
        let began = Instant::now();
        harness.pipeline.drain_translations();
        let elapsed = began.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "drain_translations after force_stop must return quickly, took {elapsed:?}"
        );
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
            language: None,
        });
        handle.join().unwrap();
        translate_call.respond(format!("t{id}"));
        pipeline.drain_translations();
    }
}
