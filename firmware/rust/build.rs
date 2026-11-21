//! - memory.x on the linker search path (cortex-m-rt's link.x includes it);
//! - optional boot logo: `$HSM01_BOOTLOGO`, else the newest `../../public/bootlogo*.{png,jpeg,jpg}`, fitted
//!   (letterboxed on black) into 480×272, RGB565, RLE runs `[count u8][color u16 LE]`
//!   → `$OUT_DIR/bootlogo.rle` + `cfg(bootlogo)`, drawn by `gfx::boot_screen`;
//! - screen-saver badge « SIGNAL NOT FOUND » (pixel font, frame, glow; 4-bit alpha) →
//!   `$OUT_DIR/saver.bin`.
use std::{
    env, fs,
    path::{Path, PathBuf},
};

use image::{RgbImage, imageops};

const W: u32 = 480;
const H: u32 = 272;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rustc-link-arg=-Tlink.x");

    println!("cargo::rustc-check-cfg=cfg(bootlogo)");
    println!("cargo:rerun-if-env-changed=HSM01_BOOTLOGO");
    println!("cargo:rerun-if-changed=../../public");
    let logo = env::var_os("HSM01_BOOTLOGO").map(PathBuf::from).or_else(newest_public_logo);
    fs::write(out.join("saver.bin"), saver_sprite()).unwrap();
    if let Some(path) = logo {
        let px = logo_pixels(&path);
        let rle = encode(&px);
        let anim = animation(&px);
        fs::write(out.join("bootlogo.rle"), &rle).unwrap();
        fs::write(out.join("bootanim.bin"), &anim).unwrap();
        println!("cargo:rustc-cfg=bootlogo");
        println!(
            "cargo:warning=boot logo {} -> {} bytes of RLE, stroke animation {} frames / {} bytes",
            path.display(),
            rle.len(),
            u16::from_le_bytes([anim[0], anim[1]]),
            anim.len()
        );
    }
}

/// Most recently modified `../../public/bootlogo*.{png,jpeg,jpg}`.
fn newest_public_logo() -> Option<PathBuf> {
    fs::read_dir("../../public")
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            name.starts_with("bootlogo") && ["png", "jpeg", "jpg"].contains(&ext.as_str())
        })
        .max_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
}

fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xf8) << 8) | ((g as u16 & 0xfc) << 3) | (b as u16 >> 3)
}

/// Logo fitted (letterboxed on black) into 480×272, RGB565, row-major.
fn logo_pixels(path: &Path) -> Vec<u16> {
    let img = image::open(path)
        .unwrap_or_else(|e| panic!("boot logo {}: {e}", path.display()))
        .to_rgb8();
    let scale = f64::min(W as f64 / img.width() as f64, H as f64 / img.height() as f64);
    let w = ((img.width() as f64 * scale).round() as u32).clamp(1, W);
    let h = ((img.height() as f64 * scale).round() as u32).clamp(1, H);
    let small = imageops::resize(&img, w, h, imageops::FilterType::Lanczos3);
    let mut canvas = RgbImage::new(W, H); // black
    imageops::overlay(&mut canvas, &small, ((W - w) / 2) as i64, ((H - h) / 2) as i64);
    canvas.pixels().map(|p| rgb565(p[0], p[1], p[2])).collect()
}

/// Keep in sync with `gfx::is_stroke`: brightest channel (8-bit expanded) ≥ STROKE_LUM.
const STROKE_LUM: u32 = 190;
/// Pen speed along a stroke (px of geodesic distance per frame) and left→right start sweep.
const PEN_PX_PER_FRAME: u32 = 4;
const SWEEP_FRAMES: u32 = 60;

fn lum565(c: u16) -> u32 {
    let (r, g, b) = ((c >> 11) as u32 & 31, (c >> 5) as u32 & 63, c as u32 & 31);
    (r * 255 / 31).max(g * 255 / 63).max(b * 255 / 31)
}

