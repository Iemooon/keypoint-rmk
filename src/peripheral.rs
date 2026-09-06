#![no_std]
#![no_main]

#[macro_use]
mod macros;
mod lpm009m360a;
mod motion_pin;
/// Only so the shared renderers.rs compiles: these values are written and
/// read on the central, this copy stays at its boot default.
#[allow(dead_code)]
mod pointer_speed;
mod renderers;
mod status_led;
/// Only so the shared renderers.rs compiles: the pad lives on the central,
/// these counters stay at zero on this half.
#[allow(dead_code)]
mod trackpad;
/// Only so the shared renderers.rs compiles: the transport tape is written by
/// the central's subscriber task, this copy stays empty.
#[allow(dead_code)]
mod usb_diag;
mod capy_art;
mod capy_tick;
mod sleep_watch;
mod trackpoint;
mod tp_diag;

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Input, Output};
use embassy_nrf::interrupt::{self, InterruptExt};
use embassy_nrf::mode::Async;
use embassy_nrf::peripherals::{RNG, SAADC, USBD};
use embassy_nrf::saadc::{self, AnyInput, Input as _, Saadc};
use embassy_nrf::{Peri, bind_interrupts, rng, usb};
use nrf_mpsl::Flash;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, mpsl};
use panic_probe as _;
use rmk::config::StorageConfig;
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::futures::future::join3;
use rmk::input_device::adc::{AnalogEventType, NrfAdc};
use rmk::input_device::battery::BatteryProcessor;
use rmk::input_device::rotary_encoder::RotaryEncoder;
use rmk::matrix::Matrix;
use rmk::run_all;
use rmk::split::peripheral::run_rmk_split_peripheral;
use rmk::storage::new_storage_without_keymap;
use rmk::watchdog::Nrf52Watchdog;
use static_cell::StaticCell;
use embassy_nrf::spim::{self, Spim};
use embassy_nrf::twim::{self, Twim};
use rmk::display::DisplayProcessor;
use lpm009m360a::{Lpm009m360a, PanelRot};
use motion_pin::PollWait;
use renderers::RightScreen;
use trackpoint::TrackPoint;

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
        .support_adv()
        .support_peripheral()
        .support_dle_peripheral()
        .support_phy_update_peripheral()
        .support_le_2m_phy()
        .peripheral_count(1)?
        .buffer_cfg(L2CAP_MTU as u16, L2CAP_MTU as u16, L2CAP_TXQ, L2CAP_RXQ)?
        .build(p, rng, mpsl, mem)
}

