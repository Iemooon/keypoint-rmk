//! Status screen renderers for the two LPM009M360A panels (72x144 portrait).
//!
//! Every coordinate below is tuned and verified on the real panel - final.
//! Do not rescale or re-derive them from datasheet or physical-glass
//! dimensions.
//!
//! ```text
//! ┌──────────────┐
//! │ ᛒ1   ▓▓▓ 68  │ y2  mode badge left (BT rune+slot, or USB plug)
//! │              │     battery bar hugging the right-aligned number
//! │   (capybara  │ y11 animation, 70x120 centred, 600 s per frame
//! │    frames)   │
//! │    RAISE     │ y134 layer name, 9x18, bottom-flush
//! └──────────────┘
//! ```
//!
//! * Mode badge (top-left): wired -> sm4tik `usb_02` plug at 2x; wireless ->
//!   Bluetooth rune (bitmap copied from rmk's crate-private icon set) plus
//!   the 0-based profile digit (`ᛒ0`..`ᛒ2`); advertising shows `ᛒ-`; radio
//!   idle shows nothing. The right half mirrors the central's status over
//!   the split link via usb_diag.
//! * Battery (top-right): bar plus bare number, right-aligned on x70; the
//!   bar hugs the number with a 1px gap so they slide together.
//! * Layer (bottom, centred): names from `LAYER_NAMES` below.
//!
//! Forensic pad/nub counters (tp_diag / trackpad getters) stay compiled as
//! the fault tripwire; nothing renders them.

use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use embassy_time::{Duration, Instant};

use embedded_graphics::mono_font::ascii::{FONT_6X13, FONT_9X18};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use heapless::String;
use rmk::display::{DisplayRenderer, RenderContext};
use rmk::types::battery::BatteryStatus;
use rmk::types::ble::BleState;

const HEAD: MonoTextStyle<'static, BinaryColor> =
    MonoTextStyle::new(&FONT_6X13, BinaryColor::On);
const BIG: MonoTextStyle<'static, BinaryColor> =
    MonoTextStyle::new(&FONT_9X18, BinaryColor::On);
const FILL_ON: PrimitiveStyle<BinaryColor> = PrimitiveStyle::with_fill(BinaryColor::On);
const FILL_OFF: PrimitiveStyle<BinaryColor> = PrimitiveStyle::with_fill(BinaryColor::Off);

/// Screen names for layers 0..4 - THE place to rename layers for the panel.
pub const LAYER_NAMES: [&str; 5] = ["BASE", "RAISE", "LOWER", "FUNC", "MOUSE"];

// --- capybara animation (ZMK peripheral_status.c algorithm, ported) ------
//
// Nine 72x120 portrait frames (tools/gen_capy.ps1 from the ZMK bitmaps),
// centred on the panel: one frame every 600 s, cycling forward. The boot
// frame is picked from the device fingerprint (FNV over the nRF52840
// DEVICEID words plus a per-half salt), so the two halves animate out of
// step like ZMK's left/right variants. Idle panels only advance when a
// render happens; capy_tick.rs wakes the processor with a harmless WPM
// event every 600 s.

const CAPY_INTERVAL: Duration = Duration::from_secs(600);
/// 0xFF = not yet seeded.
static CAPY_FRAME: AtomicU8 = AtomicU8::new(0xFF);
static CAPY_SINCE: AtomicU32 = AtomicU32::new(0); // ms of last switch, wrapping

fn capy_frame(salt: u32) -> usize {
    let now_ms = Instant::now().as_millis() as u32;
    let f = CAPY_FRAME.load(Ordering::Relaxed);
    if f == 0xFF {
        // FNV-1a over DEVICEID[0..1] (FICR @ 0x1000_0060, unique per chip),
        // then the half's salt. NB: 0x1000_0100 is INFO.PART/VARIANT/... -
        // model-common values, NOT a fingerprint (was the original bug).
        let mut h: u32 = 2166136261;
        for i in 0..2u32 {
            let w = unsafe { core::ptr::read_volatile((0x1000_0060 + i * 4) as *const u32) };
            h = (h ^ (w & 0xFF)) .wrapping_mul(16777619);
            h = (h ^ (w >> 8 & 0xFF)).wrapping_mul(16777619);
            h = (h ^ (w >> 16 & 0xFF)).wrapping_mul(16777619);
            h = (h ^ (w >> 24)).wrapping_mul(16777619);
        }
        let idx = (h ^ salt).wrapping_mul(16777619) % 9;
        CAPY_FRAME.store(idx as u8, Ordering::Relaxed);
        CAPY_SINCE.store(now_ms, Ordering::Relaxed);
        idx as usize
    } else if now_ms.wrapping_sub(CAPY_SINCE.load(Ordering::Relaxed))
        >= CAPY_INTERVAL.as_millis() as u32
    {
        let nf = ((f as usize + 1) % 9) as u8;
        CAPY_FRAME.store(nf, Ordering::Relaxed);
        CAPY_SINCE.store(now_ms, Ordering::Relaxed);
        nf as usize
    } else {
        f as usize
    }
}

