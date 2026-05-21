use std::sync::atomic::{AtomicBool, Ordering};

static SHEET_CONTENT_AT_TOP: AtomicBool = AtomicBool::new(true);

pub fn set_sheet_content_at_top(is_at_top: bool) {
    let previous = SHEET_CONTENT_AT_TOP.swap(is_at_top, Ordering::Relaxed);
    if previous != is_at_top {
        tracing::debug!(is_at_top, "SHEET_ATTOP boundary changed");
    }
}

pub fn sheet_content_is_at_top() -> bool {
    SHEET_CONTENT_AT_TOP.load(Ordering::Relaxed)
}
