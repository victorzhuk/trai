use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Mutex;

use super::{TranslateError, Translation, Translator};

type Outcome = Result<Translation, TranslateError>;
type ProbeOutcome = Result<(), TranslateError>;

type PendingCall = Receiver<Outcome>;
type PendingProbe = Receiver<ProbeOutcome>;

/// Test-only Translator whose calls block until a test explicitly
/// releases them, so completion order is decided by the test rather
/// than by wall-clock timing.
pub struct FakeTranslator {
    pending: Mutex<HashMap<String, PendingCall>>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
    pending_probes: Mutex<VecDeque<PendingProbe>>,
    probe_count: Mutex<usize>,
}

impl FakeTranslator {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
            pending_probes: Mutex::new(VecDeque::new()),
            probe_count: Mutex::new(0),
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

    /// Queues the canned outcome for the next `probe()` call, FIFO.
    /// `respond` on the returned handle rendezvous with that call.
    pub fn expect_probe(&self) -> ProbeHandle {
        let (tx, rx) = sync_channel(0);
        self.pending_probes
            .lock()
            .expect("fake translator mutex poisoned")
            .push_back(rx);
        ProbeHandle { tx }
    }

    pub fn recorded_probes(&self) -> usize {
        *self
            .probe_count
            .lock()
            .expect("fake translator mutex poisoned")
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
            .ok_or_else(|| {
                TranslateError::unavailable("no expectation registered for this text")
            })?;

        rx.recv()
            .map_err(|_| TranslateError::unavailable("call handle dropped before responding"))?
    }

    fn probe(&self) -> ProbeOutcome {
        *self
            .probe_count
            .lock()
            .expect("fake translator mutex poisoned") += 1;

        let rx = self
            .pending_probes
            .lock()
            .expect("fake translator mutex poisoned")
            .pop_front()
            .ok_or_else(|| {
                TranslateError::unavailable("no expectation registered for this probe")
            })?;

        rx.recv()
            .map_err(|_| TranslateError::unavailable("probe handle dropped before responding"))?
    }
}

pub struct CallHandle {
    tx: SyncSender<Outcome>,
}

impl CallHandle {
    pub fn respond(self, translation: String) {
        let _ = self.tx.send(Ok(Translation {
            text: translation,
            degraded: false,
        }));
    }

    pub fn fail(self, error: TranslateError) {
        let _ = self.tx.send(Err(error));
    }
}

pub struct ProbeHandle {
    tx: SyncSender<ProbeOutcome>,
}

impl ProbeHandle {
    pub fn succeed(self) {
        let _ = self.tx.send(Ok(()));
    }

    pub fn fail(self, error: TranslateError) {
        let _ = self.tx.send(Err(error));
    }

    pub fn respond(self, outcome: ProbeOutcome) {
        let _ = self.tx.send(outcome);
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
        assert_eq!(handle.join().unwrap().unwrap().text, "hola");
    }

    #[test]
    fn probe_without_expectation_fails_immediately_instead_of_blocking() {
        let fake = FakeTranslator::new();
        let err = fake.probe().expect_err("unexpected probe should fail");
        assert!(err.to_string().contains("no expectation registered"));
    }

    #[test]
    fn probes_are_served_fifo_and_counted() {
        let fake = Arc::new(FakeTranslator::new());
        let first = fake.expect_probe();
        let second = fake.expect_probe();

        let fake_for_first = fake.clone();
        let first_handle = thread::spawn(move || fake_for_first.probe());
        first.succeed();
        assert!(first_handle.join().unwrap().is_ok());

        let fake_for_second = fake.clone();
        let second_handle = thread::spawn(move || fake_for_second.probe());
        second.fail(TranslateError::unavailable("down"));
        assert!(second_handle.join().unwrap().is_err());

        assert_eq!(fake.recorded_probes(), 2);
    }
}
