//! A320 trackpad driver, left half.
//!
//! Wiring: SDA P0.26 / SCL P0.04 on TWISPI0, MOTION P0.08.
//!
//! Publishes raw displacement in pad counts. All scaling lives in the
//! paired processors (pointer_speed.rs): scroll mode via ScrollConfig
//! divisor, cursor mode via the tier table.

use core::sync::atomic::{AtomicU32, Ordering};

use embedded_hal::i2c::Operation;
use embedded_hal_async::digital::Wait;
use embedded_hal_async::i2c::I2c;
use embassy_time::{Instant, Timer};
use rmk::event::{Axis, AxisEvent, AxisValType, PointingEvent};
use rmk::macros::input_device;

/// Forensic counters for the pad chain (tripwire-only, not rendered):
/// `M` = MOTION interrupts, `k` = I2C OK, `e` = bus errors,
/// `t` = 20 ms read timeouts, `p` = PointingEvents published.
/// M frozen at 0 => power/bus/FPC fault. M and k climbing while the cursor
/// stays put => fault is downstream of this task.
static N_MOTION: AtomicU32 = AtomicU32::new(0);
static N_OK: AtomicU32 = AtomicU32::new(0);
static N_ERR: AtomicU32 = AtomicU32::new(0);
static N_PUB: AtomicU32 = AtomicU32::new(0);
/// Error kind of the last failed transaction: 1=NoAck-at-address (device
/// absent), 2=data-phase NACK, 3=Bus, 4=ArbitrationLoss, 5=Overrun,
/// 6=other, 9=timeout, 0=none yet.
static N_LERR: AtomicU32 = AtomicU32::new(0);

#[allow(dead_code)] // forensic getter, see n_motion
pub fn n_lerr() -> u32 {
    N_LERR.load(Ordering::Relaxed)
}

fn err_kind(e: &dyn embedded_hal::i2c::Error) -> u32 {
    use embedded_hal::i2c::{ErrorKind, NoAcknowledgeSource};
    match e.kind() {
        ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address) => 1,
        ErrorKind::NoAcknowledge(_) => 2,
        ErrorKind::Bus => 3,
        ErrorKind::ArbitrationLoss => 4,
        ErrorKind::Overrun => 5,
        _ => 6,
    }
}

// Forensic getters: counters keep running as the pad tripwire even though
// nothing renders them anymore.
#[allow(dead_code)]
pub fn n_motion() -> u32 {
    N_MOTION.load(Ordering::Relaxed)
}
#[allow(dead_code)]
pub fn n_ok() -> u32 {
    N_OK.load(Ordering::Relaxed)
}
#[allow(dead_code)]
pub fn n_err() -> u32 {
    N_ERR.load(Ordering::Relaxed)
}
#[allow(dead_code)]
pub fn n_pub() -> u32 {
    N_PUB.load(Ordering::Relaxed)
}

/// A320 slave address (overlay `reg = <0x3B>`).
const A320_ADDR: u8 = 0x3B;
/// Motion register: written once as a pointer before the burst read.
const MOTION_REG: u8 = 0x82;
/// Packet length at MOTION_REG: [status, dx, dy].
const PACKET_LEN: usize = 3;

/// Scroll deadzone (ZMK Kconfig default 1): at or below this, it's noise.
const DEADZONE: i16 = 1;
/// Scroll-mode gain. Raw counts pass through; ScrollConfig divisor in
/// pointer_speed.rs is the only scale factor.
const SCROLL_GAIN: f32 = 1.0;
/// Drop accumulated residue if the pad went silent this long (ZMK
/// `A320_WDT_TIMEOUT`).
const WDT_TIMEOUT_MS: u64 = 200;
/// Pad considered untouched after this quiet period (ZMK
/// `TOUCH_IDLE_TIMEOUT`).
const TOUCH_IDLE_MS: u64 = 50;
/// ZMK's trailing `k_msleep(25)` - caps the pad at roughly 40 reports/s.
const REPORT_PAUSE_MS: u64 = 25;
/// Cursor-mode gain. Raw counts pass through (~14.8 counts/mm); the tier
/// table in pointer_speed owns all cursor speed.
const CURSOR_GAIN: f32 = 1.0;

/// A320 trackpad. Publishes relative units that the paired processor forwards
/// 1:1 as pan/wheel.
#[input_device(publish = PointingEvent)]
pub struct A320<I2C, MOTION>
where
    I2C: I2c,
    MOTION: Wait,
{
    /// RMK pointing-device id; must match the `PointingProcessor` that handles it.
    device_id: u8,
    i2c: I2C,
    motion: MOTION,
    /// Fractional scroll units carried between reports (`scroll_residual_x/y`).
    residue_x: f32,
    residue_y: f32,
    /// Instant of the last packet that actually moved (`last_activity_time`).
    last_activity: Instant,
    /// Instant of the last non-empty burst (`last_touch_time`).
    last_touch: Instant,
}

