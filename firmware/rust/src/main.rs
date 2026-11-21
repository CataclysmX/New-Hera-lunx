//! Hercules Stream 100 custom firmware – stage 2: free screen.
//!
//! * DFU escape hatches (see `escape`): knob 1 + knob 4 clicks at power-up / held ~1 s, or the
//!   official vendor request `0x40 0xF2 0 0xE053`.
//! * Display command stream on bulk EP 0x01 (FILL / BLIT / BLIT_RLE / BACKLIGHT / LEDS) into a
//!   480×272 RGB565 framebuffer scanned out by the LTDC.
//! * Input report on interrupt EP 0x81 every 10 ms: buttons + raw ADC of the 8 knob wipers.
//! * Backlight PWM TIM2 CH1 (PA5) + enable PB10; LED PWM TIM8 CH1..4 (PC6..PC9).
//!
//! Protocol: docs/firmware/protocol.md – hardware: docs/firmware/hardware.md.

#![no_std]
#![no_main]

mod escape;
mod gfx;
mod proto;

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use cortex_m_rt::entry;
use embassy_executor::Executor;
use embassy_stm32::adc::{Adc, AdcChannel, AnyAdcChannel, SampleTime};
use embassy_stm32::gpio::{Input, Level, Output, OutputType, Pull, Speed};
use embassy_stm32::ltdc::{
    Ltdc, LtdcConfiguration, LtdcLayer, LtdcLayerConfig, PixelFormat, PolarityActive, PolarityEdge,
};
use embassy_stm32::pac;
use embassy_stm32::pac::gpio::{Gpio, vals as gv};
use embassy_stm32::peripherals::{ADC1, USB_OTG_FS};
use embassy_stm32::time::khz;
use embassy_stm32::timer::Channel;
use embassy_stm32::timer::low_level::CountingMode;
use embassy_stm32::timer::simple_pwm::{PwmPin, SimplePwm};
use embassy_stm32::{bind_interrupts, usb};
use embassy_time::{Duration, Instant, Ticker, Timer};
use embassy_usb::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use embassy_usb::driver::{Endpoint as _, EndpointIn as _, EndpointOut as _};
use embassy_usb::{Builder, Handler, UsbDevice};
use panic_halt as _;
use static_cell::StaticCell;

/// Vendor request 0x02 answer `[minor, major]`: major 0xF0 (240) = the custom firmware (the
/// official one answers 1.xx), minor = its version (0x16 → "240.22").
const FW_VERSION: [u8; 2] = [0x16, 0xF0];
/// USB bcdDevice, following the version: 0xF0 then the minor in BCD (240.22 → 0xF022).
const DEVICE_RELEASE: u16 =
    0xF000 | ((FW_VERSION[0] / 10) as u16) << 4 | (FW_VERSION[0] % 10) as u16;
const _: () = assert!(FW_VERSION[0] < 100, "the minor must fit in two BCD digits");

bind_interrupts!(struct Irqs {
    OTG_FS => usb::InterruptHandler<USB_OTG_FS>;
});

type UsbDriver = usb::Driver<'static, USB_OTG_FS>;
type EpOut = <UsbDriver as embassy_usb::driver::Driver<'static>>::EndpointOut;
type EpIn = <UsbDriver as embassy_usb::driver::Driver<'static>>::EndpointIn;

