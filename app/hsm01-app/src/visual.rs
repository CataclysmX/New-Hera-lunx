//! Spectrum analyser: FFT of the latest PCM window → log-spaced bars with fall-off and peak caps,
//! plus the bar geometry and the rectangles of each column. The same rectangles are drawn into the
//! preview pixmap and sent to the device as FILL commands (only the columns that changed), which
//! costs a few KiB per frame instead of re-sending pixels.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::config::Color;

pub const FFT_SIZE: usize = 2048;
const F_MIN: f32 = 35.0;
const F_MAX: f32 = 16_000.0;
/// Automatic gain: the top of the bars follows the loudest band of the last seconds (falling
/// `AGC_FALL_DB` per second, never below `AGC_MIN_DB`); bars span `RANGE_DB` below it.
const RANGE_DB: f32 = 50.0;
const AGC_FALL_DB: f32 = 4.0;
const AGC_MIN_DB: f32 = -45.0;
const AGC_HEADROOM_DB: f32 = 3.0;
/// Music has less energy in the highs: lift them so the right side moves too.
const TILT_DB_PER_OCTAVE: f32 = 3.0;
/// Fall speeds (bar heights per second) and peak-cap hold time.
const BAR_FALL: f32 = 1.6;
const CAP_FALL: f32 = 0.45;
const CAP_HOLD: f32 = 0.35;
/// Height of the peak caps in pixels.
pub const CAP_PX: i32 = 2;

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Amplitude of a full-scale sine after windowing (normalisation).
    norm: f32,
    buf: Vec<Complex<f32>>,
    levels: Vec<f32>,
    caps: Vec<f32>,
    hold: Vec<f32>,
    /// Automatic-gain reference (dB of the loudest recent band).
    ref_db: f32,
}

impl Default for Analyzer {
    fn default() -> Self {
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        let norm = window.iter().sum::<f32>() / 2.0;
        Analyzer {
            fft: FftPlanner::new().plan_fft_forward(FFT_SIZE),
            window,
            norm,
            buf: vec![Complex::default(); FFT_SIZE],
            levels: Vec::new(),
            caps: Vec::new(),
            hold: Vec::new(),
            ref_db: AGC_MIN_DB,
        }
    }
}

impl Analyzer {
    /// New bar levels and caps (0..1) from `pcm` = (latest FFT_SIZE samples, sample rate), or a
    /// plain fall-off when no audio is available. `dt` = seconds since the previous call.
    pub fn update(&mut self, pcm: Option<(&[f32], u32)>, bars: usize, dt: f32) -> Vec<(f32, f32)> {
        if self.levels.len() != bars {
            self.levels = vec![0.0; bars];
            self.caps = vec![0.0; bars];
            self.hold = vec![0.0; bars];
        }
        let targets = pcm.map(|(s, rate)| {
            let db = self.bands(s, rate, bars);
            let loudest = db.iter().copied().fold(f32::MIN, f32::max);
            self.ref_db = loudest.max(self.ref_db - AGC_FALL_DB * dt).max(AGC_MIN_DB);
            let top = self.ref_db + AGC_HEADROOM_DB;
            db.iter()
                .map(|d| ((d - (top - RANGE_DB)) / RANGE_DB).clamp(0.0, 1.0))
                .collect::<Vec<_>>()
        });
        for i in 0..bars {
            let target = targets.as_ref().map_or(0.0, |t| t[i]);
            let level = &mut self.levels[i];
            *level = if target >= *level {
                target
            } else {
                (*level - BAR_FALL * dt).max(target)
            };
            if *level >= self.caps[i] {
                self.caps[i] = *level;
                self.hold[i] = CAP_HOLD;
            } else if self.hold[i] > 0.0 {
                self.hold[i] -= dt;
            } else {
                self.caps[i] = (self.caps[i] - CAP_FALL * dt).max(*level);
            }
        }
        self.levels
            .iter()
            .copied()
            .zip(self.caps.iter().copied())
            .collect()
    }

