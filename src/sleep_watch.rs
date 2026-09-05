//! Sleep-state mirror for the polling loops.
//!
//! rmk publishes `SleepStateEvent(bool)` (true = sleeping) when its idle
//! policy trips. Our input-device tasks (trackpoint 10 ms poll, trackpad
//! 500 ms liveness probe) are plain `loop`s that rmk's sleep machinery
//! cannot stop - left alone they pin both halves awake forever. Each poll
//! loop checks `sleeping()` here and stretches its period to seconds while
//! the host side is asleep; I2C traffic and GPIO waits go quiet.

use core::sync::atomic::{AtomicBool, Ordering};

use rmk::event::{EventSubscriber, SleepStateEvent, SubscribableEvent};

static SLEEPING: AtomicBool = AtomicBool::new(false);

/// True while rmk considers the keyboard asleep.
pub fn sleeping() -> bool {
    SLEEPING.load(Ordering::Relaxed)
}

#[embassy_executor::task]
pub async fn sleep_watch_run() -> ! {
    let mut sub = SleepStateEvent::subscriber();
    loop {
        let sleeping = sub.next_event().await.0;
        SLEEPING.store(sleeping, Ordering::Relaxed);
    }
}
