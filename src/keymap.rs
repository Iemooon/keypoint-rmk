use rmk::types::action::{EncoderAction, KeyAction};
use rmk::types::modifier::ModifierCombination;
use rmk::{a, encoder, k, lt, mo, shifted, tg, user, wm};
pub(crate) const COL: usize = 8;
pub(crate) const ROW: usize = 12;
pub(crate) const NUM_LAYER: usize = 5;
pub(crate) const NUM_ENCODER: usize = 2;

// Transcribed from keypoint-zmk-dongle/config/keypoint.keymap. ZMK's layer
// macros are `RAISE 1`, `LOWER 2`, `FUNC 3`, `MOUSE 4`; layers 0..3 map
// one-to-one, layer 4 (MOUSE) is auto-activated by pointer motion.
//
// The halves join along different axes, so the copy is a transpose:
//
//   ZMK   rows = <6>, columns = <16>: left hand owns columns 0..7, right
//         hand columns 8..15 - concatenated along COLUMNS.
//   RMK   ROW = 12 / COL = 8 with `row_offset: 6`: rows 0..5 left, rows
//         6..11 right - concatenated along ROWS.
//
//   mapping:  left  (r, c)     -> RMK (r, c)
//             right (r, c + 8) -> RMK (r + 6, c)
//
// Every layer's binding list is 56 entries in the same shape as `map`, so
// position N of any layer lands on the same physical key as position N of
// the base layer. Position order verified against the ZMK combos (<5 4>,
// <16 17>, <31 30>, <43 44>, <51 50>); pins cross-checked against the ZMK
// kscan, no row/column reversal needed.
//
// ZMK constructs not expressible here, marked inline:
//   * `&auto_shift` is written as its tap half only.
//   * `&bt BT_CLR` / `&bt BT_CLR_ALL` ride on `User(id)` keys handled by
//     rmk's `process_user` (see the BT keys in layer 3). VIA exposes the
//     same keys as QK_KB_0..31.
//
// `a!(No)` on unused cells is deliberate - those matrix positions have no
// switch, and keeping them reserved documents the physical shape.
#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        // ==================== Layer 0: QWERTY ====================
        [
            // Left rows 0..5
            [k!(Escape),   k!(Q), k!(W), k!(E), k!(R), k!(T), a!(No), a!(No)],
            [k!(Tab),      k!(A), k!(S), k!(D), k!(F), k!(G), a!(No), a!(No)],
            [k!(LShift),   k!(Z), k!(X), k!(C), k!(V), k!(B), a!(No), a!(No)],
            [k!(LCtrl),    k!(LGui), k!(LAlt), lt!(2, Space), lt!(1, Space), mo!(3), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            // pos 18 = &mkp LCLK, pos 45 = calculator. Mouse keys need no
            // feature: `is_mouse_key()` routes them into `mouse.process`.
            //
            // The first two cells are NOT switches -- (5,0) and (5,1) have no
            // physical mapping on this board, so the matrix can never read them
            // closed and they can never emit. They are two readable storage slots:
            // their key value is the left-hand tier number.
            // Read by `speed_control.rs` via `KeyMap::action_at_pos`.
            [k!(Kp5), k!(Kp5), a!(No), a!(No), a!(No), a!(No), k!(MouseBtn1), k!(Calculator)],
            // Right rows 6..11
            [k!(Y),        k!(U), k!(I), k!(O), k!(P), k!(Backspace), a!(No), a!(No)],
            [k!(H),        k!(J), k!(K), k!(L), k!(Semicolon), k!(Enter), a!(No), a!(No)],
            [k!(N),        k!(M), k!(Comma), k!(Dot), k!(Up), k!(Delete), a!(No), a!(No)],
            [k!(Slash),    k!(Space), mo!(2), k!(Left), k!(Down), k!(Right), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            // pos 19 = none, pos 52 = C_MUTE. First two cells: same trick as
            // (5,0)/(5,1), storage-only slots for the right-hand tiers. Scroll
            // preset is Kp5 (19), matching the nub's measured-good 17; the pad's
            // stays Kp3 (30) against its 34.
            [k!(Kp5), k!(Kp5), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
        ],
        // ==================== Layer 1: RAISE ====================
        [
            [a!(Transparent), k!(Kc1), k!(Kc2), k!(Kc3), k!(Kc4), k!(Kc5), a!(No), a!(No)],
            [a!(Transparent), k!(Quote), shifted!(Quote), k!(Minus), k!(Equal), k!(Enter), a!(No), a!(No)],
            [a!(Transparent), k!(Home), k!(End), shifted!(Minus), shifted!(Equal), k!(Backslash), a!(No), a!(No)],
            [a!(Transparent), wm!(Home, ModifierCombination::LCTRL), wm!(End, ModifierCombination::LCTRL), k!(Calculator), tg!(1), tg!(1), a!(No), a!(No)],
            // pos 32 = trans on the left thumb, pos 33 = none
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [k!(Kc7), k!(Kc8), k!(Kc9), k!(KpPlus), k!(KpAsterisk), a!(Transparent), a!(No), a!(No)],
            [k!(Kc4), k!(Kc5), k!(Kc6), k!(KpMinus), k!(KpSlash), a!(Transparent), a!(No), a!(No)],
            [k!(Kc1), k!(Kc2), k!(Kc3), k!(Comma), a!(Transparent), a!(Transparent), a!(No), a!(No)],
            [k!(Kc0), k!(Kc0), k!(KpDot), a!(Transparent), a!(Transparent), a!(Transparent), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
        ],
        // ==================== Layer 2: LOWER ====================
        [
            [shifted!(Grave), shifted!(Kc1), shifted!(Kc2), shifted!(Kc3), shifted!(Kc4), shifted!(Kc5), a!(No), a!(No)],
            [k!(Grave), shifted!(Kc6), shifted!(Kc7), shifted!(Kc8), shifted!(Kc9), shifted!(Kc0), a!(No), a!(No)],
            [a!(No), k!(Home), k!(End), wm!(PageUp, ModifierCombination::LCTRL), wm!(PageDown, ModifierCombination::LCTRL), wm!(F4, ModifierCombination::LALT), a!(No), a!(No)],
            [a!(No), wm!(Home, ModifierCombination::LCTRL), wm!(End, ModifierCombination::LCTRL), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            // pos 18 = none, pos 45 = trans
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [k!(NumLock), k!(CapsLock), k!(Pause), k!(PrintScreen), wm!(P, ModifierCombination::LCTRL), a!(No), a!(No), a!(No)],
            [a!(No), k!(LeftBracket), k!(RightBracket), shifted!(LeftBracket), shifted!(RightBracket), a!(No), a!(No), a!(No)],
            [a!(No), shifted!(Kc9), shifted!(Kc0), wm!(PageUp, ModifierCombination::LCTRL), k!(PageUp), wm!(PageDown, ModifierCombination::LCTRL), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), k!(Home), k!(PageDown), k!(End), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
        ],
        // ==================== Layer 3: FUNC ====================
        [
            // (0,4) = &msc SCRL_UP. Whether the wheel axes survive
            // `mouse.process` is untested on hardware - the button bits do.
            [k!(F1), k!(F2), k!(F3), wm!(PageUp, ModifierCombination::LCTRL), k!(MouseWheelUp), wm!(PageDown, ModifierCombination::LCTRL), a!(No), a!(No)],
            // (1,3..5) = &msc SCRL_LEFT / SCRL_DOWN / SCRL_RIGHT
            [k!(F4), k!(F5), k!(F6), k!(MouseWheelLeft), k!(MouseWheelDown), k!(MouseWheelRight), a!(No), a!(No)],
            [k!(F7), k!(F8), k!(F9), a!(No), k!(F2), k!(F5), a!(No), a!(No)],
            // (3,3)/(3,4) = BLE profile 0 / profile 1, handled natively by rmk's
            // `process_user`: a tap switches to that profile, a 5s hold clears
            // that profile's bond and switches to it (advertising openly for
            // fresh pairing). No custom keymap code is involved.
            [k!(F10), k!(F11), k!(F12), user!(0), user!(1), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            // (5,7) = &bt BT_CLR: with NUM_BLE_PROFILE = 3 (rmk-config default),
            // `process_user` maps id 5 = NUM_BLE_PROFILE + 2 to ClearBond for the
            // current profile (same as VIA's QK_KB_5). Do not "fix" it to 4 --
            // at NUM_BLE_PROFILE = 3, id 4 is Previous-profile.
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), user!(5)],
            [k!(Kc7), k!(Kc8), k!(Kc9), k!(KpPlus), k!(KpAsterisk), a!(Transparent), a!(No), a!(No)],
            [k!(Kc4), k!(Kc5), k!(Kc6), k!(KpMinus), k!(KpSlash), a!(Transparent), a!(No), a!(No)],
            [k!(Kc1), k!(Kc2), k!(Kc3), k!(Comma), a!(Transparent), a!(Transparent), a!(No), a!(No)],
            [k!(Kc0), k!(Kc0), k!(KpDot), a!(Transparent), a!(Transparent), a!(Transparent), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            // pos 52 (ZMK's BT_CLR_ALL) and pos 19 stay empty: per-slot bond
            // clearing is the 5s hold on user!(0)/(1) above.
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
        ],
        // ==================== Layer 4: MOUSE (auto only) ====================
        // No MO/TG key points here: rmk's auto mouse layer enters on pointer
        // motion and leaves on the idle timeout.
        //
        // Written out in full instead of leaning on `Transparent`: as the
        // highest complete layer it shadows every lower layer while up, so
        // typing here is always base-plus-thumb-buttons (same as ZMK).
        //
        // Button layout is intentional, NOT copied from ZMK's MOUSE layer -
        // each hand carries both buttons. Do not "restore" ZMK assignments:
        //
        //   left button   (3,5) and (9,0)   MouseBtn1
        //   right button  (3,3) and (9,2)   MouseBtn2
        //   middle button (5,7)             MouseBtn3
        //   scroll key    (3,4) -> pad      scroll key (9,1) -> nub
        //
        // (3,4) and (9,1) carry no keymap action: `scroll_key.rs` watches
        // those positions and arms them only while this layer is active.
        [
            // Left hand, rows 0..5 - base letters, thumb cluster turned buttons.
            [k!(Escape), k!(Q), k!(W), k!(E), k!(R), k!(T), a!(No), a!(No)],
            [k!(Tab), k!(A), k!(S), k!(D), k!(F), k!(G), a!(No), a!(No)],
            [k!(LShift), k!(Z), k!(X), k!(C), k!(V), k!(B), a!(No), a!(No)],
            [k!(LCtrl), k!(LGui), k!(LAlt), k!(MouseBtn2), a!(No), k!(MouseBtn1), a!(No), a!(No)],
            [a!(No); COL],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), k!(MouseBtn1), k!(MouseBtn3) ],
            // Right hand, rows 6..11
            [k!(Y), k!(U), k!(I), k!(O), k!(P), k!(Backspace), a!(No), a!(No)],
            [k!(H), k!(J), k!(K), k!(L), k!(Semicolon), k!(Enter), a!(No), a!(No)],
            [k!(N), k!(M), k!(Comma), k!(Dot), k!(Up), k!(Delete), a!(No), a!(No)],
            [k!(MouseBtn1), a!(No), k!(MouseBtn2), k!(Left), k!(Down), k!(Right), a!(No), a!(No) ],
            [a!(No); COL],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
        ],
    ]
}

/// Read a keymap cell value as a tier number.
/// Accepts Kp1..Kp8 and maps them to tiers 1..8. Any other value returns None,
/// which leaves the current tier in place. The keypad row is used because the
/// main number row still types digits on the RAISE layer.
/// Read through KeyMap::action_at_pos, together with TIER_CELLS.
pub fn tier_of(action: KeyAction) -> Option<u8> {
    [
        k!(Kp1),
        k!(Kp2),
        k!(Kp3),
        k!(Kp4),
        k!(Kp5),
        k!(Kp6),
        k!(Kp7),
        k!(Kp8),
    ]
    .iter()
    .position(|a| *a == action)
    .map(|i| i as u8 + 1)
}

/// The four tier cells, in order: left move, left scroll, right move, right
/// scroll. These coordinates have no switches behind them, so they are read
/// only and can never be typed.
pub const TIER_CELLS: [(u8, u8); 4] = [(5, 0), (5, 1), (11, 0), (11, 1)];

pub const fn get_default_encoder_map() -> [[EncoderAction; NUM_ENCODER]; NUM_LAYER] {
    [
        [
            encoder!(k!(PageUp), k!(PageDown)),
            encoder!(k!(KbVolumeDown), k!(KbVolumeUp)),
        ],
        [
            encoder!(a!(No), a!(No)),
            encoder!(a!(No), a!(No)),
        ],
        [
            encoder!(a!(No), a!(No)),
            encoder!(a!(No), a!(No)),
        ],
        [
            encoder!(a!(No), a!(No)),
            encoder!(a!(No), a!(No)),
        ],
        // Layer 4 (MOUSE): knobs keep their meaning on every layer.
        [
            encoder!(a!(No), a!(No)),
            encoder!(a!(No), a!(No)),
        ],
    ]
}
