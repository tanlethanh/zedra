//! Progress for one workspace opening a tunnelled page.
//!
//! Tunnel setup runs on the session runtime, while the owning workspace polls
//! this handle from GPUI. Each workspace creates its own handle so cancelling
//! or switching one workspace cannot affect another.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenStep {
    StartingServer,
    OpeningTunnel,
    LoadingPage,
}

impl OpenStep {
    pub fn label(self, subject: &str) -> String {
        match self {
            Self::StartingServer => format!("Starting {subject} server"),
            Self::OpeningTunnel => "Opening tunnel".into(),
            Self::LoadingPage => "Loading page".into(),
        }
    }
}

#[derive(Clone)]
pub struct OpenProgress {
    pub generation: u64,
    pub subject: String,
    pub icon: String,
    pub step: OpenStep,
    pub error: Option<String>,
    pub started_at: Instant,
}

impl OpenProgress {
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}

struct State {
    current: Mutex<Option<OpenProgress>>,
    version: AtomicU64,
    generation: AtomicU64,
}

/// A workspace-owned, thread-safe progress channel.
#[derive(Clone)]
pub struct Progress(Arc<State>);

impl Progress {
    pub fn new() -> Self {
        Self(Arc::new(State {
            current: Mutex::new(None),
            version: AtomicU64::new(0),
            generation: AtomicU64::new(0),
        }))
    }

    pub fn version(&self) -> u64 {
        self.0.version.load(Ordering::Relaxed)
    }

    pub fn current(&self) -> Option<OpenProgress> {
        self.0.current.lock().ok().and_then(|slot| slot.clone())
    }

    pub fn is_active(&self, generation: u64) -> bool {
        self.0
            .current
            .lock()
            .ok()
            .and_then(|slot| {
                slot.as_ref()
                    .map(|progress| progress.generation == generation)
            })
            .unwrap_or(false)
    }

    /// Starts a new open. A second user action supersedes the prior open in
    /// this workspace, but cannot disturb another workspace's handle.
    pub fn begin(
        &self,
        subject: impl Into<String>,
        icon: impl Into<String>,
        step: OpenStep,
    ) -> u64 {
        let generation = self.0.generation.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut slot) = self.0.current.lock() {
            *slot = Some(OpenProgress {
                generation,
                subject: subject.into(),
                icon: icon.into(),
                step,
                error: None,
                started_at: Instant::now(),
            });
            self.bump();
        }
        generation
    }

    pub fn advance(&self, generation: u64, step: OpenStep) {
        self.update(generation, |progress| progress.step = step);
    }

    pub fn fail(&self, generation: u64, error: impl Into<String>) {
        let error = error.into();
        self.update(generation, |progress| progress.error = Some(error));
    }

    pub fn finish(&self, generation: u64) {
        let cleared = self
            .0
            .current
            .lock()
            .ok()
            .map(|mut slot| {
                let matches = slot
                    .as_ref()
                    .is_some_and(|progress| progress.generation == generation);
                if matches {
                    *slot = None;
                }
                matches
            })
            .unwrap_or(false);
        if cleared {
            self.bump();
        }
    }

    pub fn cancel(&self) {
        if let Ok(mut slot) = self.0.current.lock() {
            *slot = None;
        }
        self.0.generation.fetch_add(1, Ordering::Relaxed);
        self.bump();
    }

    fn update(&self, generation: u64, apply: impl FnOnce(&mut OpenProgress)) {
        let changed = self
            .0
            .current
            .lock()
            .ok()
            .and_then(|mut slot| {
                let progress = slot
                    .as_mut()
                    .filter(|progress| progress.generation == generation)?;
                apply(progress);
                Some(())
            })
            .is_some();
        if changed {
            self.bump();
        }
    }

    fn bump(&self) {
        self.0.version.fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separate_workspaces_cannot_join_or_cancel_each_other() {
        let first = Progress::new();
        let second = Progress::new();
        let first_generation = first.begin("first", "icons/globe.svg", OpenStep::StartingServer);
        let second_generation = second.begin("second", "icons/globe.svg", OpenStep::OpeningTunnel);

        second.cancel();

        assert!(first.is_active(first_generation));
        assert!(!second.is_active(second_generation));
    }

    #[test]
    fn stale_finish_leaves_a_new_open_visible() {
        let progress = Progress::new();
        let stale = progress.begin("a", "icons/globe.svg", OpenStep::StartingServer);
        let live = progress.begin("b", "icons/globe.svg", OpenStep::OpeningTunnel);
        progress.finish(stale);
        assert!(progress.is_active(live));
    }
}
