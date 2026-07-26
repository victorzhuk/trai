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
}

impl Pipeline {
    pub fn new(transcriber: Arc<dyn Transcriber>, store: Store, confidence_floor: f32) -> Self {
        Self {
            transcriber,
            confidence_floor,
            next_id: AtomicU64::new(1),
            persisted: Mutex::new(Persisted {
                transcript: Vec::new(),
                store,
                in_flight: BTreeMap::new(),
            }),
        }
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

        if passes_floor(&segment, self.confidence_floor) {
            insert_ordered(&mut persisted.transcript, segment);
        }

        persisted.flush_ready_prefix()
    }

    /// Convenience for callers with no separate open-span phase (existing
    /// tests use this): begin + finish_pending combined, net-identical
    /// behavior to before this change.
    pub fn submit(&self, input: SegmentInput) -> Result<(), PipelineError> {
        self.begin(input.start_ms);
        self.finish_pending(input)
    }
}
