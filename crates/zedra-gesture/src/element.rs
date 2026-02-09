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
    PanGesture, PanGestureEvent, PinchGesture, PinchGestureEvent, TapGesture, TapGestureEvent,
};
use crate::state::GestureState;
use crate::types::{Point, TouchAction, TouchEvent, TouchPointer};

// ============================================================================
// Gesture Handler Types
// ============================================================================

/// Handler for pan gesture events
pub type PanHandler = Arc<dyn Fn(&PanGestureEvent, &mut App) + Send + Sync>;

/// Handler for tap gesture events
pub type TapHandler = Arc<dyn Fn(&TapGestureEvent, &mut App) + Send + Sync>;

/// Handler for pinch gesture events
pub type PinchHandler = Arc<dyn Fn(&PinchGestureEvent, &mut App) + Send + Sync>;

// ============================================================================
// Pan Gesture Element
// ============================================================================

/// Wraps an element with pan gesture recognition
pub struct PanGestureElement {
    child: Option<AnyElement>,
    recognizer: Arc<Mutex<PanGesture>>,
    on_begin: Option<PanHandler>,
    on_change: Option<PanHandler>,
    on_end: Option<PanHandler>,
}

/// Create a pan gesture wrapper
pub fn pan_gesture() -> PanGestureElement {
    PanGestureElement {
        child: None,
        recognizer: Arc::new(Mutex::new(PanGesture::new())),
        on_begin: None,
        on_change: None,
        on_end: None,
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
        F: Fn(&PanGestureEvent, &mut App) + Send + Sync + 'static,
    {
        self.on_begin = Some(Arc::new(handler));
        self
    }

    /// Set handler for pan changes (movement)
    pub fn on_change<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PanGestureEvent, &mut App) + Send + Sync + 'static,
    {
        self.on_change = Some(Arc::new(handler));
        self
    }

    /// Set handler for when pan ends
    pub fn on_end<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PanGestureEvent, &mut App) + Send + Sync + 'static,
    {
        self.on_end = Some(Arc::new(handler));
        self
    }

    /// Set handler for all pan events
    pub fn on_pan<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PanGestureEvent, &mut App) + Send + Sync + Clone + 'static,
    {
        let h1 = handler.clone();
        let h2 = handler.clone();
        let h3 = handler;
        self.on_begin = Some(Arc::new(move |e, cx| h1(e, cx)));
        self.on_change = Some(Arc::new(move |e, cx| h2(e, cx)));
        self.on_end = Some(Arc::new(move |e, cx| h3(e, cx)));
        self
    }

    fn handle_mouse_event(&self, position: Point, action: TouchAction, cx: &mut App) {
        let event = TouchEvent::new(action, vec![TouchPointer::new(0, position.x, position.y)]);

        if let Ok(mut recognizer) = self.recognizer.lock() {
            recognizer.on_touch(&event);

            if let Some(gesture_event) = recognizer.take_event() {
                match gesture_event.state {
                    GestureState::Began => {
                        if let Some(handler) = &self.on_begin {
                            handler(&gesture_event, cx);
                        }
                    }
                    GestureState::Changed => {
                        if let Some(handler) = &self.on_change {
                            handler(&gesture_event, cx);
                        }
                    }
                    GestureState::Ended => {
                        if let Some(handler) = &self.on_end {
                            handler(&gesture_event, cx);
                        }
                    }
                    GestureState::Cancelled => {
                        if let Some(handler) = &self.on_end {
                            handler(&gesture_event, cx);
                        }
                    }
                    _ => {}
                }
            }
        }
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

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        cx: &mut WindowContext,
    ) -> (LayoutId, Self::RequestLayoutState) {
        if let Some(mut child) = self.child.take() {
            let layout_id = child.request_layout(cx);
            (layout_id, Some(child))
        } else {
            let layout_id = cx.request_layout(gpui::Style::default(), []);
            (layout_id, None)
        }
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        cx: &mut WindowContext,
    ) -> Self::PrepaintState {
        if let Some(child) = child {
            child.prepaint(cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        cx: &mut WindowContext,
    ) {
        if let Some(child) = child {
            child.paint(cx);
        }

        // Register mouse event handlers for gesture recognition
        let recognizer = self.recognizer.clone();
        let on_begin = self.on_begin.clone();
        let on_change = self.on_change.clone();
        let on_end = self.on_end.clone();

        cx.on_mouse_event(move |event: &MouseDownEvent, phase, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }

            let position = Point::new(event.position.x.0, event.position.y.0);
            let touch_event =
                TouchEvent::new(TouchAction::Down, vec![TouchPointer::new(0, position.x, position.y)]);

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);
            }
        });

        let recognizer = self.recognizer.clone();
        let on_begin = self.on_begin.clone();
        let on_change = self.on_change.clone();

        cx.on_mouse_event(move |event: &MouseMoveEvent, phase, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }

            let position = Point::new(event.position.x.0, event.position.y.0);
            let touch_event =
                TouchEvent::new(TouchAction::Move, vec![TouchPointer::new(0, position.x, position.y)]);

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);

                if let Some(gesture_event) = rec.take_event() {
                    match gesture_event.state {
                        GestureState::Began => {
                            if let Some(handler) = &on_begin {
                                handler(&gesture_event, cx);
                            }
                        }
                        GestureState::Changed => {
                            if let Some(handler) = &on_change {
                                handler(&gesture_event, cx);
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        let recognizer = self.recognizer.clone();
        let on_end = self.on_end.clone();

        cx.on_mouse_event(move |event: &MouseUpEvent, phase, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }

            let position = Point::new(event.position.x.0, event.position.y.0);
            let touch_event =
                TouchEvent::new(TouchAction::Up, vec![TouchPointer::new(0, position.x, position.y)]);

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);

                if let Some(gesture_event) = rec.take_event() {
                    if gesture_event.state == GestureState::Ended
                        || gesture_event.state == GestureState::Cancelled
                    {
                        if let Some(handler) = &on_end {
                            handler(&gesture_event, cx);
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
    child: Option<AnyElement>,
    recognizer: Arc<Mutex<TapGesture>>,
    on_tap: Option<TapHandler>,
}

/// Create a tap gesture wrapper
pub fn tap_gesture() -> TapGestureElement {
    TapGestureElement {
        child: None,
        recognizer: Arc::new(Mutex::new(TapGesture::new())),
        on_tap: None,
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
        F: Fn(&TapGestureEvent, &mut App) + Send + Sync + 'static,
    {
        self.on_tap = Some(Arc::new(handler));
        self
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

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        cx: &mut WindowContext,
    ) -> (LayoutId, Self::RequestLayoutState) {
        if let Some(mut child) = self.child.take() {
            let layout_id = child.request_layout(cx);
            (layout_id, Some(child))
        } else {
            let layout_id = cx.request_layout(gpui::Style::default(), []);
            (layout_id, None)
        }
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        cx: &mut WindowContext,
    ) -> Self::PrepaintState {
        if let Some(child) = child {
            child.prepaint(cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        cx: &mut WindowContext,
    ) {
        if let Some(child) = child {
            child.paint(cx);
        }

        let recognizer = self.recognizer.clone();

        cx.on_mouse_event(move |event: &MouseDownEvent, phase, _cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }

            let position = Point::new(event.position.x.0, event.position.y.0);
            let touch_event =
                TouchEvent::new(TouchAction::Down, vec![TouchPointer::new(0, position.x, position.y)]);

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);
            }
        });

        let recognizer = self.recognizer.clone();
        let on_tap = self.on_tap.clone();

        cx.on_mouse_event(move |event: &MouseUpEvent, phase, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }

            let position = Point::new(event.position.x.0, event.position.y.0);
            let touch_event =
                TouchEvent::new(TouchAction::Up, vec![TouchPointer::new(0, position.x, position.y)]);

            if let Ok(mut rec) = recognizer.lock() {
                rec.on_touch(&touch_event);

                if let Some(gesture_event) = rec.take_event() {
                    if gesture_event.state == GestureState::Ended {
                        if let Some(handler) = &on_tap {
                            handler(&gesture_event, cx);
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
    child: Option<AnyElement>,
    recognizer: Arc<Mutex<PinchGesture>>,
    on_begin: Option<PinchHandler>,
    on_change: Option<PinchHandler>,
    on_end: Option<PinchHandler>,
}

/// Create a pinch gesture wrapper
pub fn pinch_gesture() -> PinchGestureElement {
    PinchGestureElement {
        child: None,
        recognizer: Arc::new(Mutex::new(PinchGesture::new())),
        on_begin: None,
        on_change: None,
        on_end: None,
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
        F: Fn(&PinchGestureEvent, &mut App) + Send + Sync + 'static,
    {
        self.on_begin = Some(Arc::new(handler));
        self
    }

    /// Set handler for pinch changes
    pub fn on_change<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PinchGestureEvent, &mut App) + Send + Sync + 'static,
    {
        self.on_change = Some(Arc::new(handler));
        self
    }

    /// Set handler for when pinch ends
    pub fn on_end<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PinchGestureEvent, &mut App) + Send + Sync + 'static,
    {
        self.on_end = Some(Arc::new(handler));
        self
    }

    /// Set handler for all pinch events
    pub fn on_pinch<F>(mut self, handler: F) -> Self
    where
        F: Fn(&PinchGestureEvent, &mut App) + Send + Sync + Clone + 'static,
    {
        let h1 = handler.clone();
        let h2 = handler.clone();
        let h3 = handler;
        self.on_begin = Some(Arc::new(move |e, cx| h1(e, cx)));
        self.on_change = Some(Arc::new(move |e, cx| h2(e, cx)));
        self.on_end = Some(Arc::new(move |e, cx| h3(e, cx)));
        self
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

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        cx: &mut WindowContext,
    ) -> (LayoutId, Self::RequestLayoutState) {
        if let Some(mut child) = self.child.take() {
            let layout_id = child.request_layout(cx);
            (layout_id, Some(child))
        } else {
            let layout_id = cx.request_layout(gpui::Style::default(), []);
            (layout_id, None)
        }
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        cx: &mut WindowContext,
    ) -> Self::PrepaintState {
        if let Some(child) = child {
            child.prepaint(cx);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        cx: &mut WindowContext,
    ) {
        if let Some(child) = child {
            child.paint(cx);
        }

        // Note: Pinch gestures require multi-touch which isn't available via mouse events
        // This will need JNI integration to receive actual multi-touch events
        // For now, this is a placeholder structure
    }
}
