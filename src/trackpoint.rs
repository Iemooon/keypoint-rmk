//! TrackPoint pointing stick, right half.
//!
//! Wiring: SDA P0.14 / SCL P1.08 on TWISPI0, slave address 0x15. MOTION P0.07,
//! active low with a pull-up; one falling edge means one packet.
//!
//! Protocol: read 7 bytes and check that byte 0 equals the magic 0x50, then drop
//! the packet if it does not. The bridge needs no configuration or calibration
//! writes; drift compensation and settling run inside the stick firmware.
//!
//! This module publishes cursor deltas with acceleration and sign applied.
//! Scrolling lives in scroll_key.rs, which switches the pointer mode instead.

use core::sync::atomic::Ordering;

use embedded_hal_async::digital::Wait;
use embedded_hal_async::i2c::I2c;
use embassy_time::{Duration, Instant, Timer};
use rmk::event::{Axis, AxisEvent, AxisValType, PointingEvent};
use rmk::macros::input_device;
use crate::tp_diag::TP;

/// RMK pointing-device id for the nub. **Must equal `scroll_key::TRACKPOINT_ID`**
/// on the central - the halves are compiled separately, so this is a shared
/// convention, not a shared constant.
pub const DEVICE_ID: u8 = 1;

/// TrackPoint slave address (overlay `reg = <0x15>`).
const TP_ADDR: u8 = 0x15;
/// `TRACKPOINT_PACKET_LEN`.
const PACKET_LEN: usize = 7;
/// `TRACKPOINT_MAGIC_BYTE0` - byte 0 of every valid packet.
const MAGIC: u8 = 0x50;
/// Safety-net tick in interrupt mode: a slow poll that keeps the nub alive if
/// this board's MOTION line ever stops toggling. Two harmless wake-ups per
/// second when edges are healthy.
const FALLBACK_POLL_MS: u64 = 500;
/// ZMK's trailing `k_msleep(5)`.
const REPORT_PAUSE_MS: u64 = 5;

/// When true, MOTION is ignored and the bridge is read on a fixed timer. When
/// false, one falling edge on MOTION triggers one read. Interrupt mode is the
/// ZMK-proven wiring (active-low MOTION, edge->active interrupt; the vendor
/// driver reads the same 7-byte packet per edge). The earlier "no edges were
/// ever seen" note came from the PollWait stand-in, not from the line itself;
/// embassy-nrf 0.11 waits through the shared port SENSE/PORT event, so this
/// costs no GPIOTE channel. If a live board ever misses nub data, flip this
/// back to true and nothing else in the driver changes. Polling re-reads
/// packets the bridge already sent; read_packet filters those out.
pub const POLL_MODE: bool = false;
/// Poll period in poll mode. 100 Hz: above what a hand can resolve, below the
/// rate that floods the split link and the panel.
const POLL_INTERVAL_MS: u64 = 10;

/// Device level gain. 1.0 passes counts through unchanged, so the tier table in
/// pointer_speed owns all speed. The ZMK factor (1.3) and the acceleration term
/// below stay in the chain.
const BASE_SPEED: f32 = 1.0;
/// `CONFIG_TRACKPOINT_MOUSE_SENS_BASE_PERCENT = 30` -> 0.3
const SENS_BASE: f32 = 0.3;
/// `CONFIG_TRACKPOINT_MOUSE_SENS_STEP_PERCENT = 1` -> 0.01 per percent.
const SENS_STEP: f32 = 0.01;
/// `TRACKPOINT_ACCEL_BASE_GAIN`
const ACCEL_BASE_GAIN: f32 = 1.307357;
/// Acceleration ceiling, matching ZMK TRACKPOINT_ACCEL_MAX_MULT_PERCENT = 150.
/// Only fast strokes reach it; slow strokes sit near 1.0. ACCEL_PERCENT sets the
/// gain that drives it, which is the finer control.
const ACCEL_MAX_MULT: f32 = 1.5;

