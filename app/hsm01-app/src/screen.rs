//! Sends only what changed: the frame is compared with the previous one in 16×16 tiles; changed
//! tiles of a tile row are merged into horizontal spans, each sent as BLIT or BLIT_RLE (smaller).
//! At most `budget` bytes per call: what is left stays pending (compared again next call), so a
//! full-screen change (animated wallpaper, new connection) is spread over several frames instead
//! of blocking the loop – and the inputs – for a third of a second.
//!
//! Scrolling (firmware 240.21+): content already on the device's screen, moved up / down / left /
//! right, is found first (`moves`) and moved there with COPY, so that only what appeared is sent.
//! Every line of a move is checked pixel for pixel: a wrong guess cannot show a wrong image.

use std::collections::HashMap;

use hsm01_proto::free;

use crate::config::{SCREEN_H, SCREEN_W};

pub const TILE: usize = 16;
const W: usize = SCREEN_W as usize;
const H: usize = SCREEN_H as usize;
const TILES_X: usize = W / TILE;
const TILES_Y: usize = H.div_ceil(TILE);

pub struct Screen {
    /// What the device shows (as far as tiles are concerned).
    prev: Vec<u16>,
    /// Tiles to send whatever `prev` says (new connection).
    dirty: Vec<bool>,
    /// The firmware knows COPY: look for moved content.
    pub copy: bool,
}

impl Default for Screen {
    fn default() -> Self {
        Screen {
            prev: vec![0; W * H],
            dirty: vec![true; TILES_X * TILES_Y],
            copy: false,
        }
    }
}

/// (x, y, w, h)
type Rect = (i32, i32, i32, i32);

/// Content moved on the screen: the rectangle (x, y, w, h) goes to (to_x, to_y).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Move {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    to_x: usize,
    to_y: usize,
}

/// Direction of the moves looked for. Vertical: "lines" are pixel rows, cut into "strips" of
/// 16 columns; horizontal: lines are pixel columns, cut into strips of 16 rows.
#[derive(Clone, Copy)]
enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    /// (lines, strips)
    fn size(self) -> (usize, usize) {
        match self {
            Axis::Vertical => (H, W / TILE),
            Axis::Horizontal => (W, H / TILE),
        }
    }

    /// Index of pixel `k` (0..16 × strips) across the strips, on line `l`.
    fn at(self, k: usize, l: usize) -> usize {
        match self {
            Axis::Vertical => l * W + k,
            Axis::Horizontal => k * W + l,
        }
    }

    /// Strips [s0, s1), lines [a, b) as a screen rectangle.
    fn rect(self, (s0, s1): (usize, usize), (a, b): (usize, usize)) -> (usize, usize, usize, usize) {
        let (k0, k1) = (s0 * TILE, s1 * TILE);
        match self {
            Axis::Vertical => (k0, a, k1 - k0, b - a),
            Axis::Horizontal => (a, k0, b - a, k1 - k0),
        }
    }
}

/// Lines a move must cover at least, and changed lines it must save at least.
const MIN_RUN: usize = 8;
const MIN_GAIN: usize = 4;

/// Hash of the 16 pixels of strip `s` on line `l`, and whether they are all the same colour
/// (such segments match anywhere: they do not vote).
fn segment(buf: &[u16], axis: Axis, s: usize, l: usize) -> (u64, bool) {
    let first = buf[axis.at(s * TILE, l)];
    let (mut h, mut flat) = (0xcbf2_9ce4_8422_2325u64, true);
    for k in s * TILE..(s + 1) * TILE {
        let p = buf[axis.at(k, l)];
        flat &= p == first;
        h = (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3);
    }
    (h, flat)
}

