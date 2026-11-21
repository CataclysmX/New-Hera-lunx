//! Full-screen custom background (480×272, 8-bit indexed + RGB565 palette).
//!
//! This is also the source of the boot logo: once the 32 tiles are on the device,
//! `Command::SplashFlash { custom: true }` (0x50 0x80) writes the current background to flash.
//!
//! Upload (DLL `FUN_180004ce0` / `FUN_180016140`, firmware 0x33/0x37 handlers):
//! * 0x33 palette (≤ 256 RGB565 entries),
//! * 32 × 0x37 tiles of 4080 bytes: progressive interlacing — tile `t` carries the pixels at
//!   `(8*col + dx, 4*row + dy)` for 68 rows × 60 columns, the firmware fills a `w×h` block
//!   for each sample so the picture refines pass after pass ([`TILES`]).
//! * the firmware keeps a bitmask of received tiles; 0x50 0x80 is refused (status error
//!   0x100) unless all 32 tiles were received.

use crate::cmd::Command;
use crate::{SCREEN_H, SCREEN_W};

/// `(dx, dy, block_w, block_h)` for each of the 32 tiles (DLL table @0x18010fbc0).
pub const TILES: [(u8, u8, u8, u8); 32] = [
    (0, 0, 8, 4),
    (4, 0, 4, 4),
    (0, 2, 4, 2),
    (4, 2, 4, 2),
    (2, 0, 2, 2),
    (6, 0, 2, 2),
    (2, 2, 2, 2),
    (6, 2, 2, 2),
    (0, 1, 2, 1),
    (2, 1, 2, 1),
    (4, 1, 2, 1),
    (6, 1, 2, 1),
    (0, 3, 2, 1),
    (2, 3, 2, 1),
    (4, 3, 2, 1),
    (6, 3, 2, 1),
    (1, 0, 1, 1),
    (3, 0, 1, 1),
    (5, 0, 1, 1),
    (7, 0, 1, 1),
    (1, 2, 1, 1),
    (3, 2, 1, 1),
    (5, 2, 1, 1),
    (7, 2, 1, 1),
    (1, 1, 1, 1),
    (3, 1, 1, 1),
    (5, 1, 1, 1),
    (7, 1, 1, 1),
    (1, 3, 1, 1),
    (3, 3, 1, 1),
    (5, 3, 1, 1),
    (7, 3, 1, 1),
];

/// Bytes per tile (68 rows × 60 samples).
pub const TILE_BYTES: usize = (SCREEN_H / 4) * (SCREEN_W / 8);

/// A 480×272 indexed image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedImage {
    /// RGB565 palette, at most 256 entries.
    pub palette: Vec<u16>,
    /// One palette index per pixel, row major (480 × 272).
    pub pixels: Vec<u8>,
}

fn rgb888_to_565(p: u32) -> u16 {
    (((p >> 8) & 0xf800) | ((p >> 5) & 0x07e0) | ((p >> 3) & 0x001f)) as u16
}

fn expand565(c: u16) -> [i32; 3] {
    let (r, g, b) = ((c >> 11) & 0x1f, (c >> 5) & 0x3f, c & 0x1f);
    [
        ((r << 3) | (r >> 2)) as i32,
        ((g << 2) | (g >> 4)) as i32,
        ((b << 3) | (b >> 2)) as i32,
    ]
}

