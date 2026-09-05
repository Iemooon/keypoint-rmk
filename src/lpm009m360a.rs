//! JDI LPM009M360A memory-in-pixel LCD - 144x72 monochrome, SPI, no MISO.
//!
//! Protocol transcribed from the ZMK driver
//! `lpm_view/display_driver/lpm009m360a.c` plus `lpm_view.overlay`:
//!
//! ```dts
//! width = <144>;  height = <72>;  color_mode = [02];  reverse = <1>;  rotation = <1>;
//! ```
//!
//! The controller's addressable column range is 1..=144: frames addressed
//! beyond it corrupt the entire transfer (measured 2026-09-06). The physical
//! glass size and the addressable range are separate facts - do not infer
//! one from the other, and do not rescale anything from datasheet numbers.
//! Layout coordinates live in renderers.rs, are tuned against the real
//! panel, and are final.
//!
//! Protocol:
//!   * init: one `ALL_CLEAR` command (`0x20`, arg `0x00`), then 1 ms. No
//!     contrast/clock/power/scan setup exists.
//!   * write: one frame per column line, `[0x88][line+1][9 bytes]`, where
//!     `0x88 = UPDATE(0x80) | (color_mode[0]=0x02) << 2`. 144 frames per flush.
//!   * close: two `[0x00][0x00]` (NO_UPDATE) frames, then CS release. CS is
//!     held across the whole flush (ZMK `SPI_HOLD_ON_CS | SPI_LOCK_ON`).
//!   * no busy polling, no reads, no delays in the write path.
//!
//! Panel-native layout (`rotation = <1>`): 144 column lines x 9 bytes, MSB
//! first, vertically tiled, X mirrored (`buf[(143 - x) * 9 + y / 8]`). The
//! framebuffer is kept in that native layout, so flush needs no repacking.
//!
//! `reverse = <1>` means a **cleared** bit is a lit pixel
//! (`BIT_IS_LIT_WHEN_CLEAR`).
//!
//! Orientation is a per-instance `PanelRot` so mirrored halves can differ;
//! `R0` is the ZMK mapping, `R90`/`R270` give the 72-wide portrait canvas.
//!
//! CS is **active-high** (`GPIO_ACTIVE_HIGH | GPIO_PULL_DOWN`), idle = low.
//! Left half: MOSI P0.05 / SCK P0.27 / CS P0.12. Right half: MOSI P0.17 /
//! SCK P0.15 / CS P0.13. Both on SPI2, 4 MHz, mode 0.

use core::convert::Infallible;

use embassy_time::Timer;
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiBus;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use rmk::display::DisplayDriver;

/// Panel width in column lines = the controller's addressable range (see
/// module doc). Not a statement about the physical glass.
pub const WIDTH: usize = 144;
/// Panel height in pixels (overlay `height = <72>`).
pub const HEIGHT: usize = 72;
/// Bytes used by one column line: the 72 rows of a single line pack into 9 bytes.
const LINE_BYTES: usize = HEIGHT / 8;
/// Framebuffer size in bytes: 144 lines x 9 bytes = 1296.
pub const FRAMEBUFFER_LEN: usize = WIDTH * LINE_BYTES;

/// `LPM009M360A_CMD_NO_UPDATE`; sent twice to close a flush.
const CMD_NO_UPDATE: u8 = 0x00;
/// `LPM009m360A_CMD_ALL_CLEAR`; the only init command the C driver sends.
const CMD_ALL_CLEAR: u8 = 0x20;
/// `LPM009M360A_CMD_UPDATE | (color_mode[0] << 2)` with `color_mode = [02]`.
const CMD_UPDATE: u8 = 0x88;
/// `LPM009M360A_RESET_TIME` / `LPM009M360A_EXIT_SLEEP_TIME`, both `K_MSEC(1)`.
const RESET_DELAY_MS: u64 = 1;

/// The overlay sets `reverse = <1>`, i.e. `PIXEL_FORMAT_MONO01`: a **cleared**
/// bit is a lit pixel. Flip to `false` only if the panel comes up with contrast
/// the wrong way round. This is the single place polarity is decided.
const BIT_IS_LIT_WHEN_CLEAR: bool = true;

