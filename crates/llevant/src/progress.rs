//! How far a long call has got, and a way to stop it: shared between the
//! thread running the call and the one watching (a frontend's progress bar
//! and Cancel button).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Steps done of a call's total, and whether it should stop. Calls that
/// take one ([`Model::volumes`](crate::Model::volumes)) set the
/// total, count steps as they finish and return
/// [`Error::Cancelled`](crate::Error::Cancelled) soon after
/// [`Progress::cancel`].
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicUsize,
    total: AtomicUsize,
    cancel: AtomicBool,
    stage: Mutex<&'static str>,
}

impl Progress {
    pub fn new() -> Progress {
        Progress::default()
    }

    /// Fraction done, 0 before the call knows its total.
    pub fn fraction(&self) -> f64 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.done.load(Ordering::Relaxed) as f64 / total as f64).min(1.0)
    }

    /// Ask the call to stop.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The step under way, named (empty before the first).
    pub fn stage(&self) -> &'static str {
        *self.stage.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Name the step under way. This and [`Progress::start`] and
    /// [`Progress::step`] are for the call doing the work (outside the
    /// engine too: a frontend's own steps around it).
    pub fn set_stage(&self, s: &'static str) {
        *self.stage.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }

    /// Set the total steps, none done yet.
    pub fn start(&self, total: usize) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }

    /// One more step done.
    pub fn step(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }
}