/// Moves along `axis` that turn `prev` (on the device) into `frame`, outside the `skip` boxes.
fn find_moves(prev: &[u16], frame: &[u16], skip: &[Rect], axis: Axis) -> Vec<Move> {
    let (lines, strips) = axis.size();
    // shift voted by each strip: changed lines of the frame found elsewhere in `prev`
    let mut shift: Vec<Option<isize>> = vec![None; strips];
    let mut votes = vec![0u16; 2 * lines];
    let mut seen: HashMap<u64, Vec<usize>> = HashMap::new();
    for (s, best) in shift.iter_mut().enumerate() {
        let old: Vec<(u64, bool)> = (0..lines).map(|l| segment(prev, axis, s, l)).collect();
        let new: Vec<(u64, bool)> = (0..lines).map(|l| segment(frame, axis, s, l)).collect();
        if old == new {
            continue;
        }
        seen.clear();
        for (l, &(h, flat)) in old.iter().enumerate() {
            let at = seen.entry(h).or_default();
            if !flat && at.len() < 4 {
                at.push(l);
            }
        }
        votes.fill(0);
        for (l, &(h, flat)) in new.iter().enumerate() {
            if flat || h == old[l].0 {
                continue;
            }
            for &from in seen.get(&h).into_iter().flatten() {
                votes[l + lines - from] += 1; // shift + lines, never 0 (h differs from old[l])
            }
        }
        if let Some((i, &v)) = votes.iter().enumerate().max_by_key(|&(_, v)| *v)
            && v >= 3
        {
            *best = Some(i as isize - lines as isize);
        }
    }
    // adjacent strips with the same shift move together; keep their longest useful run of lines
    let mut moves = Vec::new();
    let mut s0 = 0;
    while s0 < strips {
        let Some(d) = shift[s0] else {
            s0 += 1;
            continue;
        };
        let mut s1 = s0 + 1;
        while s1 < strips && shift[s1] == Some(d) {
            s1 += 1;
        }
        if let Some(m) = best_run(prev, frame, skip, axis, (s0, s1), d) {
            moves.push(m);
        }
        s0 = s1;
    }
    moves
}

/// The run of lines of strips [s0, s1) that equal the lines `d` before them in `prev` and saves
/// the most changed lines.
fn best_run(
    prev: &[u16],
    frame: &[u16],
    skip: &[Rect],
    axis: Axis,
    strips: (usize, usize),
    d: isize,
) -> Option<Move> {
    let (lines, _) = axis.size();
    let ks = strips.0 * TILE..strips.1 * TILE;
    let in_skip = |l: usize| {
        let (x, y, w, h) = axis.rect(strips, (l, l + 1));
        let (x, y, w, h) = (x as i32, y as i32, w as i32, h as i32);
        skip.iter()
            .any(|&(sx, sy, sw, sh)| x < sx + sw && sx < x + w && y < sy + sh && sy < y + h)
    };
    let usable = |l: usize| {
        let from = l as isize - d;
        (0..lines as isize).contains(&from)
            && !in_skip(l)
            && !in_skip(from as usize)
            && ks
                .clone()
                .all(|k| frame[axis.at(k, l)] == prev[axis.at(k, from as usize)])
    };
    let changed = |l: usize| ks.clone().any(|k| frame[axis.at(k, l)] != prev[axis.at(k, l)]);
    // best run (first line, end, gain) and the run in progress (first line, gain)
    let mut best: Option<(usize, usize, usize)> = None;
    let mut run: Option<(usize, usize)> = None;
    for l in 0..=lines {
        if l < lines && usable(l) {
            let (a, gain) = run.unwrap_or((l, 0));
            run = Some((a, gain + usize::from(changed(l))));
            continue;
        }
        if let Some((a, gain)) = run.take()
            && l - a >= MIN_RUN
            && gain >= MIN_GAIN
            && best.is_none_or(|(_, _, g)| gain > g)
        {
            best = Some((a, l, gain));
        }
    }
    let (a, b, _) = best?;
    let (x, y, w, h) = axis.rect(strips, (a, b));
    let (dx, dy) = match axis {
        Axis::Vertical => (0, d),
        Axis::Horizontal => (d, 0),
    };
    Some(Move {
        x: (x as isize - dx) as usize,
        y: (y as isize - dy) as usize,
        w,
        h,
        to_x: x,
        to_y: y,
    })
}

