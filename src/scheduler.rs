use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::domain::Segment;
use crate::pipeline::{
    Persisted, PipelineError, SegmentSink, TranscriptUpdate, TranscriptUpdateCallback,
};
use crate::translator::Translator;

// How often drain wakes to check force-stop while waiting on the
// worker to settle the queue.
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(100);

struct TranslationJob {
    segment_id: u64,
    text: String,
}

// Decrements the outstanding-translation counter and wakes drain
// when a job settles, even on panic.
struct TranslationSettled<'a>(&'a Arc<(Mutex<usize>, Condvar)>);

impl Drop for TranslationSettled<'_> {
    fn drop(&mut self) {
        let (lock, cvar) = &**self.0;
        let mut outstanding = lock.lock().expect("translation count mutex poisoned");
        *outstanding -= 1;
        cvar.notify_all();
    }
}

/// Serial translation scheduling for the Pipeline: one worker thread
/// draining a FIFO queue, so a slow or dead backend holds exactly one
/// thread no matter how many Segments queue up behind it.
pub(crate) struct TranslationScheduler {
    tx: Sender<TranslationJob>,
    // Queued plus running jobs; drain waits on the condvar until this
    // reaches zero.
    in_flight: Arc<(Mutex<usize>, Condvar)>,
}

impl TranslationScheduler {
    pub(crate) fn new(
        translator: Arc<dyn Translator>,
        persisted: Arc<Mutex<Persisted>>,
        store: Arc<Mutex<dyn SegmentSink>>,
        on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
        target_language: String,
    ) -> Self {
        let (tx, rx) = channel();
        let in_flight = Arc::new((Mutex::new(0usize), Condvar::new()));

        thread::spawn({
            let in_flight = in_flight.clone();
            move || {
                Self::worker(
                    rx,
                    translator,
                    persisted,
                    store,
                    on_update,
                    target_language,
                    in_flight,
                );
            }
        });

        Self { tx, in_flight }
    }

    pub(crate) fn schedule(&self, segment_id: u64, text: String) {
        {
            let (lock, _) = &*self.in_flight;
            *lock.lock().expect("translation count mutex poisoned") += 1;
        }
        if self.tx.send(TranslationJob { segment_id, text }).is_err() {
            // The worker is gone, which cannot happen while the Pipeline
            // lives; if it ever does, don't leave drain waiting on a job
            // that will never finish.
            let (lock, cvar) = &*self.in_flight;
            let mut outstanding = lock.lock().expect("translation count mutex poisoned");
            *outstanding -= 1;
            cvar.notify_all();
        }
    }

    pub(crate) fn drain(&self, force_stop: &AtomicBool) {
        let (lock, cvar) = &*self.in_flight;
        let mut outstanding = lock.lock().expect("translation count mutex poisoned");
        loop {
            if *outstanding == 0 {
                return;
            }
            if force_stop.load(Ordering::Relaxed) {
                crate::debug!(
                    "drain_translations: force-stop signaled, detaching queued translations"
                );
                return;
            }
            let (guard, _) = cvar
                .wait_timeout(outstanding, DRAIN_POLL_INTERVAL)
                .expect("translation count mutex poisoned");
            outstanding = guard;
        }
    }

    fn worker(
        rx: Receiver<TranslationJob>,
        translator: Arc<dyn Translator>,
        persisted: Arc<Mutex<Persisted>>,
        store: Arc<Mutex<dyn SegmentSink>>,
        on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
        target_language: String,
        in_flight: Arc<(Mutex<usize>, Condvar)>,
    ) {
        while let Ok(job) = rx.recv() {
            // Decrements the counter and wakes drain even if the job
            // panics mid-flight.
            let _settled = TranslationSettled(&in_flight);

            let context = {
                let persisted = persisted.lock().expect("pipeline mutex poisoned");
                persisted.translation_context(job.segment_id, &target_language)
            };

            let began = Instant::now();
            match translator.translate(&job.text, &context) {
                Ok(translation) => {
                    crate::debug!(
                        "translate: segment {} ok in {:.2}s{} \"{}\"",
                        job.segment_id,
                        began.elapsed().as_secs_f64(),
                        if translation.degraded {
                            " (degraded)"
                        } else {
                            ""
                        },
                        crate::log::preview(&translation.text, 60),
                    );
                    if let Err(e) = Self::record_translation(
                        persisted.clone(),
                        store.clone(),
                        on_update.clone(),
                        job.segment_id,
                        translation.text,
                        translation.degraded,
                    ) {
                        crate::debug!("translation persist failed: {e}");
                    }
                }
                Err(e) => {
                    crate::debug!(
                        "translate: segment {} failed after {:.2}s: {e}",
                        job.segment_id,
                        began.elapsed().as_secs_f64()
                    );
                    if e.is_misconfigured() {
                        if let Err(store_err) = Self::record_translation_error(
                            persisted.clone(),
                            on_update.clone(),
                            job.segment_id,
                            e.to_string(),
                        ) {
                            crate::debug!("translation error persist failed: {store_err}");
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn record_translation(
        persisted: Arc<Mutex<Persisted>>,
        store: Arc<Mutex<dyn SegmentSink>>,
        on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
        segment_id: u64,
        text: String,
        degraded: bool,
    ) -> Result<(), PipelineError> {
        {
            let persisted = persisted.lock().expect("pipeline mutex poisoned");
            if !persisted.contains_row(segment_id) {
                return Ok(());
            }
        }

        // Persisted first, but the fsync runs under the store lock only
        // (store -> persisted lock order).
        let mut store = store.lock().expect("store mutex poisoned");
        store.append_translation(segment_id, &text, degraded)?;
        drop(store);

        Self::mutate_row(persisted, on_update, segment_id, |row| {
            row.translation = Some(text);
            row.degraded = degraded;
            row.translation_error = None;
        })
    }

    pub(crate) fn record_translation_error(
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
    // left the live view is a no-op), mutate, then notify with the
    // changed row outside the lock.
    fn mutate_row(
        persisted: Arc<Mutex<Persisted>>,
        on_update: Arc<Mutex<Option<TranscriptUpdateCallback>>>,
        segment_id: u64,
        mutate: impl FnOnce(&mut Segment),
    ) -> Result<(), PipelineError> {
        let update = {
            let mut persisted = persisted.lock().expect("pipeline mutex poisoned");
            let Some((index, segment)) = persisted.mutate_row(segment_id, mutate) else {
                return Ok(());
            };
            TranscriptUpdate::Replace { index, segment }
        };

        if let Some(callback) = on_update.lock().expect("on_update mutex poisoned").as_ref() {
            callback(update);
        }

        Ok(())
    }
}
