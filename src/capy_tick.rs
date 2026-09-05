//! Capybara frame-switch ticker.
//!
//! The panel only repaints on events (memory-LCD discipline), so an idle
//! keyboard would freeze the animation. Every 600 s we publish a
//! `WpmUpdateEvent(0)` - DisplayProcessor subscribes to it, our renderer
//! never reads WPM, so the only visible effect is the frame check inside
//! `render_ui`.

use rmk::event::{publish_event, WpmUpdateEvent};

#[embassy_executor::task]
pub async fn capy_tick_run() -> ! {
    loop {
        rmk::embassy_time::Timer::after_secs(600).await;
        publish_event(WpmUpdateEvent(0));
    }
}