/// Median-cut quantization of an ARGB8888 480×272 image (alpha ignored) to ≤ 256 colors.
pub fn quantize(argb: &[u32]) -> IndexedImage {
    assert_eq!(
        argb.len(),
        SCREEN_W * SCREEN_H,
        "background must be 480x272"
    );
    // Histogram in RGB565 space (the panel cannot show more anyway).
    let mut hist = vec![0u32; 1 << 16];
    for &p in argb {
        hist[rgb888_to_565(p) as usize] += 1;
    }
    let colors: Vec<(u16, u32)> = hist
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(c, n)| (c as u16, *n))
        .collect();

    let mut boxes: Vec<Vec<(u16, u32)>> = vec![colors];
    while boxes.len() < 256 {
        // split the box with the widest channel range (ties: most pixels)
        let mut best: Option<(usize, usize, i32)> = None;
        for (i, b) in boxes.iter().enumerate() {
            if b.len() < 2 {
                continue;
            }
            for ch in 0..3 {
                let (lo, hi) = b.iter().fold((255, 0), |(lo, hi), (c, _)| {
                    let v = expand565(*c)[ch];
                    (lo.min(v), hi.max(v))
                });
                if best.is_none_or(|(_, _, r)| hi - lo > r) {
                    best = Some((i, ch, hi - lo));
                }
            }
        }
        let Some((i, ch, _)) = best else { break };
        let mut b = boxes.swap_remove(i);
        b.sort_by_key(|(c, _)| expand565(*c)[ch]);
        let total: u64 = b.iter().map(|(_, n)| *n as u64).sum();
        let (mut acc, mut cut) = (0u64, 1);
        for (k, (_, n)) in b.iter().enumerate() {
            acc += *n as u64;
            if acc * 2 >= total {
                cut = (k + 1).clamp(1, b.len() - 1);
                break;
            }
        }
        let tail = b.split_off(cut);
        boxes.push(b);
        boxes.push(tail);
    }

    let palette: Vec<u16> = boxes
        .iter()
        .map(|b| {
            let (mut s, mut n) = ([0u64; 3], 0u64);
            for (c, k) in b {
                let e = expand565(*c);
                for j in 0..3 {
                    s[j] += e[j] as u64 * *k as u64;
                }
                n += *k as u64;
            }
            let avg = |j: usize| (s[j] / n.max(1)) as u32;
            rgb888_to_565(avg(0) << 16 | avg(1) << 8 | avg(2))
        })
        .collect();

    // Nearest palette entry for every color present.
    let pal: Vec<[i32; 3]> = palette.iter().map(|&c| expand565(c)).collect();
    let mut lut = vec![0u8; 1 << 16];
    for (c, n) in hist.iter().enumerate() {
        if *n == 0 {
            continue;
        }
        let e = expand565(c as u16);
        let (idx, _) = pal
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (0..3).map(|j| (p[j] - e[j]).pow(2)).sum::<i32>()))
            .min_by_key(|&(_, d)| d)
            .unwrap();
        lut[c] = idx as u8;
    }
    let pixels = argb
        .iter()
        .map(|&p| lut[rgb888_to_565(p) as usize])
        .collect();
    IndexedImage { palette, pixels }
}

impl IndexedImage {
    /// Samples of tile `t` (0..32), 4080 bytes.
    pub fn tile(&self, t: usize) -> Vec<u8> {
        let (dx, dy, _, _) = TILES[t];
        let mut out = Vec::with_capacity(TILE_BYTES);
        for row in 0..SCREEN_H / 4 {
            for col in 0..SCREEN_W / 8 {
                out.push(self.pixels[(row * 4 + dy as usize) * SCREEN_W + col * 8 + dx as usize]);
            }
        }
        out
    }

    /// Palette command followed by the 32 tile commands (send each in its own frame).
    pub fn commands(&self) -> (Command, Vec<Command>) {
        let palette = Command::Palette {
            offset: 0,
            colors: self.palette.clone(),
        };
        let tiles = (0..32)
            .map(|t| Command::BackgroundTile {
                tile: t as u8,
                data: self.tile(t),
            })
            .collect();
        (palette, tiles)
    }
}

