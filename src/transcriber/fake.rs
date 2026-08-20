use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;

use super::{TranscribeError, Transcriber, Transcription};

type Outcome = Result<Transcription, TranscribeError>;

/// Test-only Transcriber whose calls block until a test explicitly
/// releases them, so completion order is decided by the test rather
/// than by wall-clock timing.
pub struct FakeTranscriber {
    pending: Mutex<HashMap<Vec<i16>, Receiver<Outcome>>>,
}

impl FakeTranscriber {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Registers the canned outcome for the next call whose `samples`
    /// argument equals `samples`. `respond`/`fail` on the returned
    /// handle rendezvous with the matching `transcribe()` call, so
    /// releasing it blocks until that call has actually received it.
    pub fn expect_call(&self, samples: Vec<i16>) -> CallHandle {
        let (tx, rx) = sync_channel(0);
        self.pending
            .lock()
            .expect("fake transcriber mutex poisoned")
            .insert(samples, rx);
        CallHandle { tx }
    }

    /// Expectations not yet consumed by a `transcribe()` call. A test
    /// polling this to zero knows every expected call has started.
    pub fn pending_count(&self) -> usize {
        self.pending
            .lock()
            .expect("fake transcriber mutex poisoned")
            .len()
    }
}

impl Default for FakeTranscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl Transcriber for FakeTranscriber {
    fn transcribe(&self, samples: &[i16], _language: Option<&str>) -> Outcome {
        let rx = self
            .pending
            .lock()
            .expect("fake transcriber mutex poisoned")
            .remove(samples)
            .ok_or_else(|| TranscribeError::new("no expectation registered for these samples"))?;
        rx.recv()
            .map_err(|_| TranscribeError::new("call handle dropped before responding"))?
    }
}

pub struct CallHandle {
    tx: SyncSender<Outcome>,
}

impl CallHandle {
    pub fn respond(self, transcription: Transcription) {
        let _ = self.tx.send(Ok(transcription));
    }

    pub fn fail(self, error: TranscribeError) {
        let _ = self.tx.send(Err(error));
    }
}