static DFU_PENDING: AtomicBool = AtomicBool::new(false);
/// Backlight and LEDs (%) as the host last set them (restored when it takes the screen back).
static BACKLIGHT_PCT: AtomicU8 = AtomicU8::new(80);
static HOST_LEDS: [AtomicU8; 4] = [const { AtomicU8::new(0) }; 4];
/// The screen is the host's: set by its drawing data, cleared by `screen_task` once the host is
/// gone (boot logo, screen saver, off otherwise).
static HOST_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Last sign of life of the host (`now_ms`): drawing data received, or an input report fetched.
static HOST_SEEN_MS: AtomicU32 = AtomicU32::new(0);
/// USB bus suspended since (`now_ms`, made odd so never 0), 0 = not suspended.
static SUSPENDED_SINCE: AtomicU32 = AtomicU32::new(0);
/// +1 each time the firmware takes the screen back from the host (input report byte 21): the
/// host redraws everything when it changes.
static SCREEN_EPOCH: AtomicU8 = AtomicU8::new(0);
/// Without host drawing data, the boot logo stays this many seconds, then the screen saver.
const LOGO_SECS: u64 = 10;
/// Host gone (app closed, killed or crashed) after this long without drawing data nor fetched
/// input report.
const HOST_GONE_MS: u32 = 3_000;
/// Without host: the logo bounces around at this backlight (nothing static: no imprint, the
/// panel imprints within minutes – docs/firmware/stage2.md), then the screen goes off.
const SAVER_BACKLIGHT_PCT: u8 = 1;
const SAVER_FPS: u64 = 30;
const IDLE_OFF_SECS: u64 = 300;
/// USB suspended this long = PC asleep: backlight and LEDs off until it resumes (debounces the
/// suspend seen before enumeration).
const SUSPEND_MS: u32 = 500;
static EXECUTOR: StaticCell<Executor> = StaticCell::new();

