//! Text typed with the wrong keyboard layout, turned into what the same keys
//! give on another layout (`convert-keyboard`, `Alt+\``).
//!
//! Every layout lists what its 47 printable keys type, unshifted and shifted, in
//! the order of [`KEY_ORDER`]. Converting looks a character up on the source
//! layout and takes the character at the same key on the target. Thai vowels and
//! tone marks are separate keystrokes, so they are separate characters here too:
//! `ouj8nv8ezbf` is `น ี ่ ค ื อ ค ำ ผ ิ ด`, one key each. To add a layout, add a
//! [`Layout`] to [`LAYOUTS`]; nothing else changes.

/// Physical keys, in order, as a reference only: these are the US QWERTY labels.
pub const KEY_ORDER: &str = "`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./";

/// One keyboard layout.
#[derive(Debug, PartialEq, Eq)]
pub struct Layout {
    /// Short id, also used by tests and settings-style tokens.
    pub id: &'static str,
    /// Shown in the dropdown and in messages.
    pub name: &'static str,
    /// Windows language id of this layout's default keyboard (`0x0409`, `0x041E`).
    pub langid: u16,
    /// What each key of [`KEY_ORDER`] types without Shift.
    normal: &'static str,
    /// What each key types with Shift.
    shifted: &'static str,
    /// Sequences read as one key before converting from this layout. Thai text
    /// sometimes stores sara am as nikhahit + sara aa.
    compose: &'static [(&'static str, char)],
}

pub static US: Layout = Layout {
    id: "us",
    name: "English (US QWERTY)",
    langid: 0x0409,
    normal: "`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./",
    shifted: "~!@#$%^&*()_+QWERTYUIOP{}|ASDFGHJKL:\"ZXCVBNM<>?",
    compose: &[],
};

pub static THAI: Layout = Layout {
    id: "th",
    name: "Thai (Kedmanee)",
    langid: 0x041E,
    normal: "_ๅ/-ภถุึคตจขชๆไำพะัีรนยบลฃฟหกดเ้่าสวงผปแอิืทมใฝ",
    shifted: "%+๑๒๓๔ู฿๕๖๗๘๙๐\"ฎฑธํ๊ณฯญฐ,ฅฤฆฏโฌ็๋ษศซ.()ฉฮฺ์?ฒฬฦ",
    compose: &[("\u{0E4D}\u{0E32}", '\u{0E33}')],
};

/// Every layout `convert-keyboard` knows, in dropdown order.
pub static LAYOUTS: &[&Layout] = &[&US, &THAI];

impl Layout {
    /// `(key index, shifted)` for a character this layout types.
    fn key_of(&self, c: char) -> Option<(usize, bool)> {
        if let Some(i) = self.normal.chars().position(|k| k == c) {
            return Some((i, false));
        }
        self.shifted.chars().position(|k| k == c).map(|i| (i, true))
    }

    fn char_at(&self, index: usize, shifted: bool) -> Option<char> {
        let keys = if shifted { self.shifted } else { self.normal };
        keys.chars().nth(index)
    }

    /// True when some key on this layout types `c`.
    pub fn types(&self, c: char) -> bool {
        self.key_of(c).is_some()
    }
}

/// The US key under a non-ASCII character another layout types: `ส` (Thai `l` key)
/// gives `l`, and a shifted character gives the shifted US one. Used for shortcuts
/// pressed with a non-US layout active, which some terminals report as the typed
/// character. `None` for ASCII and for characters no layout types.
pub fn us_key(c: char) -> Option<char> {
    if c.is_ascii() {
        return None;
    }
    LAYOUTS
        .iter()
        .filter(|layout| !std::ptr::eq(**layout, &US))
        .find_map(|layout| {
            let (index, shifted) = layout.key_of(c)?;
            US.char_at(index, shifted)
        })
}

/// The layout `id` names.
pub fn by_id(id: &str) -> Option<&'static Layout> {
    LAYOUTS.iter().copied().find(|layout| layout.id == id)
}

/// The layout `text` was most likely typed on: the one with the most characters
/// no other layout types. Thai letters count for Thai, Latin letters and digits
/// for US. With no such character (`/-`), the first layout (US) is assumed, so the
/// text goes to the other one. `None` when no character of `text` is on any layout.
pub fn detect(text: &str) -> Option<&'static Layout> {
    if !text
        .chars()
        .any(|c| LAYOUTS.iter().any(|layout| layout.types(c)))
    {
        return None;
    }
    let score = |layout: &Layout| {
        text.chars()
            .filter(|&c| {
                layout.types(c)
                    && LAYOUTS
                        .iter()
                        .all(|other| std::ptr::eq(*other, layout) || !other.types(c))
            })
            .count()
    };
    let mut best = LAYOUTS[0];
    let mut best_score = score(best);
    for layout in &LAYOUTS[1..] {
        let s = score(layout);
        if s > best_score {
            best = layout;
            best_score = s;
        }
    }
    Some(best)
}