/// Stroke-drawing animation: each 8-connected stroke component is revealed from its leftmost
/// pixel along its shape (BFS distance), components start in a left→right sweep.
/// Format: `[u16 frames]` then per frame `[u16 runs]` + runs `[offset u24][len u8]` (pixels
/// revealed in that frame; the firmware draws them in the trace colour, real colours come later).
fn animation(px: &[u16]) -> Vec<u8> {
    let (w, h) = (W as usize, H as usize);
    let stroke: Vec<bool> = px.iter().map(|&c| lum565(c) >= STROKE_LUM).collect();
    let mut frame_of = vec![u32::MAX; w * h];
    let mut seen = vec![false; w * h];
    let neighbours = |p: usize| {
        let (x, y) = ((p % w) as i32, (p / w) as i32);
        (-1..=1)
            .flat_map(move |dy| (-1..=1).map(move |dx| (x + dx, y + dy)))
            .filter(move |&(nx, ny)| (nx, ny) != (x, y) && nx >= 0 && ny >= 0 && nx < W as i32 && ny < H as i32)
            .map(move |(nx, ny)| ny as usize * w + nx as usize)
    };
    for s in 0..w * h {
        if !stroke[s] || seen[s] {
            continue;
        }
        // component
        let mut comp = vec![s];
        seen[s] = true;
        let mut i = 0;
        while i < comp.len() {
            for q in neighbours(comp[i]) {
                if stroke[q] && !seen[q] {
                    seen[q] = true;
                    comp.push(q);
                }
            }
            i += 1;
        }
        // geodesic distance from the leftmost (then topmost) pixel
        let start = *comp.iter().min_by_key(|&&p| (p % w, p / w)).unwrap();
        let first = (start % w) as u32 * SWEEP_FRAMES / W;
        let mut dist = std::collections::HashMap::from([(start, 0u32)]);
        let mut queue = std::collections::VecDeque::from([start]);
        while let Some(p) = queue.pop_front() {
            let d = dist[&p];
            frame_of[p] = first + d / PEN_PX_PER_FRAME;
            for q in neighbours(p) {
                if stroke[q] && !dist.contains_key(&q) {
                    dist.insert(q, d + 1);
                    queue.push_back(q);
                }
            }
        }
    }
    let frames = frame_of.iter().filter(|&&f| f != u32::MAX).max().map_or(0, |&f| f + 1);
    let mut out = (frames as u16).to_le_bytes().to_vec();
    for f in 0..frames {
        let mut runs: Vec<(usize, usize)> = Vec::new(); // (offset, len)
        for p in (0..w * h).filter(|&p| frame_of[p] == f) {
            match runs.last_mut() {
                Some((o, l)) if *o + *l == p && *l < 255 => *l += 1,
                _ => runs.push((p, 1)),
            }
        }
        out.extend_from_slice(&(runs.len() as u16).to_le_bytes());
        for (o, l) in runs {
            out.extend_from_slice(&(o as u32).to_le_bytes()[..3]);
            out.push(l as u8);
        }
    }
    out
}

/// Screen-saver badge: `SAVER_TEXT` in a 5×7 pixel font scaled ×`SAVER_SCALE`, in a frame with
/// chamfered corners, a soft glow around; then a transparent margin: drawn one pixel further at
/// each step, it erases where the badge was.
const SAVER_TEXT: &str = "SIGNAL NOT FOUND";
const SAVER_SCALE: u32 = 2;
const SAVER_PAD: u32 = 2;
/// Text–frame gap (x, y), frame thickness, glow width (px).
const SAVER_GAP: (u32, u32) = (10, 8);
const SAVER_BORDER: u32 = 2;
const SAVER_GLOW: u32 = 2;

/// 5×7 glyphs (rows top to bottom, bit 4 = left column) of the badge's letters.
fn glyph(c: char) -> [u8; 7] {
    match c {
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'I' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111],
        'G' => [0b01111, 0b10000, 0b10000, 0b10011, 0b10001, 0b10001, 0b01111],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        ' ' => [0; 7],
        _ => panic!("saver badge: no glyph for {c:?}"),
    }
}

