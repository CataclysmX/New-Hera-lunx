//! Transport for the custom firmware ("free screen" protocol, docs/firmware/protocol.md):
//! bulk OUT command stream + interrupt IN input reports, interface 0.

use std::time::Duration;

use hsm01_proto::free::{EP_BULK_OUT, EP_INT_IN, FW_MAJOR, InputReport, parse_report};
use hsm01_proto::{PID, VID, vendor};
use rusb::{Context, DeviceHandle, UsbContext};

use crate::{Error, Result};

const TIMEOUT: Duration = Duration::from_secs(5);

/// A Stream 100 running the custom firmware.
pub struct FreeDevice {
    handle: DeviceHandle<Context>,
    version: (u8, u8),
}

impl FreeDevice {
    /// Open the device; fails with [`Error::WrongFirmware`] on the official firmware.
    pub fn open() -> Result<Self> {
        let ctx = Context::new()?;
        let handle = ctx
            .open_device_with_vid_pid(VID, PID)
            .ok_or(Error::NotFound)?;
        let mut v = [0u8; 2];
        let n = handle.read_control(
            0xc0,
            vendor::GET_FW_VERSION,
            0,
            0,
            &mut v,
            Duration::from_secs(1),
        )?;
        if n != 2 {
            return Err(Error::BadReply("GET_FW_VERSION"));
        }
        if v[1] != FW_MAJOR {
            return Err(Error::WrongFirmware(v[1], v[0]));
        }
        let _ = handle.set_auto_detach_kernel_driver(true);
        handle.claim_interface(0)?;
        Ok(FreeDevice {
            handle,
            version: (v[1], v[0]),
        })
    }

    /// Firmware version `(major, minor)` – major is always 240 here.
    pub fn version(&self) -> (u8, u8) {
        self.version
    }

    /// Send an encoded command stream (see [`hsm01_proto::free`]).
    pub fn send(&self, bytes: &[u8]) -> Result<()> {
        for chunk in bytes.chunks(64 * 1024) {
            let n = self.handle.write_bulk(EP_BULK_OUT, chunk, TIMEOUT)?;
            if n != chunk.len() {
                return Err(Error::BadReply("bulk write (short)"));
            }
        }
        Ok(())
    }

    /// Next input report, `Ok(None)` on timeout.
    pub fn read_input(&self, timeout: Duration) -> Result<Option<InputReport>> {
        let mut buf = [0u8; 64];
        match self.handle.read_interrupt(EP_INT_IN, &mut buf, timeout) {
            Ok(n) => Ok(parse_report(&buf[..n])),
            Err(rusb::Error::Timeout) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Reboot into the ST DFU bootloader (same request as the official firmware).
    pub fn reboot_to_dfu(&self) -> Result<()> {
        let _ = self
            .handle
            .write_control(0x40, 0xF2, 0, PID, &[], Duration::from_secs(1));
        Ok(())
    }
}

impl Drop for FreeDevice {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(0);
    }
}
