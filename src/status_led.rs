//! Connection-status LEDs on plain PWM.
//!
//! Two single-color PWM lamps (ZMK's `custom_led` nodes): left half P0.07,
//! right half P0.06. The touchpad backlight P1.09 is unused and PWM1 is not
//! initialized. (The ws2812 the keyboard dts advertises on SPI3 drives
//! nothing on this PCB.)
//!
//! Division of labor:
//! - LEFT half LED carries the HOST connection: blink (1 Hz) while not
//!   linked to the PC, dark once linked;
//! - RIGHT half LED carries the SPLIT connection: blink while no central is
//!   attached, dark once linked to the left half.
//!
//! Status arrives through event subscribers: `link_state_task` (central,
//! host leg via `ConnectionStatusChangeEvent`), `central_link_task`
//! (peripheral, split leg via `CentralConnectedEvent`). `SimplePwm` takes no
//! interrupt; each LED task refreshes duty every 50 ms.

use core::sync::atomic::{AtomicU8, Ordering};

use embassy_nrf::pwm::{DutyCycle, SimpleConfig, SimplePwm};
use rmk::event::{CentralConnectedEvent, ConnectionStatusChangeEvent, EventSubscriber, SubscribableEvent};
use rmk::types::ble::BleState;
use rmk::types::connection::{ConnectionStatus, UsbState};

/// Snapshotted link status. The initial value is advertising on purpose:
/// there is no way to read the framework's current state from outside, so
/// "searching for the host" is the honest default and the first event
/// corrects it.
#[allow(dead_code)] // consumed by note_status, which only runs on the central
const ST_IDLE: u8 = 0;
const ST_ADV: u8 = 1;
const ST_CONN: u8 = 2;

static LINK_STATE: AtomicU8 = AtomicU8::new(ST_ADV);

fn link_state() -> u8 {
    LINK_STATE.load(Ordering::Relaxed)
}

#[allow(dead_code)] // reached only through link_state_task (central half)
fn note_status(status: &ConnectionStatus) {
    // NOT `decide_active()`: it counts BLE as ready only when Connected, so a
    // keyboard mid-pairing (Advertising) reads as "no transport" there and the
    // blink would stop during pairing. Read the legs directly instead: a live
    // USB leg is "linked" whatever BLE is doing, otherwise the raw BleState
    // is the truth.
    let usb_up = matches!(status.usb, UsbState::Configured | UsbState::Suspended);
    let code = if usb_up {
        ST_CONN
    } else {
        match status.ble.state {
            BleState::Advertising => ST_ADV,
            BleState::Connected => ST_CONN,
            _ => ST_IDLE,
        }
    };
    LINK_STATE.store(code, Ordering::Relaxed);
}

/// Cadence constants, in 50 ms ticks: half LED 1 Hz.
const SLOW_ON_TICKS: u8 = 10;
const SLOW_CYCLE_TICKS: u8 = 20;

/// Blink brightness, one notch under full - 255 is indistinguishable in
/// practice and the only thing it buys is battery.
const BLINK_LEVEL: u8 = 240;

pub fn pwm_config() -> SimpleConfig {
    let mut cfg = SimpleConfig::default();
    cfg.max_duty = 1000;
    cfg
}

pub struct StatusLed<'d> {
    pwm: SimplePwm<'d>,
}

impl<'d> StatusLed<'d> {
    pub fn new(pwm: SimplePwm<'d>) -> Self {
        Self { pwm }
    }

    /// 0 = off ..= 255 = full. `inverted` puts the high phase at the start of
    /// the period, so a bigger level reads as brighter.
    pub fn set_level(&mut self, level: u8) {
        let duty = (level as u16 * 1000 + 127) / 255;
        self.pwm.set_duty(0, DutyCycle::inverted(duty));
    }
}

#[embassy_executor::task]
pub async fn link_state_task() -> ! {
    let mut sub = ConnectionStatusChangeEvent::subscriber();
    loop {
        note_status(&sub.next_event().await.0);
    }
}

/// Right-half alternative to `link_state_task`: the half LED there reports
/// the split leg, not the host leg - blink while no central is attached,
/// dark once connected to the left half. Fed by rmk's own
/// `CentralConnectedEvent`, published around its central connect/disconnect
/// loop.
#[embassy_executor::task]
pub async fn central_link_task() -> ! {
    let mut sub = CentralConnectedEvent::subscriber();
    loop {
        let connected = sub.next_event().await.connected;
        LINK_STATE.store(if connected { ST_CONN } else { ST_ADV }, Ordering::Relaxed);
    }
}

/// The half-mounted indicator: blinks as long as there is no link to the
/// host, whatever the reason (advertising, radio idle); dark while linked.
/// While the keyboard sleeps the lamp is dark regardless of link state -
/// nobody is looking at an unattended keyboard, and the old 1 Hz blink ran
/// through the night unthrottled (it is not covered by the sleep manager's
/// poll slowdown). The 500 ms re-check keeps wake-up snappy without the
/// 50 ms tick cost.
#[embassy_executor::task]
pub async fn custom_led_task(mut led: StatusLed<'static>) -> ! {
    let mut tick: u8 = 0;
    loop {
        if crate::sleep_watch::sleeping() {
            led.set_level(0);
            embassy_time::Timer::after_millis(500).await;
            continue;
        }
        if link_state() == ST_CONN {
            led.set_level(0);
        } else if tick % SLOW_CYCLE_TICKS < SLOW_ON_TICKS {
            led.set_level(BLINK_LEVEL);
        } else {
            led.set_level(0);
        }
        tick = tick.wrapping_add(1);
        embassy_time::Timer::after_millis(50).await;
    }
}
