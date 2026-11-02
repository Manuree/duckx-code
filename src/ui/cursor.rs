//! Screen cell for the block cursor.
//!
//! The frame flush writes a changed cell by moving the terminal cursor onto
//! it. The wait spinner is that cell on most frames, so the block blinks on
//! the icon. Rendering only records the cell. The main loop places the cursor
//! after the flush, while it is still hidden.

use std::cell::Cell;

thread_local! {
    static CURSOR: Cell<Option<(u16, u16)>> = const { Cell::new(None) };
}

pub fn clear() {
    CURSOR.with(|slot| slot.set(None));
}

pub fn place(x: u16, y: u16) {
    CURSOR.with(|slot| slot.set(Some((x, y))));
}

pub fn position() -> Option<(u16, u16)> {
    CURSOR.with(|slot| slot.get())
}
