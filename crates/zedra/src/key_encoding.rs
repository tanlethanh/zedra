/// Terminal key encoder for the native keyboard accessory bar.
///
/// Native bars (iOS UIKit, Android View) send keystrokes as `(name, mods)`
/// where `name` is a stable identifier ("escape", "tab", "char:c", ...) and
/// `mods` is a `Mods` bitmask. This module turns that into the byte sequence
/// the remote shell expects.
///
/// Only the legacy xterm/VT encoding is implemented. CSI-u / kitty negotiation
/// would replace `encode_legacy` with a per-PTY mode in a later change.
/// Active modifier keys for a keystroke.
///
/// Stored as a `u8` so it crosses the FFI boundary as-is. Bits are deliberately
/// laid out to match the order used in the kitty keyboard protocol, so a future
/// CSI-u encoder can reuse the same value.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub struct Mods(pub u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(0b001);
    pub const ALT: Mods = Mods(0b010);
    pub const CTRL: Mods = Mods(0b100);

    pub fn from_bits(bits: u8) -> Self {
        Self(bits & 0b111)
    }

    pub fn contains(self, other: Mods) -> bool {
        (self.0 & other.0) == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        Mods(self.0 | rhs.0)
    }
}

impl std::ops::Sub for Mods {
    type Output = Mods;
    fn sub(self, rhs: Mods) -> Mods {
        Mods(self.0 & !rhs.0)
    }
}

/// Logical key the user pressed on the accessory bar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Key {
    Char(char),
    Esc,
    Tab,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PgUp,
    PgDn,
}

impl Key {
    /// Parse a wire name from the native bar. Char keys use a `char:` prefix
    /// so single bytes don't collide with named keys.
    pub fn parse(name: &str) -> Option<Self> {
        if let Some(rest) = name.strip_prefix("char:") {
            let mut chars = rest.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            return Some(Key::Char(c));
        }
        Some(match name {
            "escape" => Key::Esc,
            "tab" => Key::Tab,
            "enter" => Key::Enter,
            "backspace" => Key::Backspace,
            "delete" => Key::Delete,
            "left" => Key::Left,
            "right" => Key::Right,
            "up" => Key::Up,
            "down" => Key::Down,
            "home" => Key::Home,
            "end" => Key::End,
            "page_up" => Key::PgUp,
            "page_down" => Key::PgDn,
            _ => return None,
        })
    }
}

/// Encode `(key, mods)` as the bytes a legacy xterm-style PTY expects.
///
/// Encoding rules:
/// - `Ctrl + ASCII letter`        -> single byte `c & 0x1f`
/// - `Ctrl + @ [ \ ] ^ _ ?`       -> their classic control codes
/// - `Alt + key`                  -> `ESC` prefix + encode(key without Alt)
/// - `Shift + Tab`                -> `ESC [ Z` (BackTab)
/// - `Shift + Enter`              -> `LF` (`\n`)
/// - Arrow / Home / End / PgUp / PgDn with mods other than zero
///                                -> `CSI 1 ; <n> <final>` (or `CSI <code> ; <n> ~`)
///   where `n = 1 + shift + 2*alt + 4*ctrl`
/// - Shift alone on a letter      -> uppercase byte; on other chars Shift is ignored
///   (legacy terminals can't distinguish Shift on punctuation).
pub fn encode_legacy(key: &Key, mods: Mods) -> Vec<u8> {
    if mods.contains(Mods::ALT) {
        let without_alt = mods - Mods::ALT;
        let mut out = vec![0x1b];
        out.extend(encode_legacy(key, without_alt));
        return out;
    }

    match key {
        Key::Char(c) => encode_char(*c, mods),
        Key::Esc => vec![0x1b],
        Key::Tab => {
            if mods.contains(Mods::SHIFT) {
                b"\x1b[Z".to_vec()
            } else {
                vec![0x09]
            }
        }
        Key::Enter => {
            if mods.contains(Mods::SHIFT) {
                vec![0x0a]
            } else {
                vec![0x0d]
            }
        }
        Key::Backspace => vec![0x7f],
        Key::Delete => csi_tilde(b'3', mods),
        Key::Left => csi_arrow(b'D', mods),
        Key::Right => csi_arrow(b'C', mods),
        Key::Up => csi_arrow(b'A', mods),
        Key::Down => csi_arrow(b'B', mods),
        Key::Home => csi_arrow(b'H', mods),
        Key::End => csi_arrow(b'F', mods),
        Key::PgUp => csi_tilde(b'5', mods),
        Key::PgDn => csi_tilde(b'6', mods),
    }
}

fn encode_char(c: char, mods: Mods) -> Vec<u8> {
    if mods.contains(Mods::CTRL) {
        if let Some(byte) = ctrl_byte(c) {
            return vec![byte];
        }
    }
    let ch = if mods.contains(Mods::SHIFT) && c.is_ascii_alphabetic() {
        c.to_ascii_uppercase()
    } else {
        c
    };
    let mut buf = [0u8; 4];
    ch.encode_utf8(&mut buf).as_bytes().to_vec()
}

fn ctrl_byte(c: char) -> Option<u8> {
    let lower = c.to_ascii_lowercase();
    if lower.is_ascii_alphabetic() {
        return Some((lower as u8) & 0x1f);
    }
    Some(match c {
        '@' | ' ' => 0x00,
        '[' => 0x1b,
        '\\' => 0x1c,
        ']' => 0x1d,
        '^' => 0x1e,
        '_' => 0x1f,
        '?' => 0x7f,
        _ => return None,
    })
}