/// How the rendered canvas maps onto the panel.
///
/// `R0` is a verbatim transcription of the ZMK `rotation = <1>` mapping, so the
/// two halves look exactly like they do under ZMK. The other variants rotate the
/// canvas within the same panel, which is what you want when the display is
/// mounted with its long edge along the keyboard's short axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Three of the four variants are deliberately unused in the current build: they
// exist so the mounting direction can be changed at the two `new()` call sites
// without touching anything else.
#[allow(dead_code)]
pub enum PanelRot {
    /// 144x72 landscape, ZMK mapping as-is.
    R0,
    /// 72x144 portrait, content rotated 90 degrees clockwise.
    R90,
    /// 144x72 landscape, upside down.
    R180,
    /// 72x144 portrait, content rotated 270 degrees clockwise.
    R270,
}

impl PanelRot {
    /// True when the canvas is rotated into the portrait shape.
    const fn portrait(self) -> bool {
        matches!(self, PanelRot::R90 | PanelRot::R270)
    }

    /// Canvas size the renderer gets to draw into.
    pub const fn logical_size(self) -> Size {
        if self.portrait() {
            Size::new(HEIGHT as u32, WIDTH as u32)
        } else {
            Size::new(WIDTH as u32, HEIGHT as u32)
        }
    }

    /// Canvas point -> panel-native point, the `x` whose column line is
    /// `143 - x`. `None` means the point is outside the canvas.
    const fn to_native(self, x: usize, y: usize) -> Option<(usize, usize)> {
        let (w, h) = if self.portrait() {
            (HEIGHT, WIDTH)
        } else {
            (WIDTH, HEIGHT)
        };
        if x >= w || y >= h {
            return None;
        }
        Some(match self {
            PanelRot::R0 => (x, y),
            PanelRot::R90 => (h - 1 - y, x),
            PanelRot::R180 => (w - 1 - x, h - 1 - y),
            PanelRot::R270 => (y, w - 1 - x),
        })
    }
}

