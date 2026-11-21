//! 480×272 RGB565 framebuffer scanned out by the LTDC.

use core::ptr::addr_of_mut;

pub const W: usize = 480;
pub const H: usize = 272;

static mut FB: [u16; W * H] = [0; W * H];

/// Address given to the LTDC layer.
pub fn fb_addr() -> u32 {
    addr_of_mut!(FB) as u32
}

#[inline(always)]
fn fb() -> *mut u16 {
    addr_of_mut!(FB) as *mut u16
}

/// Write one pixel (ignored outside the screen).
#[inline(always)]
pub fn put(x: usize, y: usize, c: u16) {
    if x < W && y < H {
        // SAFETY: bounds checked; single writer (the USB receive task).
        unsafe { fb().add(y * W + x).write_volatile(c) };
    }
}

/// Copy `px.len() / 2` little-endian RGB565 pixels to row `y` from column `x` (clipped).
/// The CPU is little-endian, so the bytes are already in framebuffer layout.
pub fn span(x: usize, y: usize, px: &[u8]) {
    if x < W && y < H {
        let n = (px.len() / 2).min(W - x);
        // SAFETY: clipped to row y; the FB address escaped to the LTDC so stores are kept.
        unsafe { core::ptr::copy_nonoverlapping(px.as_ptr(), fb().add(y * W + x) as *mut u8, 2 * n) };
    }
}

/// `n` pixels of `c` on row `y` from column `x` (clipped), two pixels per 32-bit store.
pub fn hline(x: usize, y: usize, n: usize, c: u16) {
    if x < W && y < H {
        let n = n.min(W - x);
        // SAFETY: clipped to row y.
        let row = unsafe { core::slice::from_raw_parts_mut(fb().add(y * W + x), n) };
        // SAFETY: every bit pattern is a valid u16 / u32.
        let (head, pairs, tail) = unsafe { row.align_to_mut::<u32>() };
        head.fill(c);
        pairs.fill(u32::from(c) * 0x0001_0001);
        tail.fill(c);
    }
}

/// Fill a rectangle (clipped).
pub fn fill(x: u16, y: u16, w: u16, h: u16, c: u16) {
    let (x0, y0) = (x as usize, y as usize);
    for yy in y0..(y0 + h as usize).min(H) {
        hline(x0, yy, w as usize, c);
    }
}

/// Copy the rectangle (x, y, w, h) so that its top-left corner lands on (to_x, to_y). Both are
/// clipped to the screen (the same amount on both sides); overlapping areas are handled (rows in
/// the right order, `copy` = memmove within a row). Returns the pixels copied.
pub fn copy(x: u16, y: u16, w: u16, h: u16, to_x: u16, to_y: u16) -> u32 {
    let (mut x, mut y, mut w, mut h) = (x as isize, y as isize, w as isize, h as isize);
    let (mut tx, mut ty) = (to_x as isize, to_y as isize);
    // clip the left / top of both rectangles
    for (a, b, len) in [(&mut x, &mut tx, &mut w), (&mut y, &mut ty, &mut h)] {
        let cut = (-(*a).min(0)).max(-(*b).min(0));
        *a += cut;
        *b += cut;
        *len -= cut;
    }
    // right / bottom
    w = w.min(W as isize - x).min(W as isize - tx);
    h = h.min(H as isize - y).min(H as isize - ty);
    if w <= 0 || h <= 0 {
        return 0;
    }
    let (w, h) = (w as usize, h as usize);
    let row = |r: usize| {
        let (sy, dy) = (y as usize + r, ty as usize + r);
        // SAFETY: both row segments clipped to the screen; `copy` allows overlap.
        unsafe { core::ptr::copy(fb().add(sy * W + x as usize), fb().add(dy * W + tx as usize), w) };
    };
    if ty > y {
        (0..h).rev().for_each(row); // moving down: bottom rows first
    } else {
        (0..h).for_each(row);
    }
    (w * h) as u32
}

#[cfg_attr(bootlogo, allow(dead_code))] // only the fallback boot screen uses it
pub const fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xf8) << 8) | ((g as u16 & 0xfc) << 3) | (b as u16 >> 3)
}

/// Shown until the host draws something: dark background + colour strip.
/// Boot logo baked in by build.rs (RLE runs `[count u8][color u16 LE]`, 480×272).
#[cfg(bootlogo)]
static BOOTLOGO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bootlogo.rle"));

#[cfg(bootlogo)]
pub fn boot_screen() {
    let mut i = 0;
    for run in BOOTLOGO.chunks_exact(3) {
        let end = (i + run[0] as usize).min(W * H);
        // SAFETY: i <= end <= W*H, inside the framebuffer.
        unsafe { core::slice::from_raw_parts_mut(fb().add(i), end - i) }.fill(u16::from_le_bytes([run[1], run[2]]));
        i = end;
    }
}

/// Stroke animation precomputed by build.rs: `[u16 frames]`, then per frame `[u16 runs]` + runs
/// `[offset u24][len u8]` (pixels revealed in that frame).
#[cfg(bootlogo)]
static BOOTANIM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bootanim.bin"));

/// Trace colour #006C68 (0x036D) and pen tip #007C78 (0x03EF, same hue, one step brighter):
/// picked on the panel itself with tools/fw_swatches.py (the logo's #00674F looked fluo green on
/// this TFT).
#[cfg(bootlogo)]
pub const TRACE: u16 = 0x036D;
#[cfg(bootlogo)]
pub const PEN_TIP: u16 = 0x03EF;

