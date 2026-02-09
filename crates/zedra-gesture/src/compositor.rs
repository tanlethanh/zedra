//! Gesture compositor for handling multiple recognizers and arbitration
//!
//! The compositor manages gesture relationships and resolves conflicts:
//! - Simultaneous gestures can run together
//! - RequiresFailure gestures wait until dependencies fail
//! - Exclusive gestures cancel others when they begin

use std::collections::HashSet;
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

/// State of a gesture in the arbitration process
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArbitrationState {
    /// Ready to begin when conditions are met
    Ready,
    /// Waiting for other gestures to fail (requiresFailure)
    Awaiting,
    /// Currently active
    Active,
    /// Gesture has ended or failed
    Finished,
}

/// A registered gesture with its relationships and arbitration state
struct RegisteredGesture {
    id: usize,
    recognizer: Arc<Mutex<dyn GestureRecognizer>>,
    relations: Vec<(usize, GestureRelation)>,
    arbitration_state: ArbitrationState,
}

/// Compositor that manages multiple gesture recognizers with improved arbitration
///
/// # Arbitration Rules
///
/// 1. **Simultaneous**: Both gestures can be active at the same time
/// 2. **RequiresFailure**: Gesture waits until the required gesture fails
/// 3. **Exclusive**: When gesture begins, non-simultaneous gestures are cancelled
///
/// # Example
/// ```ignore
/// let mut compositor = GestureCompositor::new();
///
/// let tap_id = compositor.add(TapGesture::new().number_of_taps(2));
/// let single_tap_id = compositor.add(TapGesture::new());
///
/// // Single tap waits for double tap to fail
/// compositor.requires_failure(single_tap_id, tap_id);
/// ```
pub struct GestureCompositor {
    gestures: Vec<RegisteredGesture>,
    next_id: usize,
    /// Gestures currently awaiting dependencies to fail
    awaiting: HashSet<usize>,
    /// Gestures that have been activated in the current touch sequence
    active: HashSet<usize>,
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
            awaiting: HashSet::new(),
            active: HashSet::new(),
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
            arbitration_state: ArbitrationState::Ready,
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

    /// Make gesture exclusive (cancels others when it begins)
    pub fn exclusive(&mut self, gesture_id: usize, other_ids: &[usize]) {
        for &other_id in other_ids {
            self.set_relation(gesture_id, other_id, GestureRelation::Exclusive);
        }
    }

