//! Zero-size element that owns the pinned key bar's native lifetime.
//!
//! The bar itself is the native keyboard accessory view, re-hosted above the
//! safe area while the keyboard is collapsed. Tying it to element state means
//! GPUI hides it as soon as the terminal stops being painted (navigation,
//! terminal close), which no `WorkspaceTerminal` callback covers on its own.

use std::sync::atomic::{AtomicU32, Ordering};

use gpui::*;

use crate::platform_bridge;

pub fn pinned_key_bar(id: impl Into<ElementId>) -> PinnedKeyBar {
    PinnedKeyBar { id: id.into() }
}

/// Last non-zero bar height in physical pixels, so the space stays reserved while
/// the bar is temporarily hidden.
static LAST_HEIGHT_PX: AtomicU32 = AtomicU32::new(0);

/// Height the terminal reserves for the pinned key bar, in logical pixels.
///
/// Falls back to the last known height while the bar is hidden: with the setting
/// on, the bar is coming straight back, and collapsing the inset in between makes
/// terminal content jump on every drawer toggle.
pub fn pinned_key_bar_inset() -> Pixels {
    let bridge = platform_bridge::bridge();
    let height = bridge.pinned_key_bar_height();
    if height > 0 {
        LAST_HEIGHT_PX.store(height, Ordering::Relaxed);
    }
    let height = if height > 0 {
        height
    } else {
        LAST_HEIGHT_PX.load(Ordering::Relaxed)
    };

    let density = bridge.density();
    if density > 0.0 {
        px(height as f32 / density)
    } else {
        px(0.0)
    }
}

pub struct PinnedKeyBar {
    id: ElementId,
}

struct PinnedKeyBarState;

impl Drop for PinnedKeyBarState {
    fn drop(&mut self) {
        platform_bridge::bridge().set_pinned_key_bar_visible(false);
    }
}

impl Element for PinnedKeyBar {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let Some(id) = id else {
            return;
        };
        window.with_element_state(id, |state: Option<PinnedKeyBarState>, _window| {
            let state = state.unwrap_or_else(|| {
                platform_bridge::bridge().set_pinned_key_bar_visible(true);
                PinnedKeyBarState
            });
            ((), state)
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

impl IntoElement for PinnedKeyBar {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