#[entry]
fn main() -> ! {
    // SAFETY: first instruction path, reset state.
    unsafe { escape::check_at_boot() };

    let mut config = embassy_stm32::Config::default();
    {
        use embassy_stm32::rcc::*;
        // No LSE: the official firmware never starts it (its MSI PLL-mode call is a no-op) and
        // waiting for LSERDY hangs this board (240.05 test build). USB clock: HSI48 + CRS.
        config.rcc.msi = Some(MSIRange::RANGE4M);
        let pll = Pll {
            source: PllSource::MSI,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL40,
            divp: None,
            divq: None,
            divr: Some(PllRDiv::DIV2),
        };
        config.rcc.pll = Some(pll); // 80 MHz
        config.rcc.pllsai2 = Some(pll); // LTDC kernel clock, ÷8 below
        config.rcc.sys = Sysclk::PLL1_R;
        config.rcc.hsi48 = Some(Hsi48Config { sync_from_usb: true });
        config.rcc.mux.clk48sel = mux::Clk48sel::HSI48;
        // ADCSEL resets to "no clock": Adc::new() would spin forever on ADCAL.
        config.rcc.mux.adcsel = mux::Adcsel::SYS;
    }
    let p = embassy_stm32::init(config);

    // ---- display -------------------------------------------------------------------------
    // 10 MHz pixel clock: PLLSAI2R 80 MHz / 8. PLLSAI2DIVR is only writable while PLLSAI2 is off
    // (embassy has already started it): a plain write is ignored and leaves /2 = 40 MHz, i.e. a
    // 252 Hz refresh that made the panel imprint within minutes (measured with tools/fw_regs.py).
    pac::RCC.cr().modify(|w| w.set_pllsai2on(false));
    while pac::RCC.cr().read().pllsai2rdy() {}
    pac::RCC.ccipr2().modify(|w| w.set_ltdcdiv(2));
    pac::RCC.cr().modify(|w| w.set_pllsai2on(true));
    while !pac::RCC.cr().read().pllsai2rdy() {}
    ltdc_pins(pac::GPIOE, 0xffff);
    ltdc_pins(pac::GPIOB, 1 << 2);
    ltdc_pins(pac::GPIOD, 0xdf4b);
    let mut ltdc = Ltdc::new(p.LTDC);
    ltdc.init(&LtdcConfiguration {
        active_width: gfx::W as u16,
        active_height: gfx::H as u16,
        h_back_porch: 43,
        h_front_porch: 8,
        v_back_porch: 12,
        v_front_porch: 4,
        h_sync: 1,
        v_sync: 10,
        h_sync_polarity: PolarityActive::ActiveLow,
        v_sync_polarity: PolarityActive::ActiveLow,
        data_enable_polarity: PolarityActive::ActiveLow,
        pixel_clock_polarity: PolarityEdge::RisingEdge,
    });
    ltdc.init_layer(
        &LtdcLayerConfig {
            layer: LtdcLayer::Layer1,
            pixel_format: PixelFormat::RGB565,
            window_x0: 0,
            window_x1: gfx::W as u16,
            window_y0: 0,
            window_y1: gfx::H as u16,
        },
        None,
    );
    // framebuffer starts black (.bss): boot_task animates the logo once the executor runs
    // embassy 0.6 sets CFBLL = line bytes + 7; the L4+ LTDC wants + 3 (official firmware,
    // LTDC_SetConfig). With + 7 every line fetches 2 extra pixels that spill into the next line:
    // the picture shears by 2 px per line and wraps (seen on the device, 240.06).
    pac::LTDC.layer(0).cfblr().write(|w| {
        w.set_cfbp((gfx::W * 2) as u16);
        w.set_cfbll((gfx::W * 2 + 3) as u16);
    });
    pac::LTDC.layer(0).cfbar().write(|w| w.set_cfbadd(gfx::fb_addr()));
    pac::LTDC.layer(0).cr().modify(|w| w.set_len(true));
    pac::LTDC.srcr().write(|w| w.set_imr(pac::ltdc::vals::Imr::RELOAD));
    core::mem::forget(ltdc);
    core::mem::forget(Output::new(p.PA8, Level::High, Speed::Low)); // panel enable
    core::mem::forget(Output::new(p.PB10, Level::High, Speed::Low)); // backlight enable

    // ---- PWM: backlight (TIM2 CH1 / PA5) and LEDs (TIM8 CH1..4 / PC6..PC9) -----------------
    let mut bl = SimplePwm::new(
        p.TIM2,
        Some(PwmPin::new(p.PA5, OutputType::PushPull)),
        // PB3 = TIM2 CH2 exactly as the official firmware (MX_TIM2_Init: PWM1, pulse 0, channel
        // never started). Function unknown (panel-side?); left as JTDO it floats. Do not enable.
        Some(PwmPin::new(p.PB3, OutputType::PushPull)),
        None,
        None,
        khz(240),
        CountingMode::EdgeAlignedUp,
    );
    bl.ch1().set_duty_cycle_percent(BACKLIGHT_PCT.load(Ordering::Relaxed));
    bl.ch1().enable();
    let mut leds = SimplePwm::new(
        p.TIM8,
        Some(PwmPin::new(p.PC6, OutputType::PushPull)),
        Some(PwmPin::new(p.PC7, OutputType::PushPull)),
        Some(PwmPin::new(p.PC8, OutputType::PushPull)),
        Some(PwmPin::new(p.PC9, OutputType::PushPull)),
        khz(240),
        CountingMode::EdgeAlignedUp,
    );
    for ch in [Channel::Ch1, Channel::Ch2, Channel::Ch3, Channel::Ch4] {
        leds.channel(ch).set_duty_cycle_percent(0);
        leds.channel(ch).enable();
    }
    // set up and running: from now on duties are written straight to the compare registers
    // (`set_backlight`, `set_leds`) by the receive and screen tasks
    core::mem::forget(bl);
    core::mem::forget(leds);

    // ---- inputs ----------------------------------------------------------------------------
    // bit order = report bits: knob clicks 1..4, buttons 1..4, power.
    let buttons = [
        Input::new(p.PC12, Pull::Up),
        Input::new(p.PC11, Pull::Up),
        Input::new(p.PC10, Pull::Up),
        Input::new(p.PA15, Pull::Up),
        Input::new(p.PB15, Pull::Up),
        Input::new(p.PB14, Pull::Up),
        Input::new(p.PB13, Pull::Up),
        Input::new(p.PB12, Pull::Up),
        Input::new(p.PB11, Pull::Up),
    ];
    let adc = Adc::new(p.ADC1);
    let wipers = [
        p.PC0.degrade_adc(),
        p.PC1.degrade_adc(),
        p.PC2.degrade_adc(),
        p.PC3.degrade_adc(),
        p.PA0.degrade_adc(),
        p.PA1.degrade_adc(),
        p.PA2.degrade_adc(),
        p.PC4.degrade_adc(),
    ];

    // ---- USB -------------------------------------------------------------------------------
    static EP_OUT_BUF: StaticCell<[u8; 1088]> = StaticCell::new();
    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 64]> = StaticCell::new();
    static MSOS_DESC: StaticCell<[u8; 64]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static HANDLER: StaticCell<VendorHandler> = StaticCell::new();

    let mut otg = usb::Config::default();
    otg.vbus_detection = false;
    let driver = usb::Driver::new_fs(p.USB_OTG_FS, Irqs, p.PA12, p.PA11, EP_OUT_BUF.init([0; 1088]), otg);
    let mut uc = embassy_usb::Config::new(0x06f8, 0xe053);
    uc.manufacturer = Some("Hercules");
    uc.product = Some("Stream 100 (hsm01-fw)");
    uc.device_release = DEVICE_RELEASE;
    uc.max_power = 500;
    let mut builder = Builder::new(
        driver,
        uc,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 64]),
        MSOS_DESC.init([0; 64]),
        CONTROL_BUF.init([0; 64]),
    );
    builder.handler(HANDLER.init(VendorHandler));
    let (ep_out, ep_in) = {
        let mut func = builder.function(0xff, 0, 0);
        let mut iface = func.interface();
        let mut alt = iface.alt_setting(0xff, 0, 0, None);
        (alt.endpoint_bulk_out(None, 64), alt.endpoint_interrupt_in(None, 64, 2))
    };
    let usb = builder.build();

    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(usb_task(usb).unwrap());
        spawner.spawn(rx_task(ep_out, Hw { work: 0 }).unwrap());
        spawner.spawn(input_task(ep_in, adc, wipers, buttons).unwrap());
        spawner.spawn(guard_task().unwrap());
        spawner.spawn(screen_task().unwrap());
    })
}