/// The same copy as the firmware's (rows in the right order when the areas overlap).
fn apply(buf: &mut [u16], m: Move) {
    let row = |buf: &mut [u16], r: usize| {
        let src = (m.y + r) * W + m.x;
        buf.copy_within(src..src + m.w, (m.to_y + r) * W + m.to_x);
    };
    if m.to_y > m.y {
        (0..m.h).rev().for_each(|r| row(buf, r));
    } else {
        (0..m.h).for_each(|r| row(buf, r));
    }
}

impl Screen {
    /// Next updates resend everything (new connection).
    pub fn invalidate(&mut self) {
        self.dirty.fill(true);
    }

    /// Move what the device already shows to where the frame has it (COPY), vertical moves
    /// first, then horizontal ones (on the result).
    fn moves(&mut self, frame: &[u16], skip: &[Rect], out: &mut Vec<u8>) {
        if !self.copy || self.dirty.iter().any(|&d| d) || self.prev == frame {
            return;
        }
        for axis in [Axis::Vertical, Axis::Horizontal] {
            for m in find_moves(&self.prev, frame, skip, axis) {
                let c = |v: usize| v as u16;
                free::copy(c(m.x), c(m.y), c(m.w), c(m.h), c(m.to_x), c(m.to_y), out);
                apply(&mut self.prev, m);
            }
        }
    }

    fn tile_differs(&self, frame: &[u16], tx: usize, ty: usize) -> bool {
        let (x0, y0) = (tx * TILE, ty * TILE);
        (y0..(y0 + TILE).min(H)).any(|y| {
            frame[y * W + x0..y * W + x0 + TILE] != self.prev[y * W + x0..y * W + x0 + TILE]
        })
    }

