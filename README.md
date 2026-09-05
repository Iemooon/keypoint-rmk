# Keypoint-RMK

Firmware for the **Keypoint** split keyboard (Zitaotech), ported from ZMK to
[RMK](https://github.com/rmk-rs/rmk) in Rust. Two nRF52840 halves connected
over BLE split, each with its own status display.

## Hardware

| | Left half (central) | Right half (peripheral) |
|---|---|---|
| MCU | nRF52840 (Adafruit bootloader) | nRF52840 (Adafruit bootloader) |
| Matrix | 6 rows x 8 cols (rows 0..5) | 6 rows x 8 cols (rows 6..11, `row_offset: 6`) |
| Display | JDI LPM009M360A memory LCD | JDI LPM009M360A memory LCD |
| Pointer | A320 trackpad (I2C @ 0x3B) | TrackPoint via PS/2-I2C bridge (0x15) |
| Encoder | 1 rotary (host volume / paging) | 1 rotary (pointer speed tiers) |
| LED | host-link indicator (P0.07) | split-link indicator (P0.06) |

The LPM009M360A is a 144x72 memory-in-pixel LCD driven over write-only SPI;
both halves render it in 72x144 portrait. The panel protocol is transcribed
from the ZMK `lpm_view` driver (see `src/lpm009m360a.rs`).

## Firmware features

- **BLE split + USB**: central talks to the host over USB or BLE (3 profiles);
  the right half connects to the central over BLE.
- **Status panels**: connection badge (Bluetooth rune + profile slot, or USB
  plug), battery bar + percentage, layer name, and a capybara animation that
  advances one frame every 10 minutes. Frame layout is tuned on real hardware.
- **Auto mouse layer**: pointer motion enters layer 4, idle times out
  (500 ms pad / 1000 ms nub). Thumb keys act as mouse buttons on that layer.
- **Scroll keys**: hold the left thumb key to turn the trackpad into a scroll
  wheel, the right thumb key for the TrackPoint.
- **Speed tiers**: the right encoder and FUNC-layer `Kp1..Kp8` cells drive
  per-device pointer/scroll tiers stored in keymap cells (VIA-editable).
- **Sleep awareness**: rmk's sleep state is mirrored into an atomic; the
  trackpad/trackpoint poll loops drop to a 2 s heartbeat while asleep.
- **Vial support**: full Vial config over USB (`vial.json`); `QK_KB_0..31`
  map to rmk `process_user` for BLE profile switch / bond clearing.
- **Watchdog** on both halves; flip-link stack-overflow check at link time.

## Repository layout

```
src/central.rs        left-half binary (host link, matrix, trackpad, panel)
src/peripheral.rs     right-half binary (split link, matrix, TrackPoint, panel)
src/lpm009m360a.rs    JDI memory-LCD driver (write-only SPI, held-CS flush)
src/renderers.rs      panel UI: badges, battery, layer name, capybara
src/trackpad.rs       A320 trackpad driver (I2C + MOTION, forensic counters)
src/trackpoint.rs     TrackPoint driver (PS/2 bridge, polling, ZMK accel curve)
src/pointer_speed.rs  runtime pointer/scroll tiers (atomics + tier tables)
src/speed_control.rs  reads tier cells from the keymap on every key event
src/scroll_key.rs     thumb-key scroll mode controller
src/motion_pin.rs     polled `Wait` impl (GPIOTE channels are scarce)
src/sleep_watch.rs    sleep-state mirror for the poll loops
src/status_led.rs     PWM link-indicator LEDs
src/usb_diag.rs       USB-presence snapshot + transport transition tape
src/tp_diag.rs        TrackPoint diagnostic counters
src/keymap.rs         12x8x5 keymap, transcribed from the ZMK keymap
keyboard.toml         rmk event-channel quotas (read the comments!)
tools/                capybara frame generator, screen preview, XBM helpers
```

## Build

Toolchain: stable Rust with the `thumbv7em-none-eabihf` target, plus
`cargo-make`, `cargo-binutils`, `cargo-hex-to-uf2` and `flip-link`.

```shell
rustup target add thumbv7em-none-eabihf
cargo install cargo-make cargo-binutils cargo-hex-to-uf2 flip-link

# Build both halves and produce UF2 files
cargo make uf2 --release
```

Output: `rmk-central.uf2` (left half) and `rmk-peripheral.uf2` (right half).

## Flash

Put the half into bootloader mode (double-tap reset); a USB drive appears.
Drag the matching `.uf2` onto it. Flash the central first, then the
peripheral. Remember to unplug USB afterwards - rmk prefers USB over BLE
while a cable is connected.

## Notes

- The rmk dependency is pinned to a git rev; bump `Cargo.toml` deliberately.
- `keyboard.toml` carries the event-channel subscriber quotas. rmk's
  generated code hard-panics when a channel runs out of slots, and one panic
  resets the whole board - read the file's comments before adding any
  processor or subscriber.
- The panel's addressable column range is 1..=144; the layout coordinates in
  `renderers.rs` are tuned on real hardware and must not be rescaled from
  datasheet numbers.

## Credits

- [RMK](https://github.com/rmk-rs/rmk) - keyboard firmware framework
- [ZMK](https://zmk.dev/) - the original Keypoint firmware, source of the
  keymap, panel protocol and pointer transfer functions
- [sm4tik-xbm-icons](https://github.com/pablopalacios/sm4tik-xbm-icons)
  (MIT) - USB badge bitmap

## License

MIT OR Apache-2.0