#[embassy_executor::task]
async fn usb_task(mut usb: UsbDevice<'static, UsbDriver>) {
    usb.run().await;
}

/// Hardware driven by the command stream.
struct Hw {
    /// Pixels written since the receive task last yielded.
    work: u32,
}

fn now_ms() -> u32 {
    Instant::now().as_millis() as u32 // wraps after 49 days: compared with wrapping_sub
}

fn host_seen() {
    HOST_SEEN_MS.store(now_ms(), Ordering::Release);
}

/// Duty `pct` % on channel `ch` (0-3) of TIM2 or TIM8 (set up by SimplePwm in `main`; same
/// formula as its `set_duty_cycle_percent`).
fn set_duty(tim: *mut (), ch: usize, pct: u8) {
    // SAFETY: TIM2 / TIM8 registers seen as a 32-bit timer, as embassy does; thread mode only.
    let t = unsafe { pac::timer::TimGp32::from_ptr(tim) };
    let max = t.arr().read() + 1;
    t.ccr(ch).write_value(max * u32::from(pct.min(100)) / 100);
}

fn set_backlight(pct: u8) {
    set_duty(pac::TIM2.as_ptr(), 0, pct);
}

fn set_leds(pct: [u8; 4]) {
    for (ch, v) in pct.into_iter().enumerate() {
        set_duty(pac::TIM8.as_ptr(), ch, v);
    }
}

/// Backlight and LEDs as the host last set them.
fn restore_host_outputs() {
    set_backlight(BACKLIGHT_PCT.load(Ordering::Relaxed));
    set_leds(core::array::from_fn(|i| HOST_LEDS[i].load(Ordering::Relaxed)));
}

/// Pixels drawn between two yields of the receive task (~1.5 ms at 11 Mpx/s): the input task
/// keeps its 10 ms rhythm whatever the host sends.
const WORK_PER_YIELD: u32 = 16_000;
/// Bytes handed to the parser at once: a 512-byte read can hold ~28 full-screen FILLs (300 ms of
/// drawing); 32 bytes complete at most 2 FILL messages (18 bytes each) between two budget checks.
const FEED_SLICE: usize = 32;

