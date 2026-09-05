//! Scroll key controller: while a thumb key is held, that half points a wheel
//! instead of a cursor -- the left key drives the trackpad, the right key the
//! TrackPoint. Each half keeps its own key and divisor, which works because rmk
//! gives every pointer a separate PointingProcessor keyed by device_id.
//!
//! The scroll scale comes from the rmk ScrollConfig divisor, so no scaling is
//! applied in the driver.
//!
//! Arming checks the mouse layer; holding does not. rmk counts only Rel X/Y as
//! pointer motion, so wheel events cannot keep layer 4 alive. Checking on every
//! event would drop the mode mid scroll, so it is checked at press, and a scroll
//! started on the mouse layer runs until the key is released.

use rmk::event::{KeyboardEvent, KeyboardEventPos};
use rmk::keymap::KeyMap;
use rmk::macros::processor;

use crate::pointer_speed::{go_cursor, go_scroll, is_scrolling, NUB_ID, PAD_ID};

/// `device_id` of the TrackPoint's `PointingProcessor` on the central. Kept as a
/// name here because `central.rs` builds the processor with it.
pub const TRACKPOINT_ID: u8 = NUB_ID;

/// Auto mouse layer in keymap.rs - the only layer where the scroll keys arm.
pub const MOUSE_LAYER: u8 = 4;

/// Left-hand scroll key: ZMK position 47, left thumb row 3 col 4. Drives the
/// **pad**. Note this is the cell ZMK's MOUSE layer uses for LCLK; the left
/// button is still available at pos 49 and pos 18.
pub const PAD_SCROLL_KEY: (u8, u8) = (3, 4);

/// Right-hand scroll key: ZMK position 50, right thumb row 3 col 1. Drives the
/// **nub**. NB: on the 12x8 merged matrix (right half on rows 6..11), position
/// 50 is (9, 1) - position numbering is NOT `row * 8 + col`.
pub const NUB_SCROLL_KEY: (u8, u8) = (9, 1);

/// Which device the divisor belongs to is decided by which key went down.
#[processor(subscribe = [KeyboardEvent])]
pub struct ScrollKeyController<'a> {
    keymap: &'a KeyMap<'a>,
}

impl<'a> ScrollKeyController<'a> {
    /// Needs the keymap solely to ask `active_layer()` when a scroll key is
    /// pressed.
    pub fn new(keymap: &'a KeyMap<'a>) -> Self {
        Self { keymap }
    }

    async fn on_keyboard_event(&mut self, event: KeyboardEvent) {
        let KeyboardEventPos::Key(pos) = event.pos else {
            // Rotary encoder events are the other variant; ignore them.
            return;
        };
        let device_id = match (pos.row, pos.col) {
            r if r == PAD_SCROLL_KEY => PAD_ID,
            r if r == NUB_SCROLL_KEY => NUB_ID,
            _ => return,
        };

        if event.pressed {
            if self.keymap.active_layer() == MOUSE_LAYER {
                go_scroll(device_id);
            }
        } else if is_scrolling(device_id) {
            // Only republish for a device that actually entered scroll mode, so
            // tapping either key on another layer cannot disturb the other side.
            go_cursor(device_id);
        }
    }
}
