//! `Wait` implemented by polling, for when GPIOTE channels run out.
//!
//! Every async GPIO edge wait needs a GPIOTE channel, and the nRF52840 has only
//! eight. This firmware already spends them on the rotary encoder and the async
//! matrix scan, so adding the trackpad MOTION pin and the nub MOTION pin as
//! interrupt inputs may not fit. If the board panics at start with a channel
//! allocation failure - or the pointing device simply never reports - swap that
//! pin for `PollWait` at the call site and nothing else in the driver changes.
//!
//! Level semantics: "edge" here means "the line is in that state now", not
//! "the line just transitioned". That is deliberate. A spurious wake costs one
//! packet read, which both drivers already treat as the end-of-burst no-op.
//!
//! `embedded_hal_async::digital::Wait` has no default method bodies, so all five
//! must be written even though only `wait_for_falling_edge` is used here.

use core::convert::Infallible;

use embedded_hal::digital::{ErrorType, InputPin};
use embedded_hal_async::digital::Wait;
use embassy_time::Timer;

/// Polling `Wait` over any `InputPin`.
pub struct PollWait<P> {
    pin: P,
    /// Poll period. 500 us keeps the loop cheap while staying well under the
    /// ~40 Hz (trackpad) and ~200 Hz (nub) report cadences.
    period_us: u64,
}

impl<P: InputPin> PollWait<P> {
    pub fn new(pin: P) -> Self {
        Self { pin, period_us: 500 }
    }

    // `&mut` because embedded-hal 1.0's InputPin::is_low/is_high take `&mut self`.
    fn is_low(&mut self) -> bool {
        matches!(self.pin.is_low(), Ok(true))
    }

    fn is_high(&mut self) -> bool {
        matches!(self.pin.is_high(), Ok(true))
    }
}

impl<P: InputPin> ErrorType for PollWait<P> {
    type Error = Infallible;
}

impl<P: InputPin> Wait for PollWait<P> {
    async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
        loop {
            if self.is_low() {
                return Ok(());
            }
            Timer::after_micros(self.period_us).await;
        }
    }

    async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
        loop {
            if self.is_high() {
                return Ok(());
            }
            Timer::after_micros(self.period_us).await;
        }
    }

    async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
        self.wait_for_low().await
    }

    async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
        self.wait_for_high().await
    }

    async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
        self.wait_for_low().await
    }
}
