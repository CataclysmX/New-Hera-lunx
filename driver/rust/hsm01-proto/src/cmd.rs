//! Commands carried inside an SM message (see `docs/protocol/commands.md`).
//!
//! Only the commands used by the official app are exposed. Factory commands (0x20-0x22)
//! are intentionally absent. Background/splash commands (0x33, 0x37, 0x50) are available
//! through [`crate::background`]; [`Command::SplashFlash`] writes to the device flash.

/// LED state for command 0x30.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LedState {
    Off = 0,
    On = 1,
    Blink = 2,
}

/// Image bank for icon/label commands (the official SDK always fills both).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Bank {
    A = 0,
    B = 1,
}

/// One command. Use [`Command::encode`] or [`encode_all`] to serialize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 0x10 – host description sent in every keep-alive: OS version (4 × u16) and SDK version.
    HostInfo {
        os: [u16; 4],
        sdk_minor_bcd: u8,
        sdk_major: u8,
    },
    /// 0x11 – "host present" flag (keep-alive). The device then repeats its device-info
    /// message until [`Command::AckDeviceInfo`].
    Ping(u8),
    /// 0x12 – screen on.
    ScreenOn,
    /// 0x13 – screen off (standby).
    ScreenOff,
    /// 0x14 – acknowledge the device-info message.
    AckDeviceInfo,
    /// 0x17 – reset the display state (sent by the SDK on forced refresh).
    ResetDisplay,
    /// 0x30 – button LED `index` (0..4).
    Led { index: u8, state: LedState },
    /// 0x31 – screen backlight, percent (0..=100).
    ScreenBrightness(u8),
    /// 0x39 – LED intensity, percent (the SDK uses 50 = low, 90 = high).
    LedIntensity(u8),
    /// 0x32 – per-channel display block (6 opaque bytes each, see docs; captured default
    /// `81 01 07 07 00 64`).
    ChannelDisplay([[u8; 6]; 4]),
    /// 0x34 – VU meter style: three RGB565 colors + gradient (≤ 16 RGB565 entries).
    VuStyle {
        channel: u8,
        sub: u8,
        colors: [u16; 3],
        gradient: Vec<u16>,
    },
    /// 0x35 – 32×32 icon of `channel`, RLE encoded (see [`crate::rle`]).
    Icon {
        bank: Bank,
        channel: u8,
        rle: Vec<u8>,
    },
    /// 0x36 – 110×16 label of `channel`, RLE encoded.
    Label {
        bank: Bank,
        channel: u8,
        rle: Vec<u8>,
    },
    /// 0x38 – screen/background configuration (colors are 0x00RRGGBB).
    ScreenConfig {
        screen_type: u8,
        background_type: u8,
        darkest_rgb: u32,
        lightest_rgb: u32,
        action_zone_style: u8,
        action_zone_rgb: u32,
    },
    /// 0x40 – VU meter geometry/markers (semantics partially known). `wide` = 16-bit values.
    VuMarkers {
        channel: u8,
        sub: u8,
        values: [u16; 4],
        wide: bool,
    },
    /// 0x41 – VU meter level: `level`/`peak` in 0..=0xffff, `clip` paints the bar red.
    VuLevel {
        channel: u8,
        sub: u8,
        level: u16,
        peak: u16,
        clip: bool,
    },
    /// 0x33 – background palette entries `offset..offset+len` (RGB565, ≤ 256 in total).
    Palette { offset: u8, colors: Vec<u16> },
    /// 0x37 – one of the 32 interlaced background tiles (4080 palette indices),
    /// see [`crate::background`].
    BackgroundTile { tile: u8, data: Vec<u8> },
    /// 0x50 – boot logo: `custom = true` stores the current background (all 32 tiles must have
    /// been sent) in flash, `false` restores the factory logo. **Persistent.**
    SplashFlash { custom: bool },
    /// 0x00 – end of the command list.
    End,
}

impl Command {
    /// Keep-alive content used by the SDK every 500 ms (HOST_INFO + 0x11 0x00).
    pub fn keepalive() -> [Command; 2] {
        [
            Command::HostInfo {
                os: [6, 1, 0, 0],
                sdk_minor_bcd: 0x53,
                sdk_major: 0x01,
            },
            Command::Ping(0),
        ]
    }

