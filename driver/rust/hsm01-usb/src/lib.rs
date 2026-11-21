//! libusb transport for the Hercules Stream 100 (userspace, no kernel driver needed).
//!
//! * [`Device::open`] performs the same init as the Windows driver (config 1, interfaces 0/1,
//!   alt setting 1 on interface 1).
//! * [`Device::send`] frames commands and writes them as one isochronous transfer.
//! * [`Device::poll`] reads one interrupt report and acknowledges device-info messages.
//! * [`Device::spawn_keepalive`] keeps the device in "host present" mode (500 ms period).

use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use hsm01_proto::cmd::{Command, encode_all};
use hsm01_proto::frame::{FrameBuilder, FrameError};
use hsm01_proto::input::{DeviceMessage, ParseError, Report, parse_report};
use hsm01_proto::{EP_INT_IN, EP_ISO_OUT, INT_REPORT, ISO_PACKET, PID, VID, vendor};
use rusb::{Context, DeviceHandle, UsbContext, ffi};

pub mod free;

/// Transport errors.
#[derive(Debug)]
pub enum Error {
    /// No Stream 100 (06f8:e053) found.
    NotFound,
    Usb(rusb::Error),
    /// Raw libusb error code (isochronous path).
    Libusb(i32),
    /// Isochronous transfer finished with a non-completed status.
    IsoStatus(i32),
    Frame(FrameError),
    Parse(ParseError),
    /// Unexpected vendor-request answer.
    BadReply(&'static str),
    /// The device does not run the expected firmware (official vs custom), with its version.
    WrongFirmware(u8, u8),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotFound => write!(f, "Hercules Stream 100 ({VID:04x}:{PID:04x}) not found"),
            Error::Usb(e) => write!(f, "usb: {e}"),
            Error::Libusb(c) => write!(f, "libusb error {c}"),
            Error::IsoStatus(s) => write!(f, "isochronous transfer status {s}"),
            Error::Frame(e) => write!(f, "frame: {e}"),
            Error::Parse(e) => write!(f, "report: {e}"),
            Error::BadReply(w) => write!(f, "unexpected reply to {w}"),
            Error::WrongFirmware(maj, min) => write!(
                f,
                "device runs firmware {maj}.{min:02} (custom firmware reports 240.xx, official 1.xx)"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusb::Error> for Error {
    fn from(e: rusb::Error) -> Self {
        Error::Usb(e)
    }
}

impl From<FrameError> for Error {
    fn from(e: FrameError) -> Self {
        Error::Frame(e)
    }
}

impl From<ParseError> for Error {
    fn from(e: ParseError) -> Self {
        Error::Parse(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

const CTRL_TIMEOUT: Duration = Duration::from_millis(1000);
const ISO_TIMEOUT_MS: u32 = 1000;

/// An opened and initialised Stream 100.
pub struct Device {
    ctx: Context,
    handle: DeviceHandle<Context>,
    frames: Mutex<FrameBuilder>,
    auto_ack: AtomicBool,
}

impl Device {
    /// Open the first Stream 100 and run the init sequence.
    pub fn open() -> Result<Self> {
        let ctx = Context::new()?;
        let handle = ctx
            .open_device_with_vid_pid(VID, PID)
            .ok_or(Error::NotFound)?;
        // No kernel driver binds to these vendor interfaces, but be tolerant.
        let _ = handle.set_auto_detach_kernel_driver(true);
        if handle.active_configuration().unwrap_or(0) != 1 {
            handle.set_active_configuration(1)?;
        }
        handle.claim_interface(0)?;
        handle.claim_interface(1)?;
        handle.set_alternate_setting(1, 1)?;
        Ok(Device {
            ctx,
            handle,
            frames: Mutex::new(FrameBuilder::new()),
            auto_ack: AtomicBool::new(true),
        })
    }

    /// Firmware version `(major, minor)` (vendor request 0x02).
    pub fn firmware_version(&self) -> Result<(u8, u8)> {
        let mut b = [0u8; 2];
        let n =
            self.handle
                .read_control(0xc0, vendor::GET_FW_VERSION, 0, 0, &mut b, CTRL_TIMEOUT)?;
        if n != 2 {
            return Err(Error::BadReply("GET_FW_VERSION"));
        }
        Ok((b[1], b[0]))
    }

    /// 8-byte serial number (vendor request 0x52, bank 1).
    pub fn serial(&self) -> Result<[u8; 8]> {
        let mut b = [0u8; 9];
        let n = self
            .handle
            .read_control(0xc0, vendor::READ_INFO, 1, 0, &mut b, CTRL_TIMEOUT)?;
        if n != 9 || b[0] != 8 {
            return Err(Error::BadReply("READ_INFO"));
        }
        Ok(b[1..].try_into().unwrap())
    }

    /// Acknowledge device-info messages automatically in [`Device::poll`] (default: on).
    pub fn set_auto_ack(&self, on: bool) {
        self.auto_ack.store(on, Ordering::Relaxed);
    }

    /// Frame already-encoded command bytes and send them.
    pub fn send_raw(&self, commands: &[u8]) -> Result<()> {
        let buf = self.frames.lock().unwrap().build(commands)?;
        self.write_iso(buf)
    }

    /// Send commands (an `End` marker is appended).
    pub fn send(&self, cmds: &[Command]) -> Result<()> {
        self.send_raw(&encode_all(cmds, true))
    }

    /// Send one keep-alive frame (HOST_INFO + 0x11 0x00), as the SDK does every 500 ms.
    pub fn keepalive(&self) -> Result<()> {
        self.send_raw(&encode_all(&Command::keepalive(), false))
    }

    /// Read one interrupt report. `Ok(None)` on timeout.
    pub fn poll(&self, timeout: Duration) -> Result<Option<Report>> {
        let mut buf = [0u8; INT_REPORT];
        let n = match self.handle.read_interrupt(EP_INT_IN, &mut buf, timeout) {
            Ok(n) => n,
            Err(rusb::Error::Timeout) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let report = parse_report(&buf[..n])?;
        if matches!(report.message, Some(DeviceMessage::Info(_)))
            && self.auto_ack.load(Ordering::Relaxed)
        {
            self.send(&[Command::AckDeviceInfo])?;
        }
        Ok(Some(report))
    }

    /// Start a background thread sending keep-alives every 500 ms. Stops when dropped.
    pub fn spawn_keepalive(self: &Arc<Self>) -> KeepAlive {
        let stop = Arc::new(AtomicBool::new(false));
        let (dev, flag) = (Arc::clone(self), Arc::clone(&stop));
        let join = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                if let Err(e) = dev.keepalive() {
                    eprintln!("hsm01: keep-alive failed: {e}");
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
        KeepAlive {
            stop,
            join: Some(join),
        }
    }

    /// Synchronous isochronous OUT transfer of `buf` (multiple of 952 bytes).
    fn write_iso(&self, mut buf: Vec<u8>) -> Result<()> {
        debug_assert_eq!(buf.len() % ISO_PACKET, 0);
        let packets = (buf.len() / ISO_PACKET) as c_int;
        let completed: *mut c_int = Box::into_raw(Box::new(0));
        // SAFETY: the transfer, its buffer and the completion flag outlive the event loop below;
        // the transfer is freed only after its callback has run (or submission failed).
        unsafe {
            let xfer = ffi::libusb_alloc_transfer(packets);
            if xfer.is_null() {
                drop(Box::from_raw(completed));
                return Err(Error::Usb(rusb::Error::NoMem));
            }
            ffi::libusb_fill_iso_transfer(
                xfer,
                self.handle.as_raw(),
                EP_ISO_OUT,
                buf.as_mut_ptr(),
                buf.len() as c_int,
                packets,
                iso_done,
                completed as *mut c_void,
                ISO_TIMEOUT_MS,
            );
            ffi::libusb_set_iso_packet_lengths(xfer, ISO_PACKET as u32);
            let rc = ffi::libusb_submit_transfer(xfer);
            if rc < 0 {
                ffi::libusb_free_transfer(xfer);
                drop(Box::from_raw(completed));
                return Err(Error::Libusb(rc));
            }
            while std::ptr::read_volatile(completed) == 0 {
                let rc = ffi::libusb_handle_events_completed(self.ctx.as_raw(), completed);
                if rc < 0 && rc != ffi::constants::LIBUSB_ERROR_INTERRUPTED {
                    ffi::libusb_cancel_transfer(xfer);
                }
            }
            let status = (*xfer).status;
            ffi::libusb_free_transfer(xfer);
            drop(Box::from_raw(completed));
            if status != ffi::constants::LIBUSB_TRANSFER_COMPLETED {
                return Err(Error::IsoStatus(status));
            }
        }
        Ok(())
    }
}

extern "system" fn iso_done(xfer: *mut ffi::libusb_transfer) {
    // SAFETY: user_data is the completion flag allocated in `write_iso`.
    unsafe { *((*xfer).user_data as *mut c_int) = 1 };
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = self.handle.set_alternate_setting(1, 0);
        let _ = self.handle.release_interface(1);
        let _ = self.handle.release_interface(0);
    }
}

/// Guard returned by [`Device::spawn_keepalive`].
pub struct KeepAlive {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for KeepAlive {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
