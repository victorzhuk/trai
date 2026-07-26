use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::confidence::passes_floor;
use crate::domain::{Segment, SpeakerTag};
use crate::store::Store;
use crate::transcriber::{TranscribeError, Transcriber};
use crate::transcript::insert_ordered;

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
}

// Snapshot callback fired from `finish_pending` with a full, sorted
// copy of `live_view` each time a Segment passes the confidence floor.
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
    confidence_floor: f32,
    next_id: AtomicU64,
    persisted: Mutex<Persisted>,
    on_update: Mutex<Option<TranscriptUpdateCallback>>,
}

impl Pipeline {
    pub fn new(transcriber: Arc<dyn Transcriber>, store: Store, confidence_floor: f32) -> Self {
        Self {
            transcriber,
            confidence_floor,
            next_id: AtomicU64::new(1),
            persisted: Mutex::new(Persisted {
                transcript: Vec::new(),
                live_view: Vec::new(),
                store,
                in_flight: BTreeMap::new(),
            }),
            on_update: Mutex::new(None),
        }
    }

    /// Registers a callback fired (off the Pipeline's internal lock)
    /// with a full, sorted snapshot of `live_view` every time a
    /// Segment passes the confidence floor. No-op if never called.
    /// The callback receives `Vec<Segment>` by value and must be
    /// `Send + Sync` because it is invoked from the submission
    /// threads, not the UI thread.
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

        // Clone the live-view snapshot under the lock, run the disk
        // flush under the lock, then drop the guard before invoking
        // the user-supplied callback -- the callback may call back
        // into slint::invoke_from_event_loop and must never run with
        // the Pipeline's persisted lock held.
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
            };

            let mut snapshot = None;
            if passes_floor(&segment, self.confidence_floor) {
                insert_ordered(&mut persisted.transcript, segment.clone());
                insert_ordered(&mut persisted.live_view, segment);
                snapshot = Some(persisted.live_view.clone());
            }

            persisted.flush_ready_prefix()?;
            snapshot
        };

        if let Some(snapshot) = snapshot {
            if let Some(callback) = self
                .on_update
                .lock()
                .expect("on_update mutex poisoned")
                .as_ref()
            {
                callback(snapshot);
            }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SpeakerTag;
    use crate::transcriber::{FakeTranscriber, Transcription};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

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
        let pipeline = Arc::new(Pipeline::new(fake.clone(), store, 0.0));

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
}
