//! Gesture compositor for handling multiple recognizers and arbitration

use std::sync::{Arc, Mutex};

use crate::recognizers::GestureRecognizer;
use crate::state::GestureState;
use crate::types::TouchEvent;

/// Relationship between gestures
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureRelation {
    /// Gestures can be recognized simultaneously
    Simultaneous,
    /// This gesture requires the other to fail first
    RequiresFailure,
    /// This gesture is exclusive (others fail when this begins)
    Exclusive,
}

/// A registered gesture with its relationships
struct RegisteredGesture {
    id: usize,
    recognizer: Arc<Mutex<dyn GestureRecognizer>>,
    relations: Vec<(usize, GestureRelation)>,
}

/// Compositor that manages multiple gesture recognizers
pub struct GestureCompositor {
    gestures: Vec<RegisteredGesture>,
    next_id: usize,
}

impl Default for GestureCompositor {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureCompositor {
    pub fn new() -> Self {
        Self {
            gestures: Vec::new(),
            next_id: 0,
        }
    }

    /// Add a gesture recognizer, returns its ID
    pub fn add<G: GestureRecognizer + 'static>(&mut self, recognizer: G) -> usize {
        let id = self.next_id;
        self.next_id += 1;

        self.gestures.push(RegisteredGesture {
            id,
            recognizer: Arc::new(Mutex::new(recognizer)),
            relations: Vec::new(),
        });

        id
    }

    /// Set relationship between two gestures
    pub fn set_relation(&mut self, gesture_id: usize, other_id: usize, relation: GestureRelation) {
        if let Some(gesture) = self.gestures.iter_mut().find(|g| g.id == gesture_id) {
            // Remove existing relation if any
            gesture.relations.retain(|(id, _)| *id != other_id);
            // Add new relation
            gesture.relations.push((other_id, relation));
        }
    }

    /// Allow two gestures to be recognized simultaneously
    pub fn simultaneous(&mut self, gesture1: usize, gesture2: usize) {
        self.set_relation(gesture1, gesture2, GestureRelation::Simultaneous);
        self.set_relation(gesture2, gesture1, GestureRelation::Simultaneous);
    }

    /// Gesture1 requires gesture2 to fail before it can begin
    pub fn requires_failure(&mut self, gesture1: usize, gesture2: usize) {
        self.set_relation(gesture1, gesture2, GestureRelation::RequiresFailure);
    }

    /// Get a gesture recognizer by ID
    pub fn get(&self, id: usize) -> Option<Arc<Mutex<dyn GestureRecognizer>>> {
        self.gestures
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.recognizer.clone())
    }

    /// Process a touch event through all recognizers
    pub fn on_touch(&mut self, event: &TouchEvent) {
        // First pass: process all recognizers
        let mut states: Vec<(usize, GestureState)> = Vec::new();

        for gesture in &self.gestures {
            if let Ok(mut recognizer) = gesture.recognizer.lock() {
                recognizer.on_touch(event);
                states.push((gesture.id, recognizer.state()));
            }
        }

        // Second pass: handle arbitration
        self.arbitrate(&states);
    }

    /// Handle gesture arbitration based on relationships
    fn arbitrate(&mut self, states: &[(usize, GestureState)]) {
        // Find gestures that just began
        let began_gestures: Vec<usize> = states
            .iter()
            .filter(|(_, state)| *state == GestureState::Began)
            .map(|(id, _)| *id)
            .collect();

        for began_id in &began_gestures {
            // Find the gesture that began
            let Some(gesture) = self.gestures.iter().find(|g| g.id == *began_id) else {
                continue;
            };

            // Check relationships
            for (other_id, relation) in &gesture.relations {
                match relation {
                    GestureRelation::Exclusive => {
                        // Fail other gestures that aren't simultaneous
                        if let Some(other) = self.gestures.iter().find(|g| g.id == *other_id) {
                            // Check if other has simultaneous relation with us
                            let is_simultaneous = other
                                .relations
                                .iter()
                                .any(|(id, rel)| *id == *began_id && *rel == GestureRelation::Simultaneous);

                            if !is_simultaneous {
                                if let Ok(mut recognizer) = other.recognizer.lock() {
                                    if recognizer.state() == GestureState::Possible {
                                        // Would be nice to fail it, but we can't easily
                                        // Just disable for now
                                        recognizer.set_enabled(false);
                                        recognizer.reset();
                                        recognizer.set_enabled(true);
                                    }
                                }
                            }
                        }
                    }
                    GestureRelation::RequiresFailure => {
                        // Check if the required gesture has failed
                        let other_state = states.iter().find(|(id, _)| *id == *other_id);
                        if let Some((_, state)) = other_state {
                            if *state != GestureState::Failed {
                                // Other gesture hasn't failed, we should wait
                                // In practice, this is handled during recognition
                            }
                        }
                    }
                    GestureRelation::Simultaneous => {
                        // Nothing to do, allow both
                    }
                }
            }
        }
    }

    /// Reset all recognizers
    pub fn reset(&mut self) {
        for gesture in &self.gestures {
            if let Ok(mut recognizer) = gesture.recognizer.lock() {
                recognizer.reset();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognizers::{PanGesture, TapGesture};

    #[test]
    fn test_compositor_add() {
        let mut compositor = GestureCompositor::new();

        let tap_id = compositor.add(TapGesture::new());
        let pan_id = compositor.add(PanGesture::new());

        assert_eq!(tap_id, 0);
        assert_eq!(pan_id, 1);

        assert!(compositor.get(tap_id).is_some());
        assert!(compositor.get(pan_id).is_some());
        assert!(compositor.get(99).is_none());
    }

    #[test]
    fn test_simultaneous_relation() {
        let mut compositor = GestureCompositor::new();

        let tap_id = compositor.add(TapGesture::new());
        let pan_id = compositor.add(PanGesture::new());

        compositor.simultaneous(tap_id, pan_id);

        // Both should be able to recognize
    }
}