fn draw_capy<D: DrawTarget<Color = BinaryColor>>(d: &mut D, frame: usize) {
    // 70px wide (white edges trimmed by the generator), centred on the
    // 144-line canvas, 1px above exact centre (tuned).
    blit(d, &crate::capy_art::CAPY_FRAMES[frame], crate::capy_art::CAPY_W as i32, crate::capy_art::CAPY_H as i32, 9, 1, 1, 11);
}

/// Bluetooth rune, 9x14, MSB-first, 2 bytes per row. Same bitmap rmk ships
/// in its private icon set (`display/renderers/icons.rs` is `pub(crate)`,
/// so the bytes are replicated here rather than patched upstream).
const BT_ICON: [u8; 28] = [
    0x3E, 0x00, 0x67, 0x00, 0xE3, 0x80, 0xE9, 0x80, 0x8C, 0x80, 0xC9, 0x80, 0xE3, 0x80, 0xE3, 0x80,
    0xC9, 0x80, 0x8C, 0x80, 0xE9, 0x80, 0xE3, 0x80, 0x67, 0x00, 0x3E, 0x00,
];
/// USB badge: `usb_02.xbm` from pablopalacios/sm4tik-xbm-icons (MIT) — the
/// classic USB-A plug silhouette, 8x8 pixel-native, blitted at 2x (16x16).
/// Shifted 2 columns left so the artwork's own left edge sits on x0.
const USB_ICON: [u8; 8] = [0x20, 0x28, 0xA8, 0xA8, 0xB0, 0x60, 0x20, 0x20];
const BT_W: i32 = 9;
const BT_H: i32 = 14;
const USB_W: i32 = 8;
const USB_H: i32 = 8;
const USB_SCALE: i32 = 2;

// Absolute corner plan for the 72x144 portrait canvas. All values tuned on
// the real panel; do not re-derive.
const TOP_Y: i32 = 8; //  battery bar & digits anchor here
/// Mode-badge icons & battery ride 6px above the anchor.
const ICON_Y: i32 = TOP_Y - 6;
/// Digits sit 4px below the anchor: the 6x13 cell keeps dead space above
/// its glyphs, so this matches the optical centres of the 14px rune and
/// the 16px USB badge (icon ~y8..23, glyph ~y12..22).
const NUM_Y: i32 = TOP_Y + 4;
const NUM_RIGHT_EDGE: i32 = 70; // battery number right-align edge (2px margin)
const LAYER_Y: i32 = 134; // bottom-flush on the 144-line canvas; the 9x18
                          // cell's dead rows absorb the overrun

/// Panel on the left half (the split central). Each bin only constructs its
/// own, hence the allow.
#[allow(dead_code)]
pub struct LeftScreen {
    pub snap: Option<UiSnap>,
}

/// Panel on the right half (the split peripheral).
#[allow(dead_code)]
pub struct RightScreen {
    pub snap: Option<UiSnap>,
}

/// Snapshot of everything `render_ui` reads. The keyboard fires this render
/// on every key event (throttled), but nothing on screen changes per-keystroke
/// - so an all-equal snapshot lets render return in microseconds instead of
/// repainting ~9k pixels, and flush_native's hash then skips the SPI sweep.
/// Without this, the redraw window + ring-depth-1 backpressure stall the
/// matrix scan and type lag appears.
///
/// `key_press_latch`/`modifiers`/`wpm` are deliberately absent: this UI does
/// not draw them, so a change there must NOT cost a repaint.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct UiSnap {
    frame: usize,
    usb_on: bool,
    /// BleState discriminant: 0 Inactive, 1 Advertising, 2 Connected.
    ble_state: u8,
    profile: u8,
    level: Option<u8>,
    layer: u8,
}

