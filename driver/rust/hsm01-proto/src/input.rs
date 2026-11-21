//! Device → host interrupt reports (64 bytes): `[n=12][controls][sm message]`.
//! See `docs/protocol/inputs.md` (mapping confirmed on a live device).

/// Button bits of [`Controls::buttons`].
pub mod buttons {
    pub const KNOB1: u16 = 1 << 0;
    pub const KNOB2: u16 = 1 << 1;
    pub const KNOB3: u16 = 1 << 2;
    pub const KNOB4: u16 = 1 << 3;
    pub const BUTTON1: u16 = 1 << 4;
    pub const BUTTON2: u16 = 1 << 5;
    pub const BUTTON3: u16 = 1 << 6;
    pub const BUTTON4: u16 = 1 << 7;
    pub const POWER: u16 = 1 << 8;

    /// Human readable names, indexed by bit number.
    pub const NAMES: [&str; 9] = [
        "knob1", "knob2", "knob3", "knob4", "button1", "button2", "button3", "button4", "power",
    ];
}

/// Controls block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Controls {
    /// Pressed buttons (see [`buttons`]).
    pub buttons: u16,
    /// Endless knob accumulators; only differences are meaningful (see [`encoder_delta`]).
    pub encoders: [i16; 4],
    /// Screen backlight currently on.
    pub screen_on: bool,
    /// Current screen brightness, percent (changed locally by the power button).
    pub brightness: u8,
}

/// Device information (`sm` type 1), sent until the host acknowledges it with 0x14.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub device_type: u8,
    /// Firmware version (major, minor), e.g. (1, 40).
    pub firmware: (u8, u8),
    /// Hardware revision byte followed by the 8-byte serial number.
    pub serial: [u8; 9],
    pub screen_w: u16,
    pub screen_h: u16,
    pub icon_size: (u16, u16),
    pub label_size: (u16, u16),
    pub button_count: u8,
    pub led_count: u8,
    /// Whole payload (bytes 7.. of the message) for fields not decoded yet.
    pub raw: Vec<u8>,
}

/// Device status (`sm` type 2), sent every ~16 ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// Error bits: 0x08 CRC, 0x10 signature, 0x20 length, 0x40 unknown opcode.
    pub error: u16,
    /// Sequence number of the last SM message processed.
    pub last_seq: u16,
    pub error_count: u16,
    pub serial: [u8; 9],
    pub factory_code: u16,
    /// Last rejected opcode (0 = none).
    pub last_invalid_cmd: u8,
    pub render_ms: u8,
    pub flash_status: u8,
}

/// Message part of a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceMessage {
    Info(DeviceInfo),
    Status(Status),
    Unknown { kind: u8, payload: Vec<u8> },
}

/// A parsed interrupt report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub controls: Option<Controls>,
    pub message: Option<DeviceMessage>,
}

/// Report parsing errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Truncated,
    BadSignature,
    BadChecksum,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Truncated => "truncated report",
            Self::BadSignature => "bad 'sm' signature",
            Self::BadChecksum => "bad 'sm' checksum",
        })
    }
}

impl std::error::Error for ParseError {}