/// FNV-1a over the framebuffer. A flush writes 144 separate column-line frames,
/// and the panel updates line by line as they arrive - on a static screen that
/// sweep was visible once a second. Hashing 1296 bytes costs a few microseconds
/// on the 4800h and lets an unchanged frame be skipped entirely.
fn frame_hash(fb: &[u8; FRAMEBUFFER_LEN]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in fb.iter() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// LPM009M360A driven over an async SPI bus with a software chip-select.
///
/// `Spi` only needs `SpiBus` writes - the panel has no MISO, which is why
/// `embassy_nrf::spim::Spim::new_txonly` is the right constructor on nRF52840.
/// `Cs` is any push-pull output, driven high to select (active-high CS).
pub struct Lpm009m360a<Spi, Cs> {
    spi: Spi,
    cs: Cs,
    rot: PanelRot,
    /// Hash of the frame last sent, or `None` if nothing has gone out yet. Only
    /// used to skip redundant flushes; `None` forces the first one.
    sent_hash: Option<u32>,
    /// Panel-native framebuffer: 144 column lines x 9 bytes. Handed in as
    /// `&'static mut` so 1296 bytes do not end up living in the main task's
    /// stack frame.
    fb: &'static mut [u8; FRAMEBUFFER_LEN],
}

impl<Spi, Cs> Lpm009m360a<Spi, Cs>
where
    Spi: SpiBus,
    Cs: OutputPin,
{
    /// Orientation is passed in so each half of a split board can pick its own.
    pub fn new(
        spi: Spi,
        cs: Cs,
        fb: &'static mut [u8; FRAMEBUFFER_LEN],
        rot: PanelRot,
    ) -> Self {
        Self {
            spi,
            cs,
            rot,
            sent_hash: None,
            fb,
        }
    }

    /// One `[cmd][arg][data...]` frame with CS asserted for its duration.
    /// The header is a local array because DMA wants its source in RAM.
    async fn frame(&mut self, cmd: u8, arg: u8, data: &[u8]) {
        self.cs_assert();
        self.frame_locked(cmd, arg, data).await;
        self.cs_release();
    }

    /// Same, but leaves CS where it found it. A flush holds CS across all 144
    /// frames, matching the C driver's `SPI_HOLD_ON_CS | SPI_LOCK_ON`.
    async fn frame_locked(&mut self, cmd: u8, arg: u8, data: &[u8]) {
        let hdr = [cmd, arg];
        if self.spi.write(&hdr).await.is_err() {
            return;
        }
        if !data.is_empty() {
            let _ = self.spi.write(data).await;
        }
    }

    /// CS is active-high on this board (`GPIO_ACTIVE_HIGH | GPIO_PULL_DOWN`),
    /// so the idle level is low.
    fn cs_assert(&mut self) {
        let _ = self.cs.set_high();
    }

    fn cs_release(&mut self) {
        let _ = self.cs.set_low();
    }

    /// Blank the panel through the RAM path the C driver uses on reset.
    async fn all_clear(&mut self) {
        self.frame(CMD_ALL_CLEAR, 0x00, &[]).await;
        Timer::after_millis(RESET_DELAY_MS).await;
    }

    /// Paint the whole framebuffer with one logic level and push it out.
    ///
    /// `lit = false` means every pixel is off. Used at init so the first frame
    /// after power-on is deterministic instead of whatever the RAM held.
    async fn fill_all(&mut self, lit: bool) {
        let pattern = match (lit, BIT_IS_LIT_WHEN_CLEAR) {
            (true, true) | (false, false) => 0x00,
            (true, false) | (false, true) => 0xFF,
        };
        for byte in self.fb.iter_mut() {
            *byte = pattern;
        }
        self.flush_native().await;
    }

    /// Stream the framebuffer to the panel as one held-CS transaction.
    ///
    /// Returns without touching the bus when the frame matches what is already
    /// on screen.
    async fn flush_native(&mut self) {
        let hash = frame_hash(self.fb);
        if self.sent_hash == Some(hash) {
            return;
        }
        self.cs_assert();
        for line in 0..WIDTH {
            let arg = (line + 1) as u8; // column address is 1-based, 1..=144
            let start = line * LINE_BYTES;
            // Copied out because `frame_locked` needs `&mut self`, which cannot
            // coexist with a borrow of `self.fb`. Nine bytes, and it lands in
            // RAM, which is also what a DMA-capable bus wants.
            let mut chunk = [0u8; LINE_BYTES];
            chunk.copy_from_slice(&self.fb[start..start + LINE_BYTES]);
            self.frame_locked(CMD_UPDATE, arg, &chunk).await;
        }
        // Two NO_UPDATE frames close the transfer; only then does the C driver
        // release CS.
        self.frame_locked(CMD_NO_UPDATE, 0x00, &[]).await;
        self.frame_locked(CMD_NO_UPDATE, 0x00, &[]).await;
        self.cs_release();
        self.sent_hash = Some(hash);
    }
}

impl<Spi, Cs> OriginDimensions for Lpm009m360a<Spi, Cs>
where
    Spi: SpiBus,
    Cs: OutputPin,
{
    fn size(&self) -> Size {
        self.rot.logical_size()
    }
}

impl<Spi, Cs> DrawTarget for Lpm009m360a<Spi, Cs>
where
    Spi: SpiBus,
    Cs: OutputPin,
{
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 {
                continue;
            }
            // usize on both sides: WIDTH/HEIGHT are usize, Point gives u32.
            // 32-bit target, and the negative check above makes the cast safe.
            let Some((nx, ny)) = self.rot.to_native(point.x as usize, point.y as usize) else {
                continue;
            };
            // Bounds guard: a panic here would reset the whole board via the
            // watchdog, far too expensive for one stray pixel.
            if nx >= WIDTH || ny >= HEIGHT {
                continue;
            }
            // rotation = 1 in the overlay mirrors X: column line = 143 - x.
            let line = WIDTH - 1 - nx;
            let index = line * LINE_BYTES + ny / 8;
            let mask = 1u8 << (7 - (ny % 8)); // MSB first, top of the group
            let lit = color == BinaryColor::On;
            let set_bit = lit != BIT_IS_LIT_WHEN_CLEAR;
            self.fb[index] = if set_bit {
                self.fb[index] | mask
            } else {
                self.fb[index] & !mask
            };
        }
        Ok(())
    }
}

impl<Spi, Cs> DisplayDriver for Lpm009m360a<Spi, Cs>
where
    Spi: SpiBus,
    Cs: OutputPin,
{
    async fn init(&mut self) {
        self.all_clear().await;
        // Start from a known-blank screen before the first render lands.
        self.fill_all(false).await;
    }

    async fn flush(&mut self) {
        self.flush_native().await;
    }
}