/// Battery display filter. rmk samples one-shot SAADC readings with no
/// smoothing (hardware oversample cannot work in one-shot mode) and
/// publishes on every 1% change, so raw percentages wobble +-1..2 digits -
/// worst on the right half, where 30 s samples randomly collide with BLE
/// bursts. Display rules, in order:
///   * >=5% jump: immediate (big state change, e.g. charger plug event);
///   * >=2% or two consecutive same-direction 1% samples: eligible - but
///     still rate-limited to one update per BATTERY_DISPLAY_GAP;
///   * key-event renders re-read the same raw and never touch the machine.
/// The gap exists because during charging the voltage climbs steadily and
/// the pure-value rules alone would repaint every minute - Lemon asked for
/// calmer battery behavior, and the number has no business moving that
/// often. (Hardware sample rate itself is rmk's fixed 30 s default.)
static SHOWN_LEVEL: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0xFF);
static PREV_RAW: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0xFF);
static TREND: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
/// u32 ms: thumbv7em has no AtomicU64; wraps at 49.7 days, and a wrap only
/// costs one rate-limited refresh, not correctness.
static LAST_UPD_MS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
const BATTERY_DISPLAY_GAP_MS: u32 = 120_000;

fn shown_battery(raw: Option<u8>) -> Option<u8> {
    use core::sync::atomic::Ordering::Relaxed;
    let Some(r) = raw else { return None };
    let shown = SHOWN_LEVEL.load(Relaxed);
    if shown == 0xFF {
        SHOWN_LEVEL.store(r, Relaxed);
        PREV_RAW.store(r, Relaxed);
        LAST_UPD_MS.store(embassy_time::Instant::now().as_millis() as u32, Relaxed);
        return Some(r);
    }
    let prev = PREV_RAW.load(Relaxed);
    if r == prev {
        return Some(shown); // same sample re-read: no new information
    }
    PREV_RAW.store(r, Relaxed);
    let delta = (r as i16 - shown as i16).abs();
    let trend_ok = if delta >= 2 {
        true
    } else {
        // 1-digit move: accept on the second consecutive same-direction sample.
        let dir: u8 = if r > prev { 1 } else { 2 };
        if TREND.load(Relaxed) == dir {
            TREND.store(0, Relaxed);
            true
        } else {
            TREND.store(dir, Relaxed);
            false
        }
    };
    if !trend_ok {
        return Some(shown);
    }
    if delta < 5 {
        let now = embassy_time::Instant::now().as_millis() as u32;
        if now.wrapping_sub(LAST_UPD_MS.load(Relaxed)) < BATTERY_DISPLAY_GAP_MS {
            return Some(shown); // rate-limited: repaint soon, not now
        }
        LAST_UPD_MS.store(now, Relaxed);
    }
    SHOWN_LEVEL.store(r, Relaxed);
    Some(r)
}

fn make_snap(ctx: &RenderContext, usb_on: bool, salt: u32) -> UiSnap {
    UiSnap {
        frame: capy_frame(salt),
        usb_on,
        ble_state: match ctx.ble_status.state {
            BleState::Inactive => 0,
            BleState::Advertising => 1,
            BleState::Connected => 2,
        },
        profile: ctx.ble_status.profile,
        level: shown_battery(match ctx.battery.0 {
            BatteryStatus::Available { level, .. } => level,
            _ => None,
        }),
        layer: ctx.layer as u8,
    }
}

fn txt<D: DrawTarget<Color = BinaryColor>>(d: &mut D, s: &str, x: i32, y: i32, style: MonoTextStyle<'static, BinaryColor>) {
    Text::new(s, Point::new(x, y), style).draw(d).ok();
}

/// 1-bit bitmap blit, MSB-first, `stride` bytes per row, integer `scale`.
#[allow(clippy::too_many_arguments)]
fn blit<D: DrawTarget<Color = BinaryColor>>(d: &mut D, bits: &[u8], w: i32, h: i32, stride: usize, scale: i32, x0: i32, y0: i32) {
    for row in 0..h {
        let b = &bits[(row as usize) * stride..(row as usize) * stride + stride];
        for col in 0..w {
            if b[col as usize / 8] & (0x80 >> (col as usize % 8)) != 0 {
                Rectangle::new(Point::new(x0 + col * scale, y0 + row * scale), Size::new(scale as u32, scale as u32))
                    .into_styled(FILL_ON)
                    .draw(d)
                    .ok();
            }
        }
    }
}

