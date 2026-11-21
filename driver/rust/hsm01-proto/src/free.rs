//! "Free screen" protocol of the custom firmware (`firmware/rust`, docs/firmware/protocol.md).
//!
//! Host → device: byte stream on bulk EP 0x01, messages `[0xA5][op][0 0][len u32][payload]`.
//! Device → host: 24-byte input report on interrupt EP 0x81 every 10 ms.

/// Major version reported by vendor request 0x02 on the custom firmware.
pub const FW_MAJOR: u8 = 0xF0;
pub const EP_BULK_OUT: u8 = 0x01;
pub const EP_INT_IN: u8 = 0x81;

pub mod op {
    pub const FILL: u8 = 0x01;
    pub const BLIT: u8 = 0x02;
    pub const BLIT_RLE: u8 = 0x03;
    /// 240.21 and later.
    pub const COPY: u8 = 0x04;
    pub const BACKLIGHT: u8 = 0x10;
    pub const LEDS: u8 = 0x11;
}

fn message(op: u8, payload: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&[0xA5, op, 0, 0]);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
}

fn rect(x: u16, y: u16, w: u16, h: u16) -> Vec<u8> {
    [x, y, w, h].iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// RGB888 → RGB565.
pub const fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xf8) << 8) | ((g as u16 & 0xfc) << 3) | (b as u16 >> 3)
}

/// Fill a rectangle with one colour.
pub fn fill(x: u16, y: u16, w: u16, h: u16, color: u16, out: &mut Vec<u8>) {
    let mut p = rect(x, y, w, h);
    p.extend_from_slice(&color.to_le_bytes());
    message(op::FILL, &p, out);
}

/// Raw RGB565 pixels (row major, `w*h` entries).
pub fn blit(x: u16, y: u16, w: u16, h: u16, pixels: &[u16], out: &mut Vec<u8>) {
    assert_eq!(pixels.len(), w as usize * h as usize);
    let mut p = rect(x, y, w, h);
    p.reserve(pixels.len() * 2);
    for px in pixels {
        p.extend_from_slice(&px.to_le_bytes());
    }
    message(op::BLIT, &p, out);
}

/// Run-length encoded RGB565 pixels: `[count u8][color u16]` runs.
pub fn blit_rle(x: u16, y: u16, w: u16, h: u16, pixels: &[u16], out: &mut Vec<u8>) {
    assert_eq!(pixels.len(), w as usize * h as usize);
    let mut p = rect(x, y, w, h);
    let mut i = 0;
    while i < pixels.len() {
        let c = pixels[i];
        let mut n = 1;
        while n < 255 && i + n < pixels.len() && pixels[i + n] == c {
            n += 1;
        }
        p.push(n as u8);
        p.extend_from_slice(&c.to_le_bytes());
        i += n;
    }
    message(op::BLIT_RLE, &p, out);
}

/// Smallest of raw / RLE encodings for these pixels.
pub fn blit_auto(x: u16, y: u16, w: u16, h: u16, pixels: &[u16], out: &mut Vec<u8>) {
    let mut rle = Vec::new();
    blit_rle(x, y, w, h, pixels, &mut rle);
    if rle.len() < 16 + pixels.len() * 2 {
        out.extend_from_slice(&rle);
    } else {
        blit(x, y, w, h, pixels, out);
    }
}

/// First firmware minor version (240.xx) that knows `copy`.
pub const COPY_SINCE: u8 = 0x15;
/// First firmware minor version with the screen saver: without a host reading the input reports
/// (or sending) for 3 s it takes the screen back (`InputReport::screen_epoch`).
pub const SAVER_SINCE: u8 = 0x16;

/// Copy the rectangle (x, y, w, h) of the device's screen so that its top-left corner goes to
/// (to_x, to_y), overlap allowed (scrolling) – firmware 240.21 and later (`COPY_SINCE`).
pub fn copy(x: u16, y: u16, w: u16, h: u16, to_x: u16, to_y: u16, out: &mut Vec<u8>) {
    let mut p = rect(x, y, w, h);
    p.extend_from_slice(&to_x.to_le_bytes());
    p.extend_from_slice(&to_y.to_le_bytes());
    message(op::COPY, &p, out);
}

pub fn backlight(pct: u8, out: &mut Vec<u8>) {
    message(op::BACKLIGHT, &[pct.min(100)], out);
}

pub fn leds(pct: [u8; 4], out: &mut Vec<u8>) {
    message(op::LEDS, &pct.map(|v| v.min(100)), out);
}

/// Input report (interrupt IN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputReport {
    pub seq: u8,
    /// Same bit layout as the official firmware: knob clicks 0-3, buttons 4-7, power 8.
    pub buttons: u16,
    /// Raw 12-bit ADC of the 8 wipers: knob1 A/B, knob2 A/B, knob3 A/B, knob4 A/B.
    pub wipers: [u16; 8],
    pub backlight: u8,
    /// Changes each time the firmware took the screen back (screen saver, `SAVER_SINCE`): what
    /// the host drew is gone, redraw everything. Always 0 before.
    pub screen_epoch: u8,
}

