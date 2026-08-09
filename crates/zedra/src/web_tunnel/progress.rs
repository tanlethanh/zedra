//! Shared progress for opening a tunnelled page in the in-app webview.
//!
//! Every open path funnels through here, so one overlay
//! (`web_tunnel_opening.rs`) can report what stage the open is in no matter
//! which entry point started it. The tunnel steps run on the session runtime,
//! so this is a plain global the GPUI view polls — no entity handle crosses
//! threads.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenStep {
    /// Host is spawning the agent's server and creating a session.
    StartingServer,
    /// Device is binding the loopback listener that fronts the host port.
    OpeningTunnel,
    /// Webview is up and loading the page.
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

/// A single in-flight open. Only one runs at a time: the overlay covers the
/// workspace, so a second open can't be started until this one settles.
#[derive(Clone)]
pub struct OpenProgress {
    pub generation: u64,
    /// What is being opened — an agent's display name, or the `host:port` label.
    pub subject: String,
    pub icon: String,
    pub step: OpenStep,
    /// Set once the open failed; the overlay stays up until dismissed.
    pub error: Option<String>,
    pub started_at: Instant,
}

impl OpenProgress {
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}

fn state() -> &'static Mutex<Option<OpenProgress>> {
    static STATE: OnceLock<Mutex<Option<OpenProgress>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

static VERSION: AtomicU64 = AtomicU64::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Bumped on every change so the overlay can poll cheaply for a redraw.
pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

pub fn current() -> Option<OpenProgress> {
    state().lock().ok().and_then(|s| s.clone())
}

/// Whether `generation` is still the live open — false once it was cancelled or
/// superseded, which is the guard against presenting a webview the user backed
/// out of.
pub fn is_active(generation: u64) -> bool {
    state()
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|p| p.generation == generation))
        .unwrap_or(false)
}

/// Start reporting a new open, replacing anything stale. Returns the generation
/// every later call must pass back.
pub fn begin(subject: impl Into<String>, icon: impl Into<String>, step: OpenStep) -> u64 {
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    if let Ok(mut slot) = state().lock() {
        *slot = Some(OpenProgress {
            generation,
            subject: subject.into(),
            icon: icon.into(),
            step,
            error: None,
            started_at: Instant::now(),
        });
    }
    VERSION.fetch_add(1, Ordering::Relaxed);
    generation
}

/// Join the open a caller already started (keeping its subject and elapsed
/// time), or start one at `step` when this is the entry point.
pub fn begin_or_join(subject: impl Into<String>, icon: impl Into<String>, step: OpenStep) -> u64 {
    let joined = state().lock().ok().and_then(|mut slot| {
        let progress = slot.as_mut().filter(|p| p.error.is_none())?;
        progress.step = step;
        Some(progress.generation)
    });
    match joined {
        Some(generation) => {
            VERSION.fetch_add(1, Ordering::Relaxed);
            generation
        }
        None => begin(subject, icon, step),
    }
}

pub fn advance(generation: u64, step: OpenStep) {
    update(generation, |progress| progress.step = step);
}

pub fn fail(generation: u64, error: impl Into<String>) {
    let error = error.into();
    update(generation, |progress| progress.error = Some(error.clone()));
}

/// Clear a settled open. No-op once superseded, so a late finish from an
/// abandoned open can't hide a newer one.
pub fn finish(generation: u64) {
    let cleared = state()
        .lock()
        .ok()
        .map(|mut slot| {
            let matches = slot.as_ref().is_some_and(|p| p.generation == generation);
            if matches {
                *slot = None;
            }
            matches
        })
        .unwrap_or(false);
    if cleared {
        VERSION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Drop the overlay and invalidate the in-flight open so it stops short of
/// presenting a webview.
pub fn cancel() {
    if let Ok(mut slot) = state().lock() {
        *slot = None;
    }
    GENERATION.fetch_add(1, Ordering::Relaxed);
    VERSION.fetch_add(1, Ordering::Relaxed);
}

fn update(generation: u64, apply: impl FnOnce(&mut OpenProgress)) {
    let changed = state()
        .lock()
        .ok()
        .and_then(|mut slot| {
            let progress = slot.as_mut().filter(|p| p.generation == generation)?;
            apply(progress);
            Some(())
        })
        .is_some();
    if changed {
        VERSION.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The state is global, so these run behind one lock to stay deterministic.
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn begin_or_join_keeps_the_original_subject() {
        let _g = guard();
        let started = begin("opencode", "icons/opencode.svg", OpenStep::StartingServer);
        let joined = begin_or_join("localhost:4096", "icons/globe.svg", OpenStep::OpeningTunnel);
        assert_eq!(started, joined);
        let progress = current().unwrap();
        assert_eq!(progress.subject, "opencode");
        assert_eq!(progress.step, OpenStep::OpeningTunnel);
        finish(started);
    }

    #[test]
    fn begin_or_join_starts_fresh_when_nothing_is_open() {
        let _g = guard();
        cancel();
        let generation =
            begin_or_join("localhost:4096", "icons/globe.svg", OpenStep::OpeningTunnel);
        let progress = current().unwrap();
        assert_eq!(progress.subject, "localhost:4096");
        assert_eq!(progress.step, OpenStep::OpeningTunnel);
        finish(generation);
        assert!(current().is_none());
    }

    #[test]
    fn cancel_invalidates_the_in_flight_generation() {
        let _g = guard();
        let generation = begin("opencode", "icons/opencode.svg", OpenStep::StartingServer);
        assert!(is_active(generation));
        cancel();
        assert!(!is_active(generation));
        // A late step from the abandoned open must not resurrect the overlay.
        advance(generation, OpenStep::LoadingPage);
        assert!(current().is_none());
    }

    #[test]
    fn finish_from_a_stale_generation_leaves_the_live_open_alone() {
        let _g = guard();
        let stale = begin("a", "icons/globe.svg", OpenStep::StartingServer);
        let live = begin("b", "icons/globe.svg", OpenStep::StartingServer);
        finish(stale);
        assert!(is_active(live));
        finish(live);
    }

    #[test]
    fn fail_records_the_error_and_keeps_the_overlay() {
        let _g = guard();
        let generation = begin("opencode", "icons/opencode.svg", OpenStep::StartingServer);
        fail(generation, "boom");
        assert_eq!(current().unwrap().error.as_deref(), Some("boom"));
        // A failed open is settled: a later join starts a new one.
        let next = begin_or_join("localhost:1", "icons/globe.svg", OpenStep::OpeningTunnel);
        assert_ne!(next, generation);
        finish(next);
    }
}
