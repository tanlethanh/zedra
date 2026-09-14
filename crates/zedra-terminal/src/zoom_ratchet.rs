use gpui::Pixels;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TerminalFontSize(u8);

impl TerminalFontSize {
    pub const DEFAULT: Self = Self(12);
    pub const MIN: Self = Self(9);
    pub const MAX: Self = Self(24);

    pub const fn new(value: u8) -> Self {
        let value = if value < Self::MIN.0 {
            Self::MIN.0
        } else if value > Self::MAX.0 {
            Self::MAX.0
        } else {
            value
        };
        Self(value)
    }

    pub const fn as_u8(self) -> u8 {
        self.0
    }

    pub const fn line_height_f32(self) -> f32 {
        self.0 as f32 / 0.75
    }

    pub fn logical_units(self) -> Pixels {
        gpui::px(self.0 as f32)
    }

    pub fn line_height(self) -> Pixels {
        gpui::px(self.line_height_f32())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoomStep {
    pub from: TerminalFontSize,
    pub to: TerminalFontSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ZoomDirection {
    Increase,
    Decrease,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerminalZoomRatchet {
    font_size: TerminalFontSize,
    active: bool,
    direction: Option<ZoomDirection>,
    remainder: f32,
    reversal_debt: f32,
}

impl Default for TerminalZoomRatchet {
    fn default() -> Self {
        Self::new(TerminalFontSize::DEFAULT)
    }
}

impl TerminalZoomRatchet {
    pub const STEP_THRESHOLD: f32 = 0.10;
    pub const REVERSAL_DEADBAND: f32 = 0.03;

    pub const fn new(font_size: TerminalFontSize) -> Self {
        Self {
            font_size,
            active: false,
            direction: None,
            remainder: 0.0,
            reversal_debt: 0.0,
        }
    }

    pub const fn font_size(self) -> TerminalFontSize {
        self.font_size
    }

    pub const fn remainder(self) -> f32 {
        self.remainder
    }

    pub const fn is_active(self) -> bool {
        self.active
    }

    pub fn set_font_size(&mut self, font_size: TerminalFontSize) {
        self.font_size = font_size;
        self.remainder = 0.0;
        self.direction = None;
        self.reversal_debt = 0.0;
    }

    pub fn begin(&mut self) {
        self.active = true;
        self.remainder = 0.0;
        self.direction = None;
        self.reversal_debt = 0.0;
    }

    pub fn end(&mut self) {
        self.active = false;
        self.remainder = 0.0;
        self.direction = None;
        self.reversal_debt = 0.0;
    }

    pub fn cancel(&mut self) {
        self.end();
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.remainder = 0.0;
        self.direction = None;
        self.reversal_debt = 0.0;
    }

    pub fn push_delta(&mut self, delta: f32) -> Vec<ZoomStep> {
        if !self.active || should_reject(delta) {
            return Vec::new();
        }
        ingest_delta(self, delta)
    }

    pub fn push_scale(&mut self, scale: f32) -> Vec<ZoomStep> {
        if scale <= 0.0 || !scale.is_finite() {
            return Vec::new();
        }
        self.push_delta(scale - 1.0)
    }
}

fn should_reject(delta: f32) -> bool {
    !delta.is_finite() || delta == 0.0
}

fn at_clamp_limits_outward(font_size: TerminalFontSize, direction: ZoomDirection) -> bool {
    matches!(direction, ZoomDirection::Increase if font_size == TerminalFontSize::MAX)
        || matches!(direction, ZoomDirection::Decrease if font_size == TerminalFontSize::MIN)
}

fn ingest_delta(ratchet: &mut TerminalZoomRatchet, delta: f32) -> Vec<ZoomStep> {
    let direction = if delta.is_sign_positive() {
        ZoomDirection::Increase
    } else {
        ZoomDirection::Decrease
    };
    if at_clamp_limits_outward(ratchet.font_size, direction) {
        ratchet.remainder = 0.0;
        ratchet.direction = None;
        ratchet.reversal_debt = 0.0;
        return Vec::new();
    }
    if let Some(active_direction) = ratchet.direction {
        if active_direction != direction {
            ratchet.reversal_debt += delta.abs();
            if ratchet.reversal_debt <= TerminalZoomRatchet::REVERSAL_DEADBAND {
                return Vec::new();
            }
            let excess = ratchet.reversal_debt - TerminalZoomRatchet::REVERSAL_DEADBAND;
            ratchet.direction = Some(direction);
            ratchet.remainder = excess;
            ratchet.reversal_debt = 0.0;
        } else {
            if ratchet.reversal_debt != 0.0 {
                ratchet.reversal_debt = 0.0;
            }
            ratchet.remainder += delta.abs();
        }
    } else {
        ratchet.direction = Some(direction);
        ratchet.remainder = delta.abs();
    }
    accumulate_steps(ratchet)
}

fn accumulate_steps(ratchet: &mut TerminalZoomRatchet) -> Vec<ZoomStep> {
    let direction = match ratchet.direction {
        Some(direction) => direction,
        None => return Vec::new(),
    };
    let requested = (ratchet.remainder / TerminalZoomRatchet::STEP_THRESHOLD).floor() as usize;
    if requested == 0 {
        return Vec::new();
    }
    let available = match direction {
        ZoomDirection::Increase => {
            (TerminalFontSize::MAX.as_u8() - ratchet.font_size.as_u8()) as usize
        }
        ZoomDirection::Decrease => {
            (ratchet.font_size.as_u8() - TerminalFontSize::MIN.as_u8()) as usize
        }
    };
    if available == 0 {
        ratchet.remainder = 0.0;
        return Vec::new();
    }
    let accepted = requested.min(available);
    ratchet.remainder -= accepted as f32 * TerminalZoomRatchet::STEP_THRESHOLD;
    if accepted < requested {
        ratchet.remainder = 0.0;
    }
    let mut steps = Vec::with_capacity(accepted);
    for _ in 0..accepted {
        let from = ratchet.font_size;
        let to = TerminalFontSize::new(match direction {
            ZoomDirection::Increase => from.as_u8() + 1,
            ZoomDirection::Decrease => from.as_u8() - 1,
        });
        ratchet.font_size = to;
        steps.push(ZoomStep { from, to });
        if to == TerminalFontSize::MAX || to == TerminalFontSize::MIN {
            ratchet.remainder = 0.0;
            ratchet.direction = None;
            ratchet.reversal_debt = 0.0;
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::{TerminalFontSize, TerminalZoomRatchet};

    #[test]
    fn zoom_ratchet_trace_prints_accepted_step_sequence_and_final_font_size() {
        let mut ratchet = TerminalZoomRatchet::default();
        ratchet.begin();
        println!(
            "[ratchet-trace] initial font size: {:?}",
            ratchet.font_size()
        );
        let s1 = ratchet.push_delta(0.04);
        println!("[ratchet-trace] push_delta(0.04) -> steps: {:?}", s1);
        let s2 = ratchet.push_delta(0.04);
        println!("[ratchet-trace] push_delta(0.04) -> steps: {:?}", s2);
        let s3 = ratchet.push_delta(0.03);
        println!("[ratchet-trace] push_delta(0.03) -> steps: {:?}", s3);
        println!("[ratchet-trace] final font size: {:?}", ratchet.font_size());
        println!(
            "[ratchet-trace] final line height: {:?}",
            ratchet.font_size().line_height()
        );
        assert_eq!(s3.len(), 1);
        assert_eq!(ratchet.font_size(), TerminalFontSize::new(13));
    }

    #[test]
    fn failing_first_sequence_004_004_003_emits_exactly_one_12_to_13_step() {
        let mut ratchet = TerminalZoomRatchet::default();
        ratchet.begin();
        assert!(ratchet.push_delta(0.04).is_empty());
        assert!(ratchet.push_delta(0.04).is_empty());
        let steps = ratchet.push_delta(0.03);
        assert_eq!(steps.len(), 1);
        let step = steps[0];
        assert_eq!(step.from, TerminalFontSize::new(12));
        assert_eq!(step.to, TerminalFontSize::new(13));
        assert_eq!(ratchet.font_size(), TerminalFontSize::new(13));
    }

    #[test]
    fn reversal_deadband_discards_remainder_and_does_not_oscillate() {
        let mut ratchet = TerminalZoomRatchet::default();
        ratchet.begin();
        assert!(ratchet.push_delta(0.06).is_empty());
        let neg_within_deadband = ratchet.push_delta(-0.02);
        assert!(neg_within_deadband.is_empty());
        assert!(ratchet.push_delta(-0.02).is_empty());
        assert_eq!(ratchet.font_size(), TerminalFontSize::DEFAULT);
    }

    #[test]
    fn reversal_beyond_deadband_applies_excess_not_prior_remainder() {
        let mut ratchet = TerminalZoomRatchet::default();
        ratchet.begin();
        assert!(ratchet.push_delta(0.08).is_empty());
        let steps = ratchet.push_delta(-0.14);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].to, TerminalFontSize::new(11));
    }

    #[test]
    fn large_finite_delta_emits_bounded_steps_and_preserves_overshoot() {
        let mut ratchet = TerminalZoomRatchet::default();
        ratchet.begin();
        let steps = ratchet.push_delta(0.27);
        assert_eq!(steps.len(), 2);
        assert_eq!(ratchet.font_size(), TerminalFontSize::new(14));
        assert!((ratchet.remainder() - 0.07).abs() < 1e-6);
    }

    #[test]
    fn upper_and_lower_clamp_stop_outward_movement_and_clear_debt() {
        let mut upper = TerminalZoomRatchet::new(TerminalFontSize::MAX);
        upper.begin();
        assert!(upper.push_delta(0.50).is_empty());
        assert_eq!(upper.font_size(), TerminalFontSize::MAX);
        assert_eq!(upper.remainder(), 0.0);
        let down = upper.push_delta(-0.11);
        assert_eq!(down.len(), 1);
        assert_eq!(upper.font_size(), TerminalFontSize::new(23));

        let mut lower = TerminalZoomRatchet::new(TerminalFontSize::MIN);
        lower.begin();
        assert!(lower.push_delta(-0.50).is_empty());
        assert_eq!(lower.font_size(), TerminalFontSize::MIN);
        let up = lower.push_delta(0.11);
        assert_eq!(up.len(), 1);
        assert_eq!(lower.font_size(), TerminalFontSize::new(10));
    }

    #[test]
    fn rejects_nan_and_infinities_without_state_poison() {
        let mut ratchet = TerminalZoomRatchet::default();
        ratchet.begin();
        assert!(ratchet.push_delta(f32::NAN).is_empty());
        assert!(ratchet.push_delta(f32::INFINITY).is_empty());
        assert!(ratchet.push_delta(f32::NEG_INFINITY).is_empty());
        assert!(ratchet.push_scale(f32::NAN).is_empty());
        assert!(ratchet.push_scale(0.0).is_empty());
        assert_eq!(ratchet.font_size(), TerminalFontSize::DEFAULT);
        assert!(ratchet.push_delta(0.11).len() == 1);
    }

    #[test]
    fn stale_move_after_end_emits_no_effects() {
        let mut ratchet = TerminalZoomRatchet::default();
        ratchet.begin();
        ratchet.end();
        assert!(ratchet.push_delta(0.50).is_empty());
        assert_eq!(ratchet.font_size(), TerminalFontSize::DEFAULT);
    }
}
