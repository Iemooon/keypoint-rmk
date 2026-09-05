//! Runtime-tunable pointer settings, shared by the two pointer controllers.
//!
//! Why this is its own module: rmk's `PointingProcessorEvent` carries a whole
//! `PointingMode`, so "change the scroll divisor" and "switch between cursor and
//! scroll" are the *same* operation on that channel. The knob controller
//! therefore has to know whether a device is currently scrolling before it
//! republishes - otherwise tuning scroll speed would kick the nub back to the
//! cursor, and tuning pointer speed would eject you from a scroll in progress.
//! Both controllers read and write the atomics here so neither can drift.
//!
//! Per-device rather than global, because the two pointers were never going to
//! share a scale: the pad travels under a whole finger, the nub under thumb
//! pressure.
//!
//! Everything is `Relaxed`: each value is a single counter read and written
//! independently of the others, and there is no ordering to establish between
//! them and any other memory.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use rmk::event::{publish_event, PointingProcessorEvent};
use rmk::input_device::pointing::{PointingMode, ScrollConfig, SniperConfig};

/// `device_id` of the A320 pad's `PointingProcessor` (left pointer).
pub const PAD_ID: u8 = 0;
/// `device_id` of the TrackPoint's `PointingProcessor` (right pointer). Matches
/// `trackpoint::DEVICE_ID`, which is the same number on the peripheral.
pub const NUB_ID: u8 = 1;

/// Boot values, shown on the left panel as `cP`/`cN` (pointer tiers 1..8) and
/// `sP`/`sN` (scroll divisors).
pub const PAD_CURSOR0: u8 = 2; // tier 2 of SPEED_TIERS below, i.e. 0.333x
pub const NUB_CURSOR0: u8 = 3; // tier 3 of NUB_SPEED_TIERS, i.e. 0.4x

/// Trackpad tiers: 0.25 0.333 0.444 0.6 0.8 1.1 1.5 2.0. Index is tier minus one.
/// Built on rmk Sniper rather than Cursor because Cursor takes integer
/// multipliers only, so 1x is its floor. Sniper divides, and its
/// MotionAccumulator carries the remainder to the next whole unit, which keeps
/// slow moves from losing steps.
pub const SPEED_TIERS: [(u8, u8); 8] = [
    (1, 4),    // tier 1: 0.25x
    (1, 3),    // tier 2: 0.333x
    (4, 9),    // tier 3: 0.444x
    (3, 5),    // tier 4: 0.6x
    (4, 5),    // tier 5: 0.8x
    (11, 10),  // tier 6: 1.1x
    (3, 2),    // tier 7: 1.5x
    (2, 1),    // tier 8: 2x
];

/// TrackPoint tiers: 0.2 0.3 0.4 0.5 0.6 0.7 0.8 1.0, all exact fractions.
/// A thumb presses a nub directly, so the same multiplier moves less than on the
/// trackpad and this curve sits below SPEED_TIERS.
pub const NUB_SPEED_TIERS: [(u8, u8); 8] = [
    (1, 5),   // 0.2x
    (3, 10),  // 0.3x
    (2, 5),   // 0.4x
    (1, 2),   // 0.5x
    (3, 5),   // 0.6x
    (7, 10),  // 0.7x
    (4, 5),   // 0.8x
    (1, 1),   // 1.0x
];
/// ZMK's `TRACKPOINT_SCROLL_SCALE 0.03` as a divisor: 1 / 0.03 ~= 33.
pub const NUB_SCROLL0: u8 = 17; // half of ZMK's 33; a thumb covers less than a finger
/// The pad's own driver already scales its deltas, and a finger covers more
/// ground than a thumb pressing a nub, so it starts roughly half as sensitive.
pub const PAD_SCROLL0: u8 = 34; // divisor used at boot

static PAD_CURSOR: AtomicU8 = AtomicU8::new(PAD_CURSOR0);
static NUB_CURSOR: AtomicU8 = AtomicU8::new(NUB_CURSOR0);
static PAD_SCROLL: AtomicU8 = AtomicU8::new(PAD_SCROLL0);
static NUB_SCROLL: AtomicU8 = AtomicU8::new(NUB_SCROLL0);
static PAD_SCROLLING: AtomicBool = AtomicBool::new(false);
static NUB_SCROLLING: AtomicBool = AtomicBool::new(false);

fn cursor_slot(device_id: u8) -> &'static AtomicU8 {
    if device_id == PAD_ID {
        &PAD_CURSOR
    } else {
        &NUB_CURSOR
    }
}

