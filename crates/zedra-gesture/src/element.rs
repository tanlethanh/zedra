//! GPUI element wrappers for gesture handling
//!
//! Provides a fluent API for adding gesture recognition to GPUI elements:
//! ```ignore
//! pan_gesture()
//!     .min_distance(10.0)
//!     .on_pan(|event, cx| { ... })
//!     .child(my_view)
//! ```

use std::sync::{Arc, Mutex};

use gpui::*;

use crate::recognizers::{
    GestureRecognizer, PanGesture, PanGestureEvent, PinchGesture, PinchGestureEvent, TapGesture,
    TapGestureEvent,
};
use crate::state::GestureState;
use crate::types::{GestureId, Point, TouchAction, TouchEvent, TouchPointer};

// ============================================================================
// Gesture Handler Types
// ============================================================================

/// Handler for pan gesture events
pub type PanHandler = Arc<dyn Fn(&PanGestureEvent, &mut Window, &mut App) + Send + Sync>;

/// Handler for tap gesture events
pub type TapHandler = Arc<dyn Fn(&TapGestureEvent, &mut Window, &mut App) + Send + Sync>;

/// Handler for pinch gesture events
pub type PinchHandler = Arc<dyn Fn(&PinchGestureEvent, &mut Window, &mut App) + Send + Sync>;

// ============================================================================
// Pan Gesture Element
// ============================================================================

/// Wraps an element with pan gesture recognition
pub struct PanGestureElement {
    /// Unique identifier for this gesture
    gesture_id: GestureId,
    child: Option<AnyElement>,
    recognizer: Arc<Mutex<PanGesture>>,
    on_begin: Option<PanHandler>,
    on_change: Option<PanHandler>,
    on_end: Option<PanHandler>,
    /// Gestures that must fail before this one can begin
    requires_failure: Vec<GestureId>,
    /// Gestures that can run simultaneously with this one
    simultaneous: Vec<GestureId>,
}

/// Create a pan gesture wrapper
pub fn pan_gesture() -> PanGestureElement {
    PanGestureElement {
        gesture_id: GestureId::new(),
        child: None,
        recognizer: Arc::new(Mutex::new(PanGesture::new())),
        on_begin: None,
        on_change: None,
        on_end: None,
        requires_failure: Vec::new(),
        simultaneous: Vec::new(),
    }
}

impl PanGestureElement {
    /// Set the child element to wrap with gesture recognition
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.child = Some(child.into_any_element());
        self
    }

    /// Set minimum distance to start recognizing
    pub fn min_distance(self, distance: f32) -> Self {
        if let Ok(mut recognizer) = self.recognizer.lock() {
            *recognizer = std::mem::take(&mut *recognizer).min_distance(distance);
        }
        self
    }

    /// Set handler for when pan begins
    pub fn on_begin<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PanGestureEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.on_begin = Some(Arc::new(handler));
        self
    }

    /// Set handler for pan changes (movement)
    pub fn on_change<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PanGestureEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.on_change = Some(Arc::new(handler));
        self
    }

    /// Set handler for when pan ends
    pub fn on_end<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PanGestureEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.on_end = Some(Arc::new(handler));
        self
    }

    /// Set handler for all pan events
    pub fn on_pan<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PanGestureEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);
        let h1 = handler.clone();
        let h2 = handler.clone();
        let h3 = handler;
        self.on_begin = Some(Arc::new(move |e, w, cx| h1(e, w, cx)));
        self.on_change = Some(Arc::new(move |e, w, cx| h2(e, w, cx)));
        self.on_end = Some(Arc::new(move |e, w, cx| h3(e, w, cx)));
        self
    }

    /// Get the unique ID of this gesture for establishing relationships
    pub fn id(&self) -> GestureId {
        self.gesture_id
    }

    /// This gesture waits for `other` to fail before activating
    ///
    /// Useful when a gesture like double-tap should not interfere with single-tap
    pub fn requires_failure_of(mut self, other: GestureId) -> Self {
        if !self.requires_failure.contains(&other) {
            self.requires_failure.push(other);
        }
        self
    }

    /// This gesture can run simultaneously with `other`
    ///
    /// By default, only one gesture can be active at a time.
    /// Use this to allow multiple gestures to recognize together.
    pub fn simultaneous_with(mut self, other: GestureId) -> Self {
        if !self.simultaneous.contains(&other) {
            self.simultaneous.push(other);
        }
        self
    }

    /// Get the list of gestures this one requires to fail
    pub fn requires_failure_ids(&self) -> &[GestureId] {
        &self.requires_failure
    }

    /// Get the list of gestures this one can run simultaneously with
    pub fn simultaneous_ids(&self) -> &[GestureId] {
        &self.simultaneous
    }
}