fn modifier_param(mods: Mods) -> u8 {
    let mut n: u8 = 1;
    if mods.contains(Mods::SHIFT) {
        n += 1;
    }
    if mods.contains(Mods::ALT) {
        n += 2;
    }
    if mods.contains(Mods::CTRL) {
        n += 4;
    }
    n
}

fn csi_arrow(final_byte: u8, mods: Mods) -> Vec<u8> {
    let param = modifier_param(mods);
    if param == 1 {
        vec![0x1b, b'[', final_byte]
    } else {
        let mut out = Vec::with_capacity(8);
        out.extend_from_slice(b"\x1b[1;");
        out.extend_from_slice(param.to_string().as_bytes());
        out.push(final_byte);
        out
    }
}

fn csi_tilde(code: u8, mods: Mods) -> Vec<u8> {
    let param = modifier_param(mods);
    let mut out = Vec::with_capacity(8);
    out.push(0x1b);
    out.push(b'[');
    out.push(code);
    if param != 1 {
        out.push(b';');
        out.extend_from_slice(param.to_string().as_bytes());
    }
    out.push(b'~');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_named_keys_match_legacy_sequences() {
        assert_eq!(encode_legacy(&Key::Esc, Mods::NONE), b"\x1b");
        assert_eq!(encode_legacy(&Key::Tab, Mods::NONE), b"\t");
        assert_eq!(encode_legacy(&Key::Enter, Mods::NONE), b"\r");
        assert_eq!(encode_legacy(&Key::Backspace, Mods::NONE), b"\x7f");
        assert_eq!(encode_legacy(&Key::Left, Mods::NONE), b"\x1b[D");
        assert_eq!(encode_legacy(&Key::Down, Mods::NONE), b"\x1b[B");
        assert_eq!(encode_legacy(&Key::Up, Mods::NONE), b"\x1b[A");
        assert_eq!(encode_legacy(&Key::Right, Mods::NONE), b"\x1b[C");
        assert_eq!(encode_legacy(&Key::Home, Mods::NONE), b"\x1b[H");
        assert_eq!(encode_legacy(&Key::End, Mods::NONE), b"\x1b[F");
        assert_eq!(encode_legacy(&Key::PgUp, Mods::NONE), b"\x1b[5~");
        assert_eq!(encode_legacy(&Key::PgDn, Mods::NONE), b"\x1b[6~");
        assert_eq!(encode_legacy(&Key::Delete, Mods::NONE), b"\x1b[3~");
    }

    #[test]
    fn shift_tab_emits_backtab_and_shift_enter_emits_lf() {
        assert_eq!(encode_legacy(&Key::Tab, Mods::SHIFT), b"\x1b[Z");
        assert_eq!(encode_legacy(&Key::Enter, Mods::SHIFT), b"\n");
    }

    #[test]
    fn ctrl_letter_uses_legacy_bitmask() {
        assert_eq!(encode_legacy(&Key::Char('c'), Mods::CTRL), b"\x03");
        assert_eq!(encode_legacy(&Key::Char('d'), Mods::CTRL), b"\x04");
        assert_eq!(encode_legacy(&Key::Char('r'), Mods::CTRL), b"\x12");
        assert_eq!(encode_legacy(&Key::Char('A'), Mods::CTRL), b"\x01");
    }

    #[test]
    fn ctrl_punctuation_maps_to_control_codes() {
        assert_eq!(encode_legacy(&Key::Char('['), Mods::CTRL), b"\x1b");
        assert_eq!(encode_legacy(&Key::Char('\\'), Mods::CTRL), b"\x1c");
        assert_eq!(encode_legacy(&Key::Char('?'), Mods::CTRL), b"\x7f");
        assert_eq!(encode_legacy(&Key::Char('@'), Mods::CTRL), b"\x00");
    }

    #[test]
    fn alt_prefixes_with_escape_for_any_key() {
        assert_eq!(encode_legacy(&Key::Char('b'), Mods::ALT), b"\x1bb");
        assert_eq!(encode_legacy(&Key::Enter, Mods::ALT), b"\x1b\r");
        assert_eq!(
            encode_legacy(&Key::Char('c'), Mods::ALT | Mods::CTRL),
            b"\x1b\x03"
        );
    }

    #[test]
    fn shift_uppercases_letters_but_leaves_other_chars_alone() {
        assert_eq!(encode_legacy(&Key::Char('a'), Mods::SHIFT), b"A");
        assert_eq!(encode_legacy(&Key::Char('1'), Mods::SHIFT), b"1");
    }

    #[test]
    fn modified_navigation_uses_csi_1_n_final_form() {
        // mod number: 1 + shift + 2*alt + 4*ctrl
        assert_eq!(encode_legacy(&Key::Left, Mods::SHIFT), b"\x1b[1;2D");
        assert_eq!(encode_legacy(&Key::Up, Mods::CTRL), b"\x1b[1;5A");
        assert_eq!(
            encode_legacy(&Key::End, Mods::SHIFT | Mods::CTRL),
            b"\x1b[1;6F"
        );
    }

    #[test]
    fn modified_tilde_keys_use_csi_code_n_tilde_form() {
        assert_eq!(encode_legacy(&Key::PgUp, Mods::CTRL), b"\x1b[5;5~");
        assert_eq!(encode_legacy(&Key::Delete, Mods::SHIFT), b"\x1b[3;2~");
    }

    #[test]
    fn parse_round_trips_wire_names() {
        assert_eq!(Key::parse("escape"), Some(Key::Esc));
        assert_eq!(Key::parse("char:c"), Some(Key::Char('c')));
        assert_eq!(Key::parse("char:"), None);
        assert_eq!(Key::parse("char:cd"), None);
        assert_eq!(Key::parse("bogus"), None);
    }
}