fn scroll_slot(device_id: u8) -> &'static AtomicU8 {
    if device_id == PAD_ID {
        &PAD_SCROLL
    } else {
        &NUB_SCROLL
    }
}

fn scrolling_slot(device_id: u8) -> &'static AtomicBool {
    if device_id == PAD_ID {
        &PAD_SCROLLING
    } else {
        &NUB_SCROLLING
    }
}

pub fn cursor_mult(device_id: u8) -> u8 {
    cursor_slot(device_id).load(Ordering::Relaxed)
}

pub fn scroll_div(device_id: u8) -> u8 {
    scroll_slot(device_id).load(Ordering::Relaxed)
}

/// Whether this device is in scroll mode. Written by the scroll key controller
/// and read before republishing a mode, so a scroll in progress keeps running.
pub fn is_scrolling(device_id: u8) -> bool {
    scrolling_slot(device_id).load(Ordering::Relaxed)
}

pub fn set_scrolling(device_id: u8, on: bool) {
    scrolling_slot(device_id).store(on, Ordering::Relaxed);
}

/// Set device `id`'s pointer tier directly (1..8). Same slot the knob writes, so
/// the FUNC-layer keys and the keymap cells cannot disagree.
pub fn set_cursor_tier(device_id: u8, tier: u8) -> bool {
    if !(1..=8).contains(&tier) {
        return false;
    }
    cursor_slot(device_id).store(tier, Ordering::Release);
    true
}

/// Scroll tiers, used as the HID wheel divisor; larger scrolls slower.
/// Shares the 1..8 numbering with the move tiers, so one Kp1..Kp8 value drives
/// either kind.
const SCROLL_TIERS: [u8; 8] = [120, 55, 30, 24, 19, 15, 12, 9];

/// Set device `id`'s scroll divisor from a 1..8 tier. `false` = tier out of
/// range, current value untouched.
pub fn set_scroll_tier(device_id: u8, tier: u8) -> bool {
    if !(1..=8).contains(&tier) {
        return false;
    }
    scroll_slot(device_id).store(SCROLL_TIERS[(tier - 1) as usize], Ordering::Release);
    true
}

/// Pointer mode for one device, built from its tier. The name stays `cursor_mode`
/// because central.rs and go_cursor call it; it returns Sniper, which carries the
/// divisor that sub-1x speeds need. Both forms emit the same MouseReport, so
/// auto_mouse layer entry and timeout are unchanged.
pub fn cursor_mode(device_id: u8) -> PointingMode {
    // Unknown tier falls back to 1x passthrough: the tier is read at runtime,
    // so a bad value costs one step rather than the keyboard.
    // pad uses SPEED_TIERS, nub uses NUB_SPEED_TIERS; both map tier number
    // straight to multiplier.
    let tier = (cursor_mult(device_id) as usize).saturating_sub(1);
    let table = if device_id == PAD_ID {
        &SPEED_TIERS
    } else {
        &NUB_SPEED_TIERS
    };
    let (multiplier, divisor) = table.get(tier).copied().unwrap_or((1, 1));
    PointingMode::Sniper(SniperConfig {
        multiplier,
        divisor,
        // Only the pad needed X inverted, on hardware. This is the mirror
        // compensation; the separate scroll-convention negation lives in
        // trackpad.rs scaled() - the two coexist, see the comment there.
        invert_x: device_id == PAD_ID,
        invert_y: false,
    })
}

/// Scroll mode for one device, built from the stored divisor.
pub fn scroll_mode(device_id: u8) -> PointingMode {
    PointingMode::Scroll(ScrollConfig {
        multiplier_x: 1,
        divisor_x: scroll_div(device_id),
        multiplier_y: 1,
        divisor_y: scroll_div(device_id),
        // ZMK's TrackPoint scroll maps X to the horizontal wheel without
        // negation; it flips sign downstream instead.
        invert_x: false,
        invert_y: false,
    })
}

/// Send one device's current cursor config and record the mode.
pub fn go_cursor(device_id: u8) {
    set_scrolling(device_id, false);
    publish_event(PointingProcessorEvent {
        device_id,
        mode: cursor_mode(device_id),
    });
}

/// Send one device's current scroll config and record the mode.
pub fn go_scroll(device_id: u8) {
    set_scrolling(device_id, true);
    publish_event(PointingProcessorEvent {
        device_id,
        mode: scroll_mode(device_id),
    });
}
