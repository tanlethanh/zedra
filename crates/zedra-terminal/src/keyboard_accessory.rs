use alacritty_terminal::term::TermMode;
use gpui::{Keystroke, Modifiers};

use crate::keys::to_esc_str;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalKeyboardAccessoryAction {
    Escape,
    Tab,
    Left,
    Down,
    Up,
    Right,
    Enter,
    ShiftEnter,
}

impl TerminalKeyboardAccessoryAction {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "escape" => Self::Escape,
            "tab" => Self::Tab,
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

#[cfg(test)]
mod tests {
    use super::TerminalKeyboardAccessoryAction;

    #[test]
    fn parses_terminal_keyboard_accessory_actions() {
        for name in [
            "escape",
            "tab",
            "left",
            "down",
            "up",
            "right",
            "enter",
            "shift_enter",
        ] {
            assert!(TerminalKeyboardAccessoryAction::from_name(name).is_some());
        }
        assert_eq!(TerminalKeyboardAccessoryAction::from_name("unknown"), None);
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
            let bytes = TerminalKeyboardAccessoryAction::from_name(name)
                .and_then(|action| action.legacy_bytes())
                .unwrap();
            assert_eq!(bytes, expected);
        }
    }
}