/// Pointer speed setting, 100 = ZMK default (factor 0.3 + 0.01*100 = 1.30).
pub const SPEED_PERCENT: f32 = 100.0;
/// Acceleration gain setting, 0..300, step 25, default 100.
pub const ACCEL_PERCENT: f32 = 100.0;

/// TrackPoint nub. Publishes cursor deltas; the paired `PointingProcessor` should
/// stay in `Cursor` mode with multiplier 1 on both axes.
#[input_device(publish = PointingEvent)]
pub struct TrackPoint<I2C, MOTION>
where
    I2C: I2c,
    MOTION: Wait,
{
    /// RMK pointing-device id, matched by the central's `PointingProcessor`.
    device_id: u8,
    i2c: I2C,
    motion: MOTION,
    /// `last_packet_time` - the acceleration term divides by the gap to it.
    last_packet: Instant,
}

impl<I2C, MOTION> TrackPoint<I2C, MOTION>
where
    I2C: I2c,
    MOTION: Wait,
{
    pub fn new(rmk_id: u8, i2c: I2C, motion: MOTION) -> Self {
        Self {
            device_id: rmk_id,
            i2c,
            motion,
            last_packet: Instant::now(),
        }
    }

    /// Read one packet, check the magic byte, return dx = buf[2], dy = buf[3].
    /// Sign is applied in read_pointing_event, matching where ZMK negates.
    async fn read_packet(&mut self) -> Option<(i8, i8)> {
        let mut buf = [0u8; PACKET_LEN];
        // The 20 ms bound is required. A PS/2 bridge holds the clock low while it
        // has nothing to send, and TWIM waits for DEVSTOP, so an unbounded read
        // would block forever and stop the whole task.
        match embassy_time::with_timeout(
            embassy_time::Duration::from_millis(20),
            self.i2c.read(TP_ADDR, &mut buf),
        )
        .await
        {
            Ok(Ok(())) => {}
        // Bus error or deadline; both count in the `e` line on screen.
            Ok(Err(_)) | Err(_) => {
                TP.n_err.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        }
        TP.n_read.fetch_add(1, Ordering::Relaxed);
        TP.last_b0.store(buf[0] as u32, Ordering::Relaxed);
        TP.last_b1.store(buf[1] as u32, Ordering::Relaxed);
        if buf[0] != MAGIC {
            TP.n_bad.fetch_add(1, Ordering::Relaxed);
            TP.last_bad_b0.store(buf[0] as u32, Ordering::Relaxed);
            return None;
        }
        let raw_x = buf[2] as i8;
        let raw_y = buf[3] as i8;

        // Drop repeated packets: polling has no MOTION edge to mark new data
        // and the bridge resends its last packet while idle, so without this
        // filter one stroke keeps reporting at 100 Hz and the pointing channel
        // backs up. Cost: a truly even stroke can repeat a value and be
        // dropped.
        static LAST_SENT: core::sync::atomic::AtomicU32 =
            core::sync::atomic::AtomicU32::new(u32::MAX);
        let packed = ((raw_x as u8 as u32) << 8) | (raw_y as u8 as u32);
        TP.last_dx.store(raw_x as i32 as u32, Ordering::Relaxed);
        TP.last_dy.store(raw_y as i32 as u32, Ordering::Relaxed);
        if LAST_SENT.load(Ordering::Relaxed) == packed {
            return None;
        }
        LAST_SENT.store(packed, Ordering::Relaxed);

        // Ignore tiny residuals, where real drift settles.
        const DRIFT_DEADZONE: i8 = 2;
        let dx = if raw_x.abs() <= DRIFT_DEADZONE { 0 } else { raw_x };
        let dy = if raw_y.abs() <= DRIFT_DEADZONE { 0 } else { raw_y };
        Some((dx, dy))
    }
    /// ZMK trackpoint_exponential_factor: exp((|dx|+|dy|)/gap_ms * 1.307357 *
    /// accel%), capped at ACCEL_MAX_MULT. A gap of 0 counts as 1 ms.
    fn accel_factor(&self, dx: i8, dy: i8, gap: Duration) -> f32 {
        let dist = (dx.unsigned_abs() + dy.unsigned_abs()) as f32;
        if dist < 1.0 {
            return 1.0;
        }
        let delta_ms = gap.as_millis().max(1) as f32;
        let speed = dist / delta_ms;
        let gain = ACCEL_BASE_GAIN * (ACCEL_PERCENT / 100.0);
        // `expf` in ZMK's C code; libm keeps this a soft-float call (~tens of
        // cycles), which is nothing at a 200 Hz worst-case report rate.
        let mult = libm::expf(speed * gain);
        if mult > ACCEL_MAX_MULT {
            ACCEL_MAX_MULT
        } else {
            mult
        }
    }

    pub async fn read_pointing_event(&mut self) -> PointingEvent {
        loop {
            // Asleep: stop the 100 Hz I2C hammering, 2 s heartbeat instead
            // (sleep_watch.rs).
            if crate::sleep_watch::sleeping() {
                Timer::after_secs(2).await;
                continue;
            }
            if POLL_MODE {
                // Count ticks so `q` still answers "is this task running" even
                // though the MOTION line is no longer involved.
                Timer::after_millis(POLL_INTERVAL_MS).await;
                TP.n_irq.fetch_add(1, Ordering::Relaxed);
            } else {
                // A falling edge carries one packet; the tick is a safety net
                // (see FALLBACK_POLL_MS). The former 200 ms "soft watchdog"
                // lived here and caused the outage it now guards against in
                // memory: it compared the edge stamp against `last_activity`,
                // a field refreshed only by *successful reads*, so every first
                // edge after a pause was discarded - forever. ZMK's original
                // stamps in the ISR before the check, which never self-locks;
                // with one read per wake there is nothing to guard here.
                let _ = embassy_futures::select::select(
                    self.motion.wait_for_falling_edge(),
                    Timer::after_millis(FALLBACK_POLL_MS),
                )
                .await;
                TP.n_irq.fetch_add(1, Ordering::Relaxed);
            }

            let now = Instant::now();

            let Some((dx, dy)) = self.read_packet().await else {
                // Covers both the I2C failure and the magic mismatch; ZMK logs
                // "read failed (soft recover)" and drops the round without retry.
                continue;
            };

            let gap = now.saturating_duration_since(self.last_packet);
            let factor = SENS_BASE + SENS_STEP * SPEED_PERCENT;
            let mult = self.accel_factor(dx, dy, gap);
            let fx = dx as f32 * BASE_SPEED * factor * mult;
            let fy = dy as f32 * BASE_SPEED * factor * mult;
            self.last_packet = now;

            // ZMK reports -(int)fx / -(int)fy, i.e. truncation toward zero, then
            // HID clamps to i8. We clamp here so a hard shove cannot wrap.
            let out_x = -(fx as i32) as i16;
            let out_y = -(fy as i32) as i16;

            Timer::after_millis(REPORT_PAUSE_MS).await;
            if out_x == 0 && out_y == 0 {
                // Sub-unit pressure: drop it silently like the C code does, since
                // ZMK has no residue accumulation on the cursor path either.
                continue;
            }

            TP.n_pub.fetch_add(1, Ordering::Relaxed);
            return PointingEvent {
                device_id: self.device_id,
                axes: [
                    AxisEvent {
                        typ: AxisValType::Rel,
                        axis: Axis::X,
                        value: out_x.clamp(i16::from(i8::MIN), i16::from(i8::MAX)),
                    },
                    AxisEvent {
                        typ: AxisValType::Rel,
                        axis: Axis::Y,
                        value: out_y.clamp(i16::from(i8::MIN), i16::from(i8::MAX)),
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
