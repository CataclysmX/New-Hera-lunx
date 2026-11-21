//! Host → device framing (isochronous OUT).
//!
//! Buffer = N × 952 bytes: packet 0 = `"HERCULES"` + zeros, then the SM message
//! `[ 'S' 'M' | len u16 | crc u16 | seq u16 | commands… ]` zero-padded to a multiple of 952.
//! `len` counts the whole message, the CRC covers bytes `0..4` and `6..len`.

use crate::ISO_PACKET;
use crate::crc::crc16;

/// Largest SM message the official SDK ever sends (9 packets after the `HERCULES` one).
pub const MAX_SM_LEN: usize = 9 * ISO_PACKET;

/// Errors produced while building or parsing frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The command payload does not fit in [`MAX_SM_LEN`].
    TooLong(usize),
    /// Missing `SM` signature or truncated message.
    BadHeader,
    /// CRC mismatch (`expected`, `found`).
    BadCrc(u16, u16),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong(n) => write!(f, "SM message too long ({n} > {MAX_SM_LEN} bytes)"),
            Self::BadHeader => write!(f, "bad SM header"),
            Self::BadCrc(e, g) => write!(f, "bad SM CRC (expected {e:#06x}, found {g:#06x})"),
        }
    }
}

impl std::error::Error for FrameError {}

/// CRC of a complete SM message (bytes 4..6 are excluded).
pub fn sm_crc(msg: &[u8]) -> u16 {
    crc16(&msg[6..], crc16(&msg[..4], 0))
}

/// Build the SM message alone (no `HERCULES` packet, no padding).
pub fn build_sm(seq: u16, commands: &[u8]) -> Result<Vec<u8>, FrameError> {
    let len = 8 + commands.len();
    if len > MAX_SM_LEN {
        return Err(FrameError::TooLong(len));
    }
    let mut m = Vec::with_capacity(len);
    m.extend_from_slice(b"SM");
    m.extend_from_slice(&(len as u16).to_le_bytes());
    m.extend_from_slice(&[0, 0]);
    m.extend_from_slice(&seq.to_le_bytes());
    m.extend_from_slice(commands);
    let crc = sm_crc(&m);
    m[4..6].copy_from_slice(&crc.to_le_bytes());
    Ok(m)
}

/// Wrap an SM message into the isochronous buffer (`HERCULES` packet + padding).
pub fn wrap_iso(sm: &[u8]) -> Vec<u8> {
    let body = sm.len().div_ceil(ISO_PACKET) * ISO_PACKET;
    let mut buf = vec![0u8; ISO_PACKET + body];
    buf[..8].copy_from_slice(b"HERCULES");
    buf[ISO_PACKET..ISO_PACKET + sm.len()].copy_from_slice(sm);
    buf
}

/// Parsed view of an SM message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmView<'a> {
    pub seq: u16,
    pub commands: &'a [u8],
}

/// Validate an SM message (signature, length, CRC) and return its sequence + commands.
pub fn parse_sm(msg: &[u8]) -> Result<SmView<'_>, FrameError> {
    if msg.len() < 8 || &msg[..2] != b"SM" {
        return Err(FrameError::BadHeader);
    }
    let len = u16::from_le_bytes([msg[2], msg[3]]) as usize;
    if !(8..=msg.len()).contains(&len) {
        return Err(FrameError::BadHeader);
    }
    let msg = &msg[..len];
    let found = u16::from_le_bytes([msg[4], msg[5]]);
    let expected = sm_crc(msg);
    if found != expected {
        return Err(FrameError::BadCrc(expected, found));
    }
    Ok(SmView {
        seq: u16::from_le_bytes([msg[6], msg[7]]),
        commands: &msg[8..],
    })
}

/// Stateful builder: allocates sequence numbers like the official SDK (+1 per frame).
#[derive(Debug, Default)]
pub struct FrameBuilder {
    seq: u16,
}

impl FrameBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sequence number that the next frame will carry.
    pub fn next_seq(&self) -> u16 {
        self.seq
    }

    /// Build a complete isochronous buffer for already-encoded `commands`.
    pub fn build(&mut self, commands: &[u8]) -> Result<Vec<u8>, FrameError> {
        let sm = build_sm(self.seq, commands)?;
        self.seq = self.seq.wrapping_add(1);
        Ok(wrap_iso(&sm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // Captured from the official Windows app (re_work/captures/wireshark.tsv, frames 311/2079/2049).
    const KEEPALIVE: &str = "534d2200b89a010010010a000000f465c80251010000000000000000000000001100";
    const CONFIG: &str = "534d1e007a7c24001431010064380004ffffffffffffffff04ffffffff00";
    const ACK: &str = "534d0a00ad1a1d001400";

    #[test]
    fn crc_matches_official_frames() {
        for v in [KEEPALIVE, CONFIG, ACK] {
            let m = hex(v);
            assert_eq!(sm_crc(&m), u16::from_le_bytes([m[4], m[5]]), "{v}");
            assert!(parse_sm(&m).is_ok());
        }
    }

    #[test]
    fn rebuild_is_byte_identical() {
        for v in [KEEPALIVE, CONFIG, ACK] {
            let m = hex(v);
            let view = parse_sm(&m).unwrap();
            assert_eq!(build_sm(view.seq, view.commands).unwrap(), m);
        }
    }

    #[test]
    fn iso_wrapping() {
        let mut fb = FrameBuilder::new();
        let buf = fb.build(&[0x14, 0x00]).unwrap();
        assert_eq!(buf.len(), 2 * ISO_PACKET);
        assert_eq!(&buf[..8], b"HERCULES");
        assert_eq!(&buf[ISO_PACKET..ISO_PACKET + 2], b"SM");
        assert_eq!(fb.next_seq(), 1);
        assert_eq!(
            parse_sm(&buf[ISO_PACKET..]).unwrap().commands,
            &[0x14, 0x00]
        );
    }

    #[test]
    fn corrupted_crc_is_rejected() {
        let mut m = hex(ACK);
        m[9] ^= 1;
        assert!(matches!(parse_sm(&m), Err(FrameError::BadCrc(..))));
    }
}