impl IntoElement for PanGestureElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PanGestureElement {
    type RequestLayoutState = Option<AnyElement>;
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
        if let Some(mut child) = self.child.take() {
            let layout_id = child.request_layout(window, cx);
            (layout_id, Some(child))
        } else {
            let layout_id = window.request_layout(gpui::Style::default(), [], cx);
            (layout_id, None)
        }
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(child) = child {
            child.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(child) = child {
            child.paint(window, cx);
        }

        // Register mouse event handlers for gesture recognition
        let recognizer = self.recognizer.clone();
        let on_begin = self.on_begin.clone();
        let on_change = self.on_change.clone();
        let on_end = self.on_end.clone();

        log::info!(
            "PanGesture: paint() registering handlers, bounds=({:.1},{:.1})-({:.1},{:.1})",
            bounds.origin.x,
            bounds.origin.y,
            bounds.origin.x + bounds.size.width,
            bounds.origin.y + bounds.size.height
        );

        window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, _cx| {
            log::info!(
                "PanGesture: MouseDown phase={:?}, pos=({:.1}, {:.1}), contains={}",
                phase,
                event.position.x,
                event.position.y,
                bounds.contains(&event.position)
            );

            if phase != DispatchPhase::Bubble {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }

            let position = Point::new(event.position.x.into(), event.position.y.into());
            let touch_event = TouchEvent::new(
                TouchAction::Down,
                vec![TouchPointer::new(0, position.x, position.y)],
            );

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);
                log::info!("PanGesture: Touch down processed");
            }
        });

        let recognizer = self.recognizer.clone();
        let on_begin = on_begin.clone();
        let on_change = on_change.clone();

        // Note: On Android, GPUI converts touch drag to ScrollWheel events, not MouseMove
        // So we need to listen to both MouseMove (for desktop) and ScrollWheel (for Android)
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }

            let position = Point::new(event.position.x.into(), event.position.y.into());
            let touch_event = TouchEvent::new(
                TouchAction::Move,
                vec![TouchPointer::new(0, position.x, position.y)],
            );

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);

                if let Some(gesture_event) = rec.take_event() {
                    log::info!(
                        "PanGesture: Gesture event state={:?}, translation=({:.1}, {:.1})",
                        gesture_event.state,
                        gesture_event.translation.x,
                        gesture_event.translation.y
                    );
                    match gesture_event.state {
                        GestureState::Began => {
                            log::info!("PanGesture: Calling on_begin handler");
                            if let Some(handler) = &on_begin {
                                handler(&gesture_event, window, cx);
                            }
                        }
                        GestureState::Changed => {
                            if let Some(handler) = &on_change {
                                handler(&gesture_event, window, cx);
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        // Handle ScrollWheel events for Android touch drag (GPUI converts touch drag to ScrollWheel)
        let recognizer = self.recognizer.clone();
        let on_begin_scroll = self.on_begin.clone();
        let on_change_scroll = self.on_change.clone();

        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }

            // Get the scroll delta as movement
            let (dx, dy) = match event.delta {
                ScrollDelta::Pixels(p) => (p.x.into(), p.y.into()),
                ScrollDelta::Lines(l) => (l.x as f32 * 20.0, l.y as f32 * 20.0),
            };

            log::info!(
                "PanGesture: ScrollWheel delta=({:.1}, {:.1}), pos=({:.1}, {:.1})",
                dx, dy,
                event.position.x,
                event.position.y
            );

            if let Ok(mut rec) = recognizer.lock() {
                // Update the recognizer with scroll delta as movement
                rec.on_scroll(dx, dy, event.position.x.into(), event.position.y.into());

                if let Some(gesture_event) = rec.take_event() {
                    log::info!(
                        "PanGesture: ScrollWheel gesture event state={:?}, translation=({:.1}, {:.1})",
                        gesture_event.state,
                        gesture_event.translation.x,
                        gesture_event.translation.y
                    );
                    match gesture_event.state {
                        GestureState::Began => {
                            log::info!("PanGesture: ScrollWheel calling on_begin handler");
                            if let Some(handler) = &on_begin_scroll {
                                handler(&gesture_event, window, cx);
                            }
                        }
                        GestureState::Changed => {
                            if let Some(handler) = &on_change_scroll {
                                handler(&gesture_event, window, cx);
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        let recognizer = self.recognizer.clone();

        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            log::info!(
                "PanGesture: MouseUp phase={:?}, pos=({:.1}, {:.1})",
                phase,
                event.position.x,
                event.position.y
            );

            if phase != DispatchPhase::Bubble {
                return;
            }

            let position = Point::new(event.position.x.into(), event.position.y.into());
            let touch_event = TouchEvent::new(
                TouchAction::Up,
                vec![TouchPointer::new(0, position.x, position.y)],
            );

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);

                if let Some(gesture_event) = rec.take_event() {
                    log::info!(
                        "PanGesture: MouseUp gesture event state={:?}",
                        gesture_event.state
                    );
                    if gesture_event.state == GestureState::Ended
                        || gesture_event.state == GestureState::Cancelled
                    {
                        log::info!("PanGesture: Calling on_end handler");
                        if let Some(handler) = &on_end {
                            handler(&gesture_event, window, cx);
                        }
                    }
                }
            }
        });
    }
}

// ============================================================================
// Tap Gesture Element
// ============================================================================

/// Wraps an element with tap gesture recognition
pub struct TapGestureElement {
    /// Unique identifier for this gesture
    gesture_id: GestureId,
    child: Option<AnyElement>,
    recognizer: Arc<Mutex<TapGesture>>,
    on_tap: Option<TapHandler>,
    /// Gestures that must fail before this one can begin
    requires_failure: Vec<GestureId>,
    /// Gestures that can run simultaneously with this one
    simultaneous: Vec<GestureId>,
}

/// Create a tap gesture wrapper
pub fn tap_gesture() -> TapGestureElement {
    TapGestureElement {
        gesture_id: GestureId::new(),
        child: None,
        recognizer: Arc::new(Mutex::new(TapGesture::new())),
        on_tap: None,
        requires_failure: Vec::new(),
        simultaneous: Vec::new(),
    }
}

impl TapGestureElement {
    /// Set the child element to wrap with gesture recognition
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.child = Some(child.into_any_element());
        self
    }

    /// Set number of taps required
    pub fn number_of_taps(self, count: u8) -> Self {
        if let Ok(mut recognizer) = self.recognizer.lock() {
            *recognizer = std::mem::take(&mut *recognizer).number_of_taps(count);
        }
        self
    }

    /// Set handler for tap events
    pub fn on_tap<F>(mut self, handler: F) -> Self
    where
        F: Fn(&TapGestureEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.on_tap = Some(Arc::new(handler));
        self
    }

    /// Get the unique ID of this gesture for establishing relationships
    pub fn id(&self) -> GestureId {
        self.gesture_id
    }

    /// This gesture waits for `other` to fail before activating
    ///
    /// Useful when a gesture like double-tap should not interfere with single-tap
    pub fn requires_failure_of(mut self, other: GestureId) -> Self {
        if !self.requires_failure.contains(&other) {
            self.requires_failure.push(other);
        }
        self
    }

    /// This gesture can run simultaneously with `other`
    ///
    /// By default, only one gesture can be active at a time.
    /// Use this to allow multiple gestures to recognize together.
    pub fn simultaneous_with(mut self, other: GestureId) -> Self {
        if !self.simultaneous.contains(&other) {
            self.simultaneous.push(other);
        }
        self
    }

    /// Get the list of gestures this one requires to fail
    pub fn requires_failure_ids(&self) -> &[GestureId] {
        &self.requires_failure
    }

    /// Get the list of gestures this one can run simultaneously with
    pub fn simultaneous_ids(&self) -> &[GestureId] {
        &self.simultaneous
    }
}

impl IntoElement for TapGestureElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TapGestureElement {
    type RequestLayoutState = Option<AnyElement>;
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
        if let Some(mut child) = self.child.take() {
            let layout_id = child.request_layout(window, cx);
            (layout_id, Some(child))
        } else {
            let layout_id = window.request_layout(gpui::Style::default(), [], cx);
            (layout_id, None)
        }
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(child) = child {
            child.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(child) = child {
            child.paint(window, cx);
        }

        let recognizer = self.recognizer.clone();

        window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, _cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }

            let position = Point::new(event.position.x.into(), event.position.y.into());
            let touch_event = TouchEvent::new(
                TouchAction::Down,
                vec![TouchPointer::new(0, position.x, position.y)],
            );

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);
            }
        });

        let recognizer = self.recognizer.clone();
        let on_tap = self.on_tap.clone();

        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }

            let position = Point::new(event.position.x.into(), event.position.y.into());
            let touch_event = TouchEvent::new(
                TouchAction::Up,
                vec![TouchPointer::new(0, position.x, position.y)],
            );

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);

                if let Some(gesture_event) = rec.take_event() {
                    if gesture_event.state == GestureState::Ended {
                        if let Some(handler) = &on_tap {
                            handler(&gesture_event, window, cx);
                        }
                    }
                }
            }
        });
    }
}