    /// Get a gesture recognizer by ID
    pub fn get(&self, id: usize) -> Option<Arc<Mutex<dyn GestureRecognizer>>> {
        self.gestures
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.recognizer.clone())
    }

    /// Get the current state of all gestures
    pub fn states(&self) -> Vec<(usize, GestureState)> {
        self.gestures
            .iter()
            .filter_map(|g| g.recognizer.lock().ok().map(|rec| (g.id, rec.state())))
            .collect()
    }

    /// Check if a gesture has any pending requirements (requires_failure)
    fn has_pending_requirements(&self, gesture_id: usize) -> bool {
        let Some(gesture) = self.gestures.iter().find(|g| g.id == gesture_id) else {
            return false;
        };

        for (other_id, relation) in &gesture.relations {
            if *relation == GestureRelation::RequiresFailure {
                // Check if the required gesture has failed
                if let Some(other) = self.gestures.iter().find(|g| g.id == *other_id) {
                    if let Ok(recognizer) = other.recognizer.lock() {
                        let state = recognizer.state();
                        // If not failed and not finished, requirement is pending
                        if state != GestureState::Failed
                            && state != GestureState::Ended
                            && state != GestureState::Cancelled
                        {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Check if gesture can run simultaneously with active gestures
    fn can_run_simultaneously(&self, gesture_id: usize) -> bool {
        let Some(gesture) = self.gestures.iter().find(|g| g.id == gesture_id) else {
            return true;
        };

        for &active_id in &self.active {
            if active_id == gesture_id {
                continue;
            }

            // Check if we have a simultaneous relation with the active gesture
            let is_simultaneous = gesture
                .relations
                .iter()
                .any(|(id, rel)| *id == active_id && *rel == GestureRelation::Simultaneous);

            if !is_simultaneous {
                // Check the other direction
                let other = self.gestures.iter().find(|g| g.id == active_id);
                if let Some(other) = other {
                    let other_simultaneous = other.relations.iter().any(|(id, rel)| {
                        *id == gesture_id && *rel == GestureRelation::Simultaneous
                    });
                    if !other_simultaneous {
                        return false;
                    }
                }
            }
        }

        true
    }

    /// Process a touch event through all recognizers
    pub fn on_touch(&mut self, event: &TouchEvent) {
        // First pass: process all recognizers
        let mut states: Vec<(usize, GestureState, GestureState)> = Vec::new();

        for gesture in &self.gestures {
            if let Ok(mut recognizer) = gesture.recognizer.lock() {
                let old_state = recognizer.state();
                recognizer.on_touch(event);
                let new_state = recognizer.state();
                states.push((gesture.id, old_state, new_state));
            }
        }

        // Second pass: handle state transitions and arbitration
        self.on_state_changes(&states);
    }

    /// Handle gesture state changes and perform arbitration
    fn on_state_changes(&mut self, changes: &[(usize, GestureState, GestureState)]) {
        // Process each state change
        for &(id, old_state, new_state) in changes {
            if old_state == new_state {
                continue;
            }

            self.on_single_state_change(id, new_state);
        }
    }

    /// Handle a single gesture state change
    fn on_single_state_change(&mut self, id: usize, new_state: GestureState) {
        match new_state {
            GestureState::Began => {
                // Check if this gesture has pending requirements
                if self.has_pending_requirements(id) {
                    // Put in awaiting state
                    self.awaiting.insert(id);

                    // Reset the gesture so it can try again when requirements are met
                    if let Some(gesture) = self.gestures.iter().find(|g| g.id == id) {
                        if let Ok(mut recognizer) = gesture.recognizer.lock() {
                            recognizer.reset();
                        }
                    }
                } else {
                    // Can activate - cancel conflicting gestures
                    self.active.insert(id);
                    self.cancel_conflicting(id);
                }
            }

            GestureState::Failed => {
                // Remove from active/awaiting
                self.active.remove(&id);
                self.awaiting.remove(&id);

                // Check if any awaiting gestures can now proceed
                self.activate_waiting_on_failure(id);
            }

            GestureState::Ended | GestureState::Cancelled => {
                // Remove from active
                self.active.remove(&id);
                self.awaiting.remove(&id);

                // For ended, also check if any awaiting gestures can proceed
                self.activate_waiting_on_failure(id);
            }

            GestureState::Changed => {
                // Gesture continues - no arbitration needed
            }

            GestureState::Possible => {
                // Reset state - clear from tracking
                self.active.remove(&id);
                self.awaiting.remove(&id);
            }
        }
    }

    /// Cancel gestures that conflict with the activating gesture
    fn cancel_conflicting(&mut self, activating_id: usize) {
        let Some(activating) = self.gestures.iter().find(|g| g.id == activating_id) else {
            return;
        };

        // Find exclusive relations
        let exclusive_targets: Vec<usize> = activating
            .relations
            .iter()
            .filter_map(|(id, rel)| {
                if *rel == GestureRelation::Exclusive {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();

        // Cancel exclusive targets
        for target_id in exclusive_targets {
            if let Some(target) = self.gestures.iter().find(|g| g.id == target_id) {
                if let Ok(mut recognizer) = target.recognizer.lock() {
                    let state = recognizer.state();
                    if state == GestureState::Possible || state.is_active() {
                        recognizer.set_enabled(false);
                        recognizer.reset();
                        recognizer.set_enabled(true);
                    }
                }
            }
        }

        // Also cancel gestures that don't have simultaneous relationship
        for gesture in &self.gestures {
            if gesture.id == activating_id {
                continue;
            }

            // Check if this gesture has a simultaneous relation with the activating one
            let has_simultaneous_with_activating = gesture
                .relations
                .iter()
                .any(|(id, rel)| *id == activating_id && *rel == GestureRelation::Simultaneous);

            let activating_has_simultaneous = activating
                .relations
                .iter()
                .any(|(id, rel)| *id == gesture.id && *rel == GestureRelation::Simultaneous);

            // If neither has simultaneous relation, the gesture should be cancelled
            if !has_simultaneous_with_activating && !activating_has_simultaneous {
                if let Ok(mut recognizer) = gesture.recognizer.lock() {
                    let state = recognizer.state();
                    if state == GestureState::Possible {
                        // Only cancel if still possible, not if already active
                        recognizer.set_enabled(false);
                        recognizer.reset();
                        recognizer.set_enabled(true);
                    }
                }
            }
        }
    }

    /// Activate gestures that were waiting for the failed gesture
    fn activate_waiting_on_failure(&mut self, failed_id: usize) {
        let awaiting: Vec<usize> = self.awaiting.iter().copied().collect();

        for gesture_id in awaiting {
            // Check if this gesture was waiting for the failed one
            let was_waiting = self
                .gestures
                .iter()
                .find(|g| g.id == gesture_id)
                .map(|g| {
                    g.relations.iter().any(|(id, rel)| {
                        *id == failed_id && *rel == GestureRelation::RequiresFailure
                    })
                })
                .unwrap_or(false);

            if was_waiting && !self.has_pending_requirements(gesture_id) {
                // All requirements met - remove from awaiting
                self.awaiting.remove(&gesture_id);

                // The gesture will try to activate on the next touch event
            }
        }
    }

    /// Reset all recognizers and clear arbitration state
    pub fn reset(&mut self) {
        for gesture in &self.gestures {
            if let Ok(mut recognizer) = gesture.recognizer.lock() {
                recognizer.reset();
            }
        }

        self.awaiting.clear();
        self.active.clear();
    }

    /// Get number of active gestures
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Get number of awaiting gestures
    pub fn awaiting_count(&self) -> usize {
        self.awaiting.len()
    }

    /// Check if a specific gesture is active
    pub fn is_active(&self, id: usize) -> bool {
        self.active.contains(&id)
    }

    /// Check if a specific gesture is awaiting requirements
    pub fn is_awaiting(&self, id: usize) -> bool {
        self.awaiting.contains(&id)
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

        // Both should be able to recognize (verified by relations being set)
        assert!(compositor
            .gestures
            .iter()
            .find(|g| g.id == tap_id)
            .unwrap()
            .relations
            .contains(&(pan_id, GestureRelation::Simultaneous)));
    }

    #[test]
    fn test_requires_failure_relation() {
        let mut compositor = GestureCompositor::new();

        let double_tap_id = compositor.add(TapGesture::new().number_of_taps(2));
        let single_tap_id = compositor.add(TapGesture::new());

        // Single tap waits for double tap to fail
        compositor.requires_failure(single_tap_id, double_tap_id);

        assert!(compositor.has_pending_requirements(single_tap_id));
        assert!(!compositor.has_pending_requirements(double_tap_id));
    }

    #[test]
    fn test_active_and_awaiting_counts() {
        let compositor = GestureCompositor::new();

        assert_eq!(compositor.active_count(), 0);
        assert_eq!(compositor.awaiting_count(), 0);
    }

    #[test]
    fn test_reset_clears_state() {
        let mut compositor = GestureCompositor::new();

        compositor.add(TapGesture::new());
        compositor.add(PanGesture::new());

        compositor.reset();

        assert_eq!(compositor.active_count(), 0);
        assert_eq!(compositor.awaiting_count(), 0);
    }
}
