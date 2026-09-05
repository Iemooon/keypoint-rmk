//! Transport transition tape plus the wired-presence snapshot.
//!
//! The tape (one nibble per transition, newest in the low bits of a 32-bit
//! word) was the screen's forensic head line; it is no longer rendered but
//! the writer stays as a tripwire for suspend/resume swallowing: a lag spell
//! that begins after idle seconds on the USB leg reads as `s` before the
//! keystroke and `u` only after wakeup. Wireless builds record the BLE leg
//! (`b`/`a`), so the same tape answers both cases.
//!
//! `USB_NOW` is the live input for the panel's wired badge: the tape head
//! tracks the *active* transport (BLE can shadow a plugged USB leg), the
//! snapshot tracks the USB state itself.
//!
//! Cortex-M4 has no 64-bit atomics, hence the tape's 8-slot limit.

use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use rmk::event::{ConnectionStatusChangeEvent, EventSubscriber, SubscribableEvent};
use rmk::types::ble::BleState;
use rmk::types::connection::{ConnectionType, UsbState};

const USB_ON: u32 = 1;
const USB_SUSPENDED: u32 = 2;
const BLE_CONNECTED: u32 = 3;
const BLE_ADVERTISING: u32 = 4;
const INACTIVE: u32 = 5;

static TAPE: AtomicU32 = AtomicU32::new(0);
/// Current snapshot of the USB *physical* connection. The tape head tracks
/// the active transport (BLE can shadow a plugged USB leg), so the badge
/// reads this instead: 0 = Disconnected, non-zero = Default/Suspended (both
/// mean "cable plugged").
static USB_NOW: AtomicU8 = AtomicU8::new(0);

/// For the UI: is the USB cable connected (Suspended counts as plugged).
pub fn usb_connected() -> bool {
    USB_NOW.load(Ordering::Relaxed) != 0
}

fn record(code: u32) {
    let v = TAPE.load(Ordering::Relaxed);
    TAPE.store((v << 4) | code, Ordering::Relaxed);
}

/// One-char transport summary for the screen head line. Every state change
/// shifts into the tape, so the newest nibble *is* the current state:
/// '+' configured, 's' suspended, '-' something else (BLE or idle),
/// '?' no transition recorded yet.
#[allow(dead_code)] // diagnostic interface kept beside the tape writer
pub fn usb_mark() -> char {
    match (TAPE.load(Ordering::Relaxed) >> 28) & 0xF {
        USB_ON => '+',
        USB_SUSPENDED => 's',
        0 => '?',
        _ => '-',
    }
}

/// Tape as a string, newest transition first.
#[allow(dead_code)] // diagnostic interface, see usb_mark
pub fn snapshot<const N: usize>(out: &mut heapless::String<N>) {
    let v = TAPE.load(Ordering::Relaxed);
    for i in 0..8 {
        let c = match (v >> (4 * i)) & 0xF {
            USB_ON => 'u',
            USB_SUSPENDED => 's',
            BLE_CONNECTED => 'b',
            BLE_ADVERTISING => 'a',
            INACTIVE => 'x',
            _ => continue, // untouched slot
        };
        out.push(c).ok();
    }
}

#[embassy_executor::task]
pub async fn usb_diag_run() -> ! {
    let mut sub = ConnectionStatusChangeEvent::subscriber();
    loop {
        let status = sub.next_event().await.0;
        USB_NOW.store(
            match status.usb {
                UsbState::Disabled => 0,
                UsbState::Suspended => 2,
                _ => 1,
            },
            Ordering::Relaxed,
        );
        let code = match status.decide_active() {
            Some(ConnectionType::Usb) => {
                if status.usb == UsbState::Suspended {
                    USB_SUSPENDED
                } else {
                    USB_ON
                }
            }
            Some(ConnectionType::Ble) => match status.ble.state {
                BleState::Connected => BLE_CONNECTED,
                BleState::Advertising => BLE_ADVERTISING,
                _ => INACTIVE,
            },
            None => INACTIVE,
        };
        record(code);
    }
}