/// Knob movement between two reports (16-bit wrap-around).
pub fn encoder_delta(old: i16, new: i16) -> i16 {
    new.wrapping_sub(old)
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn parse_controls(c: &[u8]) -> Option<Controls> {
    if c.len() < 12 {
        return None;
    }
    let mut encoders = [0i16; 4];
    for (k, e) in encoders.iter_mut().enumerate() {
        *e = u16le(c, 3 + 2 * k) as i16;
    }
    Some(Controls {
        buttons: u16le(c, 1),
        encoders,
        screen_on: c[11] & 0x80 != 0,
        brightness: c[11] & 0x7f,
    })
}

/// Parse an `sm` message (starting at `'s'`).
pub fn parse_message(sm: &[u8]) -> Result<DeviceMessage, ParseError> {
    if sm.len() < 7 {
        return Err(ParseError::Truncated);
    }
    if &sm[..2] != b"sm" {
        return Err(ParseError::BadSignature);
    }
    let n = sm[2] as usize;
    if n < 7 || n > sm.len() {
        return Err(ParseError::Truncated);
    }
    let sm = &sm[..n];
    if sm.iter().fold(0u8, |a, &b| a.wrapping_add(b)) != 0 {
        return Err(ParseError::BadChecksum);
    }
    let serial = |i: usize| -> [u8; 9] { sm[i..i + 9].try_into().unwrap() };
    Ok(match sm[6] {
        1 if n >= 40 => DeviceMessage::Info(DeviceInfo {
            device_type: sm[7],
            firmware: (sm[9], sm[8]),
            serial: serial(10),
            screen_w: u16le(sm, 21),
            screen_h: u16le(sm, 23),
            icon_size: (u16le(sm, 29), u16le(sm, 31)),
            label_size: (u16le(sm, 33), u16le(sm, 35)),
            button_count: sm[38],
            led_count: sm[39],
            raw: sm[7..].to_vec(),
        }),
        2 if n >= 33 => DeviceMessage::Status(Status {
            error: u16le(sm, 7),
            last_seq: u16le(sm, 9),
            error_count: u16le(sm, 11),
            serial: serial(15),
            factory_code: u16le(sm, 24),
            last_invalid_cmd: sm[28],
            render_ms: sm[29],
            flash_status: sm[32],
        }),
        kind => DeviceMessage::Unknown {
            kind,
            payload: sm[7..].to_vec(),
        },
    })
}

/// Parse a full 64-byte interrupt report.
pub fn parse_report(buf: &[u8]) -> Result<Report, ParseError> {
    let n = *buf.first().ok_or(ParseError::Truncated)? as usize;
    if n > buf.len() {
        return Err(ParseError::Truncated);
    }
    let controls = parse_controls(&buf[..n]);
    let rest = &buf[n..];
    let message = if rest.len() >= 2 && &rest[..2] == b"sm" {
        Some(parse_message(rest)?)
    } else {
        None
    };
    Ok(Report { controls, message })
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

    #[test]
    fn captured_status_report() {
        // Interrupt report captured from the official app session (frame 303).
        let r = parse_report(&hex(
            "0c0000a4fff8ff0f002400bc736d2100e8d10200003c0109005a03026ec2a707079b00000300000000030e0cff6e001000040904000000000000000000000000",
        ))
        .unwrap();
        let c = r.controls.unwrap();
        assert_eq!(c.buttons, 0);
        assert_eq!(c.encoders, [-92, -8, 15, 36]);
        assert!(c.screen_on);
        assert_eq!(c.brightness, 60);
        match r.message.unwrap() {
            DeviceMessage::Status(s) => {
                assert_eq!(s.error, 0);
                assert_eq!(s.error_count, 9);
                assert_eq!(&s.serial[1..], &[0x6e, 0xc2, 0xa7, 0x07, 0x07, 0x9b, 0, 0]);
                assert_eq!(s.factory_code, 3);
                assert_eq!(s.flash_status, 0xff);
            }
            m => panic!("unexpected {m:?}"),
        }
    }

    #[test]
    fn device_info_message() {
        // Payload seen live; prefix 'sm', len 50, pk, ?, type 1. Checksum byte fixed below.
        let mut sm = hex("736d3200030001");
        sm.extend(hex("012801026ec2a707079b00000300e001100104000402200020006e0010000409040000000000000000000000"));
        sm.truncate(50);
        let sum = sm.iter().fold(0u8, |a, &b| a.wrapping_add(b));
        sm[5] = sm[5].wrapping_sub(sum);
        match parse_message(&sm).unwrap() {
            DeviceMessage::Info(i) => {
                assert_eq!(i.firmware, (1, 40));
                assert_eq!((i.screen_w, i.screen_h), (480, 272));
                assert_eq!(i.icon_size, (32, 32));
                assert_eq!(i.label_size, (110, 16));
                assert_eq!((i.button_count, i.led_count), (9, 4));
            }
            m => panic!("unexpected {m:?}"),
        }
    }

    #[test]
    fn delta_wraps() {
        assert_eq!(encoder_delta(i16::MAX, i16::MIN), 1);
        assert_eq!(encoder_delta(215, 219), 4);
    }
}
