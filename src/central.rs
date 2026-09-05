#![no_std]
#![no_main]

mod vial;
#[macro_use]
mod macros;
mod keymap;
mod trackpad;
mod motion_pin;
mod scroll_key;
mod renderers;
mod speed_control;
mod pointer_speed;
mod lpm009m360a;
mod tp_diag;
mod usb_diag;
mod capy_art;
mod capy_tick;
mod sleep_watch;
mod status_led;

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Input, Output};
use embassy_nrf::interrupt::{self, InterruptExt};
use embassy_nrf::mode::Async;
use embassy_nrf::peripherals::{RNG, SAADC, USBD};
use embassy_nrf::saadc::{self, AnyInput, Input as _, Saadc};
use embassy_nrf::usb::Driver;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::{Peri, bind_interrupts, rng, usb};
use nrf_mpsl::Flash;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, mpsl};
use panic_probe as _;
use rmk::ble::BleTransport;
use rmk::config::{
    BehaviorConfig, BleBatteryConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig,
};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::event::*;
use rmk::host::HostService;
use rmk::input_device::adc::{AnalogEventType, NrfAdc};
use rmk::input_device::battery::BatteryProcessor;
use rmk::input_device::rotary_encoder::RotaryEncoder;
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::processor::builtin::wpm::WpmProcessor;
use rmk::split::PeripheralMatrixConfig;
use rmk::usb::UsbTransport;
use rmk::watchdog::Nrf52Watchdog;
use rmk::{KeymapData, initialize_keymap_and_storage, run_all};
use static_cell::StaticCell;
use vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};
use embassy_nrf::spim::{self, Spim};
use embassy_nrf::twim::{self, Twim};
use rmk::display::DisplayProcessor;
use rmk::input_device::pointing::{PointingProcessor, PointingProcessorConfig};
use trackpad::A320;
use speed_control::SpeedController;
use rmk::config::AutoMouseLayerConfig;
use rmk::AutoMouseLayerRunner;
use lpm009m360a::{Lpm009m360a, PanelRot};
use motion_pin::PollWait;
use scroll_key::{ScrollKeyController, TRACKPOINT_ID};
use pointer_speed::{cursor_mode, NUB_ID, PAD_ID};
use renderers::LeftScreen;

bind_interrupts!(struct Irqs {
    TWISPI0 => twim::InterruptHandler<embassy_nrf::peripherals::TWISPI0>;
    SPI2 => spim::InterruptHandler<embassy_nrf::peripherals::SPI2>;
    USBD => usb::InterruptHandler<USBD>;
    SAADC => saadc::InterruptHandler;
    RNG => rng::InterruptHandler<RNG>;
    EGU0_SWI0 => nrf_sdc::mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => nrf_sdc::mpsl::ClockInterruptHandler, usb::vbus_detect::InterruptHandler;
    RADIO => nrf_sdc::mpsl::HighPrioInterruptHandler;
    TIMER0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    RTC0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
});

#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}


/// How many outgoing L2CAP buffers per link
const L2CAP_TXQ: u8 = 3;

/// How many incoming L2CAP buffers per link
const L2CAP_RXQ: u8 = 3;

/// Size of L2CAP packets
const L2CAP_MTU: usize = 251;

fn build_sdc<'d, const N: usize>(
    p: nrf_sdc::Peripherals<'d>,
    rng: &'d mut rng::Rng<Async>,
    mpsl: &'d MultiprotocolServiceLayer,
    mem: &'d mut sdc::Mem<N>,
) -> Result<nrf_sdc::SoftdeviceController<'d>, nrf_sdc::Error> {
    sdc::Builder::new()?
        .support_scan()
        .support_central()
        .support_adv()
        .support_peripheral()
        .support_dle_peripheral()
        .support_dle_central()
        .support_phy_update_central()
        .support_phy_update_peripheral()
        .support_le_2m_phy()
        .central_count(1)?
        .peripheral_count(1)?
        .buffer_cfg(L2CAP_MTU as u16, L2CAP_MTU as u16, L2CAP_TXQ, L2CAP_RXQ)?
        .build(p, rng, mpsl, mem)
}