    /// Instant level (dBFS, tilted) of each log-spaced band.
    fn bands(&mut self, samples: &[f32], rate: u32, bars: usize) -> Vec<f32> {
        let n = samples.len().min(FFT_SIZE);
        for (i, c) in self.buf.iter_mut().enumerate() {
            let s = if i < n {
                samples[samples.len() - n + i]
            } else {
                0.0
            };
            *c = Complex::new(s * self.window[i], 0.0);
        }
        self.fft.process(&mut self.buf);
        let bin_hz = rate as f32 / FFT_SIZE as f32;
        let half = FFT_SIZE / 2;
        (0..bars)
            .map(|b| {
                let f0 = F_MIN * (F_MAX / F_MIN).powf(b as f32 / bars as f32);
                let f1 = F_MIN * (F_MAX / F_MIN).powf((b + 1) as f32 / bars as f32);
                let k0 = ((f0 / bin_hz).floor() as usize).clamp(1, half - 1);
                let k1 = ((f1 / bin_hz).ceil() as usize).clamp(k0 + 1, half);
                let mag = self.buf[k0..k1]
                    .iter()
                    .map(|c| c.norm())
                    .fold(0.0, f32::max);
                let centre = (f0 * f1).sqrt();
                20.0 * (mag / self.norm).max(1e-9).log10()
                    + TILT_DB_PER_OCTAVE * (centre / 1000.0).log2()
            })
            .collect()
    }
}

/// Left x and width of each bar, centred in the box.
pub fn bar_columns(x: i32, w: i32, bars: usize, gap: i32) -> Vec<(i32, i32)> {
    let n = bars.max(1) as i32;
    let bw = ((w - gap * (n - 1)) / n).max(1);
    let x0 = x + (w - (n * bw + (n - 1) * gap)) / 2;
    (0..n).map(|i| (x0 + i * (bw + gap), bw)).collect()
}

/// Pixel state of one column: bar height and peak-cap position (row offset from the top, or -1).
pub fn column_state(h: i32, level: f32, cap: f32, with_cap: bool) -> (i32, i32) {
    let bar = (level.clamp(0.0, 1.0) * h as f32).round() as i32;
    let bar_top = h - bar;
    let cap_y = if with_cap && cap > 0.01 {
        let y =
            (h - (cap.clamp(0.0, 1.0) * h as f32).round() as i32 - CAP_PX).min(bar_top - CAP_PX);
        if y >= 0 { y } else { -1 }
    } else {
        -1
    };
    (bar, cap_y)
}

/// Rectangles (y, height, colour) covering the whole column `[y, y+h)` from top to bottom:
/// background, peak cap, background, then the bar split into equal-height colour zones
/// (`colors[0]` at the bottom).
pub fn column_rects(
    y: i32,
    h: i32,
    state: (i32, i32),
    colors: &[Color],
    peak: Color,
    bg: Color,
) -> Vec<(i32, i32, Color)> {
    let (bar, cap_y) = state;
    let bar_top = h - bar;
    let mut out = Vec::new();
    let mut push = |from: i32, to: i32, c: Color| {
        if to > from {
            out.push((y + from, to - from, c));
        }
    };
    if cap_y >= 0 {
        push(0, cap_y, bg);
        push(cap_y, cap_y + CAP_PX, peak);
        push(cap_y + CAP_PX, bar_top, bg);
    } else {
        push(0, bar_top, bg);
    }
    let zones = colors.len().max(1) as i32;
    for z in (0..zones).rev() {
        // zone z spans rows [h - (z+1)·h/zones, h - z·h/zones)
        let (z_top, z_bottom) = (h - (z + 1) * h / zones, h - z * h / zones);
        let c = colors
            .get(z as usize)
            .copied()
            .unwrap_or(Color(0, 0x6c, 0x68));
        push(z_top.max(bar_top), z_bottom, c);
    }
    out
}

