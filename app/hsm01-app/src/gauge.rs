//! Styles of the level meters (`vu`) and volume gauges (`volume`): `gauge = { style = … }`.
//! Angles are in degrees, 0 = top, clockwise (default arc: -135 → 135, open at the bottom).
//! Every style keeps the widget's colours / gradient (`color`, from empty to full), greys out
//! while muted and shows the meters' peak hold.

use std::collections::HashMap;

use tiny_skia::{FillRule, LineCap, Mask, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

use crate::config::{Color, Gauge, Paint, Widget};
use crate::render::{MUTED, TRACK, axis_shader, bar, fill_round_rect, paint};

/// What a gauge shows.
pub struct Meter<'a> {
    /// 0..1.
    pub value: f32,
    /// Peak hold of the level meters (0..1).
    pub peak: Option<f32>,
    pub color: &'a Paint,
    pub vertical: bool,
    pub muted: bool,
    /// Rounded corners of the plain bar (volume gauges).
    pub radius: f32,
}

const WHITE: Color = Color(255, 255, 255);
const TICK: Color = Color(0x8a, 0x90, 0x90);

/// Images of the `image` style fitted to their widget: (path, w, h) → pixmap.
pub type Images = HashMap<(String, u32, u32), Option<Pixmap>>;

pub fn draw(pm: &mut Pixmap, w: &Widget, gauge: &Gauge, m: &Meter, images: &mut Images) {
    let v = m.value.clamp(0.0, 1.0);
    match gauge {
        Gauge::Bar => {
            fill_round_rect(pm, w.x, w.y, w.w, w.h, 3.0, TRACK);
            let sk = if m.muted {
                paint(MUTED)
            } else {
                axis_shader(m.color, w, m.vertical)
            };
            bar(pm, w, v, m.vertical, m.radius, &sk);
            if let Some(p) = m.peak.filter(|p| *p > 0.01) {
                let len = if m.vertical { w.h } else { w.w } as f32;
                let p = (p.min(1.0) * len).round() as i32;
                if m.vertical {
                    fill_round_rect(pm, w.x, w.y + w.h - p, w.w, 2, 0.0, WHITE);
                } else {
                    fill_round_rect(pm, w.x + p - 2, w.y, 2, w.h, 0.0, WHITE);
                }
            }
        }
        Gauge::Segments { count, gap } => segments(pm, w, m, v, (*count).max(1), *gap),
        Gauge::Arc {
            start,
            end,
            thickness,
        } => {
            let d = Dial::new(w, *thickness);
            d.arc(pm, *start, *end, *thickness, &paint(TRACK), LineCap::Round);
            if v > 0.0 {
                d.colored_arc(pm, (*start, *end), v, *thickness, m);
            }
            if let Some(p) = m.peak.filter(|p| *p > 0.01) {
                let a = lerp(*start, *end, p.min(1.0));
                d.radial(
                    pm,
                    a,
                    d.r - thickness / 2.0 - 2.0,
                    d.r + thickness / 2.0 + 2.0,
                    2.0,
                    WHITE,
                );
            }
        }
        Gauge::Needle {
            start,
            end,
            thickness,
        } => {
            let d = Dial::new(w, thickness + 8.0);
            // coloured scale, ticks every 10 % (longer at 0, 50, 100 %)
            let scale = Meter { muted: false, ..*m };
            d.colored_arc(pm, (*start, *end), 1.0, *thickness, &scale);
            for i in 0..=10 {
                let a = lerp(*start, *end, i as f32 / 10.0);
                let inner = d.r - thickness / 2.0 - if i % 5 == 0 { 9.0 } else { 5.0 };
                d.radial(pm, a, inner, d.r - thickness / 2.0 - 1.0, 1.5, TICK);
            }
            if let Some(p) = m.peak.filter(|p| *p > 0.01) {
                let a = lerp(*start, *end, p.min(1.0));
                d.radial(
                    pm,
                    a,
                    d.r + thickness / 2.0 + 1.0,
                    d.r + thickness / 2.0 + 5.0,
                    2.0,
                    WHITE,
                );
            }
            let needle = if m.muted { MUTED } else { WHITE };
            d.radial(
                pm,
                lerp(*start, *end, v),
                0.0,
                d.r - thickness / 2.0 - 3.0,
                3.0,
                needle,
            );
            fill_circle(pm, d.cx, d.cy, 5.0, needle);
        }
        Gauge::Image {
            track,
            fill,
            sweep,
            start,
            end,
        } => {
            let (bw, bh) = (w.w.max(1) as u32, w.h.max(1) as u32);
            let mut get = |path: &str| {
                images
                    .entry((path.to_owned(), bw, bh))
                    .or_insert_with(|| load_fitted(path, bw, bh))
                    .clone()
            };
            if let Some(t) = get(track) {
                pm.draw_pixmap(
                    w.x,
                    w.y,
                    t.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
            let (Some(f), Some(mask)) = (
                get(fill),
                reveal_mask(w, v, m.vertical, *sweep, (*start, *end)),
            ) else {
                return;
            };
            let opacity = if m.muted { 0.3 } else { 1.0 };
            let p = PixmapPaint {
                opacity,
                ..Default::default()
            };
            pm.draw_pixmap(w.x, w.y, f.as_ref(), &p, Transform::identity(), Some(&mask));
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// LED-like blocks from the bottom (or the left); lit blocks take the colour of their position.
fn segments(pm: &mut Pixmap, w: &Widget, m: &Meter, v: f32, n: u8, gap: u8) {
    let n_f = n as f32;
    let len = if m.vertical { w.h } else { w.w } as f32;
    let seg = ((len - gap as f32 * (n_f - 1.0)) / n_f).max(1.0);
    let peak = m
        .peak
        .filter(|p| *p > 0.01)
        .map(|p| ((p.min(1.0) * n_f - 0.5).round().max(0.0) as u8).min(n - 1));
    for i in 0..n {
        let from = i as f32 * (seg + gap as f32);
        let lit = v * n_f >= i as f32 + 0.5;
        let c = if Some(i) == peak && !lit {
            WHITE
        } else if !lit {
            TRACK
        } else if m.muted {
            MUTED
        } else {
            m.color.at((i as f32 + 0.5) / n_f)
        };
        let (a, b) = (from.round() as i32, (from + seg).round() as i32);
        if m.vertical {
            fill_round_rect(pm, w.x, w.y + w.h - b, w.w, b - a, 1.5, c);
        } else {
            fill_round_rect(pm, w.x + a, w.y, b - a, w.h, 1.5, c);
        }
    }
}

/// Circle inscribed in the widget, leaving room for a stroke of width `margin`.
struct Dial {
    cx: f32,
    cy: f32,
    r: f32,
}

impl Dial {
    fn new(w: &Widget, margin: f32) -> Self {
        let r = (w.w.min(w.h) as f32 / 2.0 - margin / 2.0 - 1.0).max(2.0);
        Dial {
            cx: w.x as f32 + w.w as f32 / 2.0,
            cy: w.y as f32 + w.h as f32 / 2.0,
            r,
        }
    }

    fn point(&self, deg: f32, r: f32) -> (f32, f32) {
        let a = deg.to_radians();
        (self.cx + r * a.sin(), self.cy - r * a.cos())
    }

    fn arc(
        &self,
        pm: &mut Pixmap,
        a0: f32,
        a1: f32,
        width: f32,
        sk: &tiny_skia::Paint,
        cap: LineCap,
    ) {
        let n = ((a1 - a0).abs() / 3.0).ceil().max(1.0) as usize;
        let mut pb = PathBuilder::new();
        let (x, y) = self.point(a0, self.r);
        pb.move_to(x, y);
        for i in 1..=n {
            let (x, y) = self.point(lerp(a0, a1, i as f32 / n as f32), self.r);
            pb.line_to(x, y);
        }
        if let Some(path) = pb.finish() {
            let stroke = Stroke {
                width,
                line_cap: cap,
                ..Default::default()
            };
            pm.stroke_path(&path, sk, &stroke, Transform::identity(), None);
        }
    }

    /// Arc from `start` to the fraction `v` of the way to `end`, each piece coloured by its
    /// position (a gradient follows the arc).
    fn colored_arc(
        &self,
        pm: &mut Pixmap,
        (start, end): (f32, f32),
        v: f32,
        width: f32,
        m: &Meter,
    ) {
        let solid = match m.color {
            _ if m.muted => Some(MUTED),
            Paint::Solid(c) => Some(*c),
            _ => None,
        };
        let to = lerp(start, end, v);
        if let Some(c) = solid {
            self.arc(pm, start, to, width, &paint(c), LineCap::Butt);
            return;
        }
        let n = ((to - start).abs() / 2.0).ceil().max(1.0) as usize;
        for i in 0..n {
            let (t0, t1) = (i as f32 / n as f32 * v, (i + 1) as f32 / n as f32 * v);
            let overlap = if i + 1 < n {
                0.6f32.copysign(end - start)
            } else {
                0.0
            };
            let c = m.color.at((t0 + t1) / 2.0);
            self.arc(
                pm,
                lerp(start, end, t0),
                lerp(start, end, t1) + overlap,
                width,
                &paint(c),
                LineCap::Butt,
            );
        }
    }

    /// Line along the radius at angle `deg`, from `r0` to `r1` from the centre.
    fn radial(&self, pm: &mut Pixmap, deg: f32, r0: f32, r1: f32, width: f32, c: Color) {
        let ((x0, y0), (x1, y1)) = (self.point(deg, r0), self.point(deg, r1));
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        if let Some(path) = pb.finish() {
            let stroke = Stroke {
                width,
                line_cap: LineCap::Round,
                ..Default::default()
            };
            pm.stroke_path(&path, &paint(c), &stroke, Transform::identity(), None);
        }
    }
}

fn fill_circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, c: Color) {
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        pm.fill_path(
            &path,
            &paint(c),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Part of the screen where the fill image shows: along the meter, or the wedge from `start`
/// to the level's angle around the widget's centre.
fn reveal_mask(
    w: &Widget,
    v: f32,
    vertical: bool,
    sweep: bool,
    (start, end): (f32, f32),
) -> Option<Mask> {
    let mut mask = Mask::new(
        crate::config::SCREEN_W as u32,
        crate::config::SCREEN_H as u32,
    )?;
    let (x, y, ww, h) = (w.x as f32, w.y as f32, w.w as f32, w.h as f32);
    let path = if sweep {
        let d = Dial {
            cx: x + ww / 2.0,
            cy: y + h / 2.0,
            r: ww.max(h),
        };
        let to = lerp(start, end, v);
        let n = ((to - start).abs() / 3.0).ceil().max(1.0) as usize;
        let mut pb = PathBuilder::new();
        pb.move_to(d.cx, d.cy);
        for i in 0..=n {
            let (px, py) = d.point(lerp(start, to, i as f32 / n as f32), d.r);
            pb.line_to(px, py);
        }
        pb.close();
        pb.finish()?
    } else if vertical {
        PathBuilder::from_rect(tiny_skia::Rect::from_xywh(
            x,
            y + h * (1.0 - v),
            ww,
            (h * v).max(0.01),
        )?)
    } else {
        PathBuilder::from_rect(tiny_skia::Rect::from_xywh(x, y, (ww * v).max(0.01), h)?)
    };
    if v <= 0.0 {
        return None;
    }
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
    Some(mask)
}

/// Image fitted (whole, centred) in a `w`×`h` pixmap.
fn load_fitted(path: &str, w: u32, h: u32) -> Option<Pixmap> {
    let src = crate::render::to_pixmap(image::open(path).ok()?.into_rgba8())?;
    let k = (w as f32 / src.width() as f32).min(h as f32 / src.height() as f32);
    let (dx, dy) = (
        (w as f32 - src.width() as f32 * k) / 2.0,
        (h as f32 - src.height() as f32 * k) / 2.0,
    );
    let mut pm = Pixmap::new(w, h)?;
    let p = PixmapPaint {
        quality: tiny_skia::FilterQuality::Bicubic,
        ..Default::default()
    };
    pm.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &p,
        Transform::from_row(k, 0.0, 0.0, k, dx, dy),
        None,
    );
    Some(pm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Page, WidgetKind};
    use crate::render::Renderer;
    use crate::state::{TargetState, View};

    /// Every style: `TMPDIR=… cargo test -- --ignored gauge_snapshot` → `$TMPDIR/hsm01-gauges.png`.
    #[test]
    #[ignore]
    fn gauge_snapshot() {
        let dir = std::env::temp_dir();
        // ring images for the free style: grey track, teal fill
        let ring = |c: Color, path: &std::path::Path| {
            let mut pm = Pixmap::new(100, 100).unwrap();
            let mut pb = PathBuilder::new();
            pb.push_circle(50.0, 50.0, 40.0);
            let path_ = pb.finish().unwrap();
            let stroke = Stroke {
                width: 14.0,
                ..Default::default()
            };
            pm.stroke_path(&path_, &paint(c), &stroke, Transform::identity(), None);
            pm.save_png(path).unwrap();
        };
        let (track, fill) = (dir.join("hsm01-track.png"), dir.join("hsm01-fill.png"));
        ring(Color(0x30, 0x34, 0x34), &track);
        ring(Color(0x00, 0xb4, 0xa8), &fill);
        let vu = crate::config::vu_paint();
        let teal = Paint::Gradient(vec![Color(0x00, 0x6c, 0x68), Color(0x9a, 0xd8, 0xd0)]);
        let widget = |x, y, w, h, kind| Widget {
            x,
            y,
            w,
            h,
            frame: None,
            kind,
        };
        let page = Page {
            widgets: vec![
                widget(
                    10,
                    20,
                    26,
                    230,
                    WidgetKind::Vu {
                        knob: 1,
                        vertical: true,
                        color: vu.clone(),
                        gauge: Gauge::Bar,
                    },
                ),
                widget(
                    50,
                    20,
                    26,
                    230,
                    WidgetKind::Vu {
                        knob: 1,
                        vertical: true,
                        color: vu.clone(),
                        gauge: Gauge::template("segments"),
                    },
                ),
                widget(
                    90,
                    30,
                    120,
                    120,
                    WidgetKind::Volume {
                        knob: 2,
                        color: teal.clone(),
                        vertical: true,
                        gauge: Gauge::template("arc"),
                    },
                ),
                widget(
                    220,
                    30,
                    120,
                    120,
                    WidgetKind::Vu {
                        knob: 1,
                        vertical: true,
                        color: vu.clone(),
                        gauge: Gauge::template("needle"),
                    },
                ),
                widget(
                    350,
                    30,
                    120,
                    120,
                    WidgetKind::Volume {
                        knob: 2,
                        color: teal.clone(),
                        vertical: true,
                        gauge: Gauge::Image {
                            track: track.to_string_lossy().into(),
                            fill: fill.to_string_lossy().into(),
                            sweep: true,
                            start: 0.0,
                            end: 360.0,
                        },
                    },
                ),
                widget(
                    90,
                    180,
                    380,
                    30,
                    WidgetKind::Volume {
                        knob: 3,
                        color: teal,
                        vertical: false,
                        gauge: Gauge::template("segments"),
                    },
                ),
            ],
            ..Config::default().pages[0].clone()
        };
        let mut view = View::default();
        view.knobs[0] = TargetState {
            present: true,
            level: 0.82,
            peak_hold: 0.93,
            ..Default::default()
        };
        view.knobs[1] = TargetState {
            present: true,
            volume: 0.62,
            ..Default::default()
        };
        view.knobs[2] = TargetState {
            present: true,
            volume: 0.45,
            muted: true,
            ..Default::default()
        };
        let mut r = Renderer::new().expect("fonts");
        let mut pm = Renderer::new_pixmap();
        r.render(&page, &view, &mut pm);
        pm.save_png(dir.join("hsm01-gauges.png")).unwrap();
        // the same with frames: border gradient + background on the arc, plain border elsewhere
        let mut framed = page.clone();
        for (k, w) in framed.widgets.iter_mut().enumerate() {
            w.frame = Some(crate::config::Frame {
                fill: (k == 2).then_some(Paint::Solid(Color(0x10, 0x18, 0x18))),
                border: if k == 2 {
                    Paint::Gradient(vec![Color(0x00, 0xb4, 0xa8), Color(0xe0, 0x50, 0xd0)])
                } else {
                    Paint::Solid(Color(0x00, 0xb4, 0xa8))
                },
                ..Default::default()
            });
        }
        let mut pm = Renderer::new_pixmap();
        r.render(&framed, &view, &mut pm);
        pm.save_png(dir.join("hsm01-frames.png")).unwrap();
    }
}