impl proto::Sink for Hw {
    fn fill(&mut self, x: u16, y: u16, w: u16, h: u16, color: u16) {
        gfx::fill(x, y, w, h, color);
        self.work += w as u32 * h as u32;
    }
    fn pixel(&mut self, x: usize, y: usize, color: u16) {
        gfx::put(x, y, color);
        self.work += 1;
    }
    fn span(&mut self, x: usize, y: usize, px: &[u8]) {
        gfx::span(x, y, px);
        self.work += px.len() as u32 / 2;
    }
    fn hline(&mut self, x: usize, y: usize, n: usize, color: u16) {
        gfx::hline(x, y, n, color);
        self.work += n as u32;
    }
    fn copy(&mut self, x: u16, y: u16, w: u16, h: u16, to_x: u16, to_y: u16) {
        self.work += gfx::copy(x, y, w, h, to_x, to_y);
    }
    fn backlight(&mut self, pct: u8) {
        let pct = pct.min(100);
        set_backlight(pct);
        BACKLIGHT_PCT.store(pct, Ordering::Relaxed);
    }
    fn leds(&mut self, pct: [u8; 4]) {
        for (stored, v) in HOST_LEDS.iter().zip(pct) {
            stored.store(v.min(100), Ordering::Relaxed);
        }
        set_leds(pct);
    }
}

/// Display command stream (bulk OUT).
#[embassy_executor::task]
async fn rx_task(mut ep: EpOut, mut hw: Hw) {
    let mut parser = proto::Parser::new();
    let mut buf = [0u8; 512];
    loop {
        ep.wait_enabled().await;
        while let Ok(n) = ep.read(&mut buf).await {
            host_seen();
            // the host takes the screen back (logo, saver, off): its backlight and LEDs again;
            // `screen_task` sees the flag before drawing anything else
            if !HOST_ACTIVE.swap(true, Ordering::AcqRel) {
                restore_host_outputs();
            }
            // While the host keeps sending, `read` is ready at once and never yields: without the
            // yields below the input task (buttons, knobs) did not run at all under load
            // (240.17: 0 report/s). Yield after each read and whenever enough pixels were drawn.
            for slice in buf[..n].chunks(FEED_SLICE) {
                parser.feed(slice, &mut hw);
                if hw.work >= WORK_PER_YIELD {
                    hw.work = 0;
                    embassy_futures::yield_now().await;
                }
            }
            embassy_futures::yield_now().await;
        }
        parser = proto::Parser::new(); // host went away: drop any partial message
    }
}

