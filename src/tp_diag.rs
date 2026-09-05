#![allow(dead_code)] // forensic getters kept as the nub tripwire even though
                     // the counters are no longer rendered
//! Diagnostic counters for the right-half TrackPoint.
//!
//! The right half has no USB and no debug probe, so "the nub does nothing"
//! cannot distinguish a dead I2C, a wrong header byte, a missing MOTION edge,
//! and an event dying in the central's HID path. The counters are incremented
//! at each decision point in trackpoint.rs; the getters are the read side for
//! any future tripwire or panel row.
//!
//! A separate module because renderers.rs is compiled into both bins while the
//! driver exists only in the peripheral one.

use core::sync::atomic::{AtomicU32, Ordering};

/// Counters and the last raw bytes seen, all written by the driver task and read
/// by the display task. Relaxed ordering is fine: these are observations, not
/// synchronization.
pub struct TpStats {
    /// `wait_for_falling_edge()` returned - the MOTION line actually pulsed.
    pub n_irq: AtomicU32,
    /// An I2C read completed without error.
    pub n_read: AtomicU32,
    /// An I2C read failed (NACK / bus error / timeout).
    pub n_err: AtomicU32,
    /// A read succeeded but byte 0 was not the expected magic.
    pub n_bad: AtomicU32,
    /// Byte 0 of the last bad packet. Decodes the failure mode: 0 means the
    /// bridge answers nothing, 255 (0xFF) means the line floats (pull-up or
    /// supply sagging), anything else is torn or mis-sampled data.
    pub last_bad_b0: AtomicU32,
    /// Events handed back to rmk.
    pub n_pub: AtomicU32,
    /// Last raw byte 0 / byte 1, whatever they were.
    pub last_b0: AtomicU32,
    pub last_b1: AtomicU32,
    /// Last raw dx / dy, taken before the deadzone. Read these while the stick is
    /// idle to size DRIFT_DEADZONE.
    pub last_dx: AtomicU32,
    pub last_dy: AtomicU32,
}

impl TpStats {
    const fn new() -> Self {
        Self {
            n_irq: AtomicU32::new(0),
            n_read: AtomicU32::new(0),
            n_err: AtomicU32::new(0),
            n_bad: AtomicU32::new(0),
            last_bad_b0: AtomicU32::new(0),
            n_pub: AtomicU32::new(0),
            last_b0: AtomicU32::new(0),
            last_b1: AtomicU32::new(0),
            last_dx: AtomicU32::new(0),
            last_dy: AtomicU32::new(0),
        }
    }
}

/// The one and only instance.
pub static TP: TpStats = TpStats::new();

fn get(c: &AtomicU32) -> u32 {
    c.load(Ordering::Relaxed)
}

pub fn n_irq() -> u32 {
    get(&TP.n_irq)
}
pub fn n_read() -> u32 {
    get(&TP.n_read)
}
pub fn n_err() -> u32 {
    get(&TP.n_err)
}
pub fn n_bad() -> u32 {
    get(&TP.n_bad)
}
pub fn last_bad_b0() -> u32 {
    get(&TP.last_bad_b0)
}
pub fn n_pub() -> u32 {
    get(&TP.n_pub)
}
pub fn last_b0() -> u32 {
    get(&TP.last_b0)
}
pub fn last_b1() -> u32 {
    get(&TP.last_b1)
}
/// Raw displacement restored to a signed value, for the display.
pub fn last_dx() -> i8 {
    get(&TP.last_dx) as i32 as i8
}
pub fn last_dy() -> i8 {
    get(&TP.last_dy) as i32 as i8
}

// What the counter pattern means.
// (Plain comments: a trailing `///` block would be a doc comment with no item
// after it, which rustc rejects.)
//
//   irq = 0                MOTION never pulses -> wrong pin/polarity, or the
//                          bridge only drives it after being configured.
//                          => try ignoring the IRQ and reading on a timer.
//   read = 0, err > 0      bus-level failure -> wrong address, no pull-ups,
//                          SDA/SCL swapped, or the wrong TWIM instance.
//   bad > 0                we ARE talking to the chip; byte0 shows what the
//                          real header is => fix MAGIC or the packet layout.
//   pub > 0, nothing moves event left the device but the host saw nothing
//                          => central side: device id, processor, or report.
