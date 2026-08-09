use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static SHEET_CONTENT_AT_TOP: AtomicBool = AtomicBool::new(true);

/// Native modals currently on screen (alert, selection, list picker, text input).
/// Views that must yield to them (the pinned terminal key bar) hold no handle to
/// the presentation, so it is tracked globally like `any_drawer_open`.
static ACTIVE_PRESENTATIONS: AtomicUsize = AtomicUsize::new(0);
static PRESENTATION_CHANGED: AtomicBool = AtomicBool::new(false);

pub fn begin_native_presentation() {
    ACTIVE_PRESENTATIONS.fetch_add(1, Ordering::Relaxed);
    PRESENTATION_CHANGED.store(true, Ordering::Relaxed);
}

/// Balanced with `begin_native_presentation`; each presentation ends exactly once
/// because the caller only ends after taking its callback out of the registry.
pub fn end_native_presentation() {
    let _ = ACTIVE_PRESENTATIONS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        Some(count.saturating_sub(1))
    });
    PRESENTATION_CHANGED.store(true, Ordering::Relaxed);
}

/// The in-app webview and the custom sheet are single presentations that a new
/// `open`/`show` replaces rather than stacks, so each tracks as a flag instead of
/// joining the counter.
static WEBVIEW_PRESENTED: AtomicBool = AtomicBool::new(false);
static CUSTOM_SHEET_PRESENTED: AtomicBool = AtomicBool::new(false);

pub fn set_native_webview_presented(presented: bool) {
    set_flag(&WEBVIEW_PRESENTED, presented);
}

pub fn set_native_custom_sheet_presented(presented: bool) {
    set_flag(&CUSTOM_SHEET_PRESENTED, presented);
}

fn set_flag(flag: &AtomicBool, presented: bool) {
    if flag.swap(presented, Ordering::Relaxed) != presented {
        PRESENTATION_CHANGED.store(true, Ordering::Relaxed);
    }
}

pub fn any_native_presentation() -> bool {
    ACTIVE_PRESENTATIONS.load(Ordering::Relaxed) > 0
        || WEBVIEW_PRESENTED.load(Ordering::Relaxed)
        || CUSTOM_SHEET_PRESENTED.load(Ordering::Relaxed)
}

/// True once per change, so a poller can refresh windows for views that gate on
/// `any_native_presentation` but observe no entity.
pub fn take_native_presentation_change() -> bool {
    PRESENTATION_CHANGED.swap(false, Ordering::Relaxed)
}

pub fn set_sheet_content_at_top(is_at_top: bool) {
    let previous = SHEET_CONTENT_AT_TOP.swap(is_at_top, Ordering::Relaxed);
    if previous != is_at_top {
        tracing::debug!(is_at_top, "SHEET_ATTOP boundary changed");
    }
}

pub fn sheet_content_is_at_top() -> bool {
    SHEET_CONTENT_AT_TOP.load(Ordering::Relaxed)
}
