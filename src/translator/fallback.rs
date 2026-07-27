use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{TranslateError, TranslateErrorKind, Translation, Translator};

/// Wraps an ordered list of backends so a Segment's translate() call
/// walks down the list on `Unavailable` errors, staying on whichever
/// backend last succeeded until a `reprobe_interval` has elapsed with
/// it still degraded. `Misconfigured` errors from the current backend
/// propagate without advancing: a bad model name or API key on backend
/// N doesn't get "fixed" by silently routing future segments to N+1.
pub struct FallbackTranslator {
    backends: Vec<Arc<dyn Translator>>,
    reprobe_interval: Duration,
    state: Mutex<State>,
}

struct State {
    current_index: usize,
    last_probe_at: Option<Instant>,
}

impl FallbackTranslator {
    pub fn new(backends: Vec<Arc<dyn Translator>>, reprobe_interval: Duration) -> Self {
        assert!(
            !backends.is_empty(),
            "FallbackTranslator requires at least one backend"
        );
        Self {
            backends,
            reprobe_interval,
            state: Mutex::new(State {
                current_index: 0,
                last_probe_at: None,
            }),
        }
    }

    // Starting the reprobe clock at the moment of failover (rather
    // than at the moment a reprobe attempt runs) is what keeps a
    // just-degraded chain from reprobing on the very next segment:
    // this call's `last_probe_at` write is what the next call's
    // elapsed-since check reads.
    fn maybe_reprobe(&self) {
        let current_index = {
            let state = self
                .state
                .lock()
                .expect("fallback translator mutex poisoned");
            if state.current_index == 0 {
                return;
            }

            let due = match state.last_probe_at {
                None => true,
                Some(at) => at.elapsed() >= self.reprobe_interval,
            };
            if !due {
                return;
            }

            state.current_index
        };

        let mut recovered = None;
        for candidate in 0..current_index {
            if self.backends[candidate].probe().is_ok() {
                recovered = Some(candidate);
                break;
            }
        }

        let mut state = self
            .state
            .lock()
            .expect("fallback translator mutex poisoned");
        if let Some(candidate) = recovered {
            state.current_index = candidate;
        }
        state.last_probe_at = Some(Instant::now());
    }
}