/// Reconstruct the picture the firmware shows after receiving tiles `0..=last` (preview/tests).
pub fn render_progressive(img: &IndexedImage, last: usize) -> Vec<u8> {
    let mut fb = vec![0u8; SCREEN_W * SCREEN_H];
    for (t, &(dx, dy, bw, bh)) in TILES.iter().enumerate().take(last + 1) {
        let data = img.tile(t);
        for row in 0..SCREEN_H / 4 {
            for col in 0..SCREEN_W / 8 {
                let v = data[row * (SCREEN_W / 8) + col];
                for y in 0..bh as usize {
                    for x in 0..bw as usize {
                        let (px, py) = (col * 8 + dx as usize + x, row * 4 + dy as usize + y);
                        if px < SCREEN_W && py < SCREEN_H {
                            fb[py * SCREEN_W + px] = v;
                        }
                    }
                }
            }
        }
    }
    fb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::{command_len, encode_all};
    use crate::rle::rgb;

    fn gradient() -> Vec<u32> {
        (0..SCREEN_W * SCREEN_H)
            .map(|i| {
                let (x, y) = (i % SCREEN_W, i / SCREEN_W);
                rgb((x * 255 / SCREEN_W) as u8, (y * 255 / SCREEN_H) as u8, 128)
            })
            .collect()
    }

    /// FNV-1a 64 of the palette (RGB565 little-endian) then the pixels: the C port
    /// (`driver/c/tests/test_background.c`) must find the same values on the same pictures.
    fn fingerprint(img: &IndexedImage) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let bytes = img
            .palette
            .iter()
            .flat_map(|c| c.to_le_bytes())
            .chain(img.pixels.iter().copied());
        for b in bytes {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
        h
    }

    /// A busier picture than the gradient: more colours than the palette holds, ties in the
    /// median-cut sorts (shared with the C port).
    fn pattern() -> Vec<u32> {
        (0..SCREEN_W * SCREEN_H)
            .map(|i| {
                let (x, y) = (i % SCREEN_W, i / SCREEN_W);
                rgb(
                    ((x * 7 + y * 3) % 256) as u8,
                    ((x ^ y) % 256) as u8,
                    ((x * y / 64) % 256) as u8,
                )
            })
            .collect()
    }

    #[test]
    fn same_result_as_the_c_port() {
        let g = quantize(&gradient());
        let p = quantize(&pattern());
        assert_eq!((g.palette.len(), fingerprint(&g)), (GRADIENT.0, GRADIENT.1));
        assert_eq!((p.palette.len(), fingerprint(&p)), (PATTERN.0, PATTERN.1));
    }

    /// (colours, fingerprint) of `quantize` on `gradient()` and `pattern()`.
    const GRADIENT: (usize, u64) = (256, 7_991_872_763_294_034_825);
    const PATTERN: (usize, u64) = (256, 6_255_433_330_176_470_139);

    #[test]
    fn tiles_cover_every_pixel_exactly_once() {
        let mut seen = [0u8; 8 * 4];
        for (dx, dy, _, _) in TILES {
            seen[dy as usize * 8 + dx as usize] += 1;
        }
        assert!(seen.iter().all(|&n| n == 1));
    }

    #[test]
    fn full_upload_reconstructs_the_image() {
        let img = quantize(&gradient());
        assert!(img.palette.len() <= 256);
        assert_eq!(render_progressive(&img, 31), img.pixels);
    }

    #[test]
    fn few_colors_are_exact() {
        let px: Vec<u32> = (0..SCREEN_W * SCREEN_H)
            .map(|i| {
                if i % 3 == 0 {
                    rgb(255, 0, 0)
                } else {
                    rgb(0, 0, 255)
                }
            })
            .collect();
        let img = quantize(&px);
        assert_eq!(img.palette.len(), 2);
        assert!(img.palette.contains(&0xf800) && img.palette.contains(&0x001f));
    }

    #[test]
    fn commands_match_firmware_lengths() {
        let img = quantize(&gradient());
        let (pal, tiles) = img.commands();
        let p = encode_all(&[pal], false);
        assert_eq!(command_len(&p), Some(p.len()));
        let t = encode_all(&tiles[..1], false);
        assert_eq!(t.len(), 9 + TILE_BYTES);
        assert_eq!(command_len(&t), Some(t.len()));
    }
}