/// Only the rows of a column whose colour changes from state `old` to `new` (same layout as
/// `column_rects`): a few rows per frame instead of the whole column. Filling whole columns cost
/// ~90 000 pixels per frame, which kept the firmware too busy to send its input reports.
pub fn column_delta(
    y: i32,
    h: i32,
    old: (i32, i32),
    new: (i32, i32),
    colors: &[Color],
    peak: Color,
    bg: Color,
) -> Vec<(i32, i32, Color)> {
    let rows = |state| {
        let mut v = vec![bg; h.max(0) as usize];
        for (ry, rh, c) in column_rects(0, h, state, colors, peak, bg) {
            v[ry as usize..(ry + rh) as usize].fill(c);
        }
        v
    };
    let (a, b) = (rows(old), rows(new));
    let mut out = Vec::new();
    let mut r = 0;
    while r < b.len() {
        if a[r] == b[r] {
            r += 1;
            continue;
        }
        let start = r;
        while r < b.len() && a[r] != b[r] && b[r] == b[start] {
            r += 1;
        }
        out.push((y + start as i32, (r - start) as i32, b[start]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sine_lights_its_band() {
        let mut a = Analyzer::default();
        let rate = 48_000;
        let tone: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / rate as f32).sin())
            .collect();
        let bars = a.update(Some((&tone, rate)), 32, 0.016);
        let (loudest, _) = bars
            .iter()
            .enumerate()
            .fold((0, 0.0), |m, (i, b)| if b.0 > m.1 { (i, b.0) } else { m });
        let centre = |b: usize| F_MIN * (F_MAX / F_MIN).powf((b as f32 + 0.5) / 32.0);
        assert!(
            (centre(loudest) / 1000.0).log2().abs() < 0.3,
            "loudest band {loudest} ({} Hz)",
            centre(loudest)
        );
        assert!(
            bars[0].0 < 0.3 && bars[31].0 < 0.5,
            "far bands stay low: {bars:?}"
        );
    }

    #[test]
    fn bars_fall_and_caps_hold() {
        let mut a = Analyzer::default();
        let loud = vec![0.9f32; FFT_SIZE];
        a.update(Some((&loud, 48_000)), 8, 0.016);
        let after = a.update(None, 8, 0.1);
        assert!(after.iter().all(|&(l, c)| c >= l), "caps stay above bars");
    }

    #[test]
    fn columns_cover_exactly_the_box() {
        let colors = [Color(1, 0, 0), Color(2, 0, 0), Color(3, 0, 0)];
        for (level, cap) in [(0.0, 0.0), (0.5, 0.8), (1.0, 1.0), (0.3, 0.31)] {
            let r = column_rects(
                10,
                200,
                column_state(200, level, cap, true),
                &colors,
                Color(9, 9, 9),
                Color(0, 0, 0),
            );
            assert_eq!(r.first().unwrap().0, 10);
            assert_eq!(
                r.iter().map(|x| x.1).sum::<i32>(),
                200,
                "{level} {cap}: {r:?}"
            );
            assert!(
                r.windows(2).all(|p| p[0].0 + p[0].1 == p[1].0),
                "contiguous"
            );
        }
        // the delta turns one state into the other
        let (old, new) = (
            column_state(200, 0.5, 0.8, true),
            column_state(200, 0.55, 0.78, true),
        );
        let paint = |rects: &[(i32, i32, Color)], rows: &mut Vec<Color>| {
            for &(y, h, c) in rects {
                rows[(y - 10) as usize..(y - 10 + h) as usize].fill(c);
            }
        };
        let (peak, bg) = (Color(9, 9, 9), Color(0, 0, 0));
        let mut rows = vec![bg; 200];
        paint(&column_rects(10, 200, old, &colors, peak, bg), &mut rows);
        let delta = column_delta(10, 200, old, new, &colors, peak, bg);
        assert!(
            delta.iter().map(|d| d.1).sum::<i32>() < 40,
            "small delta: {delta:?}"
        );
        paint(&delta, &mut rows);
        let mut want = vec![bg; 200];
        paint(&column_rects(10, 200, new, &colors, peak, bg), &mut want);
        assert_eq!(rows, want);
        let cols = bar_columns(0, 480, 48, 2);
        assert_eq!(cols.len(), 48);
        assert!(cols.last().unwrap().0 + cols.last().unwrap().1 <= 480);
    }
}
