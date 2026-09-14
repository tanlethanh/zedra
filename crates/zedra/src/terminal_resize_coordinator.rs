use std::{fmt, num::NonZeroU16};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalGeometry {
    columns: NonZeroU16,
    rows: NonZeroU16,
}

impl TerminalGeometry {
    pub fn new(columns: u16, rows: u16) -> Result<Self, GeometryError> {
        let columns = NonZeroU16::new(columns).ok_or(GeometryError::ZeroColumns)?;
        let rows = NonZeroU16::new(rows).ok_or(GeometryError::ZeroRows)?;
        Ok(Self { columns, rows })
    }

    pub fn from_usize(columns: usize, rows: usize) -> Result<Self, GeometryError> {
        let columns = u16::try_from(columns).map_err(|_| GeometryError::ColumnsOverflow)?;
        let rows = u16::try_from(rows).map_err(|_| GeometryError::RowsOverflow)?;
        Self::new(columns, rows)
    }

    pub const fn columns(self) -> u16 {
        self.columns.get()
    }

    pub const fn rows(self) -> u16 {
        self.rows.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    ZeroColumns,
    ZeroRows,
    ColumnsOverflow,
    RowsOverflow,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ZeroColumns => "terminal columns must be nonzero",
            Self::ZeroRows => "terminal rows must be nonzero",
            Self::ColumnsOverflow => "terminal columns exceed u16",
            Self::RowsOverflow => "terminal rows exceed u16",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for GeometryError {}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ResizeGeneration(u64);

impl ResizeGeneration {
    pub const INITIAL: Self = Self(0);

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ReclaimEpoch(u64);

impl ReclaimEpoch {
    pub const INITIAL: Self = Self(0);

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RequestSequence(u64);

impl RequestSequence {
    pub const INITIAL: Self = Self(0);

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResizeReason {
    Layout,
    PinchStep,
    ActivationReclaim,
    InteractionReclaim,
    PostInputReclaim,
    ReconnectReclaim,
}

impl ResizeReason {
    pub const fn is_forced(self) -> bool {
        matches!(
            self,
            Self::ActivationReclaim
                | Self::InteractionReclaim
                | Self::PostInputReclaim
                | Self::ReconnectReclaim
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResizeIntent {
    geometry: TerminalGeometry,
    reason: ResizeReason,
    reclaim_epoch: ReclaimEpoch,
    generation: ResizeGeneration,
    request_id: RequestSequence,
}

impl ResizeIntent {
    pub const fn geometry(&self) -> TerminalGeometry {
        self.geometry
    }

    pub const fn reason(&self) -> ResizeReason {
        self.reason
    }

    pub const fn reclaim_epoch(&self) -> ReclaimEpoch {
        self.reclaim_epoch
    }

    pub const fn generation(&self) -> ResizeGeneration {
        self.generation
    }

    pub const fn request_id(&self) -> RequestSequence {
        self.request_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CoordinatorTransition {
    abort: Option<ResizeIntent>,
    dispatch: Option<ResizeIntent>,
}

impl CoordinatorTransition {
    pub const fn empty() -> Self {
        Self {
            abort: None,
            dispatch: None,
        }
    }

    pub fn abort_request(&self) -> Option<&ResizeIntent> {
        self.abort.as_ref()
    }

    pub fn dispatch_request(&self) -> Option<&ResizeIntent> {
        self.dispatch.as_ref()
    }

    pub fn into_actions(self) -> (Option<ResizeIntent>, Option<ResizeIntent>) {
        (self.abort, self.dispatch)
    }

    fn dispatch(intent: ResizeIntent) -> Self {
        Self {
            abort: None,
            dispatch: Some(intent),
        }
    }

    fn abort_and_dispatch(abort: Option<ResizeIntent>, dispatch: ResizeIntent) -> Self {
        Self {
            abort,
            dispatch: Some(dispatch),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoordinatorError {
    GenerationExhausted,
    ReclaimEpochExhausted,
    RequestSequenceExhausted,
}

impl fmt::Display for CoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::GenerationExhausted => "resize generation exhausted",
            Self::ReclaimEpochExhausted => "resize reclaim epoch exhausted",
            Self::RequestSequenceExhausted => "resize request sequence exhausted",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CoordinatorError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoordinatorSnapshot {
    generation: ResizeGeneration,
    desired: Option<ResizeIntent>,
    in_flight: Option<ResizeIntent>,
    pending: Option<ResizeIntent>,
    last_successful_geometry: Option<TerminalGeometry>,
}

impl CoordinatorSnapshot {
    pub const fn generation(&self) -> ResizeGeneration {
        self.generation
    }

    pub fn desired(&self) -> Option<&ResizeIntent> {
        self.desired.as_ref()
    }

    pub fn in_flight(&self) -> Option<&ResizeIntent> {
        self.in_flight.as_ref()
    }

    pub fn pending(&self) -> Option<&ResizeIntent> {
        self.pending.as_ref()
    }

    pub const fn last_successful_geometry(&self) -> Option<TerminalGeometry> {
        self.last_successful_geometry
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalResizeCoordinator {
    generation: ResizeGeneration,
    reclaim_epoch: ReclaimEpoch,
    request_sequence: RequestSequence,
    desired: Option<ResizeIntent>,
    in_flight: Option<ResizeIntent>,
    pending: Option<ResizeIntent>,
    last_successful_geometry: Option<TerminalGeometry>,
}

impl TerminalResizeCoordinator {
    pub const fn new() -> Self {
        Self {
            generation: ResizeGeneration::INITIAL,
            reclaim_epoch: ReclaimEpoch::INITIAL,
            request_sequence: RequestSequence::INITIAL,
            desired: None,
            in_flight: None,
            pending: None,
            last_successful_geometry: None,
        }
    }

    pub fn snapshot(&self) -> CoordinatorSnapshot {
        CoordinatorSnapshot {
            generation: self.generation,
            desired: self.desired.clone(),
            in_flight: self.in_flight.clone(),
            pending: self.pending.clone(),
            last_successful_geometry: self.last_successful_geometry,
        }
    }

    pub fn submit(
        &mut self,
        geometry: TerminalGeometry,
        reason: ResizeReason,
    ) -> Result<CoordinatorTransition, CoordinatorError> {
        if !reason.is_forced() && self.ordinary_is_redundant(geometry) {
            return Ok(CoordinatorTransition::empty());
        }

        let (intent, next_sequence, next_epoch) =
            self.allocate_intent(geometry, reason, self.generation)?;
        self.request_sequence = next_sequence;
        self.reclaim_epoch = next_epoch;
        self.desired = Some(intent.clone());

        if self.in_flight.is_some() {
            self.pending = Some(intent);
            Ok(CoordinatorTransition::empty())
        } else {
            self.in_flight = Some(intent.clone());
            Ok(CoordinatorTransition::dispatch(intent))
        }
    }

    pub fn complete(
        &mut self,
        completed: &ResizeIntent,
        outcome: DeliveryOutcome,
    ) -> CoordinatorTransition {
        if self.in_flight.as_ref() != Some(completed) {
            return CoordinatorTransition::empty();
        }

        let completed = match self.in_flight.take() {
            Some(intent) => intent,
            None => return CoordinatorTransition::empty(),
        };
        if outcome == DeliveryOutcome::Succeeded {
            self.last_successful_geometry = Some(completed.geometry);
        }

        let pending = self.pending.take();
        match pending {
            Some(next)
                if outcome == DeliveryOutcome::Succeeded
                    && !next.reason.is_forced()
                    && next.geometry == completed.geometry =>
            {
                self.desired = Some(completed);
                CoordinatorTransition::empty()
            }
            Some(next) => {
                self.desired = Some(next.clone());
                self.in_flight = Some(next.clone());
                CoordinatorTransition::dispatch(next)
            }
            None => {
                self.desired = Some(completed);
                CoordinatorTransition::empty()
            }
        }
    }

    pub fn reconnect(
        &mut self,
        geometry: TerminalGeometry,
    ) -> Result<CoordinatorTransition, CoordinatorError> {
        let next_generation = self.next_generation()?;
        let (intent, next_sequence, next_epoch) =
            self.allocate_intent(geometry, ResizeReason::ReconnectReclaim, next_generation)?;
        let abort = self.in_flight.take();

        self.generation = next_generation;
        self.request_sequence = next_sequence;
        self.reclaim_epoch = next_epoch;
        self.desired = Some(intent.clone());
        self.in_flight = Some(intent.clone());
        self.pending = None;
        self.last_successful_geometry = None;

        Ok(CoordinatorTransition::abort_and_dispatch(abort, intent))
    }

    pub fn invalidate_generation(&mut self) -> Result<CoordinatorTransition, CoordinatorError> {
        let next_generation = self.next_generation()?;
        let abort = self.in_flight.take();
        self.generation = next_generation;
        self.desired = None;
        self.pending = None;
        self.last_successful_geometry = None;
        Ok(CoordinatorTransition {
            abort,
            dispatch: None,
        })
    }

    pub fn replace_terminal(&mut self) -> Result<CoordinatorTransition, CoordinatorError> {
        self.invalidate_generation()
    }

    fn ordinary_is_redundant(&self, geometry: TerminalGeometry) -> bool {
        if let Some(pending) = &self.pending {
            pending.geometry == geometry
        } else if let Some(in_flight) = &self.in_flight {
            in_flight.geometry == geometry
        } else {
            self.last_successful_geometry == Some(geometry)
        }
    }

    fn next_generation(&self) -> Result<ResizeGeneration, CoordinatorError> {
        self.generation
            .0
            .checked_add(1)
            .map(ResizeGeneration)
            .ok_or(CoordinatorError::GenerationExhausted)
    }

    fn allocate_intent(
        &self,
        geometry: TerminalGeometry,
        reason: ResizeReason,
        generation: ResizeGeneration,
    ) -> Result<(ResizeIntent, RequestSequence, ReclaimEpoch), CoordinatorError> {
        let next_sequence = self
            .request_sequence
            .0
            .checked_add(1)
            .map(RequestSequence)
            .ok_or(CoordinatorError::RequestSequenceExhausted)?;
        let next_epoch = if reason.is_forced() {
            self.reclaim_epoch
                .0
                .checked_add(1)
                .map(ReclaimEpoch)
                .ok_or(CoordinatorError::ReclaimEpochExhausted)?
        } else {
            self.reclaim_epoch
        };
        Ok((
            ResizeIntent {
                geometry,
                reason,
                reclaim_epoch: next_epoch,
                generation,
                request_id: next_sequence,
            },
            next_sequence,
            next_epoch,
        ))
    }
}

impl Default for TerminalResizeCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CoordinatorTransition, DeliveryOutcome, ResizeIntent, ResizeReason, TerminalGeometry,
        TerminalResizeCoordinator,
    };

    #[derive(Default)]
    struct LocalResizeDedupFixture {
        last_emitted: Option<TerminalGeometry>,
        emitted: Vec<TerminalGeometry>,
    }

    impl LocalResizeDedupFixture {
        fn observe(&mut self, geometry: TerminalGeometry) {
            if self.last_emitted == Some(geometry) {
                return;
            }
            self.last_emitted = Some(geometry);
            self.emitted.push(geometry);
        }
    }

    fn geometry(columns: u16, rows: u16) -> TerminalGeometry {
        TerminalGeometry::new(columns, rows).expect("test geometry must be valid")
    }

    fn submit(
        coordinator: &mut TerminalResizeCoordinator,
        geometry: TerminalGeometry,
        reason: ResizeReason,
    ) -> CoordinatorTransition {
        coordinator
            .submit(geometry, reason)
            .expect("test sequence must not exhaust coordinator counters")
    }

    fn complete(
        coordinator: &mut TerminalResizeCoordinator,
        intent: &ResizeIntent,
        outcome: DeliveryOutcome,
    ) -> CoordinatorTransition {
        coordinator.complete(intent, outcome)
    }

    #[test]
    fn baseline_local_resize_dedup_suppresses_equal_geometry() {
        let mut fixture = LocalResizeDedupFixture::default();
        let initial = geometry(80, 24);
        fixture.observe(initial);
        fixture.observe(initial);
        assert_eq!(fixture.emitted, vec![initial]);
    }

    #[test]
    fn latest_wins_dispatches_a_then_c_when_b_is_replaced() {
        let mut coordinator = TerminalResizeCoordinator::new();
        let a = geometry(80, 24);
        let b = geometry(60, 20);
        let c = geometry(50, 16);
        let mut dispatched = Vec::new();
        let first = submit(&mut coordinator, a, ResizeReason::Layout);
        dispatched.push(
            first
                .dispatch_request()
                .expect("A must dispatch")
                .geometry(),
        );
        let _ = submit(&mut coordinator, b, ResizeReason::Layout);
        let _ = submit(&mut coordinator, c, ResizeReason::PinchStep);
        let in_flight_a = coordinator
            .snapshot()
            .in_flight()
            .cloned()
            .expect("A remains blocked");
        let after_a = complete(&mut coordinator, &in_flight_a, DeliveryOutcome::Succeeded);
        dispatched.push(
            after_a
                .dispatch_request()
                .expect("latest C must dispatch")
                .geometry(),
        );
        let in_flight_c = coordinator
            .snapshot()
            .in_flight()
            .cloned()
            .expect("C is now in flight");
        let _ = complete(&mut coordinator, &in_flight_c, DeliveryOutcome::Succeeded);
        assert_eq!(dispatched, vec![a, c]);
        assert_eq!(coordinator.snapshot().last_successful_geometry(), Some(c));
        assert!(coordinator.snapshot().pending().is_none());
    }

    #[test]
    fn ordinary_a_b_a_collapses_redundant_final_a_after_success() {
        let mut coordinator = TerminalResizeCoordinator::new();
        let a = geometry(80, 24);
        let b = geometry(60, 20);
        let first = submit(&mut coordinator, a, ResizeReason::Layout);
        let request_a = first.dispatch_request().cloned().expect("A must dispatch");
        let _ = submit(&mut coordinator, b, ResizeReason::Layout);
        let _ = submit(&mut coordinator, a, ResizeReason::Layout);
        let after_a = complete(&mut coordinator, &request_a, DeliveryOutcome::Succeeded);
        assert!(after_a.dispatch_request().is_none());
        assert_eq!(coordinator.snapshot().last_successful_geometry(), Some(a));
        assert_eq!(
            coordinator.snapshot().desired().map(ResizeIntent::geometry),
            Some(a)
        );
    }

    #[test]
    fn equal_size_forced_reclaims_have_distinct_epoch_identity() {
        let mut coordinator = TerminalResizeCoordinator::new();
        let a = geometry(80, 24);
        let layout = submit(&mut coordinator, a, ResizeReason::Layout);
        let request_layout = layout
            .dispatch_request()
            .cloned()
            .expect("layout must dispatch");
        let _ = complete(
            &mut coordinator,
            &request_layout,
            DeliveryOutcome::Succeeded,
        );
        let activation = submit(&mut coordinator, a, ResizeReason::ActivationReclaim);
        let request_activation = activation
            .dispatch_request()
            .cloned()
            .expect("activation reclaim must bypass geometry dedup");
        let activation_epoch = request_activation.reclaim_epoch();
        let _ = complete(
            &mut coordinator,
            &request_activation,
            DeliveryOutcome::Succeeded,
        );
        let interaction = submit(&mut coordinator, a, ResizeReason::InteractionReclaim);
        let request_interaction = interaction
            .dispatch_request()
            .cloned()
            .expect("later interaction reclaim must be retained");
        assert!(request_interaction.reclaim_epoch() > activation_epoch);
        assert_ne!(
            request_interaction.request_id(),
            request_activation.request_id()
        );
    }

    #[test]
    fn later_forced_reclaim_is_not_erased_by_equal_ordinary_intent() {
        let mut coordinator = TerminalResizeCoordinator::new();
        let a = geometry(80, 24);
        let first = submit(&mut coordinator, a, ResizeReason::Layout);
        let request_a = first.dispatch_request().cloned().expect("A must dispatch");
        let forced = submit(&mut coordinator, a, ResizeReason::ActivationReclaim);
        assert!(forced.dispatch_request().is_none());
        let _ = submit(&mut coordinator, a, ResizeReason::Layout);
        let after_a = complete(&mut coordinator, &request_a, DeliveryOutcome::Succeeded);
        let forced_request = after_a
            .dispatch_request()
            .cloned()
            .expect("forced reclaim must survive ordinary duplicate");
        assert_eq!(forced_request.reason(), ResizeReason::ActivationReclaim);
    }

    #[test]
    fn failure_does_not_mark_success_and_dispatches_latest_pending_once() {
        let mut coordinator = TerminalResizeCoordinator::new();
        let a = geometry(80, 24);
        let b = geometry(60, 20);
        let first = submit(&mut coordinator, a, ResizeReason::Layout);
        let request_a = first.dispatch_request().cloned().expect("A must dispatch");
        let _ = submit(&mut coordinator, b, ResizeReason::PinchStep);
        let after_failure = complete(&mut coordinator, &request_a, DeliveryOutcome::Failed);
        let request_b = after_failure
            .dispatch_request()
            .cloned()
            .expect("B must dispatch after A fails");
        assert_eq!(coordinator.snapshot().last_successful_geometry(), None);
        let after_b = complete(&mut coordinator, &request_b, DeliveryOutcome::Succeeded);
        assert!(after_b.dispatch_request().is_none());
        assert_eq!(coordinator.snapshot().last_successful_geometry(), Some(b));
    }

    #[test]
    fn stale_completion_cannot_clear_new_generation_work() {
        let mut coordinator = TerminalResizeCoordinator::new();
        let a = geometry(80, 24);
        let c = geometry(50, 16);
        let first = submit(&mut coordinator, a, ResizeReason::Layout);
        let request_a = first.dispatch_request().cloned().expect("A must dispatch");
        let reconnect = coordinator
            .reconnect(c)
            .expect("reconnect must create a new generation");
        let request_c = reconnect
            .dispatch_request()
            .cloned()
            .expect("C must dispatch immediately");
        let stale = complete(&mut coordinator, &request_a, DeliveryOutcome::Succeeded);
        assert!(stale.dispatch_request().is_none());
        assert_eq!(coordinator.snapshot().last_successful_geometry(), None);
        assert_eq!(coordinator.snapshot().in_flight(), Some(&request_c));
        let _ = complete(&mut coordinator, &request_c, DeliveryOutcome::Succeeded);
        assert_eq!(coordinator.snapshot().last_successful_geometry(), Some(c));
    }

    #[test]
    fn reconnect_aborts_blocked_a_and_dispatches_latest_c_immediately() {
        let mut coordinator = TerminalResizeCoordinator::new();
        let a = geometry(80, 24);
        let c = geometry(50, 16);
        let first = submit(&mut coordinator, a, ResizeReason::Layout);
        let request_a = first.dispatch_request().cloned().expect("A must dispatch");
        let reconnect = coordinator
            .reconnect(c)
            .expect("reconnect must invalidate the blocked generation");
        assert_eq!(reconnect.abort_request(), Some(&request_a));
        assert_eq!(
            reconnect.dispatch_request().map(ResizeIntent::geometry),
            Some(c)
        );
        assert_ne!(
            request_a.generation(),
            reconnect.dispatch_request().expect("C").generation()
        );
    }

    #[test]
    fn terminal_replacement_aborts_old_work_and_clears_success() {
        let mut coordinator = TerminalResizeCoordinator::new();
        let a = geometry(80, 24);
        let first = submit(&mut coordinator, a, ResizeReason::Layout);
        let request_a = first.dispatch_request().cloned().expect("A must dispatch");
        let replacement = coordinator
            .replace_terminal()
            .expect("replacement must invalidate the generation");
        assert_eq!(replacement.abort_request(), Some(&request_a));
        assert!(replacement.dispatch_request().is_none());
        assert!(coordinator.snapshot().last_successful_geometry().is_none());
        let stale = complete(&mut coordinator, &request_a, DeliveryOutcome::Succeeded);
        assert!(stale.dispatch_request().is_none());
        let next = submit(&mut coordinator, a, ResizeReason::Layout);
        assert!(next.dispatch_request().is_some());
    }

    #[test]
    fn geometry_rejects_zero_and_overflow_without_wrapping() {
        assert!(TerminalGeometry::new(0, 24).is_err());
        assert!(TerminalGeometry::new(80, 0).is_err());
        assert!(TerminalGeometry::from_usize(usize::from(u16::MAX) + 1, 24).is_err());
        assert!(TerminalGeometry::from_usize(80, usize::from(u16::MAX) + 1).is_err());
    }

    #[test]
    fn deterministic_fake_dispatch_trace_covers_blocked_replacement_and_reconnect_abort() {
        let mut coordinator = TerminalResizeCoordinator::new();
        let a = geometry(80, 24);
        let b = geometry(60, 20);
        let c = geometry(50, 16);
        let mut calls = Vec::new();
        let first = submit(&mut coordinator, a, ResizeReason::Layout);
        calls.push(first.dispatch_request().expect("A").geometry());
        println!(
            "fake-rpc dispatch A={a:?} state={:?}",
            coordinator.snapshot()
        );
        let _ = submit(&mut coordinator, b, ResizeReason::Layout);
        let _ = submit(&mut coordinator, c, ResizeReason::PinchStep);
        println!(
            "fake-rpc blocked A, replaced B with C state={:?}",
            coordinator.snapshot()
        );
        let request_a = coordinator.snapshot().in_flight().cloned().expect("A");
        let after_a = complete(&mut coordinator, &request_a, DeliveryOutcome::Succeeded);
        calls.push(after_a.dispatch_request().expect("C").geometry());
        println!(
            "fake-rpc release A, dispatch C={c:?} state={:?}",
            coordinator.snapshot()
        );
        let request_c = coordinator.snapshot().in_flight().cloned().expect("C");
        let _ = complete(&mut coordinator, &request_c, DeliveryOutcome::Succeeded);
        let reconnect = coordinator.reconnect(c).expect("reconnect");
        println!(
            "fake-rpc reconnect abort path state={:?}",
            coordinator.snapshot()
        );
        assert_eq!(calls, vec![a, c]);
        assert!(reconnect.dispatch_request().is_some());
    }
}