    /// Append the commands for what changed, up to about `budget` bytes (at least one span);
    /// returns the number of tiles sent and whether everything is up to date. Tiles entirely
    /// inside a `skip` rectangle (x, y, w, h) are left out: their content is sent another way
    /// (spectrum fills) but kept in the reference copy.
    pub fn update(
        &mut self,
        frame: &[u16],
        skip: &[(i32, i32, i32, i32)],
        out: &mut Vec<u8>,
        budget: usize,
    ) -> (usize, bool) {
        assert_eq!(frame.len(), W * H);
        self.moves(frame, skip, out);
        let skipped = |tx: usize, ty: usize| {
            let (x0, y0) = ((tx * TILE) as i32, (ty * TILE) as i32);
            skip.iter().any(|&(x, y, w, h)| {
                x0 >= x && y0 >= y && x0 + TILE as i32 <= x + w && y0 + TILE as i32 <= y + h
            })
        };
        let needs = |s: &Self, tx: usize, ty: usize| {
            !skipped(tx, ty) && (s.dirty[ty * TILES_X + tx] || s.tile_differs(frame, tx, ty))
        };
        let start_len = out.len();
        let mut changed = 0;
        let mut complete = true;
        let mut px = Vec::new();
        'rows: for ty in 0..TILES_Y {
            let (y0, y1) = (ty * TILE, ((ty + 1) * TILE).min(H));
            let mut tx = 0;
            while tx < TILES_X {
                if skipped(tx, ty) {
                    // drawn by the spectrum fills: the reference follows the frame
                    for y in y0..y1 {
                        let r = y * W + tx * TILE..y * W + (tx + 1) * TILE;
                        self.prev[r.clone()].copy_from_slice(&frame[r]);
                    }
                    tx += 1;
                    continue;
                }
                if !needs(self, tx, ty) {
                    tx += 1;
                    continue;
                }
                if out.len() - start_len >= budget {
                    complete = false;
                    break 'rows;
                }
                let start = tx;
                while tx < TILES_X && needs(self, tx, ty) {
                    tx += 1;
                }
                let (x0, x1) = (start * TILE, tx * TILE);
                px.clear();
                for y in y0..y1 {
                    px.extend_from_slice(&frame[y * W + x0..y * W + x1]);
                    self.prev[y * W + x0..y * W + x1]
                        .copy_from_slice(&frame[y * W + x0..y * W + x1]);
                }
                self.dirty[ty * TILES_X + start..ty * TILES_X + tx].fill(false);
                free::blit_auto(
                    x0 as u16,
                    y0 as u16,
                    (x1 - x0) as u16,
                    (y1 - y0) as u16,
                    &px,
                    out,
                );
                changed += tx - start;
            }
        }
        (changed, complete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (op, x, y, w, h) of each message in a command stream.
    fn rects(stream: &[u8]) -> Vec<(u8, u16, u16, u16, u16)> {
        let (mut i, mut out) = (0, vec![]);
        while i < stream.len() {
            assert_eq!(stream[i], 0xA5);
            let len = u32::from_le_bytes(stream[i + 4..i + 8].try_into().unwrap()) as usize;
            let p = &stream[i + 8..i + 8 + len];
            let v = |k: usize| u16::from_le_bytes([p[2 * k], p[2 * k + 1]]);
            out.push((stream[i + 1], v(0), v(1), v(2), v(3)));
            i += 8 + len;
        }
        out
    }

    #[test]
    fn only_changed_tiles_are_sent() {
        let mut s = Screen::default();
        let mut frame = vec![0u16; W * H];
        let mut out = vec![];
        assert_eq!(
            s.update(&frame, &[], &mut out, usize::MAX).0,
            30 * 17,
            "first frame is complete"
        );
        assert_eq!(rects(&out).len(), 17, "one span per tile row");

        out.clear();
        assert_eq!(s.update(&frame, &[], &mut out, usize::MAX).0, 0);
        assert!(out.is_empty());

        frame[100 * W + 200] = 0xffff; // tile (12, 6)
        frame[100 * W + 220] = 0xffff; // tile (13, 6): same span
        out.clear();
        assert_eq!(s.update(&frame, &[], &mut out, usize::MAX).0, 2);
        let r = rects(&out);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].1, r[0].2, r[0].3, r[0].4), (192, 96, 32, 16));
    }

    /// The device: applies a command stream to its framebuffer (like the firmware).
    fn play(fb: &mut [u16], stream: &[u8]) {
        let mut i = 0;
        while i < stream.len() {
            let (op, len) = (
                stream[i + 1],
                u32::from_le_bytes(stream[i + 4..i + 8].try_into().unwrap()) as usize,
            );
            let p = &stream[i + 8..i + 8 + len];
            let v = |k: usize| u16::from_le_bytes([p[2 * k], p[2 * k + 1]]) as usize;
            let (x, y, w, h) = (v(0), v(1), v(2), v(3));
            match op {
                free::op::FILL => {
                    for yy in y..y + h {
                        fb[yy * W + x..yy * W + x + w].fill(v(4) as u16);
                    }
                }
                free::op::BLIT => {
                    for (n, px) in p[8..].as_chunks::<2>().0.iter().enumerate() {
                        fb[(y + n / w) * W + x + n % w] = u16::from_le_bytes([px[0], px[1]]);
                    }
                }
                free::op::BLIT_RLE => {
                    let mut n = 0;
                    for run in p[8..].as_chunks::<3>().0 {
                        let c = u16::from_le_bytes([run[1], run[2]]);
                        for _ in 0..run[0] {
                            fb[(y + n / w) * W + x + n % w] = c;
                            n += 1;
                        }
                    }
                }
                free::op::COPY => {
                    let m = Move { x, y, w, h, to_x: v(4), to_y: v(5) };
                    apply(fb, m);
                }
                _ => panic!("op {op:#x}"),
            }
            i += 8 + len;
        }
    }

    /// A frame without flat runs (nothing matches by chance), `seed` changes it.
    fn noise(seed: usize) -> Vec<u16> {
        (0..W * H)
            .map(|i| (((i + seed) * 2_654_435_761) >> 7) as u16)
            .collect()
    }

    /// `frame` with the box (x, y, w, h) scrolled by (dx, dy), the uncovered part new content.
    fn scrolled(frame: &[u16], (x, y, w, h): (usize, usize, usize, usize), dx: isize, dy: isize) -> Vec<u16> {
        let fresh = noise(99);
        let mut out = frame.to_vec();
        for yy in y..y + h {
            for xx in x..x + w {
                let (sx, sy) = (xx as isize - dx, yy as isize - dy);
                let inside = (x as isize..(x + w) as isize).contains(&sx)
                    && (y as isize..(y + h) as isize).contains(&sy);
                out[yy * W + xx] = if inside {
                    frame[sy as usize * W + sx as usize]
                } else {
                    fresh[yy * W + xx]
                };
            }
        }
        out
    }

    /// Bytes sent and the device's image after `a` then `b`.
    fn send_two(a: &[u16], b: &[u16], copy: bool, skip: &[Rect]) -> (usize, Vec<u16>, Vec<u8>) {
        let mut s = Screen { copy, ..Default::default() };
        let mut fb = vec![0u16; W * H];
        let mut out = vec![];
        s.update(a, skip, &mut out, usize::MAX);
        play(&mut fb, &out);
        out.clear();
        s.update(b, skip, &mut out, usize::MAX);
        play(&mut fb, &out);
        (out.len(), fb, out)
    }

    #[test]
    fn scrolling_is_sent_as_a_copy_plus_what_appeared() {
        let a = noise(1);
        let boxed = (40, 40, 300, 190); // not aligned on tiles
        for (dx, dy) in [(0, -7), (0, 12), (-9, 0), (5, 0)] {
            let b = scrolled(&a, boxed, dx, dy);
            let (with, fb, out) = send_two(&a, &b, true, &[]);
            assert!(fb == b, "device image wrong for ({dx}, {dy})");
            assert!(rects(&out).iter().any(|r| r.0 == free::op::COPY), "no COPY for ({dx}, {dy})");
            let (without, fb, _) = send_two(&a, &b, false, &[]);
            assert!(fb == b);
            assert!(with * 4 < without, "({dx}, {dy}): {with} bytes with COPY, {without} without");
        }
    }

    #[test]
    fn no_copy_without_firmware_support_nor_over_a_skipped_area() {
        let a = noise(1);
        let b = scrolled(&a, (0, 0, W, H), 0, -10);
        let (_, _, out) = send_two(&a, &b, false, &[]);
        assert!(rects(&out).iter().all(|r| r.0 != free::op::COPY));
        // a spectrum box in the middle: moves stay clear of it, the image is still right
        let skip = [(160, 96, 160, 64)];
        let (_, fb, out) = send_two(&a, &b, true, &skip);
        for r in rects(&out).iter().filter(|r| r.0 == free::op::COPY) {
            // the source rows must not meet the box
            let (y0, y1) = (r.2 as i32, (r.2 + r.4) as i32);
            let meets = |y: i32, h: i32| y < 96 + 64 && 96 < y + h && (r.1 as i32) < 320 && 160 < (r.1 + r.3) as i32;
            assert!(!meets(y0, y1 - y0), "COPY source over the skipped box: {r:?}");
        }
        for y in 0..H {
            for x in 0..W {
                let in_box = (160..320).contains(&x) && (96..160).contains(&y);
                if !in_box {
                    assert_eq!(fb[y * W + x], b[y * W + x], "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn a_budget_spreads_a_full_screen_over_several_calls() {
        let mut s = Screen::default();
        let frame: Vec<u16> = (0..W * H).map(|i| (i * 7919) as u16).collect(); // no RLE gain
        let (mut tiles, mut calls) = (0, 0);
        loop {
            let mut out = vec![];
            let (n, done) = s.update(&frame, &[], &mut out, 20_000);
            assert!(out.len() < 20_000 + 2 * W * TILE + 64, "about the budget");
            tiles += n;
            calls += 1;
            if done {
                break;
            }
        }
        assert_eq!(tiles, TILES_X * TILES_Y, "every tile sent once");
        assert!(calls > 5, "{calls} calls");
        let mut out = vec![];
        assert_eq!(s.update(&frame, &[], &mut out, 20_000), (0, true));
    }
}
