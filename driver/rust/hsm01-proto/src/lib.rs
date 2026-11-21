//! Hercules Stream 100 ("HSM01") wire protocol – pure, no I/O.
//!
//! Reverse-engineered from `hsm_api_core_x64.dll` v1.53 and firmware v1.42; validated on
//! captures and on a live device (see `docs/protocol/*.md`).
//!
//! * [`frame`] – `HERCULES` + `SM` message framing, CRC16, sequence numbers (host → device)
//! * [`cmd`]   – display/LED/VU commands carried inside an SM message
//! * [`rle`]   – icon/label image encoding (ARGB8888 → RLE RGB565 + 4-bit alpha)
//! * [`input`] – interrupt-IN reports (controls block + `sm` device messages)

pub mod background;
pub mod cmd;
pub mod crc;
pub mod frame;
pub mod free;
pub mod input;
pub mod rle;

/// USB vendor id (Guillemot / Hercules).
pub const VID: u16 = 0x06f8;
/// USB product id of the Stream 100 in normal mode.
pub const PID: u16 = 0xe053;
/// Isochronous OUT endpoint (interface 1, alt setting 1).
pub const EP_ISO_OUT: u8 = 0x01;
/// Interrupt IN endpoint (interface 0).
pub const EP_INT_IN: u8 = 0x81;
/// Size of one isochronous packet (wMaxPacketSize).
pub const ISO_PACKET: usize = 952;
/// Size of an interrupt IN report.
pub const INT_REPORT: usize = 64;

/// Screen geometry reported by the device-info message.
pub const SCREEN_W: usize = 480;
pub const SCREEN_H: usize = 272;
/// Number of channels (knob + icon + label + VU meter).
pub const CHANNELS: usize = 4;
/// Number of button LEDs.
pub const LEDS: usize = 4;
/// Per-channel icon size (command 0x35).
pub const ICON_W: usize = 32;
pub const ICON_H: usize = 32;
/// Per-channel label size (command 0x36).
pub const LABEL_W: usize = 110;
pub const LABEL_H: usize = 16;

/// Vendor control requests (bmRequestType 0xC0, device-to-host).
pub mod vendor {
    /// Firmware version: returns `[minor, major]` (e.g. `28 01` = 1.40).
    pub const GET_FW_VERSION: u8 = 0x02;
    /// Read info memory: wValue = bank (0/1), wIndex = offset; returns `[n][n bytes]`.
    /// Bank 1 offset 0 holds the 8-byte serial number.
    pub const READ_INFO: u8 = 0x52;
    // 0xF2 with wValue = 0xE053 reboots into the ST DFU bootloader: deliberately not exposed.
}