impl Translator for FallbackTranslator {
    fn translate(&self, text: &str, context: &[String]) -> Result<Translation, TranslateError> {
        self.maybe_reprobe();

        loop {
            let index = self
                .state
                .lock()
                .expect("fallback translator mutex poisoned")
                .current_index;

            match self.backends[index].translate(text, context) {
                Ok(translation) => {
                    return Ok(Translation {
                        text: translation.text,
                        degraded: index > 0,
                    });
                }
                Err(e) if e.kind() == TranslateErrorKind::Unavailable => {
                    let mut state = self
                        .state
                        .lock()
                        .expect("fallback translator mutex poisoned");
                    if state.current_index == index {
                        if index + 1 >= self.backends.len() {
                            return Err(e);
                        }
                        state.current_index = index + 1;
                        state.last_probe_at = Some(Instant::now());
                    }
                    // else: some other path already advanced past this
                    // point; drop this stale write and let the loop
                    // re-read the fresh index instead of regressing it.
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn probe(&self) -> Result<(), TranslateError> {
        let index = self
            .state
            .lock()
            .expect("fallback translator mutex poisoned")
            .current_index;
        self.backends[index].probe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translator::FakeTranslator;
    use std::sync::Arc;
    use std::thread;

    fn translate_async(
        fallback: &Arc<FallbackTranslator>,
        text: &'static str,
    ) -> thread::JoinHandle<Result<Translation, TranslateError>> {
        let fallback = fallback.clone();
        thread::spawn(move || fallback.translate(text, &[]))
    }

    // 1.1
    #[test]
    fn failing_primary_falls_over_and_marks_result_degraded() {
        let backend0 = Arc::new(FakeTranslator::new());
        let backend1 = Arc::new(FakeTranslator::new());
        let fallback = Arc::new(FallbackTranslator::new(
            vec![
                backend0.clone() as Arc<dyn Translator>,
                backend1.clone() as Arc<dyn Translator>,
            ],
            Duration::from_secs(60),
        ));

        let call0 = backend0.expect_call("segment 1");
        let call1 = backend1.expect_call("segment 1");
        let handle = translate_async(&fallback, "segment 1");

        call0.fail(TranslateError::unavailable("backend0 down"));
        call1.respond("t1".to_string());

        let result = handle.join().unwrap().unwrap();
        assert_eq!(result.text, "t1");
        assert!(result.degraded);

        let call1_second = backend1.expect_call("segment 2");
        let handle2 = translate_async(&fallback, "segment 2");
        call1_second.respond("t2".to_string());
        let result2 = handle2.join().unwrap().unwrap();

        assert_eq!(result2.text, "t2");
        assert!(result2.degraded);
        assert_eq!(
            backend0.recorded_calls().len(),
            1,
            "backend0 must not be consulted again for the following segment"
        );
    }

    // 1.3
    #[test]
    fn healthy_primary_backend_stays_selected_and_secondary_is_untouched() {
        let backend0 = Arc::new(FakeTranslator::new());
        let backend1 = Arc::new(FakeTranslator::new());
        let fallback = Arc::new(FallbackTranslator::new(
            vec![
                backend0.clone() as Arc<dyn Translator>,
                backend1.clone() as Arc<dyn Translator>,
            ],
            Duration::from_secs(60),
        ));

        let call1 = backend0.expect_call("segment 1");
        let handle1 = translate_async(&fallback, "segment 1");
        call1.respond("t1".to_string());
        let result1 = handle1.join().unwrap().unwrap();

        let call2 = backend0.expect_call("segment 2");
        let handle2 = translate_async(&fallback, "segment 2");
        call2.respond("t2".to_string());
        let result2 = handle2.join().unwrap().unwrap();

        assert!(!result1.degraded);
        assert!(!result2.degraded);
        assert!(backend1.recorded_calls().is_empty());
    }

    // 1.4
    #[test]
    fn misconfigured_error_does_not_advance_and_repeats_on_same_backend() {
        let backend0 = Arc::new(FakeTranslator::new());
        let backend1 = Arc::new(FakeTranslator::new());
        let fallback = Arc::new(FallbackTranslator::new(
            vec![
                backend0.clone() as Arc<dyn Translator>,
                backend1.clone() as Arc<dyn Translator>,
            ],
            Duration::from_secs(60),
        ));

        let call1 = backend0.expect_call("segment 1");
        let handle1 = translate_async(&fallback, "segment 1");
        call1.fail(TranslateError::misconfigured("bad api key"));
        let err = handle1.join().unwrap().unwrap_err();
        assert!(err.is_misconfigured());

        let call2 = backend0.expect_call("segment 2");
        let handle2 = translate_async(&fallback, "segment 2");
        call2.respond("t2".to_string());
        let result2 = handle2.join().unwrap().unwrap();

        assert_eq!(result2.text, "t2");
        assert!(!result2.degraded);
        assert_eq!(backend0.recorded_calls().len(), 2);
        assert!(backend1.recorded_calls().is_empty());
    }

    // 1.6
    #[test]
    fn after_reprobe_interval_elapses_next_translate_call_reprobes_and_recovers_primary() {
        let backend0 = Arc::new(FakeTranslator::new());
        let backend1 = Arc::new(FakeTranslator::new());
        let fallback = Arc::new(FallbackTranslator::new(
            vec![
                backend0.clone() as Arc<dyn Translator>,
                backend1.clone() as Arc<dyn Translator>,
            ],
            Duration::from_millis(20),
        ));

        let call0 = backend0.expect_call("segment 1");
        let call1 = backend1.expect_call("segment 1");
        let handle1 = translate_async(&fallback, "segment 1");
        call0.fail(TranslateError::unavailable("backend0 down"));
        call1.respond("t1".to_string());
        let result1 = handle1.join().unwrap().unwrap();
        assert!(result1.degraded);

        thread::sleep(Duration::from_millis(50));

        let probe = backend0.expect_probe();
        let call0_second = backend0.expect_call("segment 2");
        let handle2 = translate_async(&fallback, "segment 2");
        probe.succeed();
        call0_second.respond("t2".to_string());
        let result2 = handle2.join().unwrap().unwrap();

        assert_eq!(result2.text, "t2");
        assert!(!result2.degraded);
        assert_eq!(backend0.recorded_probes(), 1);
        assert_eq!(
            backend1.recorded_calls().len(),
            1,
            "backend1 must receive no further calls after recovery"
        );
    }

    #[test]
    fn failed_reprobe_leaves_fallback_backend_serving_and_index_unchanged() {
        let backend0 = Arc::new(FakeTranslator::new());
        let backend1 = Arc::new(FakeTranslator::new());
        let fallback = Arc::new(FallbackTranslator::new(
            vec![
                backend0.clone() as Arc<dyn Translator>,
                backend1.clone() as Arc<dyn Translator>,
            ],
            Duration::from_millis(20),
        ));

        let call0 = backend0.expect_call("segment 1");
        let call1 = backend1.expect_call("segment 1");
        let handle1 = translate_async(&fallback, "segment 1");
        call0.fail(TranslateError::unavailable("backend0 down"));
        call1.respond("t1".to_string());
        let result1 = handle1.join().unwrap().unwrap();
        assert!(result1.degraded);

        thread::sleep(Duration::from_millis(50));

        let probe = backend0.expect_probe();
        let call1_second = backend1.expect_call("segment 2");
        let handle2 = translate_async(&fallback, "segment 2");
        probe.fail(TranslateError::unavailable("backend0 still down"));
        call1_second.respond("t2".to_string());
        let result2 = handle2.join().unwrap().unwrap();

        assert_eq!(result2.text, "t2");
        assert!(
            result2.degraded,
            "failed probe must not recover backend0; backend1 keeps serving"
        );
        assert_eq!(backend0.recorded_probes(), 1);
        assert_eq!(
            backend0.recorded_calls().len(),
            1,
            "backend0 must not be consulted for translate after a failed probe"
        );
        assert_eq!(
            backend1.recorded_calls().len(),
            2,
            "segment 2 must still be served by backend1, not the probed backend0"
        );
    }

    // 1.8
    #[test]
    fn when_all_backends_fail_the_final_error_is_returned_unavailable() {
        let backend0 = Arc::new(FakeTranslator::new());
        let backend1 = Arc::new(FakeTranslator::new());
        let fallback = Arc::new(FallbackTranslator::new(
            vec![
                backend0.clone() as Arc<dyn Translator>,
                backend1.clone() as Arc<dyn Translator>,
            ],
            Duration::from_secs(60),
        ));

        let call0 = backend0.expect_call("segment 1");
        let call1 = backend1.expect_call("segment 1");
        let handle = translate_async(&fallback, "segment 1");

        call0.fail(TranslateError::unavailable("backend0 down"));
        call1.fail(TranslateError::unavailable("backend1 down"));

        let err = handle.join().unwrap().unwrap_err();
        assert_eq!(err.kind(), TranslateErrorKind::Unavailable);
    }
}
