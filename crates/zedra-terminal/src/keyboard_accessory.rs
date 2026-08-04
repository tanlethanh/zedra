use alacritty_terminal::term::TermMode;
use gpui::{Keystroke, Modifiers};

use crate::keys::to_esc_str;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessoryKey {
    Escape,
    Tab,
    Backspace,
    Left,
    Down,
    Up,
    Right,
    Enter,
    ShiftEnter,
}

impl AccessoryKey {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "escape" => Self::Escape,
            "tab" => Self::Tab,
            "backspace" => Self::Backspace,
            "left" => Self::Left,
            "down" => Self::Down,
            "up" => Self::Up,
            "right" => Self::Right,
            "enter" => Self::Enter,
            "shift_enter" => Self::ShiftEnter,
            _ => return None,
        })
    }

    pub fn keystroke(self) -> Keystroke {
        let (key, modifiers) = match self {
            Self::Escape => ("escape", Modifiers::default()),
            Self::Tab => ("tab", Modifiers::default()),
            Self::Backspace => ("backspace", Modifiers::default()),
            Self::Left => ("left", Modifiers::default()),
            Self::Down => ("down", Modifiers::default()),
            Self::Up => ("up", Modifiers::default()),
            Self::Right => ("right", Modifiers::default()),
            Self::Enter => ("enter", Modifiers::default()),
            Self::ShiftEnter => (
                "enter",
                Modifiers {
                    shift: true,
                    ..Default::default()
                },
            ),
        };
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: None,
        }
    }

    pub fn legacy_bytes(self) -> Option<Vec<u8>> {
        let mode = TermMode::empty();
        to_esc_str(&self.keystroke(), &mode, false).map(|bytes| bytes.as_bytes().to_vec())
    }
}

/// Sticky modifiers held by the extended keypad.
///
/// State lives here rather than in the native bar because it must apply to two
/// input paths that never meet natively: keys pressed on the bar itself, and
/// characters committed by the software keyboard's IME. A one-shot modifier is
/// consumed by the next key from either path; a locked one persists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierKey {
    Shift,
    Ctrl,
    Alt,
    Cmd,
}

impl ModifierKey {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "shift" => Self::Shift,
            "ctrl" => Self::Ctrl,
            "alt" => Self::Alt,
            "cmd" => Self::Cmd,
            _ => return None,
        })
    }

    fn armed_bit(self) -> u32 {
        match self {
            Self::Shift => 1,
            Self::Ctrl => 2,
            Self::Alt => 4,
            Self::Cmd => 8,
        }
    }

    fn modifiers(self) -> Modifiers {
        match self {
            Self::Shift => Modifiers::shift(),
            Self::Ctrl => Modifiers::control(),
            Self::Alt => Modifiers::alt(),
            Self::Cmd => Modifiers::command(),
        }
    }

    fn locked_bit(self) -> u32 {
        self.armed_bit() << MODIFIER_LOCK_SHIFT
    }
}

/// How far the locked bits sit above the armed ones.
const MODIFIER_LOCK_SHIFT: u32 = 4;
const MODIFIER_BITS: u32 = 0b1111;

/// Bits 0-3 armed (shift, ctrl, alt, cmd), bits 4-7 locked. Native bars read this
/// to render key highlights, so the encoding is part of the platform contract.
pub fn modifiers_from_mask(mask: u32) -> Modifiers {
    [
        ModifierKey::Shift,
        ModifierKey::Ctrl,
        ModifierKey::Alt,
        ModifierKey::Cmd,
    ]
    .into_iter()
    .filter(|key| mask & key.armed_bit() != 0)
    .fold(Modifiers::none(), |acc, key| acc | key.modifiers())
}

pub fn cycled_mask(mask: u32, key: ModifierKey) -> u32 {
    let armed = mask & key.armed_bit() != 0;
    let locked = mask & key.locked_bit() != 0;
    match (armed, locked) {
        (false, _) => mask | key.armed_bit(),
        (true, false) => mask | key.locked_bit(),
        (true, true) => mask & !(key.armed_bit() | key.locked_bit()),
    }
}

/// Drop every armed modifier that is not locked.
pub fn consumed_mask(mask: u32) -> u32 {
    let locked = (mask >> MODIFIER_LOCK_SHIFT) & MODIFIER_BITS;
    (locked << MODIFIER_LOCK_SHIFT) | locked
}

/// A single character typed on the extended keypad, e.g. `char:@`.
pub fn literal_keystroke(ch: char) -> Keystroke {
    Keystroke {
        modifiers: Modifiers::default(),
        key: ch.to_string(),
        key_char: Some(ch.to_string()),
    }
}

/// Whether a committed string is a single character a modifier can fold into.
/// Multi-character commits stay on the normal IME path.
pub fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_terminal_keyboard_accessory_actions() {
        for name in [
            "escape",
            "tab",
            "backspace",
            "left",
            "down",
            "up",
            "right",
            "enter",
            "shift_enter",
        ] {
            assert!(AccessoryKey::from_name(name).is_some());
        }
        assert_eq!(AccessoryKey::from_name("unknown"), None);
    }

    #[test]
    fn legacy_bytes_match_existing_native_accessory_route() {
        let cases = [
            ("escape", b"\x1b".as_slice()),
            ("tab", b"\x09".as_slice()),
            ("left", b"\x1b[D".as_slice()),
            ("down", b"\x1b[B".as_slice()),
            ("up", b"\x1b[A".as_slice()),
            ("right", b"\x1b[C".as_slice()),
            ("enter", b"\r".as_slice()),
            ("shift_enter", b"\n".as_slice()),
        ];

        for (name, expected) in cases {
            let bytes = AccessoryKey::from_name(name)
                .and_then(|action| action.legacy_bytes())
                .unwrap();
            assert_eq!(bytes, expected);
        }
    }

    #[test]
    fn sticky_modifier_cycles_armed_locked_cleared() {
        let armed = cycled_mask(0, ModifierKey::Ctrl);
        assert!(modifiers_from_mask(armed).control);

        // An armed modifier survives nothing but the next key.
        assert!(!modifiers_from_mask(consumed_mask(armed)).control);

        // Second tap locks: the modifier now outlives the keys it applies to.
        let locked = cycled_mask(armed, ModifierKey::Ctrl);
        assert!(modifiers_from_mask(consumed_mask(locked)).control);

        // Third tap clears it entirely.
        assert_eq!(cycled_mask(locked, ModifierKey::Ctrl), 0);
    }

    #[test]
    fn consuming_clears_armed_modifiers_but_keeps_locked_ones() {
        let locked_ctrl = cycled_mask(cycled_mask(0, ModifierKey::Ctrl), ModifierKey::Ctrl);
        let with_armed_alt = cycled_mask(locked_ctrl, ModifierKey::Alt);

        let after = modifiers_from_mask(consumed_mask(with_armed_alt));
        assert!(after.control, "locked ctrl survives");
        assert!(!after.alt, "armed alt is consumed");
    }

    #[test]
    fn literal_keystroke_carries_the_character() {
        let keystroke = literal_keystroke('@');
        assert_eq!(keystroke.key, "@");
        assert_eq!(keystroke.key_char.as_deref(), Some("@"));
    }

    #[test]
    fn accessory_keys_accept_modifiers() {
        let keystroke = AccessoryKey::Left.keystroke();
        assert_eq!(keystroke.key, "left");
        assert!(!keystroke.modifiers.control);
    }
}