pub fn parse_report(b: &[u8]) -> Option<InputReport> {
    if b.len() < 21 || b[0] != 0x01 {
        return None;
    }
    let mut wipers = [0u16; 8];
    for (i, w) in wipers.iter_mut().enumerate() {
        *w = u16::from_le_bytes([b[4 + 2 * i], b[5 + 2 * i]]);
    }
    Some(InputReport {
        seq: b[1],
        buttons: u16::from_le_bytes([b[2], b[3]]),
        wipers,
        backlight: b[20],
        screen_epoch: b.get(21).copied().unwrap_or(0),
    })
}

/// Endless dual-wiper knobs (docs/firmware/stage2.md): the two wipers are quadrature triangle
/// waves, so `(a, b)` runs around a diamond `|a-mid| + |b-mid| ≈ const` once per cycle.
pub mod knob {
    const MID: i32 = 2048;
    /// Phase units per cycle (fixed point).
    const CYCLE: i32 = 1 << 16;
    /// Counts per cycle, same unit and sign as the official firmware's encoder accumulators.
    pub const COUNTS_PER_CYCLE: i32 = 512;
    const STEP: i32 = CYCLE / COUNTS_PER_CYCLE;

    /// Position on the diamond, in 1/65536 cycle (gain-normalised by `|x| + |y|`).
    pub fn phase(a: u16, b: u16) -> u16 {
        let (x, y) = (a as i32 - MID, b as i32 - MID);
        let s = (x.abs() + y.abs()).max(1);
        let q = CYCLE / 4;
        let p = match (x >= 0, y >= 0) {
            (true, true) => y * q / s,
            (false, true) => q + -x * q / s,
            (false, false) => 2 * q + -y * q / s,
            (true, false) => 3 * q + x * q / s,
        };
        p as u16 // wraps 65536 → 0
    }

    /// Turns raw wiper pairs into count deltas, with one count of hysteresis (ADC noise ≈ ±0.4).
    #[derive(Debug, Default, Clone, Copy)]
    pub struct Decoder {
        reference: Option<u16>,
    }

    impl Decoder {
        /// Delta in counts since the last call (positive = official firmware's "+").
        pub fn update(&mut self, a: u16, b: u16) -> i32 {
            let p = phase(a, b);
            let Some(r) = self.reference else {
                self.reference = Some(p);
                return 0;
            };
            let n = p.wrapping_sub(r) as i16 as i32 / STEP;
            self.reference = Some(r.wrapping_add((n * STEP) as u16));
            n
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quadrature triangle wipers at `t` cycles (a leads: phase 0 = a max, b mid).
    fn wipers(t: f64) -> (u16, u16) {
        let tri = |u: f64| {
            let u = u.rem_euclid(1.0);
            let v = if u < 0.5 {
                1.0 - 4.0 * u
            } else {
                4.0 * u - 3.0
            }; // +1 → -1 → +1
            (2048.0 + v * 2047.0).round() as u16
        };
        (tri(t), tri(t - 0.25))
    }

    #[test]
    fn knob_full_cycle_is_512_counts_both_ways() {
        for dir in [1.0, -1.0] {
            let mut d = knob::Decoder::default();
            let mut total = 0;
            for i in 0..=1000 {
                let (a, b) = wipers(0.1 + dir * i as f64 / 1000.0);
                total += d.update(a, b);
            }
            assert!((total - dir as i32 * 512).abs() <= 1, "dir {dir}: {total}");
        }
    }

    #[test]
    fn knob_noise_gives_no_counts() {
        let mut d = knob::Decoder::default();
        let (a, b) = wipers(0.3);
        let mut total = 0;
        for i in 0..1000 {
            let n = [0i32, 7, -7, 3, -5][i % 5];
            total += d.update((a as i32 + n) as u16, (b as i32 - n) as u16).abs();
        }
        assert_eq!(total, 0);
    }

    #[test]
    fn message_layout() {
        let mut v = Vec::new();
        fill(1, 2, 3, 4, 0xf800, &mut v);
        assert_eq!(
            v,
            [
                0xA5, 0x01, 0, 0, 10, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 0x00, 0xf8
            ]
        );
    }

    /// Same bytes as the C port's test (`driver/c/tests/test_free.c`).
    #[test]
    fn copy_layout() {
        let mut v = Vec::new();
        copy(1, 2, 3, 4, 5, 6, &mut v);
        assert_eq!(
            v,
            [
                0xA5, 0x04, 0, 0, 12, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0
            ]
        );
    }

    #[test]
    fn rle_runs_are_capped_and_complete() {
        let px = vec![7u16; 600];
        let mut v = Vec::new();
        blit_rle(0, 0, 600, 1, &px, &mut v);
        let payload = &v[8 + 8..];
        let total: usize = payload.chunks(3).map(|c| c[0] as usize).sum();
        assert_eq!(total, 600);
        assert!(payload.chunks(3).all(|c| c[0] >= 1));
    }

    #[test]
    fn report_parse() {
        let mut r = [0u8; 24];
        r[0] = 1;
        r[1] = 9;
        r[2] = 0x01;
        r[3] = 0x01;
        r[4] = 0xff;
        r[5] = 0x0f;
        r[20] = 80;
        r[21] = 3;
        let p = parse_report(&r).unwrap();
        assert_eq!(
            (p.seq, p.buttons, p.wipers[0], p.backlight, p.screen_epoch),
            (9, 0x0101, 0x0fff, 80, 3)
        );
    }
}
