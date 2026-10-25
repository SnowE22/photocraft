//! Unloading a model nobody has used for a while.
//!
//! A loaded model keeps its weights and working memory on the GPU: LaMa about 1.2 GB, the diffusion
//! model about 4 GB. Kept for the rest of a run, that left an 8 GB GPU with too little for the
//! canvas, which then spilled into system memory: showing or hiding a layer and painting crawled
//! after one diffusion fill (user report). A [`Keeper`] notes each use; once a model has loaded, a
//! thread checks every few seconds and drops it after its idle time. The next use loads it again.
//! A model dropped this way isn't preloaded again until it is used.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How often the watch thread looks.
const CHECK_EVERY: Duration = Duration::from_secs(5);

/// What a model's release function found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Check {
    /// Dropped.
    Released,
    /// In use, loading, or not idle long enough: look again later.
    Kept,
    /// Not loaded (dropped otherwise): stop watching.
    Gone,
}

/// When a model was last used, and whether it was dropped for being idle.
pub(crate) struct Keeper {
    last_used: Mutex<Option<Instant>>,
    released: AtomicBool,
    watching: AtomicBool,
}

impl Keeper {
    pub(crate) const fn new() -> Self {
        Self { last_used: Mutex::new(None), released: AtomicBool::new(false), watching: AtomicBool::new(false) }
    }

    /// The model was just used (or loaded).
    pub(crate) fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        self.released.store(false, Ordering::Release);
    }

    /// How long since the last use (zero if never).
    pub(crate) fn idle_for(&self) -> Duration {
        self.last_used.lock().unwrap_or_else(PoisonError::into_inner).map_or(Duration::ZERO, |t| t.elapsed())
    }

    /// Whether the model was dropped for being idle and not used since (no preloading then).
    pub(crate) fn released(&self) -> bool {
        self.released.load(Ordering::Acquire)
    }

    /// Watch a loaded model: `release` is asked every few seconds until it drops the model or finds
    /// it gone. One thread per model at a time; nothing on WebAssembly.
    pub(crate) fn watch(&'static self, name: &'static str, release: fn() -> Check) {
        if self.watching.swap(true, Ordering::AcqRel) {
            return;
        }
        let started = std::thread::Builder::new().name(format!("{name}-idle")).spawn(move || {
            loop {
                std::thread::sleep(CHECK_EVERY);
                match release() {
                    Check::Kept => continue,
                    Check::Released => {
                        self.released.store(true, Ordering::Release);
                        log::info!("[ml] {name} unloaded after {} s unused", self.idle_for().as_secs());
                        break;
                    }
                    Check::Gone => break,
                }
            }
            self.watching.store(false, Ordering::Release);
        });
        if started.is_err() {
            self.watching.store(false, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keeper_measures_idle_time_and_remembers_a_release() {
        static K: Keeper = Keeper::new();
        assert_eq!(K.idle_for(), Duration::ZERO);
        K.touch();
        std::thread::sleep(Duration::from_millis(20));
        assert!(K.idle_for() >= Duration::from_millis(20));
        K.released.store(true, Ordering::Release);
        assert!(K.released());
        K.touch();
        assert!(!K.released(), "a use clears it");
    }
}
