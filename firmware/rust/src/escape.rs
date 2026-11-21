//! Ways back to the STM32 ROM bootloader (ST DFU `0483:df11`). Keep this module dependency-free:
//! [`check_at_boot`] runs before clocks/peripherals are initialised.

use core::ptr::{addr_of_mut, read_volatile, write_volatile};

use embassy_stm32::pac;
use embassy_stm32::pac::gpio::vals as gv;

const DFU_MAGIC: u32 = 0xDF00_B007;
const BOOTLOADER: u32 = 0x1FFF_0000;

/// Survives a software reset (placed after .bss, never zeroed by the runtime).
#[unsafe(link_section = ".uninit.DFU")]
static mut DFU_REQUEST: u32 = 0;

/// Knob 1 click (PC12) and knob 4 click (PA15), active low.
pub fn knob1_and_knob4_pressed() -> bool {
    pac::GPIOC.idr().read().idr(12) == gv::Idr::LOW && pac::GPIOA.idr().read().idr(15) == gv::Idr::LOW
}

/// Must be the first thing `main` does (MSI 4 MHz reset state).
///
/// # Safety
/// Touches RCC/GPIO/SYSCFG registers directly; call once, before `embassy_stm32::init`.
pub unsafe fn check_at_boot() {
    let rcc = pac::RCC;
    rcc.ahb2enr().modify(|w| {
        w.set_gpioaen(true);
        w.set_gpiocen(true);
    });
    let _ = rcc.ahb2enr().read();
    for (port, pin) in [(pac::GPIOA, 15usize), (pac::GPIOC, 12usize)] {
        port.pupdr().modify(|w| w.set_pupdr(pin, gv::Pupdr::PULL_UP));
        port.moder().modify(|w| w.set_moder(pin, gv::Moder::INPUT));
    }
    cortex_m::asm::delay(40_000); // ~10 ms @ 4 MHz: pull-ups settle, contacts debounce

    let requested = unsafe { read_volatile(addr_of_mut!(DFU_REQUEST)) } == DFU_MAGIC;
    unsafe { write_volatile(addr_of_mut!(DFU_REQUEST), 0) };
    if requested || knob1_and_knob4_pressed() {
        // Panel to standby while the ROM bootloader runs (no LTDC scan-out there): DISP (PA8),
        // backlight PWM (PA5) and backlight enable (PB10) driven low – a powered TFT with frozen
        // pixels would otherwise sit under DC bias (image retention). GPIO state survives the jump.
        rcc.ahb2enr().modify(|w| w.set_gpioben(true));
        let _ = rcc.ahb2enr().read();
        for (port, pin) in [(pac::GPIOA, 8usize), (pac::GPIOA, 5), (pac::GPIOB, 10)] {
            port.bsrr().write(|w| w.set_br(pin, true));
            port.moder().modify(|w| w.set_moder(pin, gv::Moder::OUTPUT));
        }
        rcc.apb2enr().modify(|w| w.set_syscfgen(true));
        let _ = rcc.apb2enr().read();
        pac::SYSCFG.memrmp().modify(|w| w.set_mem_mode(1)); // system flash at 0x0
        // SAFETY: valid ROM vector table (MSP at +0, reset vector at +4).
        unsafe { cortex_m::asm::bootload(BOOTLOADER as *const u32) };
    }
}

/// Leave the magic and reset: [`check_at_boot`] then enters the ROM bootloader.
pub fn reboot_to_dfu() -> ! {
    // SAFETY: plain u32 store before reset.
    unsafe { write_volatile(addr_of_mut!(DFU_REQUEST), DFU_MAGIC) };
    cortex_m::peripheral::SCB::sys_reset()
}
