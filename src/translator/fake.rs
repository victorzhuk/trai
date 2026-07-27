use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;

use super::{TranslateError, Translator};

type Outcome = Result<String, TranslateError>;

type PendingCall = Receiver<Outcome>;

/// Test-only Translator whose calls block until a test explicitly
/// releases them, so completion order is decided by the test rather
/// than by wall-clock timing.
pub struct FakeTranslator {
    pending: Mutex<HashMap<String, PendingCall>>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl FakeTranslator {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Registers the canned outcome for the next call whose `text`
    /// argument equals `text`. `respond`/`fail` on the returned
    /// handle rendezvous with the matching `translate()` call, so
    /// releasing it blocks until that call has actually received it.
    pub fn expect_call(&self, text: impl Into<String>) -> CallHandle {
        let (tx, rx) = sync_channel(0);
        self.pending
            .lock()
            .expect("fake translator mutex poisoned")
            .insert(text.into(), rx);
        CallHandle { tx }
    }

    pub fn recorded_calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls
            .lock()
            .expect("fake translator mutex poisoned")
            .clone()
    }
}

impl Default for FakeTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for FakeTranslator {
    fn translate(&self, text: &str, context: &[String]) -> Outcome {
        self.calls
            .lock()
            .expect("fake translator mutex poisoned")
            .push((text.to_string(), context.to_vec()));

        let rx = self
            .pending
            .lock()
            .expect("fake translator mutex poisoned")
            .remove(text)
            .ok_or_else(|| TranslateError::new("no expectation registered for this text"))?;

        rx.recv()
            .map_err(|_| TranslateError::new("call handle dropped before responding"))?
    }
}

pub struct CallHandle {
    tx: SyncSender<Outcome>,
}

impl CallHandle {
    pub fn respond(self, translation: String) {
        let _ = self.tx.send(Ok(translation));
    }

    pub fn fail(self, error: TranslateError) {
        let _ = self.tx.send(Err(error));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn blocks_until_test_releases_call_and_records_args() {
        let fake = Arc::new(FakeTranslator::new());
        let call = fake.expect_call("hello");
        let fake_for_thread = fake.clone();

        let handle = thread::spawn(move || fake_for_thread.translate("hello", &["t1".to_string()]));

        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            fake.recorded_calls(),
            vec![("hello".to_string(), vec!["t1".to_string()])]
        );

        call.respond("hola".to_string());
        assert_eq!(handle.join().unwrap().unwrap(), "hola");
    }
}
