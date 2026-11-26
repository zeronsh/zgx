//! zgx's keys as Ladybird's key codes (`Web::UIEvents::KeyCode`, its
//! LibWebCommon/UIEvents/KeyCode.h) and modifier bits.

use zgx::input::{Key, Modifiers};

/// Ladybird's code for `key`, and the character it types (0 for none).
pub(super) fn code(key: &Key) -> (u32, u32) {
    match key {
        Key::Backspace => (0x08, 0),
        Key::Tab => (0x09, 0),
        Key::Enter => (0x0D, 0),
        Key::Escape => (0x1B, 0),
        Key::Space => (0x20, ' ' as u32),
        Key::PageUp => (0x21, 0),
        Key::PageDown => (0x22, 0),
        Key::End => (0x23, 0),
        Key::Home => (0x24, 0),
        Key::ArrowLeft => (0x25, 0),
        Key::ArrowUp => (0x26, 0),
        Key::ArrowRight => (0x27, 0),
        Key::ArrowDown => (0x28, 0),
        Key::Delete => (0x2E, 0),
        Key::Insert => (0x7E, 0),
        Key::ContextMenu => (0xAB, 0),
        Key::Function(n @ 1..=12) => (0x70 + u32::from(*n) - 1, 0),
        Key::Function(_) => (0, 0),
        Key::Character(text) => {
            let Some(c) = text.chars().next() else {
                return (0, 0);
            };
            (character_code(c), c as u32)
        }
    }
}

fn character_code(c: char) -> u32 {
    match c {
        'a'..='z' => 0x41 + (c as u32 - 'a' as u32),
        'A'..='Z' => 0x41 + (c as u32 - 'A' as u32),
        '0'..='9' => 0x30 + (c as u32 - '0' as u32),
        ')' => 0x60,
        '!' => 0x61,
        '@' => 0x62,
        '#' => 0x63,
        '$' => 0x64,
        '%' => 0x65,
        '^' => 0x66,
        '&' => 0x67,
        '*' => 0x68,
        '(' => 0x69,
        '+' => 0x6A,
        '-' => 0x6B,
        '/' => 0x6C,
        ',' => 0x6D,
        '.' => 0x6E,
        ':' => 0x6F,
        '"' => 0x7C,
        '\'' => 0x7D,
        ';' => 0x7F,
        '<' => 0x80,
        '=' => 0x81,
        '>' => 0x82,
        '?' => 0x83,
        '[' => 0x84,
        ']' => 0x85,
        '\\' => 0x86,
        '_' => 0x87,
        '{' => 0x88,
        '}' => 0x89,
        '|' => 0x8A,
        '~' => 0x8B,
        '`' => 0x8C,
        ' ' => 0x20,
        // A letter of another alphabet: no key of Ladybird's, its character still typed.
        _ => 0,
    }
}

/// Ladybird's modifier bits: Alt 1, Control 2, Shift 4, Super 8.
pub(super) fn modifiers(modifiers: Modifiers) -> u8 {
    u8::from(modifiers.alt)
        | u8::from(modifiers.control) << 1
        | u8::from(modifiers.shift) << 2
        | u8::from(modifiers.meta) << 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_take_ladybirds_codes() {
        assert_eq!(code(&Key::Enter), (0x0D, 0));
        assert_eq!(code(&Key::ArrowDown), (0x28, 0));
        assert_eq!(code(&Key::Function(12)), (0x7B, 0));
        assert_eq!(code(&Key::Character("a".into())), (0x41, 'a' as u32));
        assert_eq!(code(&Key::Character("Q".into())), (0x51, 'Q' as u32));
        assert_eq!(code(&Key::Character("7".into())), (0x37, '7' as u32));
        assert_eq!(code(&Key::Character("é".into())), (0, 'é' as u32));
        let all = Modifiers {
            shift: true,
            control: true,
            alt: true,
            meta: true,
        };
        assert_eq!(modifiers(all), 0b1111);
        assert_eq!(modifiers(Modifiers::default()), 0);
    }
}
