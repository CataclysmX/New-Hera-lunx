//! Icon/label image encoding (DLL `FUN_180004900` + `FUN_1800046a0`, firmware decoder in
//! `FUN_080099d0` cases 0x35/0x36).
//!
//! Per run: `(len << 4) | alpha4`, followed by a little-endian RGB565 color when
//! `alpha4 != 0`. `len` is 1..=15; `len == 0` means "until the end of the row".

/// Convert an ARGB8888 pixel to `(alpha4, premultiplied RGB565)`, exactly like the SDK.
pub fn argb_to_a4_565(argb: u32) -> (u8, u16) {
    let r = (argb >> 3) & 0x1f_0000;
    let c = (((r | (argb & 0xfc00)) >> 2) | (argb & 0xf8)) >> 3;
    let (a4, col) = if (argb & 0xff00_0000).wrapping_add(0x8000_0000) < 0x0400_0001 {
        (7u32, None)
    } else {
        let a4 = argb >> 28;
        if a4 & 8 != 0 {
            let mut v = c;
            if a4 & 4 == 0 {
                v -= (c >> 2) & 0x39e7;
            }
            if a4 & 2 == 0 {
                v -= (c >> 3) & 0x18e3;
            }
            if a4 & 1 == 0 {
                v -= (c >> 4) & 0x861;
            }
            (a4, Some(v))
        } else {
            (a4, None)
        }
    };
    let col = col.unwrap_or_else(|| {
        let mut v = if a4 & 4 != 0 { (c >> 2) & 0x39e7 } else { 0 };
        if a4 & 2 != 0 {
            v += (c >> 3) & 0x18e3;
        }
        if a4 & 1 != 0 {
            v += (c >> 4) & 0x861;
        }
        v
    });
    (a4 as u8, col as u16)
}

/// RLE-encode a `width × height` ARGB8888 image (row major).
pub fn encode(pixels: &[u32], width: usize, height: usize) -> Vec<u8> {
    assert_eq!(pixels.len(), width * height, "pixel buffer size");
    let mut out = Vec::new();
    for row in pixels.chunks_exact(width) {
        let mut x = 0;
        while x < width {
            let px = argb_to_a4_565(row[x]);
            let mut n = 1;
            while x + n < width && argb_to_a4_565(row[x + n]) == px {
                n += 1;
            }
            let field = if x + n == width {
                x = width;
                0
            } else {
                let f = n.min(15);
                x += f;
                f
            };
            out.push(px.0 | (field as u8) << 4);
            if px.0 != 0 {
                out.extend_from_slice(&px.1.to_le_bytes());
            }
        }
    }
    out
}

/// Decode RLE data back to `(alpha4, rgb565)` pixels (mirror of the firmware decoder).
pub fn decode(data: &[u8], width: usize, height: usize) -> Option<Vec<(u8, u16)>> {
    let mut out = Vec::with_capacity(width * height);
    let mut i = 0;
    for _ in 0..height {
        let mut x = 0;
        while x < width {
            let b = *data.get(i)?;
            i += 1;
            let a4 = b & 0x0f;
            let col = if a4 != 0 {
                let c = u16::from_le_bytes([*data.get(i)?, *data.get(i + 1)?]);
                i += 2;
                c
            } else {
                0
            };
            let n = match (b >> 4) as usize {
                0 => width - x,
                n => n.min(width - x),
            };
            out.extend(std::iter::repeat_n((a4, col), n));
            x += n;
        }
    }
    (i == data.len()).then_some(out)
}

/// Pack 8-bit RGB into opaque ARGB8888.
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xff00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_icon_is_one_byte_per_row() {
        // Captured: `35 01 02 2000` + 32 × 00 (fully transparent icon).
        assert_eq!(encode(&[0u32; 32 * 32], 32, 32), vec![0u8; 32]);
    }

    #[test]
    fn opaque_white_and_rgb565() {
        assert_eq!(argb_to_a4_565(rgb(255, 255, 255)), (15, 0xffff));
        assert_eq!(argb_to_a4_565(rgb(255, 0, 0)), (15, 0xf800));
        assert_eq!(argb_to_a4_565(rgb(0, 255, 0)), (15, 0x07e0));
        assert_eq!(argb_to_a4_565(rgb(0, 0, 255)), (15, 0x001f));
    }

    #[test]
    fn roundtrip() {
        let mut img = vec![0u32; 110 * 16];
        for (i, p) in img.iter_mut().enumerate() {
            *p = if (i / 7) % 3 == 0 {
                rgb(255, 0, 0)
            } else if i % 5 == 0 {
                0
            } else {
                rgb(0, 0, 255)
            };
        }
        let enc = encode(&img, 110, 16);
        let dec = decode(&enc, 110, 16).unwrap();
        let want: Vec<_> = img
            .iter()
            .map(|&p| argb_to_a4_565(p))
            .map(|(a, c)| (a, if a == 0 { 0 } else { c }))
            .collect();
        assert_eq!(dec, want);
    }
}
