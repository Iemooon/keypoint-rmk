//! Tier state for the pointing devices, read from four keymap cells.
//!
//! The tiers live in base layer cells whose key value is the tier number:
//! `(5,0)` `(5,1)` are the trackpad move and scroll tiers, `(11,0)` `(11,1)` the
//! TrackPoint ones. Those four coordinates have no switches, so they are read
//! only.
//!
//! The cells are re-read on every keyboard event and written to the tier tables
//! only when a value changed, so a VIA edit takes effect on the next keypress and
//! a cell bound to some other key leaves the current tier alone.

use rmk::event::KeyboardEvent;
use rmk::keymap::KeyMap;
use rmk::macros::processor;

use crate::pointer_speed::{
    go_cursor, go_scroll, is_scrolling, set_cursor_tier, set_scroll_tier, NUB_ID, PAD_ID,
};

/// Keyboard subscriber that copies keymap tier cells into the speed tables.
#[processor(subscribe = [KeyboardEvent])]
pub struct SpeedController<'a> {
    keymap: &'a KeyMap<'a>,
    /// Last tier read from each cell, 0 before the first read. Comparison only.
    cells: [u8; 4],
}

impl<'a> SpeedController<'a> {
    pub fn new(keymap: &'a KeyMap<'a>) -> Self {
        Self {
            keymap,
            cells: [0; 4],
        }
    }

    /// Read the four cells and store the tiers in `pointer_speed`.
    ///
    /// `action_at_pos` is a table lookup, so it works for cells with no switches.
    /// A move tier needs a republish to take effect; while a device is scrolling
    /// the scroll mode is left in place and the new tier applies when the key is
    /// released and `go_cursor` runs anyway.
    fn refresh_cells(&mut self) {
        for (idx, (row, col)) in crate::keymap::TIER_CELLS.iter().enumerate() {
            let Some(tier) = crate::keymap::tier_of(self.keymap.action_at_pos(0, *row, *col))
            else {
                continue;
            };
            if self.cells[idx] == tier {
                continue;
            }
            self.cells[idx] = tier;
            let (device_id, is_scroll) = match idx {
                0 => (PAD_ID, false),
                1 => (PAD_ID, true),
                2 => (NUB_ID, false),
                _ => (NUB_ID, true),
            };
            if is_scroll {
                set_scroll_tier(device_id, tier);
                if is_scrolling(device_id) {
                    go_scroll(device_id);
                }
            } else {
                set_cursor_tier(device_id, tier);
                if !is_scrolling(device_id) {
                    go_cursor(device_id);
                }
            }
        }
    }

    async fn on_keyboard_event(&mut self, _event: KeyboardEvent) {
        self.refresh_cells();
    }
}