fn split565(c: u16) -> (i32, i32, i32) {
    ((c >> 11) as i32 & 31, (c >> 5) as i32 & 63, c as i32 & 31)
}

/// `a` → `b` at k/n, per RGB565 channel.
#[cfg(bootlogo)]
fn lerp565(a: u16, b: u16, k: u32, n: u32) -> u16 {
    let ((ar, ag, ab), (br, bg, bb)) = (split565(a), split565(b));
    let (k, n) = (k as i32, n as i32);
    let mix = |x: i32, y: i32| (x + (y - x) * k / n) as u16;
    mix(ar, br) << 11 | mix(ag, bg) << 5 | mix(ab, bb)
}

#[cfg(bootlogo)]
pub fn anim_frames() -> u16 {
    u16::from_le_bytes([BOOTANIM[0], BOOTANIM[1]])
}

/// Byte position of the first animation frame.
#[cfg(bootlogo)]
pub const ANIM_START: usize = 2;

/// Draw the pixels of the frame at `pos` in `color`; return the next frame's position.
#[cfg(bootlogo)]
pub fn anim_draw(mut pos: usize, color: u16) -> usize {
    let runs = u16::from_le_bytes([BOOTANIM[pos], BOOTANIM[pos + 1]]);
    pos += 2;
    for _ in 0..runs {
        let off = BOOTANIM[pos] as usize | (BOOTANIM[pos + 1] as usize) << 8 | (BOOTANIM[pos + 2] as usize) << 16;
        let len = BOOTANIM[pos + 3] as usize;
        pos += 4;
        if off < W * H {
            let len = len.min(W * H - off);
            // SAFETY: clipped to the framebuffer.
            unsafe { core::slice::from_raw_parts_mut(fb().add(off), len) }.fill(color);
        }
    }
    pos
}

/// Keep in sync with build.rs `STROKE_LUM`: stroke pixels are drawn by the animation.
#[cfg(bootlogo)]
fn is_stroke(c: u16) -> bool {
    let (r, g, b) = ((c >> 11) as u32 & 31, (c >> 5) as u32 & 63, c as u32 & 31);
    (r * 255 / 31).max(g * 255 / 63).max(b * 255 / 31) >= 190
}

/// Light-up step `k` of `steps`: strokes go from the trace green to their real colour, the halo
/// (every other logo pixel) from black to its real colour.
#[cfg(bootlogo)]
pub fn glow(k: u32, steps: u32) {
    let mut i = 0;
    for run in BOOTLOGO.chunks_exact(3) {
        let end = (i + run[0] as usize).min(W * H);
        let c = u16::from_le_bytes([run[1], run[2]]);
        let from = if is_stroke(c) { TRACE } else { 0 };
        // SAFETY: i <= end <= W*H.
        unsafe { core::slice::from_raw_parts_mut(fb().add(i), end - i) }.fill(lerp565(from, c, k, steps));
        i = end;
    }
}

/// Screen-saver sprite baked in by build.rs: `[w u16][h u16]` (transparent margin included), then
/// rows of 4-bit alpha (left pixel in the high nibble, rows padded to whole bytes).
static SAVER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/saver.bin"));

/// Size of the screen-saver sprite, margin included.
pub fn saver_size() -> (usize, usize) {
    let w = u16::from_le_bytes([SAVER[0], SAVER[1]]) as usize;
    let h = u16::from_le_bytes([SAVER[2], SAVER[3]]) as usize;
    (w, h)
}

/// Draw the screen-saver sprite with its top-left corner at (x, y), in `color` scaled by its alpha
/// onto black (clipped).
pub fn saver_draw(x: usize, y: usize, color: u16) {
    let (w, h) = saver_size();
    if x >= W || y >= H {
        return;
    }
    let (r, g, b) = split565(color);
    let ramp: [u16; 16] = core::array::from_fn(|a| {
        let k = a as i32;
        ((r * k / 15) << 11 | (g * k / 15) << 5 | (b * k / 15)) as u16
    });
    let (row_bytes, n) = (w.div_ceil(2), w.min(W - x));
    for yy in 0..h.min(H - y) {
        let src = &SAVER[4 + yy * row_bytes..][..row_bytes];
        // SAFETY: row y + yy < H, columns x..x + n with n <= W - x.
        let row = unsafe { core::slice::from_raw_parts_mut(fb().add((y + yy) * W + x), n) };
        for (i, px) in row.iter_mut().enumerate() {
            let a = if i % 2 == 0 { src[i / 2] >> 4 } else { src[i / 2] & 15 };
            *px = ramp[a as usize];
        }
    }
}

/// Fallback when no logo is configured: colour strip test screen.
#[cfg(not(bootlogo))]
pub fn boot_screen() {
    fill(0, 0, W as u16, H as u16, rgb565(16, 16, 32));
    const STRIP: [u16; 4] = [rgb565(255, 64, 64), rgb565(64, 255, 64), rgb565(64, 128, 255), rgb565(255, 200, 0)];
    for (i, c) in STRIP.iter().enumerate() {
        fill(20 + i as u16 * 112, 230, 104, 20, *c);
    }
    fill(20, 120, 440, 4, rgb565(255, 255, 255));
}
