//! Gesture composition helpers
//!
//! Provides GPUI-style composition for managing multiple gestures:
//! - `race()` - First gesture to activate wins
//! - `simultaneous()` - Multiple gestures can activate together
//! - `exclusive()` - Only one gesture can be active at a time

use gpui::*;

use crate::types::GestureId;

// ============================================================================
// GestureRace - First gesture to activate wins
// ============================================================================

/// Compose gestures where the first to activate wins
///
/// Other gestures will be cancelled when one begins.
///
/// # Example
/// ```ignore
/// race()
///     .gesture(tap_gesture().on_tap(|e, w, cx| { /* tap */ }))
///     .gesture(pan_gesture().on_pan(|e, w, cx| { /* pan */ }))
///     .child(div().child("Content"))
/// ```
pub fn race() -> GestureRace {
    GestureRace::new()
}

/// Container for racing gestures
pub struct GestureRace {
    gestures: Vec<AnyElement>,
    child: Option<AnyElement>,
}

impl GestureRace {
    pub fn new() -> Self {
        Self {
            gestures: Vec::new(),
            child: None,
        }
    }

    /// Add a gesture to the race
    pub fn gesture(mut self, gesture: impl IntoElement) -> Self {
        self.gestures.push(gesture.into_any_element());
        self
    }

    /// Set the content child that all gestures will wrap
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.child = Some(child.into_any_element());
        self
    }
}

impl Default for GestureRace {
    fn default() -> Self {
        Self::new()
    }
}

impl IntoElement for GestureRace {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GestureRace {
    type RequestLayoutState = Vec<AnyElement>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // For race, we stack all gestures and the child
        // The gestures will handle mutual exclusion through their recognizers
        let mut elements = std::mem::take(&mut self.gestures);
        if let Some(child) = self.child.take() {
            elements.push(child);
        }

        // Use the layout of the first element (or create empty)
        if let Some(first) = elements.first_mut() {
            let layout_id = first.request_layout(window, cx);
            for elem in elements.iter_mut().skip(1) {
                elem.request_layout(window, cx);
            }
            (layout_id, elements)
        } else {
            let layout_id = window.request_layout(gpui::Style::default(), [], cx);
            (layout_id, elements)
        }
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        elements: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        for elem in elements.iter_mut() {
            elem.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        elements: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Paint in reverse order so first gesture gets events first
        for elem in elements.iter_mut().rev() {
            elem.paint(window, cx);
        }
    }
}

// ============================================================================
// GestureSimultaneous - Multiple gestures can run together
// ============================================================================

/// Compose gestures that can run simultaneously
///
/// All gestures can be active at the same time.
///
/// # Example
/// ```ignore
/// simultaneous()
///     .gesture(pan_gesture().on_pan(|e, w, cx| { /* pan */ }))
///     .gesture(pinch_gesture().on_pinch(|e, w, cx| { /* pinch */ }))
///     .child(div().child("Content"))
/// ```
pub fn simultaneous() -> GestureSimultaneous {
    GestureSimultaneous::new()
}

/// Container for simultaneous gestures
pub struct GestureSimultaneous {
    gestures: Vec<AnyElement>,
    gesture_ids: Vec<GestureId>,
    child: Option<AnyElement>,
}

impl GestureSimultaneous {
    pub fn new() -> Self {
        Self {
            gestures: Vec::new(),
            gesture_ids: Vec::new(),
            child: None,
        }
    }

    /// Add a gesture that can run simultaneously with others in this group
    pub fn gesture(mut self, gesture: impl IntoElement) -> Self {
        self.gestures.push(gesture.into_any_element());
        self
    }

    /// Add a gesture with its ID for relationship tracking
    pub fn gesture_with_id(mut self, gesture: impl IntoElement, id: GestureId) -> Self {
        self.gestures.push(gesture.into_any_element());
        self.gesture_ids.push(id);
        self
    }

    /// Set the content child
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.child = Some(child.into_any_element());
        self
    }
}

impl Default for GestureSimultaneous {
    fn default() -> Self {
        Self::new()
    }
}

impl IntoElement for GestureSimultaneous {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GestureSimultaneous {
    type RequestLayoutState = Vec<AnyElement>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut elements = std::mem::take(&mut self.gestures);
        if let Some(child) = self.child.take() {
            elements.push(child);
        }

        if let Some(first) = elements.first_mut() {
            let layout_id = first.request_layout(window, cx);
            for elem in elements.iter_mut().skip(1) {
                elem.request_layout(window, cx);
            }
            (layout_id, elements)
        } else {
            let layout_id = window.request_layout(gpui::Style::default(), [], cx);
            (layout_id, elements)
        }
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        elements: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        for elem in elements.iter_mut() {
            elem.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        elements: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // All gestures receive events (simultaneous recognition)
        for elem in elements.iter_mut() {
            elem.paint(window, cx);
        }
    }
}

// ============================================================================
// GestureExclusive - Only one gesture active at a time with priority
// ============================================================================

/// Compose gestures where only one can be active, with priority order
///
/// The first gesture in the list has highest priority.
/// Lower priority gestures are cancelled when a higher priority one activates.
///
/// # Example
/// ```ignore
/// exclusive()
///     .gesture(double_tap_gesture().on_tap(|e, w, cx| { /* high priority */ }))
///     .gesture(single_tap_gesture().on_tap(|e, w, cx| { /* low priority */ }))
///     .child(div().child("Content"))
/// ```
pub fn exclusive() -> GestureExclusive {
    GestureExclusive::new()
}

/// Container for exclusive gestures with priority
pub struct GestureExclusive {
    gestures: Vec<AnyElement>,
    child: Option<AnyElement>,
}

impl GestureExclusive {
    pub fn new() -> Self {
        Self {
            gestures: Vec::new(),
            child: None,
        }
    }

    /// Add a gesture (order determines priority - first is highest)
    pub fn gesture(mut self, gesture: impl IntoElement) -> Self {
        self.gestures.push(gesture.into_any_element());
        self
    }

    /// Set the content child
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.child = Some(child.into_any_element());
        self
    }
}

impl Default for GestureExclusive {
    fn default() -> Self {
        Self::new()
    }
}

impl IntoElement for GestureExclusive {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GestureExclusive {
    type RequestLayoutState = Vec<AnyElement>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut elements = std::mem::take(&mut self.gestures);
        if let Some(child) = self.child.take() {
            elements.push(child);
        }

        if let Some(first) = elements.first_mut() {
            let layout_id = first.request_layout(window, cx);
            for elem in elements.iter_mut().skip(1) {
                elem.request_layout(window, cx);
            }
            (layout_id, elements)
        } else {
            let layout_id = window.request_layout(gpui::Style::default(), [], cx);
            (layout_id, elements)
        }
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        elements: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        for elem in elements.iter_mut() {
            elem.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        elements: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Paint in priority order (first gesture gets first chance at events)
        for elem in elements.iter_mut() {
            elem.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_race_creation() {
        let _race = race();
    }

    #[test]
    fn test_simultaneous_creation() {
        let _sim = simultaneous();
    }

    #[test]
    fn test_exclusive_creation() {
        let _exc = exclusive();
    }
}