fn render_ui<D: DrawTarget<Color = BinaryColor>>(
    d: &mut D,
    ctx: &RenderContext,
    usb_on: bool,
    frame: usize,
) {
    d.clear(BinaryColor::Off).ok();
    draw_capy(d, frame); // centre band y20..140; status row & layer name
                        // overdraw it afterwards

    // --- top-left: mode badge ---
    if usb_on {
        blit(d, &USB_ICON, USB_W, USB_H, 1, USB_SCALE, 2, ICON_Y);
    } else {
        // The rune goes dark entirely only when the radio is inactive;
        // advertising keeps it with a dash where the slot digit belongs.
        if !matches!(ctx.ble_status.state, BleState::Inactive) {
            blit(d, &BT_ICON, BT_W, BT_H, 2, 1, 2, ICON_Y);
            let mut s: String<4> = String::new();
            match ctx.ble_status.state {
                BleState::Advertising => {
                    s.push('-').ok();
                }
                _ => {
                    write!(s, "{}", ctx.ble_status.profile).ok();
                }
            }
            txt(d, &s, BT_W + 4, NUM_Y, HEAD); // icon x2..10, 2px gap
        }
    }

    // --- top-right: battery bar + number, chained right-aligned columns ---
    // Displayed value passes the anti-wobble filter (shown_battery); the
    // snap made the same call with the same sample, so both agree.
    let level = shown_battery(match ctx.battery.0 {
        BatteryStatus::Available { level, .. } => level,
        _ => None,
    });
    let mut s: String<4> = String::new();
    match level {
        Some(l) => write!(s, "{}", l).ok(),
        None => s.push_str("--").ok(),
    };
    // Number rides the right edge (2px canvas margin); the bar hugs its
    // left so both slide together as the digit count changes.
    let digits = s.chars().count() as i32;
    let num_x = NUM_RIGHT_EDGE - digits * 6;
    let bar_x = num_x - 3 - 22; // cap ends 1px clear of the digits
    Rectangle::new(Point::new(bar_x, ICON_Y + 1), Size::new(22, 11))
        .into_styled(FILL_ON)
        .draw(d)
        .ok();
    Rectangle::new(Point::new(bar_x + 1, ICON_Y + 2), Size::new(20, 9))
        .into_styled(FILL_OFF)
        .draw(d)
        .ok();
    if let Some(l) = level {
        let w = (l as u32 * 18 / 100).clamp(1, 18);
        Rectangle::new(Point::new(bar_x + 2, ICON_Y + 3), Size::new(w, 7))
            .into_styled(FILL_ON)
            .draw(d)
            .ok();
    }
    Rectangle::new(Point::new(bar_x + 22, ICON_Y + 4), Size::new(2, 5))
        .into_styled(FILL_ON)
        .draw(d)
        .ok();
    txt(d, &s, num_x, NUM_Y, HEAD);

    // --- bottom row, centred: layer name, largest glyphs on the panel ---
    let label = LAYER_NAMES
        .get(ctx.layer as usize)
        .copied()
        .unwrap_or("?");
    let w = label.chars().count() as i32 * 9;
    txt(d, label, (72 - w) / 2, LAYER_Y, BIG);
}

impl DisplayRenderer<BinaryColor> for LeftScreen {
    fn render<D: DrawTarget<Color = BinaryColor>>(
        &mut self,
        ctx: &RenderContext,
        d: &mut D,
    ) {
        let usb_on = crate::usb_diag::usb_connected();
        let snap = make_snap(ctx, usb_on, 0x85eb_ca6b);
        if self.snap == Some(snap) {
            return; // nothing the UI draws has changed
        }
        self.snap = Some(snap);
        // ZMK's left/right salts (right = the _DG variant's) live in make_snap;
        // the UI body needs only the resolved frame index.
        render_ui(d, ctx, usb_on, snap.frame);
    }
}

impl DisplayRenderer<BinaryColor> for RightScreen {
    fn render<D: DrawTarget<Color = BinaryColor>>(
        &mut self,
        ctx: &RenderContext,
        d: &mut D,
    ) {
        // Wired badge comes from the central's mirrored ConnectionStatus,
        // tracked by our usb_diag task (see peripheral.rs).
        let usb_on = crate::usb_diag::usb_connected();
        let snap = make_snap(ctx, usb_on, 0x9e37_79b9);
        if self.snap == Some(snap) {
            return; // nothing the UI draws has changed
        }
        self.snap = Some(snap);
        render_ui(d, ctx, usb_on, snap.frame);
    }
}