impl<I2C, MOTION> A320<I2C, MOTION>
where
    I2C: I2c,
    MOTION: Wait,
{
    pub fn new(rmk_id: u8, i2c: I2C, motion: MOTION) -> Self {
        Self {
            device_id: rmk_id,
            i2c,
            motion,
            residue_x: 0.0,
            residue_y: 0.0,
            last_activity: Instant::now(),
            last_touch: Instant::now(),
        }
    }

    /// Read one motion packet.
    ///
    /// `ptr` is passed in rather than built as a literal on purpose: the nRF TWIM
    /// drives EasyDMA from RAM only, and `&[0x82u8]` would live in `.rodata` and
    /// silently fail. Keep the buffer in a normal local (or `static`) instead.
    ///
    /// `None` means bus error or an all-zero packet, which is the C driver's
    /// end-of-burst marker.
    async fn read_packet(&mut self, ptr: &mut [u8; 1]) -> Option<(i16, i16)> {
        ptr[0] = MOTION_REG;
        let mut buf = [0u8; PACKET_LEN];
        // The 20 ms bound is required: TWIM waits for a DEVSTOP interrupt that
        // a clock-suppressing slave never releases, so an unbounded read would
        // park this task forever. Timeout and bus error both mean end-of-burst.
        match embassy_time::with_timeout(
            embassy_time::Duration::from_millis(20),
            self.i2c.transaction(
                A320_ADDR,
                &mut [Operation::Write(&*ptr), Operation::Read(&mut buf)],
            ),
        )
        .await
        {
            Ok(Ok(())) => {
                N_OK.fetch_add(1, Ordering::Relaxed);
            }
            Ok(Err(e)) => {
                N_ERR.fetch_add(1, Ordering::Relaxed);
                N_LERR.store(err_kind(&e), Ordering::Relaxed);
                return None;
            }
            Err(_) => {
                // Timeout: own x-code, counts as a failed transaction.
                N_ERR.fetch_add(1, Ordering::Relaxed);
                N_LERR.store(9, Ordering::Relaxed);
                return None;
            }
        }
        let dx = buf[1] as i8;
        // Hardware reports +Y downwards; negate to match.
        let dy = -(buf[2] as i8);
        if dx == 0 && dy == 0 {
            return None;
        }
        Some((dx as i16, dy as i16))
    }

    /// Drain every packet queued behind this motion interrupt.
    async fn drain(&mut self, ptr: &mut [u8; 1]) -> (i16, i16, bool) {
        let (mut total_x, mut total_y) = (0i16, 0i16);
        let mut got = false;
        while let Some((dx, dy)) = self.read_packet(ptr).await {
            total_x = total_x.saturating_add(dx);
            total_y = total_y.saturating_add(dy);
            got = true;
        }
        (total_x, total_y, got)
    }

    /// Deadzone, then gain by mode, then axis directions.
    fn scaled(&self, dx: i16, dy: i16) -> (f32, f32) {
        let dead = |v: i16| -> i16 {
            if v.abs() <= DEADZONE {
                0
            } else {
                v
            }
        };
        let (dx, dy) = (dead(dx), dead(dy));
        // SCROLL_X_DIR = -1 (ZMK convention). This negation is INDEPENDENT of
        // the hardware mirror compensation (pointer_speed::cursor_mode invert_x
        // for the pad); removing either flips one axis on hardware.
        let gain = if crate::pointer_speed::is_scrolling(self.device_id) {
            SCROLL_GAIN
        } else {
            CURSOR_GAIN
        };
        (-(dx as f32) * gain, dy as f32 * gain)
    }

    pub async fn read_pointing_event(&mut self) -> PointingEvent {
        // RAM-resident write buffer for the register pointer (see read_packet).
        let mut ptr = [0u8; 1];
        loop {
            // Asleep: skip the MOTION wait and the liveness drain entirely
            // (sleep_watch.rs); the chip keeps its state until we wake.
            if crate::sleep_watch::sleeping() {
                Timer::after_secs(2).await;
                continue;
            }
            // Active-low MOTION: falling edge = data ready. The 500 ms cap
            // doubles as a liveness probe, so k/e/t keep sampling bus health
            // with no touch at all.
            match embassy_time::with_timeout(
                embassy_time::Duration::from_millis(500),
                self.motion.wait_for_falling_edge(),
            )
            .await
            {
                Ok(Ok(())) => {
                    N_MOTION.fetch_add(1, Ordering::Relaxed);
                }
                Ok(Err(_)) => {}
                Err(_elapsed) => {}
            }

            let now = Instant::now();
            // Soft watchdog: after this much silence any stored fraction is
            // stale; reset it rather than dumping a jump on the next stroke.
            if now.saturating_duration_since(self.last_activity).as_millis() > WDT_TIMEOUT_MS {
                self.residue_x = 0.0;
                self.residue_y = 0.0;
                self.last_activity = now;
            }

            let (dx, dy, got) = self.drain(&mut ptr).await;
            if !got {
                if now.saturating_duration_since(self.last_touch).as_millis() > TOUCH_IDLE_MS {
                    self.residue_x = 0.0;
                    self.residue_y = 0.0;
                }
                continue;
            }
            self.last_activity = now;
            self.last_touch = now;

            let (sx, sy) = self.scaled(dx, dy);
            self.residue_x += sx;
            self.residue_y += sy;
            let (out_x, out_y) = (self.residue_x as i16, self.residue_y as i16);
            Timer::after_millis(REPORT_PAUSE_MS).await;
            if out_x == 0 && out_y == 0 {
                // Sub-unit stroke: keep the residue so slow, careful scrolling
                // still accumulates into something.
                continue;
            }
            self.residue_x -= out_x as f32;
            self.residue_y -= out_y as f32;

            N_PUB.fetch_add(1, Ordering::Relaxed);
            return PointingEvent {
                device_id: self.device_id,
                axes: [
                    AxisEvent {
                        typ: AxisValType::Rel,
                        axis: Axis::X,
                        value: out_x,
                    },
                    AxisEvent {
                        typ: AxisValType::Rel,
                        axis: Axis::Y,
                        value: out_y,
                    },
                    AxisEvent {
                        typ: AxisValType::Rel,
                        axis: Axis::Z,
                        value: 0,
                    },
                ],
            };
        }
    }
}
