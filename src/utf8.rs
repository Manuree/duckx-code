//! Char-boundary helpers for safe byte-index cuts on UTF-8 strings.
//!
//! Rust string indexing panics when a byte index lands inside a multi-byte
//! character (em dash, Thai, emoji, …). Prefer these over raw `&s[a..b]` when
//! the index comes from a length subtraction or overlap window.

/// Largest char boundary ≤ `index` (clamped to `s.len()`).
#[inline]
pub fn floor_char_boundary(s: &str, index: usize) -> usize {
    s.floor_char_boundary(index)
}

/// Smallest char boundary ≥ `index` (clamped to `s.len()`).
#[inline]
pub fn ceil_char_boundary(s: &str, index: usize) -> usize {
    s.ceil_char_boundary(index)
}

/// Split `s` at `floor_char_boundary(index)`.
#[inline]
pub fn split_at_floor(s: &str, index: usize) -> (&str, &str) {
    let i = floor_char_boundary(s, index);
    (&s[..i], &s[i..])
}

/// Split `s` at `ceil_char_boundary(index)`.
#[inline]
pub fn split_at_ceil(s: &str, index: usize) -> (&str, &str) {
    let i = ceil_char_boundary(s, index);
    (&s[..i], &s[i..])
}

/// Prefix of at most `max_bytes` bytes, ending on a char boundary.
#[inline]
pub fn prefix_bytes(s: &str, max_bytes: usize) -> &str {
    &s[..floor_char_boundary(s, max_bytes)]
}

/// Suffix kept when discarding a leading cut of `drop_bytes`, starting on a
/// char boundary (ceil so a mid-char cut keeps the whole character).
#[inline]
pub fn suffix_after_cut(s: &str, drop_bytes: usize) -> &str {
    &s[ceil_char_boundary(s, drop_bytes)..]
}

/// Human-readable panic payload for catch_unwind recovery paths.
pub fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "unknown panic".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_ceil_around_em_dash() {
        // "ab" + em dash (3 bytes) + "cd"
        let s = "ab—cd";
        assert_eq!(floor_char_boundary(s, 3), 2);
        assert_eq!(ceil_char_boundary(s, 3), 5);
        assert_eq!(prefix_bytes(s, 3), "ab");
        assert_eq!(suffix_after_cut(s, 3), "cd");
        let (a, b) = split_at_floor(s, 3);
        assert_eq!((a, b), ("ab", "—cd"));
        let (a, b) = split_at_ceil(s, 3);
        assert_eq!((a, b), ("ab—", "cd"));
    }

    #[test]
    fn thai_and_emoji_boundaries() {
        let thai = "สวัสดี"; // each letter 3 bytes
        assert_eq!(floor_char_boundary(thai, 1), 0);
        assert_eq!(ceil_char_boundary(thai, 1), 3);
        assert_eq!(prefix_bytes(thai, 4), "ส");

        let emoji = "hi👋x"; // waving hand is 4 bytes
        assert_eq!(floor_char_boundary(emoji, 3), 2);
        assert_eq!(ceil_char_boundary(emoji, 3), 6);
        assert_eq!(suffix_after_cut(emoji, 3), "x");
    }
}