/// Screen-saver sprite (`gfx::saver_draw`), a 4-bit alpha that tints it: text 15, frame 11,
/// glow 5 then 2 away from them. Format: `[w u16][h u16]` (margin included), then rows of nibbles
/// (left pixel in the high nibble, rows padded to whole bytes).
fn saver_sprite() -> Vec<u8> {
    let s = SAVER_SCALE;
    let text_w = SAVER_TEXT.chars().count() as u32 * 6 * s - s;
    let (bw, bh) = (text_w + 2 * (SAVER_GAP.0 + SAVER_BORDER), 7 * s + 2 * (SAVER_GAP.1 + SAVER_BORDER));
    let (w, h) = (bw + 2 * SAVER_GLOW, bh + 2 * SAVER_GLOW);
    let mut core = vec![0u8; (w * h) as usize];
    // frame, corners cut at 45°
    for y in 0..bh {
        for x in 0..bw {
            let edge = x < SAVER_BORDER || y < SAVER_BORDER || x >= bw - SAVER_BORDER || y >= bh - SAVER_BORDER;
            let (cx, cy) = (x.min(bw - 1 - x), y.min(bh - 1 - y));
            if edge && cx + cy >= 2 * SAVER_BORDER - 1 {
                core[((y + SAVER_GLOW) * w + x + SAVER_GLOW) as usize] = 11;
            }
        }
    }
    // text
    let (tx, ty) = (SAVER_GLOW + SAVER_BORDER + SAVER_GAP.0, SAVER_GLOW + SAVER_BORDER + SAVER_GAP.1);
    for (i, c) in SAVER_TEXT.chars().enumerate() {
        for (row, bits) in glyph(c).iter().enumerate() {
            for col in 0..5u32 {
                if bits >> (4 - col) & 1 == 0 {
                    continue;
                }
                for (dy, dx) in (0..s).flat_map(|dy| (0..s).map(move |dx| (dy, dx))) {
                    let (x, y) = (tx + (i as u32 * 6 + col) * s + dx, ty + row as u32 * s + dy);
                    core[(y * w + x) as usize] = 15;
                }
            }
        }
    }
    // glow: 5 next to a lit pixel, 2 one further
    let lit = |x: i64, y: i64| x >= 0 && y >= 0 && x < w as i64 && y < h as i64 && core[(y * w as i64 + x) as usize] > 0;
    let alpha: Vec<u8> = (0..h as i64)
        .flat_map(|y| (0..w as i64).map(move |x| (x, y)))
        .map(|(x, y)| {
            let near = |d: i64| (-d..=d).any(|dy| (-d..=d).any(|dx| lit(x + dx, y + dy)));
            let c = core[(y * w as i64 + x) as usize];
            if c > 0 { c } else if near(1) { 5 } else if near(2) { 2 } else { 0 }
        })
        .collect();
    let (pw, ph) = (w + 2 * SAVER_PAD, h + 2 * SAVER_PAD);
    let at = |x: u32, y: u32| -> u8 {
        let inside = (SAVER_PAD..SAVER_PAD + w).contains(&x) && (SAVER_PAD..SAVER_PAD + h).contains(&y);
        if inside { alpha[((y - SAVER_PAD) * w + x - SAVER_PAD) as usize] } else { 0 }
    };
    let mut out = Vec::new();
    out.extend_from_slice(&(pw as u16).to_le_bytes());
    out.extend_from_slice(&(ph as u16).to_le_bytes());
    for y in 0..ph {
        for x in (0..pw).step_by(2) {
            out.push(at(x, y) << 4 | at(x + 1, y));
        }
    }
    out
}

fn encode(px: &[u16]) -> Vec<u8> {
    let mut rle = Vec::new();
    let mut i = 0;
    while i < px.len() {
        let mut n = 1;
        while n < 255 && i + n < px.len() && px[i + n] == px[i] {
            n += 1;
        }
        rle.push(n as u8);
        rle.extend_from_slice(&px[i].to_le_bytes());
        i += n;
    }
    rle
}
