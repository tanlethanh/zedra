//! Zero-size element that owns the pinned key bar's native lifetime.
//!
//! The bar itself is the native keyboard accessory view, re-hosted above the
//! safe area while the keyboard is collapsed. Tying it to element state means
//! GPUI hides it as soon as the terminal stops being painted (navigation,
//! terminal close), which no `WorkspaceTerminal` callback covers on its own.

use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU32, Ordering};

use gpui::*;

use crate::platform_bridge;

pub fn pinned_key_bar(id: impl Into<ElementId>) -> PinnedKeyBar {
    PinnedKeyBar { id: id.into() }
}

/// Last non-zero bar height in physical pixels, so the space stays reserved while
/// the bar is temporarily hidden.
static LAST_HEIGHT_PX: AtomicU32 = AtomicU32::new(0);

/// Whether the keypad's platform slot shows Cmd instead of `|`. Cmd only means
/// anything to a macOS host, so it follows the connected host's OS.
static CMD_SLOT: AtomicBool = AtomicBool::new(false);
/// Last layout pushed to the native bars, encoded as extended | cmd << 1.
/// `-1` forces the first push.
static PUSHED_LAYOUT: AtomicI8 = AtomicI8::new(-1);
/// Last key-row visibility pushed to the native bars. `-1` forces the first push.
static PUSHED_KEYS_VISIBLE: AtomicI8 = AtomicI8::new(-1);

/// Show or hide the key rows. Separate from availability: the composer raises its
/// own keyboard, which hides the rows without tearing the keypad down.
pub fn sync_keys_visible(visible: bool) {
    if PUSHED_KEYS_VISIBLE.swap(visible as i8, Ordering::Relaxed) != visible as i8 {
        platform_bridge::bridge().set_pinned_key_bar_visible(visible);
    }
}

pub fn host_uses_cmd_slot() -> bool {
    CMD_SLOT.load(Ordering::Relaxed)
}

/// Armed/locked modifiers of the active terminal, mirrored for the native bars.
/// The terminal entity owns the state; this is only the FFI-visible copy.
static MODIFIER_MASK: AtomicU32 = AtomicU32::new(0);

pub fn modifier_mask() -> u32 {
    MODIFIER_MASK.load(Ordering::Relaxed)
}

pub fn set_modifier_mask(mask: u32) {
    MODIFIER_MASK.store(mask, Ordering::Relaxed);
}

/// Reconcile the native keypad layout with the current setting and host OS.
/// Cheap enough to call every render; only a change reaches the platform.
pub fn sync_keypad_layout(host_os: Option<&str>) {
    let cmd_slot = host_os.is_some_and(|os| os.eq_ignore_ascii_case("macos"));
    CMD_SLOT.store(cmd_slot, Ordering::Relaxed);

    let extended = crate::settings::extended_keypad();
    let encoded = extended as i8 | ((cmd_slot as i8) << 1);
    if PUSHED_LAYOUT.swap(encoded, Ordering::Relaxed) != encoded {
        platform_bridge::bridge().set_keypad_layout(extended, cmd_slot);
    }
}

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
        // Leaving the terminal takes the composer's keyboard with it; merely hiding
        // the rows for a keyboard must not, which is why this lives on unmount.
        let bridge = platform_bridge::bridge();
        bridge.set_pinned_key_bar_visible(false);
        bridge.cancel_keypad_composer();
        PUSHED_KEYS_VISIBLE.store(-1, Ordering::Relaxed);
        PUSHED_LAYOUT.store(-1, Ordering::Relaxed);
        CMD_SLOT.store(false, Ordering::Relaxed);
        MODIFIER_MASK.store(0, Ordering::Relaxed);
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
            let state = state.unwrap_or_else(|| PinnedKeyBarState);
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
