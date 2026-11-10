//! Screen cell for the block cursor.
//!
//! The block is a reversed cell in the frame, the same idea as ratatui-textarea.
//! Rendering only records the cell. The frame paints it after every widget, and
//! the terminal cursor stays hidden so a changed cell cannot catch the block.

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
