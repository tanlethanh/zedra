use std::sync::atomic::{AtomicBool, Ordering};

static SHEET_CONTENT_AT_TOP: AtomicBool = AtomicBool::new(true);

pub fn set_sheet_content_at_top(is_at_top: bool) {
    SHEET_CONTENT_AT_TOP.store(is_at_top, Ordering::Relaxed);
}

pub fn sheet_content_is_at_top() -> bool {
    SHEET_CONTENT_AT_TOP.load(Ordering::Relaxed)
}
