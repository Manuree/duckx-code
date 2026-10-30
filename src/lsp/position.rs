//! Conversion between rope byte offsets and LSP `(line, character)` positions.
//! `character` counts UTF-8 bytes, UTF-16 code units, or code points, whichever
//! encoding the server chose during `initialize`.

use ropey::Rope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
    Utf32,
}

impl Encoding {
    /// The server's `positionEncoding`. Anything unknown is the protocol default, UTF-16.
    pub fn from_name(name: Option<&str>) -> Self {
        match name {
            Some("utf-8") => Self::Utf8,
            Some("utf-32") => Self::Utf32,
            _ => Self::Utf16,
        }
    }
}

/// `(line, character)` of byte offset `byte`, clamped to the end of the text.
pub fn position_of(rope: &Rope, byte: usize, encoding: Encoding) -> (u32, u32) {
    let byte = byte.min(rope.len_bytes());
    let line = rope.byte_to_line(byte);
    let character = match encoding {
        Encoding::Utf8 => byte - rope.line_to_byte(line),
        Encoding::Utf16 => {
            let char_idx = rope.byte_to_char(byte);
            rope.char_to_utf16_cu(char_idx) - rope.char_to_utf16_cu(rope.line_to_char(line))
        }
        Encoding::Utf32 => rope.byte_to_char(byte) - rope.line_to_char(line),
    };
    (to_u32(line), to_u32(character))
}

/// Byte offset of `(line, character)`. A line past the end is the end of the text;
/// a character past the end of its line is the end of that line, before the line break.
/// A position inside a multi-byte character lands on that character's first byte.
pub fn byte_of(rope: &Rope, line: u32, character: u32, encoding: Encoding) -> usize {
    let line = line as usize;
    if line >= rope.len_lines() {
        return rope.len_bytes();
    }
    let character = character as usize;
    let start_char = rope.line_to_char(line);
    let end_char = start_char + content_chars(rope, line);
    let char_idx = match encoding {
        Encoding::Utf8 => {
            let start_byte = rope.line_to_byte(line);
            let end_byte = rope.char_to_byte(end_char);
            rope.byte_to_char((start_byte + character).min(end_byte))
        }
        Encoding::Utf16 => {
            let start_cu = rope.char_to_utf16_cu(start_char);
            let end_cu = rope.char_to_utf16_cu(end_char);
            rope.utf16_cu_to_char((start_cu + character).min(end_cu))
        }
        Encoding::Utf32 => (start_char + character).min(end_char),
    };
    rope.char_to_byte(char_idx)
}

/// Chars on `line`, not counting its line break.
fn content_chars(rope: &Rope, line: usize) -> usize {
    let slice = rope.line(line);
    let mut len = slice.len_chars();
    if len > 0 && slice.char(len - 1) == '\n' {
        len -= 1;
    }
    if len > 0 && slice.char(len - 1) == '\r' {
        len -= 1;
    }
    len
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn byte_after(text: &str, needle: &str) -> usize {
        text.find(needle).unwrap() + needle.len()
    }

    #[test]
    fn ascii_is_the_same_in_every_encoding() {
        let rope = Rope::from_str("fn main() {\n    let x = 1;\n}\n");
        let byte = byte_after(&rope.to_string(), "let ");
        for encoding in [Encoding::Utf8, Encoding::Utf16, Encoding::Utf32] {
            assert_eq!(position_of(&rope, byte, encoding), (1, 8));
            assert_eq!(byte_of(&rope, 1, 8, encoding), byte);
        }
    }

    #[test]
    fn emoji_is_two_utf16_units_and_four_bytes() {
        let text = "a🦆b";
        let rope = Rope::from_str(text);
        let b = byte_after(text, "🦆");
        assert_eq!(position_of(&rope, b, Encoding::Utf8), (0, 5));
        assert_eq!(position_of(&rope, b, Encoding::Utf16), (0, 3));
        assert_eq!(position_of(&rope, b, Encoding::Utf32), (0, 2));
        assert_eq!(byte_of(&rope, 0, 3, Encoding::Utf16), b);
        assert_eq!(byte_of(&rope, 0, 2, Encoding::Utf32), b);
    }

    #[test]
    fn thai_combining_marks_count_as_separate_code_points() {
        // "ที่" is three code points (consonant + vowel + tone mark), nine UTF-8 bytes.
        let text = "ที่นี่ x";
        let rope = Rope::from_str(text);
        let x = text.find('x').unwrap();
        assert_eq!(position_of(&rope, x, Encoding::Utf8), (0, x as u32));
        assert_eq!(position_of(&rope, x, Encoding::Utf16), (0, 7));
        assert_eq!(byte_of(&rope, 0, 7, Encoding::Utf16), x);
    }

    #[test]
    fn crlf_is_one_line_break() {
        let text = "ab\r\ncd\r\n";
        let rope = Rope::from_str(text);
        let c = text.find('c').unwrap();
        assert_eq!(position_of(&rope, c, Encoding::Utf16), (1, 0));
        assert_eq!(byte_of(&rope, 1, 0, Encoding::Utf16), c);
        // Past the end of line 0 stops before the `\r`.
        assert_eq!(byte_of(&rope, 0, 99, Encoding::Utf16), 2);
    }

    #[test]
    fn out_of_range_positions_clamp() {
        let rope = Rope::from_str("one\ntwo");
        assert_eq!(byte_of(&rope, 9, 0, Encoding::Utf16), rope.len_bytes());
        assert_eq!(byte_of(&rope, 1, 99, Encoding::Utf8), rope.len_bytes());
        assert_eq!(position_of(&rope, 999, Encoding::Utf16), (1, 3));
    }

    #[test]
    fn a_utf8_offset_inside_a_character_snaps_to_its_start() {
        let rope = Rope::from_str("é");
        assert_eq!(byte_of(&rope, 0, 1, Encoding::Utf8), 0);
    }

    #[test]
    fn unknown_encoding_names_fall_back_to_utf16() {
        assert_eq!(Encoding::from_name(Some("utf-8")), Encoding::Utf8);
        assert_eq!(Encoding::from_name(Some("latin-1")), Encoding::Utf16);
        assert_eq!(Encoding::from_name(None), Encoding::Utf16);
    }
}