/// The other layout when only two are known; `None` with more, so the user picks.
pub fn other_of(from: &Layout, among: &[&'static Layout]) -> Option<&'static Layout> {
    if among.len() != 2 || !among.iter().any(|layout| std::ptr::eq(*layout, from)) {
        return None;
    }
    among
        .iter()
        .copied()
        .find(|layout| !std::ptr::eq(*layout, from))
}

/// `text` as the same keys would type it on `to`. Characters `from` does not
/// type (spaces, new lines, emoji, another script) stay as they are.
pub fn convert(text: &str, from: &Layout, to: &Layout) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'outer: while let Some(c) = rest.chars().next() {
        for (sequence, key) in from.compose {
            if let Some(after) = rest.strip_prefix(sequence) {
                out.push(map_char(*key, from, to));
                rest = after;
                continue 'outer;
            }
        }
        out.push(map_char(c, from, to));
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn map_char(c: char, from: &Layout, to: &Layout) -> char {
    from.key_of(c)
        .and_then(|(index, shifted)| to.char_at(index, shifted))
        .unwrap_or(c)
}

/// What the operating system can tell and do about input layouts.
pub mod os {
    use super::{LAYOUTS, Layout};

    /// The input layouts installed in the OS, each matched to a known layout
    /// (`None` for one this editor has no table for). `None` when the OS gives no
    /// list: every platform but Windows.
    pub fn installed() -> Option<Vec<Option<&'static Layout>>> {
        imp::installed().map(|ids| ids.into_iter().map(known).collect())
    }

    /// A Windows keyboard id we have a table for: the language's default
    /// keyboard only (high word equal to the low word, `0x041E041E`), so
    /// Thai Pattachote or US Dvorak are not mistaken for Kedmanee or QWERTY.
    pub(super) fn known(hkl: usize) -> Option<&'static Layout> {
        let lang = (hkl & 0xFFFF) as u16;
        let device = ((hkl >> 16) & 0xFFFF) as u16;
        if device != lang {
            return None;
        }
        LAYOUTS.iter().copied().find(|layout| layout.langid == lang)
    }

    /// The target to convert to with no prompt: the OS has exactly two input
    /// layouts, both known, and `from` is one of them.
    pub fn target(
        from: &'static Layout,
        installed: &[Option<&'static Layout>],
    ) -> Option<&'static Layout> {
        let known: Option<Vec<&'static Layout>> = installed.iter().copied().collect();
        super::other_of(from, &known?)
    }

    /// Asks the OS to make `to` the active input layout of the terminal window.
    /// Best effort: `true` only means the request was sent.
    pub fn switch_to(to: &Layout) -> bool {
        imp::switch_to(to.langid)
    }

    #[cfg(windows)]
    mod imp {
        type Hkl = isize;
        type Hwnd = isize;

        /// Sent to a window to ask it to change its input language.
        const WM_INPUTLANGCHANGEREQUEST: u32 = 0x0050;

        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetKeyboardLayoutList(count: i32, list: *mut Hkl) -> i32;
            fn GetForegroundWindow() -> Hwnd;
            fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
        }

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetConsoleWindow() -> Hwnd;
        }

        fn list() -> Vec<Hkl> {
            let count = unsafe { GetKeyboardLayoutList(0, std::ptr::null_mut()) };
            if count <= 0 {
                return Vec::new();
            }
            let mut ids = vec![0 as Hkl; count as usize];
            let got = unsafe { GetKeyboardLayoutList(count, ids.as_mut_ptr()) };
            ids.truncate(got.max(0) as usize);
            ids
        }

        pub fn installed() -> Option<Vec<usize>> {
            let ids = list();
            (!ids.is_empty()).then(|| ids.into_iter().map(|hkl| hkl as usize).collect())
        }

        pub fn switch_to(langid: u16) -> bool {
            let Some(hkl) = list()
                .into_iter()
                .find(|&hkl| super::known(hkl as usize).is_some_and(|l| l.langid == langid))
            else {
                return false;
            };
            let mut sent = false;
            // Windows Terminal and other hosts own the foreground window; the
            // classic console host owns the console window. Ask both.
            let foreground = unsafe { GetForegroundWindow() };
            let console = unsafe { GetConsoleWindow() };
            for hwnd in [foreground, console] {
                if hwnd != 0 && !(sent && hwnd == foreground) {
                    sent |= unsafe { PostMessageW(hwnd, WM_INPUTLANGCHANGEREQUEST, 0, hkl) } != 0;
                }
            }
            sent
        }
    }

    #[cfg(not(windows))]
    mod imp {
        pub fn installed() -> Option<Vec<usize>> {
            None
        }

        pub fn switch_to(_langid: u16) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layout_has_one_character_per_key_and_no_repeats() {
        let keys = KEY_ORDER.chars().count();
        assert_eq!(keys, 47);
        for layout in LAYOUTS {
            assert_eq!(layout.normal.chars().count(), keys, "{}", layout.id);
            assert_eq!(layout.shifted.chars().count(), keys, "{}", layout.id);
            let mut all: Vec<char> = layout
                .normal
                .chars()
                .chain(layout.shifted.chars())
                .collect();
            all.sort_unstable();
            let len = all.len();
            all.dedup();
            assert_eq!(
                all.len(),
                len,
                "{} types a character on two keys",
                layout.id
            );
        }
        assert_eq!(US.normal, KEY_ORDER);
    }

    #[test]
    fn the_wrong_layout_example_becomes_thai() {
        assert_eq!(convert("ouj8nv8ezbf", &US, &THAI), "นี่คือคำผิด");
        assert_eq!(detect("ouj8nv8ezbf"), Some(&US));
    }

    #[test]
    fn thai_typed_on_the_wrong_layout_becomes_english() {
        assert_eq!(detect("ทำหะ"), Some(&THAI));
        assert_eq!(convert("ทำหะ", &THAI, &US), "mest");
        // Shifted keys, punctuation the Thai layout types, and a space.
        assert_eq!(convert("ฉันรัก Rust", &US, &US), "ฉันรัก Rust");
        assert_eq!(convert("ฎ\"ฐ,ฅ", &THAI, &US), "EW{}|");
        assert_eq!(convert("Hello, World!", &US, &THAI), "็ำสสนม \"นพสก+");
    }

    #[test]
    fn every_key_round_trips_both_ways() {
        let english: String = US.normal.chars().chain(US.shifted.chars()).collect();
        let thai = convert(&english, &US, &THAI);
        assert_eq!(convert(&thai, &THAI, &US), english);
        let native: String = THAI.normal.chars().chain(THAI.shifted.chars()).collect();
        assert_eq!(convert(&convert(&native, &THAI, &US), &US, &THAI), native);
    }

    #[test]
    fn thai_marks_are_one_key_each_and_stay_in_typing_order() {
        // น + sara ii + mai ek: base, upper vowel, tone mark.
        assert_eq!(convert("นี่", &THAI, &US), "ouj");
        // Lower vowel and a tone mark above it.
        assert_eq!(convert("ปู่", &THAI, &US), "x^j");
        // Sara am stored as nikhahit + sara aa is still the `e` key.
        assert_eq!(convert("ค\u{0E4D}\u{0E32}", &THAI, &US), "8e");
        // A mark with no base before it still maps.
        assert_eq!(convert("j", &US, &THAI), "่");
    }

    #[test]
    fn detect_needs_a_character_some_layout_types() {
        assert_eq!(detect("  \n"), None);
        assert_eq!(detect("😀"), None);
        assert_eq!(detect("/-"), Some(&US), "no evidence: assume US");
        assert_eq!(detect("abc ภาษาไทยเยอะกว่า"), Some(&THAI));
        assert_eq!(by_id("th"), Some(&THAI));
    }

    #[test]
    fn the_os_pair_decides_only_when_it_is_exactly_two_known_layouts() {
        let us = os::known(0x0409_0409);
        let th = os::known(0x041E_041E);
        assert_eq!(us, Some(&US));
        assert_eq!(th, Some(&THAI));
        assert_eq!(
            os::known(0xF002_0409),
            None,
            "US Dvorak is another keyboard"
        );
        assert_eq!(os::known(0x0001_041E), None, "Thai Pattachote");
        assert_eq!(os::target(&US, &[us, th]), Some(&THAI));
        assert_eq!(os::target(&THAI, &[us, th]), Some(&US));
        assert_eq!(os::target(&US, &[us]), None);
        assert_eq!(os::target(&US, &[us, th, None]), None);
        assert_eq!(os::target(&US, &[us, None]), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn other_systems_give_no_layout_list() {
        assert_eq!(os::installed(), None);
        assert!(!os::switch_to(&THAI));
    }
}