/// Input report (interrupt IN) every 10 ms while the host listens; a report fetched is a sign of
/// life of the host (`HOST_SEEN_MS`).
#[embassy_executor::task]
async fn input_task(
    mut ep: EpIn,
    mut adc: Adc<'static, ADC1>,
    mut wipers: [AnyAdcChannel<'static, ADC1>; 8],
    buttons: [Input<'static>; 9],
) {
    let mut seq = 0u8;
    loop {
        ep.wait_enabled().await;
        // a report waits in the endpoint: the next `write` returning means the host fetched it
        let mut queued = false;
        loop {
            let mut r = [0u8; 24];
            r[0] = 0x01;
            r[1] = seq;
            seq = seq.wrapping_add(1);
            let mut bits = 0u16;
            for (i, b) in buttons.iter().enumerate() {
                if b.is_low() {
                    bits |= 1 << i;
                }
            }
            r[2..4].copy_from_slice(&bits.to_le_bytes());
            for (i, w) in wipers.iter_mut().enumerate() {
                // 3 µs sampling: potentiometer wipers are high-impedance sources.
                let v = adc.blocking_read(w, SampleTime::CYCLES247_5);
                r[4 + 2 * i..6 + 2 * i].copy_from_slice(&v.to_le_bytes());
            }
            r[20] = BACKLIGHT_PCT.load(Ordering::Relaxed);
            r[21] = SCREEN_EPOCH.load(Ordering::Relaxed);
            // `write` first waits for the previous report to be fetched. If it never is (host not
            // reading, or a transfer the controller lost: 240.19 stopped reporting for good, even
            // across USB resets), drop the queued report and start again from a clean endpoint.
            match embassy_time::with_timeout(Duration::from_millis(250), ep.write(&r)).await {
                Ok(Ok(())) => {
                    if queued {
                        host_seen();
                    }
                    queued = true;
                }
                Ok(Err(_)) => break, // endpoint disabled (reset, reconfiguration)
                Err(_) => {
                    ep.abort(); // the next `write` returns at once: proves nothing
                    queued = false;
                }
            }
            Timer::after_millis(10).await;
        }
    }
}

/// Boot screen: stroke-drawing animation of the logo at 60 fps (precomputed by build.rs, the pen
/// tip in white), halo fade-in, final logo for LOGO_SECS. Returns as soon as the host draws.
async fn boot_logo() {
    let host = || HOST_ACTIVE.load(Ordering::Acquire);
    #[cfg(bootlogo)]
    {
        const GLOW_STEPS: u32 = 45; // light-up: trace green → neon colours, halo fades in
        let mut ticker = Ticker::every(Duration::from_hz(60));
        let (mut pos, mut prev) = (gfx::ANIM_START, None);
        for _ in 0..gfx::anim_frames() {
            if host() {
                return;
            }
            if let Some(p) = prev {
                gfx::anim_draw(p, gfx::TRACE); // last frame's pen tip → trace green
            }
            prev = Some(pos);
            pos = gfx::anim_draw(pos, gfx::PEN_TIP);
            // a late Ticker is ready at once and would never let the USB / input tasks run
            embassy_futures::yield_now().await;
            ticker.next().await;
        }
        if let Some(p) = prev {
            gfx::anim_draw(p, gfx::TRACE);
        }
        for k in 1..=GLOW_STEPS {
            if host() {
                return;
            }
            gfx::glow(k, GLOW_STEPS);
            embassy_futures::yield_now().await;
            ticker.next().await;
        }
    }
    if host() {
        return;
    }
    gfx::boot_screen();
    let end = Instant::now() + Duration::from_secs(LOGO_SECS);
    while Instant::now() < end && !host() {
        Timer::after_millis(50).await;
    }
}

/// What the screen shows besides the boot logo.
#[derive(Clone, Copy)]
enum Screen {
    /// The host's pictures.
    Host,
    /// No host: the logo bounces around, since then.
    Saver(Instant),
    /// No host for IDLE_OFF_SECS: black, backlight off.
    Off,
}

/// Screen without its host: boot logo, then the logo bouncing around at SAVER_BACKLIGHT_PCT
/// (moving: nothing to imprint), off after IDLE_OFF_SECS. The host's drawing data takes the
/// screen at once (`rx_task`); its absence for HOST_GONE_MS (app closed, killed, crashed) gives
/// it back, the host is told by SCREEN_EPOCH. A suspended USB bus (PC asleep) switches the
/// backlight and the LEDs off until it resumes.
#[embassy_executor::task]
async fn screen_task() {
    boot_logo().await;
    let mut ticker = Ticker::every(Duration::from_hz(SAVER_FPS));
    let mut saver = Saver::new();
    let mut state = if HOST_ACTIVE.load(Ordering::Acquire) { Screen::Host } else { saver.start() };
    let mut asleep = false;
    loop {
        ticker.next().await;
        let now = now_ms();
        let since = SUSPENDED_SINCE.load(Ordering::Acquire);
        if since != 0 && now.wrapping_sub(since) >= SUSPEND_MS {
            if !asleep {
                asleep = true;
                set_backlight(0);
                set_leds([0; 4]);
            }
            continue;
        }
        if asleep {
            // PC awake again: its host gets HOST_GONE_MS to show up, or a new saver round
            asleep = false;
            HOST_SEEN_MS.store(now, Ordering::Release);
            if HOST_ACTIVE.load(Ordering::Acquire) {
                restore_host_outputs();
            } else {
                state = saver.start();
            }
        }
        if HOST_ACTIVE.load(Ordering::Acquire) {
            state = Screen::Host;
            if now.wrapping_sub(HOST_SEEN_MS.load(Ordering::Acquire)) > HOST_GONE_MS {
                HOST_ACTIVE.store(false, Ordering::Release);
                SCREEN_EPOCH.fetch_add(1, Ordering::Relaxed);
                state = saver.start();
            }
            continue;
        }
        state = match state {
            Screen::Saver(t) if t.elapsed() >= Duration::from_secs(IDLE_OFF_SECS) => {
                gfx::fill(0, 0, gfx::W as u16, gfx::H as u16, 0);
                set_backlight(0);
                Screen::Off
            }
            Screen::Saver(t) => {
                saver.step();
                Screen::Saver(t)
            }
            Screen::Off => Screen::Off,
            Screen::Host => saver.start(), // the host's screen but no host: start the saver
        };
    }
}

/// Colours of the bouncing logo, the next one at each wall.
const SAVER_COLOURS: [u16; 7] = [
    gfx::rgb565(0, 224, 200),
    gfx::rgb565(255, 64, 160),
    gfx::rgb565(255, 208, 0),
    gfx::rgb565(64, 160, 255),
    gfx::rgb565(176, 96, 255),
    gfx::rgb565(255, 128, 32),
    gfx::rgb565(96, 255, 96),
];

/// The logo bouncing around the screen, one pixel per frame on each axis.
struct Saver {
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    colour: usize,
}

impl Saver {
    fn new() -> Self {
        let (max_x, max_y) = Self::limits();
        Saver { x: max_x / 2, y: max_y / 3, dx: 1, dy: 1, colour: 0 }
    }

    /// Largest top-left corner that keeps the sprite on the screen.
    fn limits() -> (i32, i32) {
        let (w, h) = gfx::saver_size();
        ((gfx::W as i32 - w as i32).max(0), (gfx::H as i32 - h as i32).max(0))
    }

    /// Black screen, LEDs off, saver backlight, the logo where it was.
    fn start(&mut self) -> Screen {
        gfx::fill(0, 0, gfx::W as u16, gfx::H as u16, 0);
        set_leds([0; 4]);
        set_backlight(SAVER_BACKLIGHT_PCT);
        gfx::saver_draw(self.x as usize, self.y as usize, SAVER_COLOURS[self.colour]);
        Screen::Saver(Instant::now())
    }

    /// One pixel further (its transparent margin erases the previous position); a wall sends it
    /// back and changes its colour.
    fn step(&mut self) {
        let (max_x, max_y) = Self::limits();
        let (x, y) = (self.x + self.dx, self.y + self.dy);
        let (hit_x, hit_y) = (!(0..=max_x).contains(&x), !(0..=max_y).contains(&y));
        if hit_x {
            self.dx = -self.dx;
        }
        if hit_y {
            self.dy = -self.dy;
        }
        if hit_x || hit_y {
            self.colour = (self.colour + 1) % SAVER_COLOURS.len();
        }
        (self.x, self.y) = (x.clamp(0, max_x), y.clamp(0, max_y));
        gfx::saver_draw(self.x as usize, self.y as usize, SAVER_COLOURS[self.colour]);
    }
}

/// DFU requests: USB vendor request, or knob 1 + knob 4 clicks held ~1 s.
#[embassy_executor::task]
async fn guard_task() {
    let mut held = 0u32;
    loop {
        held = if escape::knob1_and_knob4_pressed() { held + 1 } else { 0 };
        if held > 60 || DFU_PENDING.load(Ordering::Acquire) {
            Timer::after_millis(50).await; // let the USB status stage complete
            escape::reboot_to_dfu();
        }
        Timer::after_millis(16).await;
    }
}

/// Vendor control requests (same numbering as the official firmware).
struct VendorHandler;

impl Handler for VendorHandler {
    /// USB suspend (PC asleep, or bus idle before enumeration: debounced by `screen_task`).
    fn suspended(&mut self, suspended: bool) {
        SUSPENDED_SINCE.store(if suspended { now_ms() | 1 } else { 0 }, Ordering::Release);
    }

    /// A reset also ends a suspend (some hosts reset instead of resuming).
    fn reset(&mut self) {
        SUSPENDED_SINCE.store(0, Ordering::Release);
    }

    fn control_out(&mut self, req: Request, _data: &[u8]) -> Option<OutResponse> {
        if is_vendor_device(&req) && req.request == 0xF2 && req.value == 0 && req.index == 0xE053 {
            DFU_PENDING.store(true, Ordering::Release);
            return Some(OutResponse::Accepted);
        }
        None
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        if !is_vendor_device(&req) {
            return None;
        }
        match req.request {
            0x02 if buf.len() >= 2 => {
                buf[..2].copy_from_slice(&FW_VERSION);
                Some(InResponse::Accepted(&buf[..2]))
            }
            0xF2 if req.value == 0 && req.index == 0xE053 => {
                DFU_PENDING.store(true, Ordering::Release);
                Some(InResponse::Accepted(&[]))
            }
            // Diagnostics: 16 registers (u32 LE) of page `value` from DIAG_REGS (tools/fw_regs.py).
            0x03 if buf.len() >= 64 => {
                let page = DIAG_REGS.get(req.value as usize * 16..)?;
                let n = page.len().min(16);
                for (i, &addr) in page[..n].iter().enumerate() {
                    // SAFETY: fixed list of readable peripheral registers, no read side effects.
                    let v = unsafe { core::ptr::read_volatile(addr as *const u32) };
                    buf[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
                }
                Some(InResponse::Accepted(&buf[..4 * n]))
            }
            _ => None,
        }
    }
}

/// Registers returned by vendor request 0x03 (keep in sync with tools/fw_regs.py).
const DIAG_REGS: [u32; 32] = [
    // page 0: RCC CR CFGR PLLCFGR PLLSAI1CFGR PLLSAI2CFGR CCIPR CCIPR2
    0x4002_1000, 0x4002_1008, 0x4002_100C, 0x4002_1010, 0x4002_1014, 0x4002_1088, 0x4002_109C,
    // LTDC SSCR BPCR AWCR TWCR GCR BCCR
    0x4001_6808, 0x4001_680C, 0x4001_6810, 0x4001_6814, 0x4001_6818, 0x4001_682C,
    // GPIOB/D/E OSPEEDR
    0x4800_0408, 0x4800_0C08, 0x4800_1008,
    // page 1: LTDC IER ISR CPSR CDSR, L1 CR WHPCR WVPCR PFCR CACR DCCR BFCR CFBAR CFBLR CFBLNR
    0x4001_6834, 0x4001_6838, 0x4001_6844, 0x4001_6848, 0x4001_6884, 0x4001_6888, 0x4001_688C,
    0x4001_6894, 0x4001_6898, 0x4001_689C, 0x4001_68A0, 0x4001_68AC, 0x4001_68B0, 0x4001_68B4,
    // TIM2 ARR CCR1
    0x4000_002C, 0x4000_0034,
];

fn is_vendor_device(req: &Request) -> bool {
    req.request_type == RequestType::Vendor && req.recipient == Recipient::Device
}

/// Configure `mask` pins of `port` as AF11 push-pull very-high-speed (LTDC).
fn ltdc_pins(port: Gpio, mask: u16) {
    for n in 0..16 {
        if mask & (1 << n) == 0 {
            continue;
        }
        port.afr(n / 8).modify(|w| w.set_afr(n % 8, 11));
        port.otyper().modify(|w| w.set_ot(n, gv::Ot::PUSH_PULL));
        // LOW speed like the official firmware (LTDC MspInit: Speed 0); enough for 10 MHz.
        // (Pins looked speed-sensitive only while the pixel clock was wrongly 40 MHz.)
        port.ospeedr().modify(|w| w.set_ospeedr(n, gv::Ospeedr::LOW_SPEED));
        port.pupdr().modify(|w| w.set_pupdr(n, gv::Pupdr::FLOATING));
        port.moder().modify(|w| w.set_moder(n, gv::Moder::ALTERNATE));
    }
}