// ============================================================================
// Pinch Gesture Element
// ============================================================================

/// Wraps an element with pinch gesture recognition
pub struct PinchGestureElement {
    /// Unique identifier for this gesture
    gesture_id: GestureId,
    child: Option<AnyElement>,
    recognizer: Arc<Mutex<PinchGesture>>,
    on_begin: Option<PinchHandler>,
    on_change: Option<PinchHandler>,
    on_end: Option<PinchHandler>,
    /// Gestures that must fail before this one can begin
    requires_failure: Vec<GestureId>,
    /// Gestures that can run simultaneously with this one
    simultaneous: Vec<GestureId>,
}

/// Create a pinch gesture wrapper
pub fn pinch_gesture() -> PinchGestureElement {
    PinchGestureElement {
        gesture_id: GestureId::new(),
        child: None,
        recognizer: Arc::new(Mutex::new(PinchGesture::new())),
        on_begin: None,
        on_change: None,
        on_end: None,
        requires_failure: Vec::new(),
        simultaneous: Vec::new(),
    }
}

impl PinchGestureElement {
    /// Set the child element to wrap with gesture recognition
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.child = Some(child.into_any_element());
        self
    }

    /// Set handler for when pinch begins
    pub fn on_begin<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PinchGestureEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.on_begin = Some(Arc::new(handler));
        self
    }

    /// Set handler for pinch changes
    pub fn on_change<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PinchGestureEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.on_change = Some(Arc::new(handler));
        self
    }

    /// Set handler for when pinch ends
    pub fn on_end<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PinchGestureEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        self.on_end = Some(Arc::new(handler));
        self
    }

    /// Set handler for all pinch events
    pub fn on_pinch<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PinchGestureEvent, &mut Window, &mut App) + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);
        let h1 = handler.clone();
        let h2 = handler.clone();
        let h3 = handler;
        self.on_begin = Some(Arc::new(move |e, w, cx| h1(e, w, cx)));
        self.on_change = Some(Arc::new(move |e, w, cx| h2(e, w, cx)));
        self.on_end = Some(Arc::new(move |e, w, cx| h3(e, w, cx)));
        self
    }

    /// Get the unique ID of this gesture for establishing relationships
    pub fn id(&self) -> GestureId {
        self.gesture_id
    }

    /// This gesture waits for `other` to fail before activating
    ///
    /// Useful when a gesture like double-tap should not interfere with single-tap
    pub fn requires_failure_of(mut self, other: GestureId) -> Self {
        if !self.requires_failure.contains(&other) {
            self.requires_failure.push(other);
        }
        self
    }

    /// This gesture can run simultaneously with `other`
    ///
    /// By default, only one gesture can be active at a time.
    /// Use this to allow multiple gestures to recognize together.
    pub fn simultaneous_with(mut self, other: GestureId) -> Self {
        if !self.simultaneous.contains(&other) {
            self.simultaneous.push(other);
        }
        self
    }

    /// Get the list of gestures this one requires to fail
    pub fn requires_failure_ids(&self) -> &[GestureId] {
        &self.requires_failure
    }

    /// Get the list of gestures this one can run simultaneously with
    pub fn simultaneous_ids(&self) -> &[GestureId] {
        &self.simultaneous
    }
}

impl IntoElement for PinchGestureElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PinchGestureElement {
    type RequestLayoutState = Option<AnyElement>;
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
        if let Some(mut child) = self.child.take() {
            let layout_id = child.request_layout(window, cx);
            (layout_id, Some(child))
        } else {
            let layout_id = window.request_layout(gpui::Style::default(), [], cx);
            (layout_id, None)
        }
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(child) = child {
            child.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(child) = child {
            child.paint(window, cx);
        }

        // Note: Pinch gestures require multi-touch which isn't available via mouse events
        // This will need JNI integration to receive actual multi-touch events
        // For now, this is a placeholder structure
    }
}