/// Initializes the SAADC peripheral in single-ended mode on the given pin.
fn init_adc(adc_pin: AnyInput, adc: Peri<'static, SAADC>) -> Saadc<'static, 1> {
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
    let mut sdc_mem = sdc::Mem::<4696>::new();
    let sdc = unwrap!(build_sdc(sdc_p, &mut rng, mpsl, &mut sdc_mem));

    // Initialize the ADC: one channel for battery level.
    let adc_pin = p.P0_02.degrade_saadc();
    let saadc = init_adc(adc_pin, p.SAADC);
    // Wait for ADC calibration.
    saadc.calibrate().await;

    let (row_pins, col_pins) = config_matrix_pins_nrf!(peripherals: p, input: [P1_10, P1_15, P0_28, P0_30, P0_12, P1_09], output:  [P0_03, P1_14, P0_31, P0_29, P1_13, P1_11, P0_05, P0_27]);

    // Initialize flash
    // nRF52840's bootloader starts from 0xF4000(976K)
    let storage_config = StorageConfig {
        start_addr: 0xA0000, // 640K
        num_sectors: 32,     // 128K
        ..Default::default()
    };
    let flash = Flash::take(mpsl, p.NVMC);
    let mut storage = new_storage_without_keymap(flash, storage_config).await;

    // Initialize the peripheral matrix
    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, 6, 8, true>::new(row_pins, col_pins, debouncer);

    let pin_a = Input::new(p.P0_26, embassy_nrf::gpio::Pull::None);
    let pin_b = Input::new(p.P0_04, embassy_nrf::gpio::Pull::None);
    let mut encoder = RotaryEncoder::with_resolution(pin_a, pin_b, 4, true, 1);

    // Battery monitoring for peripheral
    // 1. Initialize ADC device:
    let mut adc_device = NrfAdc::new(
        saadc,
        [AnalogEventType::Battery],
        [0],
        embassy_time::Duration::from_secs(12),
        None,
    );
    // Full-charge point calibrated to 4.15 V (matches central; see there).
    let mut battery_processor = BatteryProcessor::new(2000, 2840);

    // ==================== TrackPoint + right status panel ====================

    // --- TrackPoint: TWISPI0 on SDA P0.14 / SCL P1.08, MOTION on P0.07 ---
    // nRF TWIM drives EasyDMA from RAM only, so it needs a real write buffer.
    static TWI_TX: StaticCell<[u8; 32]> = StaticCell::new();
    let twi_tx = TWI_TX.init([0u8; 32]);
    let mut i2c_cfg = twim::Config::default();
    // 400 kHz, matching the ZMK overlay (`clock-frequency = <400000>`). This is not
    // just cosmetics: the device is a PS/2-to-I2C bridge, and a slower bus widens
    // the window in which the stick can overwrite the packet we're fetching.
    i2c_cfg.frequency = twim::Frequency::K400;
    let tp_i2c = Twim::new(p.TWISPI0, Irqs, p.P0_14, p.P1_08, i2c_cfg, twi_tx);
    // Polled, not interrupt-driven: see motion_pin.rs for why the GPIOTE budget
    // is the first thing to run out once the matrix and encoder are up.
    let tp_motion = PollWait::new(Input::new(p.P0_07, embassy_nrf::gpio::Pull::Up));
    let mut trackpoint = TrackPoint::new(trackpoint::DEVICE_ID, tp_i2c, tp_motion);
    // No PointingProcessor here on purpose - the central owns cursor/scroll
    // translation, and rmk carries these events across the split link.

    // --- Right panel: SPI2 on SCK P0.15 / MOSI P0.17, active-high CS on P0.13 ---
    static SCREEN_FB: StaticCell<[u8; lpm009m360a::FRAMEBUFFER_LEN]> = StaticCell::new();
    let fb = SCREEN_FB.init([0u8; lpm009m360a::FRAMEBUFFER_LEN]);
    let mut spi_cfg = spim::Config::default(); // mode 0 / MSB first
    spi_cfg.frequency = spim::Frequency::M4;
    let screen_spi = Spim::new_txonly(p.SPI2, Irqs, p.P0_15, p.P0_17, spi_cfg);
    let screen_cs = Output::new(
        p.P0_13,
        embassy_nrf::gpio::Level::Low,
        embassy_nrf::gpio::OutputDrive::Standard,
    );
    // Right-half panel orientation -- independent of the left on purpose.
    let screen = Lpm009m360a::new(screen_spi, screen_cs, fb, PanelRot::R270);
    // Layer / WPM / LED state reaches this panel because rmk forwards it over
    // the split link (SplitMessage::Layer / Wpm / Modifier) - requires the
    // `display` feature on BOTH halves (Cargo.toml guarantees it).
    let mut display = DisplayProcessor::with_renderer(screen, RightScreen { snap: None })
        .with_min_render_interval(embassy_time::Duration::from_millis(150));

    // --- Right status LED on P0.06 (ZMK's custom_led pin). Meaning: blink
    // until the central is attached, dark once linked; host state is the left
    // LED's job. P0.06 carries no panel signal (panel is P0.15/P0.17). ---
    let half_pwm = embassy_nrf::pwm::SimplePwm::new_1ch(p.PWM0, p.P0_06, &status_led::pwm_config());
    spawner.spawn(status_led::custom_led_task(status_led::StatusLed::new(half_pwm)).unwrap());
    spawner.spawn(status_led::central_link_task().unwrap());
    // The central's ConnectionStatus is mirrored over the split link and
    // every mirror fires a ConnectionStatusChangeEvent, so usb_diag here
    // tracks the *central's* USB leg - RightScreen shows the wired badge in
    // sync with the left panel. Our own USB stack never runs.
    spawner.spawn(usb_diag::usb_diag_run().unwrap());
    spawner.spawn(capy_tick::capy_tick_run().unwrap());
    spawner.spawn(sleep_watch::sleep_watch_run().unwrap());

    let mut watchdog_runner = Nrf52Watchdog::default_runner(p.WDT);

    // Start
    join3(
        run_all!(
            matrix,
            encoder,
            adc_device,
            storage,
            trackpoint,
            display,
            watchdog_runner
        ),
        run_all!(battery_processor),
        run_rmk_split_peripheral(0, sdc, ble_addr()),
    )
    .await;
}
