/// Lightweight gesture arena for disambiguating drawer pan vs content scroll.
///
/// Inspired by react-native-gesture-handler's key concepts:
/// - State machine: Undetermined → Active / Failed per recognizer
/// - Fail offsets: a recognizer fails early if cross-axis exceeds threshold
/// - Active offsets: a recognizer activates when primary axis exceeds threshold
/// - Arena: first recognizer to activate wins; losers are failed

/// Gesture recognizer lifecycle.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GestureState {
    /// Collecting touch data, not yet decided.
    Undetermined,
    /// Recognizer has claimed the gesture.
    Active,
    /// Recognizer lost the competition or violated its constraints.
    Failed,
}

/// What action the winning gesture drives.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GestureKind {
    /// Horizontal pan (drawer open/close).
    DrawerPan,
    /// Vertical pan (content scrolling) — also the default/fallback.
    Scroll,
}

/// A pan gesture recognizer with activation and failure thresholds.
///
/// - `active_offset`: min accumulated movement on primary axis to activate
/// - `fail_offset`: max accumulated movement on cross-axis before failing
pub struct PanRecognizer {
    pub kind: GestureKind,
    pub state: GestureState,
    /// Activate when primary-axis accumulation exceeds this (e.g. 20px for drawer).
    pub active_offset: f32,
    /// Fail when cross-axis accumulation exceeds this (e.g. 15px).
    pub fail_offset: f32,
    /// Which axis is primary (true = horizontal, false = vertical).
    pub horizontal: bool,
    accum_x: f32,
    accum_y: f32,
}

impl PanRecognizer {
    pub fn new(kind: GestureKind, horizontal: bool, active_offset: f32, fail_offset: f32) -> Self {
        Self {
            kind,
            state: GestureState::Undetermined,
            active_offset,
            fail_offset,
            horizontal,
            accum_x: 0.0,
            accum_y: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.state = GestureState::Undetermined;
        self.accum_x = 0.0;
        self.accum_y = 0.0;
    }

    /// Feed a touch delta. Returns new state.
    pub fn on_move(&mut self, dx: f32, dy: f32) -> GestureState {
        if self.state != GestureState::Undetermined {
            return self.state;
        }
        self.accum_x += dx;
        self.accum_y += dy;

        let (primary, cross) = if self.horizontal {
            (self.accum_x.abs(), self.accum_y.abs())
        } else {
            (self.accum_y.abs(), self.accum_x.abs())
        };

        // Fail-fast: cross-axis exceeded limit
        if cross > self.fail_offset {
            self.state = GestureState::Failed;
        }
        // Activate: primary axis exceeded threshold
        else if primary > self.active_offset {
            self.state = GestureState::Active;
        }
        self.state
    }
}

/// Arena that races multiple recognizers. First to activate wins.
pub struct GestureArena {
    recognizers: Vec<PanRecognizer>,
    winner: Option<usize>,
}

impl GestureArena {
    pub fn new() -> Self {
        Self {
            recognizers: Vec::new(),
            winner: None,
        }
    }

    /// Create the default arena with drawer + scroll recognizers.
    pub fn default_drawer_scroll() -> Self {
        let mut arena = Self::new();
        arena.add(PanRecognizer::new(
            GestureKind::DrawerPan,
            true,  // horizontal
            10.0,  // activate after 10px horizontal
            12.0,  // fail after 12px vertical
        ));
        arena.add(PanRecognizer::new(
            GestureKind::Scroll,
            false, // vertical
            2.0,   // activate after 2px vertical — near-instant scroll start
            12.0,  // fail after 12px horizontal
        ));
        arena
    }

    pub fn add(&mut self, recognizer: PanRecognizer) {
        self.recognizers.push(recognizer);
    }

    pub fn reset(&mut self) {
        self.winner = None;
        for rec in &mut self.recognizers {
            rec.reset();
        }
    }

    pub fn winner(&self) -> Option<GestureKind> {
        self.winner.map(|i| self.recognizers[i].kind)
    }

    /// Return the accumulated (x, y) delta for a recognizer by kind.
    /// Used to replay buffered movement when a winner is first determined.
    pub fn accumulated_delta(&self, kind: GestureKind) -> (f32, f32) {
        for rec in &self.recognizers {
            if rec.kind == kind {
                return (rec.accum_x, rec.accum_y);
            }
        }
        (0.0, 0.0)
    }

    /// Feed delta to all undetermined recognizers. Returns the winner if one just activated.
    pub fn on_move(&mut self, dx: f32, dy: f32) -> Option<GestureKind> {
        if self.winner.is_some() {
            return self.winner();
        }
        for i in 0..self.recognizers.len() {
            self.recognizers[i].on_move(dx, dy);
            if self.recognizers[i].state == GestureState::Active {
                self.winner = Some(i);
                // Fail all other undetermined recognizers
                for j in 0..self.recognizers.len() {
                    if j != i && self.recognizers[j].state == GestureState::Undetermined {
                        self.recognizers[j].state = GestureState::Failed;
                    }
                }
                return Some(self.recognizers[i].kind);
            }
        }
        None
    }
}