    /// VU level the SDK sends for an inactive channel.
    pub fn vu_inactive(channel: u8, sub: u8) -> Command {
        Command::VuLevel {
            channel,
            sub,
            level: 0xc000,
            peak: 0xc000,
            clip: false,
        }
    }

    /// Append the wire encoding of this command to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Command::HostInfo {
                os,
                sdk_minor_bcd,
                sdk_major,
            } => {
                out.extend_from_slice(&[0x10, 0x01]);
                for v in os {
                    out.extend_from_slice(&v.to_le_bytes());
                }
                out.extend_from_slice(&[*sdk_minor_bcd, *sdk_major]);
                out.extend_from_slice(&[0; 12]);
            }
            Command::Ping(v) => out.extend_from_slice(&[0x11, *v]),
            Command::ScreenOn => out.extend_from_slice(&[0x12, 0, 0, 0, 0]),
            Command::ScreenOff => out.extend_from_slice(&[0x13, 0, 0, 0, 0]),
            Command::AckDeviceInfo => out.push(0x14),
            Command::ResetDisplay => out.push(0x17),
            Command::Led { index, state } => out.extend_from_slice(&[0x30, *index, *state as u8]),
            Command::ScreenBrightness(p) => {
                out.extend_from_slice(&[0x31, 0x01, 0x00, (*p).min(100)])
            }
            Command::LedIntensity(p) => out.extend_from_slice(&[0x39, 0x01, 0x00, (*p).min(100)]),
            Command::ChannelDisplay(blocks) => {
                out.extend_from_slice(&[0x32, 0x00]);
                for b in blocks {
                    out.extend_from_slice(b);
                }
            }
            Command::VuStyle {
                channel,
                sub,
                colors,
                gradient,
            } => {
                let n = gradient.len().min(16);
                out.extend_from_slice(&[0x34, (channel & 3) | (sub & 1) << 7]);
                for c in colors {
                    out.extend_from_slice(&c.to_le_bytes());
                }
                out.push(n as u8);
                for c in &gradient[..n] {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            Command::Icon { bank, channel, rle } => image(out, 0x35, *bank, *channel, rle),
            Command::Label { bank, channel, rle } => image(out, 0x36, *bank, *channel, rle),
            Command::ScreenConfig {
                screen_type,
                background_type,
                darkest_rgb,
                lightest_rgb,
                action_zone_style,
                action_zone_rgb,
            } => {
                out.extend_from_slice(&[0x38, *screen_type, *background_type]);
                out.extend_from_slice(&darkest_rgb.to_le_bytes());
                out.extend_from_slice(&lightest_rgb.to_le_bytes());
                out.push(if *action_zone_style < 6 {
                    *action_zone_style
                } else {
                    0
                });
                out.extend_from_slice(&action_zone_rgb.to_le_bytes());
            }
            Command::VuMarkers {
                channel,
                sub,
                values,
                wide,
            } => {
                out.extend_from_slice(&[0x40, (channel & 3) | (*wide as u8) << 6 | (sub & 1) << 7]);
                for v in values {
                    if *wide {
                        out.extend_from_slice(&v.to_le_bytes());
                    } else {
                        out.push(*v as u8);
                    }
                }
            }
            Command::VuLevel {
                channel,
                sub,
                level,
                peak,
                clip,
            } => {
                let c = *clip as u8;
                out.extend_from_slice(&[0x41, (channel & 3) | c << 5 | c << 6 | (sub & 1) << 7]);
                out.extend_from_slice(&level.to_le_bytes());
                out.extend_from_slice(&peak.to_le_bytes());
            }
            Command::Palette { offset, colors } => {
                let n = colors.len().min(256 - *offset as usize);
                out.extend_from_slice(&[0x33, 0x00, 0x00, *offset]);
                out.extend_from_slice(&(n as u16).to_le_bytes());
                out.push(0x00);
                for c in &colors[..n] {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            Command::BackgroundTile { tile, data } => {
                let mut d = data.clone();
                d.resize(crate::background::TILE_BYTES, 0);
                out.extend_from_slice(&[0x37, 0x01]);
                out.extend_from_slice(&(*tile as u32 & 0x1f).to_le_bytes());
                out.push(0x00);
                out.extend_from_slice(&(d.len() as u16).to_le_bytes());
                out.extend_from_slice(&d);
            }
            Command::SplashFlash { custom } => {
                out.extend_from_slice(&[0x50, if *custom { 0x80 } else { 0x40 }])
            }
            Command::End => out.push(0x00),
        }
    }
}

fn image(out: &mut Vec<u8>, op: u8, bank: Bank, channel: u8, rle: &[u8]) {
    out.extend_from_slice(&[op, bank as u8, channel & 3]);
    out.extend_from_slice(&(rle.len() as u16).to_le_bytes());
    out.extend_from_slice(rle);
}

/// Encode a list of commands, appending [`Command::End`] if `terminate` is set.
pub fn encode_all(cmds: &[Command], terminate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for c in cmds {
        c.encode(&mut out);
    }
    if terminate {
        out.push(0x00);
    }
    out
}

/// Length of the command starting at `buf[0]` (firmware dispatcher `FUN_080099d0`),
/// or `None` for an unknown/truncated opcode. Useful to split captured messages.
pub fn command_len(buf: &[u8]) -> Option<usize> {
    let b = |i: usize| buf.get(i).copied();
    let u16at = |i: usize| Some(u16::from_le_bytes([b(i)?, b(i + 1)?]) as usize);
    let n = match b(0)? {
        0x00 | 0x14 | 0x15 | 0x16 | 0x17 => 1,
        0x10 => 24,
        0x11 | 0x20 | 0x50 => 2,
        0x12 | 0x13 => 5,
        0x21 => 4 + b(1)? as usize,
        0x22 | 0x30 => 3,
        0x31 | 0x39 => 3 + (b(1)? & 1) as usize,
        0x32 => {
            if b(1)? == 0 {
                26
            } else {
                18
            }
        }
        0x33 => 7 + 2 * u16at(4)?,
        0x34 => 9 + 2 * b(8)? as usize,
        0x35 | 0x36 => 5 + u16at(3)?,
        0x37 => 9 + 4080,
        0x38 => 16,
        0x40 => {
            if b(1)? & 0x40 != 0 {
                10
            } else {
                6
            }
        }
        0x41 => 6,
        _ => return None,
    };
    (n <= buf.len()).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keepalive_matches_capture_layout() {
        // Captured keep-alive payload (SDK 1.51 on Windows 10.0.26100.712).
        let cmds = [
            Command::HostInfo {
                os: [10, 0, 26100, 712],
                sdk_minor_bcd: 0x51,
                sdk_major: 1,
            },
            Command::Ping(0),
        ];
        let got = encode_all(&cmds, false);
        let want = [
            0x10, 0x01, 0x0a, 0x00, 0x00, 0x00, 0xf4, 0x65, 0xc8, 0x02, 0x51, 0x01, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0x11, 0x00,
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn config_frame_matches_capture() {
        // Commands of captured frame 2079: 14 31 01 00 64 38 00 04 ff.. ff.. 04 ff.. 00
        let cmds = [
            Command::AckDeviceInfo,
            Command::ScreenBrightness(100),
            Command::ScreenConfig {
                screen_type: 0,
                background_type: 4,
                darkest_rgb: 0xffff_ffff,
                lightest_rgb: 0xffff_ffff,
                action_zone_style: 4,
                action_zone_rgb: 0xffff_ffff,
            },
        ];
        let got = encode_all(&cmds, true);
        let want = [
            0x14, 0x31, 0x01, 0x00, 0x64, 0x38, 0x00, 0x04, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0x04, 0xff, 0xff, 0xff, 0xff, 0x00,
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn lengths_are_self_consistent() {
        let cmds = [
            Command::Led {
                index: 1,
                state: LedState::Blink,
            },
            Command::VuStyle {
                channel: 2,
                sub: 1,
                colors: [0x2104, 0xf800, 0xffff],
                gradient: vec![0x2945; 2],
            },
            Command::VuMarkers {
                channel: 0,
                sub: 0,
                values: [1, 1, 2, 2],
                wide: false,
            },
            Command::VuLevel {
                channel: 3,
                sub: 0,
                level: 0xffff,
                peak: 0xffff,
                clip: false,
            },
            Command::Icon {
                bank: Bank::A,
                channel: 0,
                rle: vec![0; 32],
            },
            Command::ChannelDisplay([[0x81, 1, 7, 7, 0, 0x64]; 4]),
        ];
        for c in &cmds {
            let mut v = Vec::new();
            c.encode(&mut v);
            assert_eq!(command_len(&v), Some(v.len()), "{c:?}");
        }
    }
}