/// Initializes the SAADC peripheral in single-ended mode on the given pin.
fn init_adc(adc_pin: AnyInput, adc: Peri<'static, SAADC>) -> Saadc<'static, 1> {
    // Then we initialize the ADC. We are only using one channel in this example.
    let config = saadc::Config::default();
    let channel_cfg = saadc::ChannelConfig::single_ended(adc_pin.degrade_saadc());
    interrupt::SAADC.set_priority(interrupt::Priority::P3);

    saadc::Saadc::new(adc, Irqs, config, [channel_cfg])
}

fn ble_addr() -> [u8; 6] {
    let ficr = embassy_nrf::pac::FICR;
    let high = u64::from(ficr.deviceid(1).read());
    let addr = high << 32 | u64::from(ficr.deviceid(0).read());
    let addr = addr | 0x0000_c000_0000_0000;
    unwrap!(addr.to_le_bytes()[..6].try_into())
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Hello RMK BLE!");
    // Initialize the peripherals and nrf-sdc controller
    let mut nrf_config = embassy_nrf::config::Config::default();
    nrf_config.dcdc.reg0_voltage = Some(embassy_nrf::config::Reg0Voltage::_3V3);
    nrf_config.dcdc.reg0 = true;
    nrf_config.dcdc.reg1 = true;
    let p = embassy_nrf::init(nrf_config);
    let mpsl_p = mpsl::Peripherals::new(p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31);
    let lfclk_cfg = mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: mpsl::raw::MPSL_CLOCK_LF_SRC_RC as u8,
        rc_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_CTIV as u8,
        rc_temp_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_TEMP_CTIV as u8,

        accuracy_ppm: 500,
        skip_wait_lfclk_started: mpsl::raw::MPSL_DEFAULT_SKIP_WAIT_LFCLK_STARTED != 0,
    };
    static MPSL: StaticCell<MultiprotocolServiceLayer> = StaticCell::new();
    static SESSION_MEM: StaticCell<mpsl::SessionMem<1>> = StaticCell::new();
    let mpsl = MPSL.init(unwrap!(mpsl::MultiprotocolServiceLayer::with_timeslots(
        mpsl_p,
        Irqs,
        lfclk_cfg,
        SESSION_MEM.init(mpsl::SessionMem::new())
    )));
    spawner.spawn(mpsl_task(&*mpsl).unwrap());
    let sdc_p = sdc::Peripherals::new(
        p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24, p.PPI_CH25, p.PPI_CH26,
        p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
    );
    let mut rng = rng::Rng::new(p.RNG, Irqs);
    let mut sdc_mem = sdc::Mem::<6080>::new();
    let sdc = unwrap!(build_sdc(sdc_p, &mut rng, mpsl, &mut sdc_mem));

    // Initialize usb driver
    let driver = Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));

    // Initialize flash
    let flash = Flash::take(mpsl, p.NVMC);

    // Initialize IO Pins
    let (row_pins, col_pins) = config_matrix_pins_nrf!(peripherals: p, input: [P0_24, P0_17, P0_16, P1_08, P0_31, P0_29], output:  [P0_13, P0_15, P0_19, P0_22, P0_20, P1_00, P0_28, P0_30]);

    // Initialize the ADC: one channel for battery level.
    let adc_pin = p.P0_03.degrade_saadc();
    let saadc = init_adc(adc_pin, p.SAADC);
    // Wait for ADC calibration.
    saadc.calibrate().await;

    // Keyboard config
    let keyboard_device_config = DeviceConfig {
        vid: 0x1313,
        pid: 0x1208,
        manufacturer: "ZT",
        product_name: "Keypoint",
        ..DeviceConfig::default()
    };
    let vial_config = VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF, &[(0, 0), (1, 1)]);
    let ble_battery_config = BleBatteryConfig::new(None, true, None, false);
    let storage_config = StorageConfig {
        start_addr: 0xA0000,
        num_sectors: 6,
        ..Default::default()
    };
    let rmk_config = RmkConfig {
        device_config: keyboard_device_config,
        vial_config,
        ble_battery_config,
        storage_config,
    };

    // Initialize the storage and keymap
    let mut keymap_data = KeymapData::new_with_encoder(keymap::get_default_keymap(), keymap::get_default_encoder_map());
    let mut behavior_config = BehaviorConfig::default();
    behavior_config.morse.enable_flow_tap = true;
    // Tapping term (rmk: morse hold_timeout). 75 ms keeps tap-hold snappy
    // without misfires (rmk default 250, ZMK 200). MorseProfile is a packed
    // bitfield, hence the builder. Caveat: Vial's Tapping Term writes only
    // reach memory (no flash save), so they last until reboot - this
    // constant is the real per-boot default.
    behavior_config.morse.default_profile = behavior_config.morse.default_profile.with_hold_timeout_ms(Some(75));
    // Consecutive-tap window: rmk default.

    // Auto mouse layer: pointer motion drops into layer 4 (keymap.rs "MOTION"),
    // silence drops back out. 4 must match that layer's index.
    //
    // Two entries rather than one `device_id: None` fallback: rmk picks the
    // exact-id entry first and the thresholds genuinely differ (a resting
    // thumb makes the nub twitch, a resting finger does not move the pad).
    behavior_config.auto_mouse_layer = {
        let mut entries = heapless::Vec::new();
        entries
            .push(AutoMouseLayerConfig {
                device_id: Some(PAD_ID),
                target_layer: 4, // mouse layer
                timeout: embassy_time::Duration::from_millis(500), // pad leaves the layer 500 ms after the last motion. A click counts as no motion, so a move-then-click must fit in this window.
                threshold: 1,
                deactivate_on_key: false,
                extra_mouse_keys: &[],
                reset_timeout_on_key: false,
            })
            .ok();
        entries
            .push(AutoMouseLayerConfig {
                device_id: Some(NUB_ID),
                target_layer: 4, // mouse layer
                timeout: embassy_time::Duration::from_millis(1000), // nub gets 1 s for the same move-then-click sequence
                threshold: 1, // one count of motion keeps the layer alive; a light nub push is only 1-2 counts
                deactivate_on_key: false,
                extra_mouse_keys: &[],
                reset_timeout_on_key: false,
            })
            .ok();
        entries
    };
    let key_config = PositionalConfig::default();
    let (keymap, mut storage) = initialize_keymap_and_storage(
        &mut keymap_data,
        flash,
        &storage_config,
        &mut behavior_config,
        &key_config,
    )
    .await;

    let pin_a = Input::new(p.P0_14, embassy_nrf::gpio::Pull::None);
    let pin_b = Input::new(p.P0_11, embassy_nrf::gpio::Pull::None);
    let mut encoder = RotaryEncoder::with_resolution(pin_a, pin_b, 4, false, 0);

    // Initialize the matrix and keyboard
    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, 6, 8, true>::new(row_pins, col_pins, debouncer);
    let mut keyboard = Keyboard::new(&keymap);
    let host_service = HostService::new(&keymap, &rmk_config);

    // Initialize the encoder processor
    let mut adc_device = NrfAdc::new(
        saadc,
        [AnalogEventType::Battery],
        [0],
        embassy_time::Duration::from_secs(12),
        None,
    );
    // Full-charge point calibrated to 4.15 V: the aged cell never reaches
    // 4.2, so val*2840/2000 >= 4755 reads 100%.
    let mut batt_proc = BatteryProcessor::new(2000, 2840);

    // Peripheral battery monitor controller
    use rmk::event::PeripheralBatteryEvent;
    use rmk::macros::processor;

    #[processor(subscribe = [PeripheralBatteryEvent, BatteryStatusEvent, LayerChangeEvent])]
    struct PeripheralBatteryMonitor {}

    impl PeripheralBatteryMonitor {
        async fn on_peripheral_battery_event(&mut self, event: PeripheralBatteryEvent) {
            info!("Peripheral {} battery status: {:?}", event.id, event);
        }
        async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
            info!("Central battery status: {:?}", event);
        }
        async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
            info!("Layer changed to: {}", event.0);
        }
    }

    let mut peripheral_battery_monitor = PeripheralBatteryMonitor {};

    let mut usb_transport = UsbTransport::new(driver, rmk_config.device_config).with_host_service(&host_service);
    // The other half: 6x8 at row offset 6 (keymap rows 6..12).
    let mut ble_transport = BleTransport::new(
        sdc,
        ble_addr(),
        rmk_config,
        [PeripheralMatrixConfig {
            rows: 6,
            cols: 8,
            row_offset: 6,
            col_offset: 0,
        }],
    )
    .with_host_service(&host_service);
    let mut wpm_processor = WpmProcessor::new();

    // ==================== pointing devices + status panel ====================

    // --- A320 trackpad: TWISPI0 on SDA P0.26 / SCL P0.04, MOTION on P0.08 ---
    // nRF TWIM drives EasyDMA from RAM only, so it needs a real write buffer;
    // a `&[0x82u8]` literal would sit in .rodata and silently fail.
    static TWI_TX: StaticCell<[u8; 32]> = StaticCell::new();
    let twi_tx = TWI_TX.init([0u8; 32]);
    // Config::default() is 100 kHz, ample for a 3-byte packet at ~40 Hz. ZMK ran
    // 400 kHz; to match, build a mutable config and set `frequency` instead.
    let a320_i2c = Twim::new(p.TWISPI0, Irqs, p.P0_26, p.P0_04, twim::Config::default(), twi_tx);
    // PollWait keeps this off the GPIOTE channel budget (the matrix and the
    // encoder are already spending channels). Swap for the Input directly if you
    // want interrupt-driven: `Input::new(p.P0_08, embassy_nrf::gpio::Pull::Up)`.
    let a320_motion = PollWait::new(Input::new(p.P0_08, embassy_nrf::gpio::Pull::Up));
    let mut a320 = A320::new(0, a320_i2c, a320_motion);

    // The pad moves the cursor, like a laptop touchpad.
    // Its driver already applies ZMK's scaling and float residue, so
    // multiplier 1 here is the identity.
    let mut pad_processor = PointingProcessor::new(
        &keymap,
        PointingProcessorConfig {
            device_id: 0,
            ..Default::default()
        },
    );
    // Boots from pointer_speed so the first knob notch continues from the stored
    // value instead of jumping. `cursor_mode` also carries the per-device invert
    // flags, which is where the pad's inverted X now lives.
    pad_processor.set_pointing_mode(cursor_mode(PAD_ID));

    // --- TrackPoint processor. The device itself lives in peripheral.rs; this
    // has to be here, because the right half's PointingEvents cross the split
    // link with their device id intact and only a central-side processor may
    // turn them into HID mouse reports.
    //
    // The binding stays mut because run_all! takes each task by &mut.
    let mut tp_processor = PointingProcessor::new(
        &keymap,
        PointingProcessorConfig {
            device_id: TRACKPOINT_ID,
            ..Default::default()
        },
    );
    // Set the initial mode from the tier table. The nub needs this: its raw
    // displacement is only a few counts per packet, so a bare multiplier of 1
    // leaves it barely visible, while the pad already gets an explicit mode.
    tp_processor.set_pointing_mode(cursor_mode(NUB_ID));

    // Enters the mouse layer on pointer motion and times back out when idle. The
    // scroll keys arm on that layer, so they follow it.
    let mut auto_mouse = AutoMouseLayerRunner::new(&keymap);
    let mut scroll_controller = ScrollKeyController::new(&keymap);

    // Reads the four keymap tier cells and writes the processor multipliers and
    // divisors, where scaling actually happens.
    let mut speed_controller = SpeedController::new(&keymap);

    // --- Left panel: SPI2 on SCK P0.27 / MOSI P0.05, active-high CS on P0.12 ---
    static SCREEN_FB: StaticCell<[u8; lpm009m360a::FRAMEBUFFER_LEN]> = StaticCell::new();
    let fb = SCREEN_FB.init([0u8; lpm009m360a::FRAMEBUFFER_LEN]);
    let mut spi_cfg = spim::Config::default(); // mode 0 / MSB first: what the panel wants
    spi_cfg.frequency = spim::Frequency::M4;   // ZMK ran 4 MHz
    // new_txonly exists because this panel has no MISO line at all.
    let screen_spi = Spim::new_txonly(p.SPI2, Irqs, p.P0_27, p.P0_05, spi_cfg);
    // CS is ACTIVE_HIGH, so the idle level is low.
    let screen_cs = Output::new(
        p.P0_12,
        embassy_nrf::gpio::Level::Low,
        embassy_nrf::gpio::OutputDrive::Standard,
    );
    // Left-half panel orientation. R0 = the ZMK mapping verbatim (144x72
    // landscape); R90/R270 give the 72x144 portrait canvas. Pick per half --
    // the two panels on a split board are frequently mounted mirrored.
    let screen = Lpm009m360a::new(screen_spi, screen_cs, fb, PanelRot::R270);
    // A render costs ~10k pixel writes plus a 1.6 kB transfer and the
    // processor re-renders on every key event, so throttle past the default
    // 33 ms. (rmk's native OledRenderer targets 128x64 OLEDs and reads too
    // small on this panel.)
    let mut display = DisplayProcessor::with_renderer(screen, LeftScreen)
        .with_min_render_interval(embassy_time::Duration::from_millis(150));

    // --- Status LEDs. Half indicator on P0.07 (PWM0); link_state_task feeds
    // it from connection events. P1.09 and P0.10 intentionally drive
    // nothing. ---
    let half_pwm = embassy_nrf::pwm::SimplePwm::new_1ch(p.PWM0, p.P0_07, &status_led::pwm_config());
    spawner.spawn(status_led::custom_led_task(status_led::StatusLed::new(half_pwm)).unwrap());
    spawner.spawn(status_led::link_state_task().unwrap());
    spawner.spawn(usb_diag::usb_diag_run().unwrap());
    spawner.spawn(capy_tick::capy_tick_run().unwrap());
    spawner.spawn(sleep_watch::sleep_watch_run().unwrap());

    let mut watchdog_runner = Nrf52Watchdog::default_runner(p.WDT);

    // Start
    run_all!(
        matrix,
        encoder,
        adc_device,
        storage,
        usb_transport,
        ble_transport,
        wpm_processor,
        batt_proc,
        keyboard,
        // capslock_led,
        peripheral_battery_monitor,
        // Pointing: devices first, then their processors, so nothing is dropped
        // waiting for a subscriber that has not started yet.
        a320,
        pad_processor,
        tp_processor,
        scroll_controller,
        auto_mouse,
        speed_controller,
        display,
        watchdog_runner
    )
    .await;
}
