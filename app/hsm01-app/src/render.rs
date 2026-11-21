//! Software renderer of a page (480×272) with tiny-skia + fontdue.

use std::collections::HashMap;

use fontdue::{Font, FontSettings, Metrics};
use tiny_skia::{
    FillRule, GradientStop, LinearGradient, Paint as SkPaint, PathBuilder, Pixmap, PixmapPaint,
    Point, Rect, SpreadMode, Transform,
};

use crate::i18n::tr;
use crate::config::{
    Align, Calibration, Color, Fit, IconStyle, LineStyle, Overflow, Page, Paint, SCREEN_H,
    SCREEN_W, Tile, Widget, WidgetKind,
};
use crate::state::{ConfirmView, DeckView, Overlay, TargetState, View};

const FONT_REGULAR: &[&str] = &[
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
];
/// Fonts for the characters the text font lacks (arrows, symbols…), tried in this order.
const FONT_FALLBACK: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
    "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf",
];
/// Colour emoji: the PNG image of each glyph (CBDT / sbix tables).
const FONT_EMOJI: &[&str] = &[
    "/usr/share/fonts/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
    "/usr/share/fonts/noto-emoji/NotoColorEmoji.ttf",
    "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
];

const FONT_BOLD: &[&str] = &[
    "/usr/share/fonts/noto/NotoSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
];

pub const TRACK: Color = Color(0x1c, 0x22, 0x22);
/// Area of the modules menu once slid in: a band at the bottom of the screen.
pub const MENU_BOX: (i32, i32, i32, i32) = (0, SCREEN_H - 92, SCREEN_W, 92);
/// Box (x, y, w, h) of the notification popup.
pub const NOTICE_BOX: (i32, i32, i32, i32) = (12, 10, 456, 108);
/// Band of the pill notifications at the bottom (the pill and its glow, whatever its width).
pub const PILL_BAND: (i32, i32, i32, i32) = (0, SCREEN_H - 70, SCREEN_W, 70);
/// Pill: top (once slid in), height, text size, room for a dot and the margins at each end;
/// slide in (from below) and out (s); period of the dots' glow (s).
const PILL_H: i32 = 40;
const PILL_Y: i32 = SCREEN_H - 16 - PILL_H;
const PILL_TEXT: f32 = 17.0;
const PILL_END: i32 = 38;
const PILL_SLIDE: f32 = 0.25;
const PILL_PULSE: f32 = 1.4;

/// Where a notification is drawn (spectra keep running around it).
pub fn notice_box(n: &crate::notify::Notification) -> (i32, i32, i32, i32) {
    match n.style {
        crate::notify::NoticeStyle::Card => NOTICE_BOX,
        crate::notify::NoticeStyle::Pill { .. } => PILL_BAND,
    }
}
pub const MUTED: Color = Color(0x55, 0x55, 0x55);

pub struct Renderer {
    regular: Font,
    bold: Font,
    glyphs: HashMap<(char, u32, bool), (Metrics, Vec<u8>)>,
    images: HashMap<String, Option<Pixmap>>,
    /// Wallpapers ready to draw: (path, fit, dim, background) → frames (None: unreadable).
    wallpapers: HashMap<(String, Fit, u8, Color), Option<crate::wallpaper::Frames>>,
    /// Images of the `image` gauges, fitted to their widget.
    gauge_images: crate::gauge::Images,
    /// Source icons drawn at a given size: (name, w, h, tint of a symbolic icon) → pixmap.
    icons: HashMap<(String, u32, u32, Option<Color>), Option<Pixmap>>,
    /// Where the page of a `tool` widget is drawn before being scaled into its box.
    offscreen: Option<Pixmap>,
    /// Fonts for the characters the text fonts lack.
    fallback: Vec<Font>,
    /// The colour emoji font (its bytes, read again when a new emoji is drawn).
    emoji_font: Option<Vec<u8>>,
    /// Emoji images at a size: (char, size × 10) → image (None: not in the font); whether the
    /// emoji font has a character.
    emoji: HashMap<(char, u32), Option<Pixmap>>,
    emoji_has: HashMap<char, bool>,
}

/// What draws one character.
enum Piece {
    /// A glyph of a font: its coverage, drawn in the text's colour.
    Glyph(Metrics, Vec<u8>),
    /// A colour emoji (its own colours), its advance.
    Emoji(Pixmap, f32),
    /// Nothing (variation selectors, joiners, skin tones: the base emoji is drawn alone).
    Nothing,
}

/// Characters that only modify the one before (no shaping here): not drawn.
fn modifier(ch: char) -> bool {
    matches!(ch, '\u{FE0E}' | '\u{FE0F}' | '\u{200D}' | '\u{20E3}' | '\u{1F3FB}'..='\u{1F3FF}')
}

/// Side of an emoji for a text of `size` px, and its advance.
fn emoji_side(size: f32) -> u32 {
    (size * 1.15).round().max(1.0) as u32
}

fn emoji_advance(size: f32) -> f32 {
    emoji_side(size) as f32 + (size * 0.1).round()
}

/// The PNG image of emoji `ch` in the colour font `data` (its largest strike), `side` px high.
fn emoji_png(data: &[u8], ch: char, side: u32) -> Option<Pixmap> {
    let face = ttf_parser::Face::parse(data, 0).ok()?;
    let raster = face.glyph_raster_image(face.glyph_index(ch)?, u16::MAX)?;
    if raster.format != ttf_parser::RasterImageFormat::PNG {
        return None;
    }
    let rgba = image::load_from_memory_with_format(raster.data, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let (w, h) = rgba.dimensions();
    let width = ((w as f32 * side as f32 / h.max(1) as f32).round() as u32).max(1);
    to_pixmap(image::imageops::resize(&rgba, width, side, image::imageops::FilterType::Triangle))
}

/// `img` at (x, y), only the part between the columns `x0..x1` (a scrolling text stays in its
/// box).
fn draw_in_columns(pm: &mut Pixmap, img: &Pixmap, (x, y): (i32, i32), (x0, x1): (i32, i32)) {
    let iw = img.width() as i32;
    let (from, to) = (x.max(x0), (x + iw).min(x1));
    if to <= from {
        return;
    }
    let p = PixmapPaint::default();
    if (from, to) == (x, x + iw) {
        pm.draw_pixmap(x, y, img.as_ref(), &p, Transform::identity(), None);
    } else if let Some(part) = tiny_skia::IntRect::from_xywh(from - x, 0, (to - from) as u32, img.height())
        .and_then(|r| img.clone_rect(r))
    {
        pm.draw_pixmap(from, y, part.as_ref(), &p, Transform::identity(), None);
    }
}

fn load_font(candidates: &[&str]) -> Result<Font, String> {
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            return Font::from_bytes(bytes, FontSettings::default())
                .map_err(|e| format!("{path}: {e}"));
        }
    }
    Err(format!("no usable font among {candidates:?}"))
}

pub fn paint(c: Color) -> SkPaint<'static> {
    let mut p = SkPaint::default();
    p.set_color_rgba8(c.0, c.1, c.2, 255);
    p.anti_alias = true;
    p
}

/// `fill` as a tiny-skia paint, a gradient running from `start` to `end` (pixels).
fn shader(fill: &Paint, start: (f32, f32), end: (f32, f32)) -> SkPaint<'static> {
    if let Paint::Solid(c) = fill {
        return paint(*c);
    }
    let stops: Vec<GradientStop> = fill
        .stops()
        .into_iter()
        .map(|(pos, c)| GradientStop::new(pos, tiny_skia::Color::from_rgba8(c.0, c.1, c.2, 255)))
        .collect();
    let gradient = LinearGradient::new(
        Point::from_xy(start.0, start.1),
        Point::from_xy(end.0, end.1),
        stops,
        SpreadMode::Pad,
        Transform::identity(),
    );
    match gradient {
        Some(shader) => SkPaint {
            shader,
            anti_alias: true,
            ..Default::default()
        },
        None => paint(fill.at(0.0)),
    }
}

/// Gradient across a box: left → right, or top → bottom for `vertical` paints.
fn box_shader(fill: &Paint, x: i32, y: i32, w: i32, h: i32) -> SkPaint<'static> {
    let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);
    if fill.vertical() {
        shader(fill, (x, y), (x, y + h))
    } else {
        shader(fill, (x, y), (x + w, y))
    }
}

/// Gradient along a meter, from its empty end (bottom / left) to its full end.
pub fn axis_shader(fill: &Paint, w: &Widget, vertical: bool) -> SkPaint<'static> {
    let (x, y, ww, h) = (w.x as f32, w.y as f32, w.w as f32, w.h as f32);
    if vertical {
        shader(fill, (x, y + h), (x, y))
    } else {
        shader(fill, (x, y), (x + ww, y))
    }
}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        Ok(Renderer {
            regular: load_font(FONT_REGULAR)?,
            bold: load_font(FONT_BOLD)?,
            glyphs: HashMap::new(),
            images: HashMap::new(),
            icons: HashMap::new(),
            wallpapers: HashMap::new(),
            gauge_images: HashMap::new(),
            offscreen: None,
            fallback: FONT_FALLBACK
                .iter()
                .filter_map(|p| Font::from_bytes(std::fs::read(p).ok()?, FontSettings::default()).ok())
                .collect(),
            emoji_font: FONT_EMOJI.iter().find_map(|p| std::fs::read(p).ok()),
            emoji: HashMap::new(),
            emoji_has: HashMap::new(),
        })
    }

    pub fn new_pixmap() -> Pixmap {
        Pixmap::new(SCREEN_W as u32, SCREEN_H as u32).expect("pixmap")
    }

    pub fn render(&mut self, page: &Page, view: &View, pm: &mut Pixmap) {
        let bg = page.background;
        pm.fill(tiny_skia::Color::from_rgba8(bg.0, bg.1, bg.2, 255));
        if let Some(wp) = page.wallpaper.as_ref().filter(|w| !w.path.is_empty()) {
            let key = (wp.path.clone(), wp.fit, wp.dim, bg);
            let frames = self.wallpapers.entry(key).or_insert_with(|| {
                crate::wallpaper::load(&wp.path, wp.fit, wp.dim, bg)
                    .inspect_err(|e| eprintln!("hsm01-app: {}", tf!("wallpaper: {e}", e = e)))
                    .ok()
            });
            if let Some(f) = frames {
                let frame = f.at(view.anim_time, wp.fps);
                pm.draw_pixmap(
                    0,
                    0,
                    frame.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
        }
        for (i, w) in page.widgets.iter().enumerate() {
            let Some(frame) = &w.frame else {
                self.widget(i, w, view, pm);
                continue;
            };
            // frame: background, content inside the border and padding, border on top
            let sides = frame.sides.array();
            let border = frame.width > 0.0 && sides.iter().any(|&s| s);
            if let Some(fill) = &frame.fill {
                // square corner where the border stops (one side shown, the next one not)
                let radii: [f32; 4] = std::array::from_fn(|c| {
                    let (a, b) = (sides[(c + 3) % 4], sides[c]); // corners TL, TR, BR, BL
                    if border && a != b { 0.0 } else { frame.radius }
                });
                let sk = box_shader(fill, w.x, w.y, w.w, w.h);
                if let Some(path) = round_rect_corners(
                    w.x as f32, w.y as f32, w.w as f32, w.h as f32, radii,
                ) {
                    pm.fill_path(&path, &sk, FillRule::Winding, Transform::identity(), None);
                }
            }
            self.widget(i, &w.inner(), view, pm);
            if border {
                let sk = box_shader(&frame.border, w.x, w.y, w.w, w.h);
                stroke_sides(pm, (w.x, w.y, w.w, w.h), frame.radius, frame.width, sides, &sk);
            }
        }
    }

    fn widget(&mut self, index: usize, w: &Widget, view: &View, pm: &mut Pixmap) {
        let knob = |k: u8| {
            view.knobs
                .get(k.saturating_sub(1) as usize)
                .cloned()
                .unwrap_or_default()
        };
        match &w.kind {
            WidgetKind::Rect { color, radius } => {
                let sk = box_shader(color, w.x, w.y, w.w, w.h);
                fill_shape(pm, w.x, w.y, w.w, w.h, *radius, &sk);
            }
            WidgetKind::Separator {
                color,
                thickness,
                vertical,
                style,
                fade,
            } => separator(pm, w, color, *thickness, *vertical, *style, *fade),
            WidgetKind::Text {
                text,
                knob: k,
                size,
                color,
                align,
                bold,
                scroll,
            } => {
                let t = k.map(knob).unwrap_or_default();
                let s = expand(text, &t, &view.page_name);
                let style = TextStyle {
                    size: *size,
                    bold: *bold,
                    color: color.clone(),
                    align: *align,
                    scroll: scroll.then_some(view.time),
                };
                self.text(pm, &s, &style, w);
            }
            WidgetKind::Vu {
                knob: k,
                vertical,
                color,
                gauge,
            } => {
                let t = knob(*k);
                let meter = crate::gauge::Meter {
                    value: if t.present { t.level } else { 0.0 },
                    peak: t.present.then_some(t.peak_hold),
                    color,
                    vertical: *vertical,
                    muted: t.muted,
                    radius: 0.0,
                };
                crate::gauge::draw(pm, w, gauge, &meter, &mut self.gauge_images);
            }
            WidgetKind::Volume {
                knob: k,
                color,
                vertical,
                gauge,
            } => {
                let t = knob(*k);
                let meter = crate::gauge::Meter {
                    value: if t.present { t.volume.min(1.0) } else { 0.0 },
                    peak: None,
                    color,
                    vertical: *vertical,
                    muted: t.muted,
                    radius: 3.0,
                };
                crate::gauge::draw(pm, w, gauge, &meter, &mut self.gauge_images);
            }
            WidgetKind::Spectrum {
                bars,
                gap,
                colors,
                peak,
                background,
                ..
            } => {
                fill_round_rect(pm, w.x, w.y, w.w, w.h, 0.0, *background);
                let Some(levels) = view.spectra.get(&index) else {
                    return;
                };
                let cols = crate::visual::bar_columns(w.x, w.w, *bars as usize, *gap);
                for ((x, bw), &(level, cap)) in cols.into_iter().zip(levels) {
                    let state = crate::visual::column_state(w.h, level, cap, peak.is_some());
                    let rects = crate::visual::column_rects(
                        w.y,
                        w.h,
                        state,
                        colors,
                        peak.unwrap_or(*background),
                        *background,
                    );
                    for (y, h, c) in rects {
                        fill_round_rect(pm, x, y, bw, h, 0.0, c);
                    }
                }
            }
            WidgetKind::Icon {
                knob: k,
                style,
                tint,
            } => {
                let t = knob(*k);
                if !t.present {
                    return;
                }
                let (bw, bh) = (w.w.max(1) as u32, w.h.max(1) as u32);
                let tint = match style {
                    IconStyle::Auto => t.device.then_some(*tint),
                    IconStyle::Color => None,
                    IconStyle::Symbolic => Some(*tint),
                };
                let icon = self
                    .icons
                    .entry((t.icon.clone(), bw, bh, tint))
                    .or_insert_with(|| load_icon(&t.icon, bw, bh, tint));
                if let Some(img) = icon {
                    let paint = PixmapPaint {
                        opacity: if t.muted { 0.3 } else { 1.0 },
                        ..Default::default()
                    };
                    let (x, y) = (
                        w.x + (bw - img.width()) as i32 / 2,
                        w.y + (bh - img.height()) as i32 / 2,
                    );
                    pm.draw_pixmap(x, y, img.as_ref(), &paint, Transform::identity(), None);
                }
            }
            WidgetKind::Tool { overflow, .. } => {
                // its own `tool` widgets show nothing (no `embedded` there)
                if let Some((inner, size)) = view.embedded.get(&index) {
                    self.module_view(pm, w, (inner, *size), *overflow, (view.time, view.anim_time));
                }
            }
            WidgetKind::Deck {
                vertical,
                color,
                tiles,
                ..
            } => {
                let dv = view.decks.get(&index).copied().unwrap_or_default();
                self.deck(pm, w, (*vertical, *color), tiles, dv);
            }
            WidgetKind::ThemeIcon { name, tint } => {
                let (bw, bh) = (w.w.max(1) as u32, w.h.max(1) as u32);
                let icon = self
                    .icons
                    .entry((name.clone(), bw, bh, *tint))
                    .or_insert_with(|| load_icon(name, bw, bh, *tint));
                if let Some(img) = icon {
                    let (x, y) = (
                        w.x + (bw - img.width()) as i32 / 2,
                        w.y + (bh - img.height()) as i32 / 2,
                    );
                    let p = PixmapPaint::default();
                    pm.draw_pixmap(x, y, img.as_ref(), &p, Transform::identity(), None);
                }
            }
            WidgetKind::Image { path, radius } => {
                let img = self
                    .images
                    .entry(path.clone())
                    .or_insert_with(|| Pixmap::load_png(path).ok());
                if let Some(img) = img {
                    draw_image(pm, img, (w.x as f32, w.y as f32, w.w as f32, w.h as f32), *radius);
                }
            }
        }
    }

    /// "Volume au-dessus de 100 % ?" over the dimmed page: warning, « Oui » / « Non » (selected
    /// one filled), how to answer.
    pub fn confirm(&mut self, pm: &mut Pixmap, c: &ConfirmView) {
        for p in pm.data_mut().as_chunks_mut::<4>().0 {
            for v in &mut p[..3] {
                *v = (*v as u16 * 3 / 10) as u8;
            }
        }
        const WARN: Color = Color(0xe0, 0xb0, 0x40);
        const BOX: Color = Color(0x14, 0x18, 0x18);
        const GREY: Color = Color(0xa0, 0xa8, 0xa8);
        let (bx, by, bw, bh) = (40, 36, 400, 200);
        fill_round_rect(pm, bx, by, bw, bh, 10.0, WARN);
        fill_round_rect(pm, bx + 2, by + 2, bw - 4, bh - 4, 9.0, BOX);
        let label = |by_off: i32, h: i32| (bx + 12, by + by_off, bw - 24, h);
        self.centered(
            pm,
            tr("Volume above 100%?"),
            label(10, 28),
            20.0,
            true,
            WARN,
        );
        self.centered(
            pm,
            &c.name,
            label(40, 22),
            16.0,
            false,
            Color(0x9a, 0xd8, 0xd0),
        );
        self.centered(
            pm,
            tr("This may damage the speakers"),
            label(68, 20),
            15.0,
            false,
            Color(255, 255, 255),
        );
        self.centered(
            pm,
            tr("or your hearing."),
            label(88, 20),
            15.0,
            false,
            Color(255, 255, 255),
        );
        for (i, (text, selected, fill)) in [
            (tr("Yes"), c.yes, Color(0xc0, 0x40, 0x30)),
            (tr("No"), !c.yes, Color(0x00, 0x6c, 0x68)),
        ]
        .into_iter()
        .enumerate()
        {
            let (x, y, w, h) = (bx + 60 + i as i32 * 160, by + 120, 120, 38);
            if selected {
                fill_round_rect(pm, x - 3, y - 3, w + 6, h + 6, 9.0, Color(255, 255, 255));
                fill_round_rect(pm, x, y, w, h, 7.0, fill);
            } else {
                fill_round_rect(pm, x, y, w, h, 7.0, Color(0x3a, 0x40, 0x40));
                fill_round_rect(pm, x + 2, y + 2, w - 4, h - 4, 6.0, BOX);
            }
            let color = if selected { Color(255, 255, 255) } else { GREY };
            self.centered(pm, text, (x, y, w, h), 20.0, true, color);
        }
        self.centered(
            pm,
            tr("turn: choose · click: confirm"),
            label(168, 20),
            14.0,
            false,
            GREY,
        );
    }

    /// The modules menu, slid up from the bottom by `m.shown`: a band of cards (module name,
    /// page name), the selected one in the accent colour and kept in view.
    pub fn module_menu(&mut self, pm: &mut Pixmap, m: &crate::state::MenuView) {
        const BG: Color = Color(0x0b, 0x10, 0x16);
        const ACCENT: Color = Color(0x00, 0xb4, 0xa8);
        const GREY: Color = Color(0x8a, 0x96, 0x96);
        const CARD: Color = Color(0x16, 0x1d, 0x26);
        let (_, top, w, h) = MENU_BOX;
        let y = top + ((1.0 - m.shown.clamp(0.0, 1.0)) * h as f32).round() as i32;
        if y >= SCREEN_H {
            return;
        }
        let shape = round_rect_corners(0.0, y as f32, w as f32, (h + 12) as f32, [12.0, 12.0, 0.0, 0.0]);
        if let Some(path) = shape {
            pm.fill_path(&path, &paint(BG), FillRule::Winding, Transform::identity(), None);
            let stroke = tiny_skia::Stroke { width: 1.5, ..Default::default() };
            pm.stroke_path(&path, &paint(ACCENT), &stroke, Transform::identity(), None);
        }
        self.line(pm, tr("Modules"), (12, y + 5, 200, 18), 13.0, true, ACCENT, Align::Left);
        let hint = tr("knob 1: choose · click: open");
        self.line(pm, hint, (200, y + 5, 268, 18), 12.0, false, GREY, Align::Right);
        if m.items.is_empty() {
            let text = tr("no module has a page (Modules tab)");
            self.centered(pm, text, (0, y + 34, w, 30), 14.0, false, GREY);
            return;
        }
        // cards in a row, scrolled so that the selected one stays in view
        const CW: i32 = 140;
        const GAP: i32 = 8;
        let pitch = CW + GAP;
        let total = m.items.len() as i32 * pitch - GAP;
        let scroll = (m.selected as i32 * pitch + CW / 2 - w / 2).clamp(0, (total - w + 24).max(0));
        let x0 = if total + 24 <= w { (w - total) / 2 } else { 12 - scroll };
        for (i, (name, title)) in m.items.iter().enumerate() {
            let x = x0 + i as i32 * pitch;
            if x + CW < 0 || x > w {
                continue;
            }
            let on = i == m.selected;
            let (cy, ch) = (y + 28, 54);
            fill_round_rect(pm, x, cy, CW, ch, 8.0, if on { mix(CARD, ACCENT, 0.3) } else { CARD });
            let edge = if on { ACCENT } else { Color(0x26, 0x30, 0x40) };
            stroke_shape(pm, (x, cy, CW, ch), 8.0, if on { 2.0 } else { 1.0 }, &paint(edge));
            let color = if on { Color(255, 255, 255) } else { Color(0xc8, 0xd0, 0xd0) };
            self.line(pm, name, (x + 6, cy + 8, CW - 12, 20), 14.0, true, color, Align::Center);
            self.line(pm, title, (x + 6, cy + 30, CW - 12, 16), 11.0, false, GREY, Align::Center);
        }
    }

    /// Notification popup at the top of the screen (`NOTICE_BOX`, over the page): picture,
    /// application, title, message on 2 lines at most; the bar at the bottom shrinks with the
    /// time left. `age`: seconds since it is shown, of `total`. A pill: `Renderer::pill`.
    pub fn notice(
        &mut self,
        pm: &mut Pixmap,
        n: &crate::notify::Notification,
        (age, total): (f32, f32),
    ) {
        if let crate::notify::NoticeStyle::Pill { border, dot, color } = n.style {
            return self.pill(pm, &n.summary, (border, dot, color), (age, total));
        }
        let left = 1.0 - age / total.max(0.1);
        const ACCENT: Color = Color(0x00, 0xb4, 0xa8);
        const BOX: Color = Color(0x12, 0x17, 0x17);
        const GREY: Color = Color(0x9a, 0xa4, 0xa4);
        const WHITE: Color = Color(255, 255, 255);
        let (bx, by, bw, bh) = NOTICE_BOX;
        fill_round_rect(pm, bx, by, bw, bh, 12.0, ACCENT);
        fill_round_rect(pm, bx + 2, by + 2, bw - 4, bh - 4, 11.0, BOX);
        let side = crate::notify::ICON as i32;
        let (ix, iy) = (bx + 14, by + (bh - side) / 2 - 2);
        match &n.icon {
            Some(img) => {
                let (x, y) = (
                    ix + (side - img.width() as i32) / 2,
                    iy + (side - img.height() as i32) / 2,
                );
                let p = PixmapPaint::default();
                pm.draw_pixmap(x, y, img.as_ref(), &p, Transform::identity(), None);
            }
            None => {
                // no picture: initial of the application in a disc
                fill_round_rect(pm, ix, iy, side, side, side as f32 / 2.0, ACCENT);
                let initial: String = n.app.chars().take(1).collect();
                self.centered(pm, &initial, (ix, iy, side, side), 26.0, true, WHITE);
            }
        }
        let (tx, tw) = (ix + side + 14, bx + bw - 14 - (ix + side + 14));
        let left_line = |y: i32, h: i32| (tx, y, tw, h);
        self.line(pm, &n.app, left_line(by + 8, 18), 13.0, false, GREY, Align::Left);
        let (title, body) = if n.body.is_empty() {
            (self.wrap(&n.summary, 18.0, true, tw as f32, 2), Vec::new())
        } else {
            let title = self.wrap(&n.summary, 18.0, true, tw as f32, 1);
            (title, self.wrap(&n.body, 15.0, false, tw as f32, 2))
        };
        let mut y = by + 27;
        for t in &title {
            self.line(pm, t, left_line(y, 25), 18.0, true, WHITE, Align::Left);
            y += 25;
        }
        for t in &body {
            self.line(pm, t, left_line(y, 21), 15.0, false, Color(0xd8, 0xde, 0xde), Align::Left);
            y += 20;
        }
        // remaining time
        let full = bw - 28;
        let len = (full as f32 * left.clamp(0.0, 1.0)).round() as i32;
        fill_round_rect(pm, bx + 14, by + bh - 8, full, 3, 1.5, Color(0x26, 0x30, 0x30));
        fill_round_rect(pm, bx + 14, by + bh - 8, len, 3, 1.5, ACCENT);
    }

    /// A module's page or widget (`inner`, its content `cw`×`ch`) in the box of `w`: drawn
    /// whole, then shrunk to fit (exact pixels when it fits as is), or with `Overflow::Scroll`
    /// its width fitted (1:1 when narrow enough) and what does not fit in the height scrolling
    /// down and back by itself (`time`); `Overflow::ScrollSideways`: the same across (height
    /// fitted, scrolling right and back).
    fn module_view(
        &mut self,
        pm: &mut Pixmap,
        w: &Widget,
        (inner, (cw, ch)): (&Page, (i32, i32)),
        overflow: Overflow,
        (time, anim_time): (f32, f32),
    ) {
        let (cw, ch) = (cw.max(1) as u32, ch.max(1) as u32);
        let mut off = match self.offscreen.take() {
            Some(p) if (p.width(), p.height()) == (cw, ch) => p,
            _ => Pixmap::new(cw, ch).expect("pixmap"),
        };
        let sub = View { time, anim_time, ..Default::default() };
        self.render(inner, &sub, &mut off);
        let (bw, bh, cwf, chf) = (w.w as f32, w.h as f32, cw as f32, ch as f32);
        let k = match overflow {
            Overflow::Shrink => (bw / cwf).min(bh / chf),
            Overflow::Scroll => (bw / cwf).min(1.0),
            Overflow::ScrollSideways => (bh / chf).min(1.0),
        };
        let (dw, dh) = (cwf * k, chf * k);
        let sideways = overflow == Overflow::ScrollSideways;
        // where it sits: centred, at the top of a scrolling box, at the left of a sideways one
        let ox = if sideways { w.x as f32 } else { w.x as f32 + (bw - dw) / 2.0 };
        let oy = if overflow == Overflow::Scroll { w.y as f32 } else { w.y as f32 + (bh - dh) / 2.0 };
        let smooth = PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() };
        let over = if sideways { dw.round() - bw } else { dh.round() - bh };
        if over < 1.0 {
            // it fits (shrunk if needed)
            if k >= 0.999 {
                let (x, y) = (ox.round() as i32, oy.round() as i32);
                pm.draw_pixmap(x, y, off.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            } else {
                let t = Transform::from_row(k, 0.0, 0.0, k, ox, oy);
                pm.draw_pixmap(0, 0, off.as_ref(), &smooth, t, None);
            }
        } else {
            // the scrolled band of the content (scaled first if bigger than the box across)
            let scaled;
            let src: &Pixmap = if k >= 0.999 {
                &off
            } else {
                let mut s = Pixmap::new(dw.round().max(1.0) as u32, dh.round().max(1.0) as u32)
                    .expect("pixmap");
                s.draw_pixmap(0, 0, off.as_ref(), &smooth, Transform::from_scale(k, k), None);
                scaled = s;
                &scaled
            };
            let shift = scroll_offset(over, time) as u32;
            let band = if sideways {
                let cols = (w.w.max(1) as u32).min(src.width());
                let left = shift.min(src.width() - cols);
                tiny_skia::IntRect::from_xywh(left as i32, 0, cols, src.height())
            } else {
                let rows = (w.h.max(1) as u32).min(src.height());
                let top = shift.min(src.height() - rows);
                tiny_skia::IntRect::from_xywh(0, top as i32, src.width(), rows)
            };
            if let Some(part) = band.and_then(|r| src.clone_rect(r)) {
                let (x, y) = (ox.round() as i32, oy.round() as i32);
                pm.draw_pixmap(x, y, part.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
        }
        self.offscreen = Some(off);
    }

    /// Pill notification: `text` (one line, bold) centred at the bottom in a dark rounded pill,
    /// outlined in `border` with a soft glow, a `dot` at each end whose glow breathes; as wide
    /// as the text. It slides up from below the screen when it comes (`age` 0 → `PILL_SLIDE`)
    /// and down before its `total` time ends.
    fn pill(
        &mut self,
        pm: &mut Pixmap,
        text: &str,
        (border, dot, color): (Color, Color, Color),
        (age, total): (f32, f32),
    ) {
        const FILL: Color = Color(0x0f, 0x13, 0x15);
        let ease = |t: f32| 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
        let shown = ease(age / PILL_SLIDE).min(ease((total - age) / PILL_SLIDE));
        let below = SCREEN_H - PILL_Y + 8; // out of the screen, glow included
        let max = (SCREEN_W - 24 - 2 * PILL_END) as f32;
        let text = self.ellipsize(text.trim(), PILL_TEXT, true, max);
        let w = self.text_width(&text, PILL_TEXT, true).ceil() as i32 + 2 * PILL_END;
        let y = PILL_Y + ((1.0 - shown) * below as f32).round() as i32;
        let (x, h) = ((SCREEN_W - w) / 2, PILL_H);
        let breath = 0.5 + 0.5 * (age * std::f32::consts::TAU / PILL_PULSE).sin();
        let r = h as f32 / 2.0;
        // glow: wider, fainter pills behind the outline
        for (grow, alpha) in [(6.0, 22), (4.0, 34), (2.0, 60)] {
            glow_round_rect(pm, (x as f32, y as f32, w as f32, h as f32), grow, r, (border, alpha));
        }
        fill_round_rect(pm, x, y, w, h, r, border);
        fill_round_rect(pm, x + 2, y + 2, w - 4, h - 4, r - 2.0, FILL);
        for cx in [x + PILL_END / 2, x + w - PILL_END / 2] {
            let cy = (y + h / 2) as f32;
            let glow = (35.0 + 75.0 * breath) as u8;
            glow_round_rect(pm, (cx as f32, cy, 0.0, 0.0), 7.0 + 1.5 * breath, 0.0, (dot, glow));
            glow_round_rect(pm, (cx as f32, cy, 0.0, 0.0), 4.0, 0.0, (dot, 255));
        }
        let box_ = (x + PILL_END, y, w - 2 * PILL_END, h);
        self.line(pm, &text, box_, PILL_TEXT, true, color, Align::Center);
    }

    /// `s` on `max_lines` lines of `max` px at most, cut between words when possible; the last
    /// line ends with "…" if the text is longer.
    fn wrap(&mut self, s: &str, size: f32, bold: bool, max: f32, max_lines: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let mut rest = s.split_whitespace().collect::<Vec<_>>().join(" ");
        while !rest.is_empty() && lines.len() < max_lines {
            // longest prefix that fits (byte index)
            let (mut width, mut end) = (0.0, 0);
            for (i, c) in rest.char_indices() {
                width += self.advance(c, size, bold);
                if width > max {
                    break;
                }
                end = i + c.len_utf8();
            }
            if end == rest.len() {
                lines.push(rest);
                break;
            }
            if lines.len() + 1 == max_lines {
                let cut = self.ellipsize(&format!("{}…", &rest[..end]), size, bold, max);
                lines.push(cut);
                break;
            }
            let end = end.max(rest.chars().next().map_or(0, char::len_utf8));
            // last space where a line may break (not before « ? ! : ; » » nor after « « »)
            let head = &rest[..end];
            let cut = head
                .rmatch_indices(' ')
                .map(|(i, _)| i)
                .find(|&i| {
                    i > 0
                        && !rest[i + 1..].starts_with(['?', '!', ':', ';', '»'])
                        && !head[..i].ends_with('«')
                })
                .unwrap_or(end);
            lines.push(rest[..cut].trim_end().to_owned());
            rest = rest[cut..].trim_start().to_owned();
        }
        lines
    }

    /// A `deck` track in the box of `w`: the selected tile in the middle (drawn `offset` tiles
    /// further during a slide), its neighbours before and after it, endlessly (tile i mod n).
    /// Drawn in a pixmap of the box: the tiles that leave it are cut at its edges.
    fn deck(
        &mut self,
        pm: &mut Pixmap,
        w: &Widget,
        (vertical, accent): (bool, Color),
        tiles: &[Tile],
        dv: DeckView,
    ) {
        const GAP: i32 = 6;
        let (bw, bh) = (w.w.max(1), w.h.max(1));
        let Some(mut buf) = Pixmap::new(bw as u32, bh as u32) else {
            return;
        };
        let n = tiles.len();
        if n == 0 {
            stroke_shape(&mut buf, (0, 0, bw, bh), 8.0, 1.0, &paint(DECK_EDGE));
            self.centered(&mut buf, tr("no tile"), (0, 0, bw, bh), 13.0, false, DECK_DIM);
        } else {
            // a tile: 2/3 as high as the column is wide, or 3/2 as wide as the row is high
            let (cw, ch) = if vertical {
                (bw, (bw * 2 / 3).clamp(24, bh))
            } else {
                ((bh * 3 / 2).clamp(32, bw), bh)
            };
            let (len, cell) = if vertical { (bh, ch) } else { (bw, cw) };
            let pitch = (cell + GAP) as f32;
            let centre = (len - cell) as f32 / 2.0;
            let reach = if n > 1 { (len as f32 / pitch / 2.0).ceil() as i32 + 2 } else { 0 };
            for j in -reach..=reach {
                let slot = j as f32 + dv.offset;
                let at = centre + slot * pitch;
                if at + cell as f32 <= 0.0 || at >= len as f32 {
                    continue;
                }
                let i = (dv.selected as i64 + j as i64).rem_euclid(n as i64) as usize;
                let (x, y) = if vertical { (0, at.round() as i32) } else { (at.round() as i32, 0) };
                // 1 for the tile in the middle, 0 from one tile away
                let near = (1.0 - slot.abs()).clamp(0.0, 1.0);
                let flash = if j == 0 { dv.flash } else { 0.0 };
                self.deck_tile(&mut buf, (x, y, cw, ch), &tiles[i], accent, (near, flash));
            }
        }
        let p = PixmapPaint::default();
        pm.draw_pixmap(w.x, w.y, buf.as_ref(), &p, Transform::identity(), None);
    }

    /// One tile of a deck: icon above, title, subtitle when there is room; `near` = how much it
    /// is the selected one (accent colour, brighter), `flash` = just clicked.
    fn deck_tile(
        &mut self,
        pm: &mut Pixmap,
        (x, y, w, h): (i32, i32, i32, i32),
        tile: &Tile,
        accent: Color,
        (near, flash): (f32, f32),
    ) {
        const PAD: i32 = 5;
        let fill = mix(mix(DECK_FILL, mix(DECK_FILL, accent, 0.25), near), accent, 0.55 * flash);
        fill_round_rect(pm, x, y, w, h, 8.0, fill);
        let edge = mix(DECK_EDGE, accent, near);
        stroke_shape(pm, (x, y, w, h), 8.0, 1.0 + near, &paint(edge));
        let title_size = (h as f32 * 0.17).clamp(10.0, 15.0);
        let sub_size = (title_size * 0.8).max(9.0);
        let (title_h, sub_h) = (title_size.ceil() as i32 + 4, sub_size.ceil() as i32 + 3);
        let with_sub = !tile.subtitle.is_empty() && h >= 60;
        let text_h = title_h + if with_sub { sub_h } else { 0 };
        let side = (h - text_h - 2 * PAD).min(w - 2 * PAD).min(48);
        if side >= 12 && !tile.icon.is_empty() {
            // monochrome icons in the accent colour once selected, light otherwise
            let tint = (near > 0.5).then_some(accent);
            let s = side as u32;
            let icon = self
                .icons
                .entry((tile.icon.clone(), s, s, tint))
                .or_insert_with(|| load_icon(&tile.icon, s, s, tint));
            if let Some(img) = icon {
                let (ix, iy) = (
                    x + (w - img.width() as i32) / 2,
                    y + PAD + (side - img.height() as i32) / 2,
                );
                let p = PixmapPaint {
                    opacity: 0.55 + 0.45 * near,
                    ..Default::default()
                };
                pm.draw_pixmap(ix, iy, img.as_ref(), &p, Transform::identity(), None);
            }
        }
        let ty = y + h - PAD - text_h;
        let title = mix(DECK_DIM, mix(accent, Color(0xff, 0xff, 0xff), 0.35), near);
        let r = (x + PAD, ty, w - 2 * PAD, title_h);
        self.line(pm, &tile.title, r, title_size, true, title, Align::Center);
        if with_sub {
            let r = (x + PAD, ty + title_h, w - 2 * PAD, sub_h);
            self.line(pm, &tile.subtitle, r, sub_size, false, DECK_DIM, Align::Center);
        }
    }

    /// One line of text centred in the box `r` = (x, y, w, h).
    fn centered(
        &mut self,
        pm: &mut Pixmap,
        s: &str,
        r: (i32, i32, i32, i32),
        size: f32,
        bold: bool,
        color: Color,
    ) {
        self.line(pm, s, r, size, bold, color, Align::Center);
    }

    /// One line of text in the box `r` = (x, y, w, h), cut with "…" if too long.
    #[allow(clippy::too_many_arguments)]
    fn line(
        &mut self,
        pm: &mut Pixmap,
        s: &str,
        r: (i32, i32, i32, i32),
        size: f32,
        bold: bool,
        color: Color,
        align: Align,
    ) {
        let style = TextStyle {
            size,
            bold,
            color: Paint::Solid(color),
            align,
            scroll: None,
        };
        let kind = WidgetKind::Rect {
            color: Paint::Solid(Color(0, 0, 0)),
            radius: 0.0,
        };
        let (x, y, w, h) = r;
        self.text(
            pm,
            s,
            &style,
            &Widget {
                x,
                y,
                w,
                h,
                frame: None,
                kind,
            },
        );
    }

    /// The glyph of `ch`: from the text font, else from the first fallback font that has it
    /// (arrows, symbols…), else the text font's « missing » box.
    fn glyph(&mut self, ch: char, size: f32, bold: bool) -> &(Metrics, Vec<u8>) {
        let key = (ch, (size * 10.0) as u32, bold);
        let main = if bold { &self.bold } else { &self.regular };
        let font = if main.lookup_glyph_index(ch) != 0 {
            main
        } else {
            self.fallback.iter().find(|f| f.lookup_glyph_index(ch) != 0).unwrap_or(main)
        };
        self.glyphs
            .entry(key)
            .or_insert_with(|| font.rasterize(ch, size))
    }

    /// Whether `ch` is drawn as a colour emoji: the text font lacks it, the emoji font has it.
    fn is_emoji(&mut self, ch: char, bold: bool) -> bool {
        let main = if bold { &self.bold } else { &self.regular };
        if ch.is_ascii() || main.lookup_glyph_index(ch) != 0 {
            return false;
        }
        let Some(data) = &self.emoji_font else {
            return false;
        };
        *self.emoji_has.entry(ch).or_insert_with(|| {
            ttf_parser::Face::parse(data, 0).ok().and_then(|f| f.glyph_index(ch)).is_some()
        })
    }

    /// The colour image of emoji `ch` for a text of `size` px (cached).
    fn emoji_image(&mut self, ch: char, size: f32) -> Option<Pixmap> {
        let key = (ch, (size * 10.0) as u32);
        if let Some(img) = self.emoji.get(&key) {
            return img.clone();
        }
        let img = self.emoji_font.as_deref().and_then(|d| emoji_png(d, ch, emoji_side(size)));
        self.emoji.insert(key, img.clone());
        img
    }

    /// What draws `ch`: a glyph, a colour emoji, or nothing (modifiers).
    fn piece(&mut self, ch: char, size: f32, bold: bool) -> Piece {
        if modifier(ch) {
            return Piece::Nothing;
        }
        if self.is_emoji(ch, bold)
            && let Some(img) = self.emoji_image(ch, size)
        {
            return Piece::Emoji(img, emoji_advance(size));
        }
        let (m, cov) = self.glyph(ch, size, bold).clone();
        Piece::Glyph(m, cov)
    }

    /// How far the pen moves for `ch`.
    fn advance(&mut self, ch: char, size: f32, bold: bool) -> f32 {
        if modifier(ch) {
            0.0
        } else if self.is_emoji(ch, bold) {
            emoji_advance(size)
        } else {
            self.glyph(ch, size, bold).0.advance_width
        }
    }

    fn text_width(&mut self, s: &str, size: f32, bold: bool) -> f32 {
        s.chars().map(|c| self.advance(c, size, bold)).sum()
    }

    /// `s` shortened with "…" so that it fits in `max` pixels.
    fn ellipsize(&mut self, s: &str, size: f32, bold: bool, max: f32) -> String {
        if self.text_width(s, size, bold) <= max {
            return s.to_owned();
        }
        let mut out: String = s.to_owned();
        while !out.is_empty() {
            out.pop();
            let candidate = format!("{}…", out.trim_end());
            if self.text_width(&candidate, size, bold) <= max {
                return candidate;
            }
        }
        "…".into()
    }

    fn text(&mut self, pm: &mut Pixmap, s: &str, style: &TextStyle, w: &Widget) {
        let TextStyle {
            size,
            bold,
            ref color,
            align,
            scroll,
        } = *style;
        // colour of each column (or row) of the box: a gradient stays put under a scrolling text
        let vertical = color.vertical();
        let along = if vertical { w.h } else { w.w }.max(1);
        let lut: Vec<Color> = match color {
            Paint::Solid(c) => vec![*c],
            _ => (0..along)
                .map(|i| color.at((i as f32 + 0.5) / along as f32))
                .collect(),
        };
        let colors = Colors {
            lut: &lut,
            vertical,
        };
        let full = self.text_width(s, size, bold);
        let over = full - w.w as f32;
        // too long: scroll back and forth (offset in whole pixels), or cut with "…"
        let (s, offset) = match scroll {
            Some(t) if over > 0.0 => (s.to_owned(), Some(marquee(over, t))),
            _ => (self.ellipsize(s, size, bold, w.w as f32), None),
        };
        let s = s.as_str();
        let width = self.text_width(s, size, bold);
        let lm = if bold { &self.bold } else { &self.regular }.horizontal_line_metrics(size);
        let (ascent, descent) = lm.map_or((size * 0.8, -size * 0.2), |m| (m.ascent, m.descent));
        let baseline = w.y as f32 + (w.h as f32 - (ascent - descent)) / 2.0 + ascent;
        let mut pen = match (offset, align) {
            (Some(off), _) => w.x as f32 - off,
            (None, Align::Left) => w.x as f32,
            (None, Align::Center) => w.x as f32 + (w.w as f32 - width) / 2.0,
            (None, Align::Right) => (w.x + w.w) as f32 - width,
        };
        for ch in s.chars() {
            match self.piece(ch, size, bold) {
                Piece::Glyph(m, cov) => {
                    let gx = (pen + m.xmin as f32).round() as i32;
                    let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
                    blend_coverage(pm, (gx, gy), (m.width, m.height), &cov, &colors, w);
                    pen += m.advance_width;
                }
                Piece::Emoji(img, advance) => {
                    // its own colours, sitting on the baseline like a capital letter
                    let gx = (pen + (advance - img.width() as f32) / 2.0).round() as i32;
                    let gy = (baseline - img.height() as f32 * 0.84).round() as i32;
                    draw_in_columns(pm, &img, (gx, gy), (w.x, w.x + w.w));
                    pen += advance;
                }
                Piece::Nothing => {}
            }
        }
    }
}

struct TextStyle {
    size: f32,
    bold: bool,
    color: Paint,
    align: Align,
    /// Time (s) for scrolling texts that do not fit, `None` = cut with "…".
    scroll: Option<f32>,
}

/// Scroll offset (px) of a text `over` pixels too long at time `t`: pause at the start, slide to
/// the end, pause, slide back. Moves in 2 px steps (15 redraws/s instead of 30: each step resends
/// the text's tiles).
fn marquee(over: f32, t: f32) -> f32 {
    back_and_forth(over, t, 30.0, 1.5)
}

/// Scroll offset (px) of module content `over` pixels taller than its box: slower than texts,
/// longer pauses (a list read at the top and at the bottom).
fn scroll_offset(over: f32, t: f32) -> f32 {
    back_and_forth(over, t, 20.0, 2.5)
}

/// Offset 0 → `over` → 0 at `speed` px/s with `pause` s at each end, in 2 px steps.
fn back_and_forth(over: f32, t: f32, speed: f32, pause: f32) -> f32 {
    let travel = over / speed;
    let p = t.rem_euclid(2.0 * (pause + travel));
    let x = if p < pause {
        0.0
    } else if p < pause + travel {
        (p - pause) * speed
    } else if p < 2.0 * pause + travel {
        over
    } else {
        over - (p - 2.0 * pause - travel) * speed
    };
    ((x.clamp(0.0, over) / 2.0).round() * 2.0).min(over.ceil())
}

/// `{name}`, `{volume}` (e.g. "42 %"), `{mute}` (" MUTE" when muted), `{page}`.
pub fn expand(text: &str, t: &TargetState, page: &str) -> String {
    let volume = if t.present {
        format!("{} %", (t.volume * 100.0).round() as i32)
    } else {
        "–".into()
    };
    let name = if t.present { t.name.as_str() } else { "—" };
    text.replace("{name}", name)
        .replace("{volume}", &volume)
        .replace("{mute}", if t.muted { " MUTE" } else { "" })
        .replace("{page}", page)
}

/// Icon `name` (see `icons.rs`) fitted in `w`×`h` keeping its proportions; SVG drawn at that
/// size, PNG scaled. With `tint`, its symbolic (monochrome) version in that colour if there is
/// one. Monochrome icons (Breeze colours them through the `ColorScheme-Text` style, #232629) are
/// drawn in the tint, or light without one.
pub fn load_icon(name: &str, w: u32, h: u32, tint: Option<Color>) -> Option<Pixmap> {
    let symbolic = tint.and_then(|c| Some((crate::icons::find_symbolic(name)?, c)));
    let path = match &symbolic {
        Some((p, _)) => p.clone(),
        None => crate::icons::find(name)?,
    };
    if path.extension().is_some_and(|e| e == "svg") {
        let mut data = std::fs::read(&path).ok()?;
        let text = String::from_utf8_lossy(&data).into_owned();
        // monochrome theme icons take their colour from the `ColorScheme-Text` style (Breeze:
        // #232629, dark grey, invisible on the device's dark pages): drawn in the tint, or light
        if symbolic.is_some() || text.contains("ColorScheme-Text") {
            let c = tint.unwrap_or(ICON_LIGHT);
            let hex = format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2);
            let mut svg = text_colored(&text, &hex).replace("#232629", &hex);
            if symbolic.is_some() {
                // GNOME symbolic icons (Adwaita…) hard-code their foreground grey
                for fg in ["#2e3436", "#222222", "#bebebe", "#474747"] {
                    svg = svg.replace(fg, &hex);
                }
            }
            data = svg.into_bytes();
        }
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
        let s = tree.size();
        let k = (w as f32 / s.width()).min(h as f32 / s.height());
        let (iw, ih) = (
            (s.width() * k).round().max(1.0) as u32,
            (s.height() * k).round().max(1.0) as u32,
        );
        let mut pm = Pixmap::new(iw, ih)?;
        resvg::render(&tree, Transform::from_scale(k, k), &mut pm.as_mut());
        Some(pm)
    } else {
        // PNG, JPEG, ICO (largest entry) … through the image crate
        let src = to_pixmap(open_image(&path)?.into_rgba8())?;
        let k = (w as f32 / src.width() as f32).min(h as f32 / src.height() as f32);
        let (iw, ih) = (
            (src.width() as f32 * k).round().max(1.0) as u32,
            (src.height() as f32 * k).round().max(1.0) as u32,
        );
        let mut pm = Pixmap::new(iw, ih)?;
        let paint = PixmapPaint {
            quality: tiny_skia::FilterQuality::Bicubic,
            ..Default::default()
        };
        pm.draw_pixmap(
            0,
            0,
            src.as_ref(),
            &paint,
            Transform::from_scale(k, k),
            None,
        );
        Some(pm)
    }
}

/// Monochrome theme icons without a tint: light, for the device's dark pages.
const ICON_LIGHT: Color = Color(0xe8, 0xee, 0xee);

/// The SVG with its `ColorScheme-Text` colour (the CSS rule that `currentColor` follows) set to
/// `hex`; without that rule, `currentColor` itself becomes `hex`. Other scheme colours
/// (highlight, negative text…) stay.
fn text_colored(svg: &str, hex: &str) -> String {
    let Some(i) = svg.find(".ColorScheme-Text") else {
        return svg.replace("currentColor", hex);
    };
    let Some(c) = svg[i..].find("color:") else {
        return svg.to_owned();
    };
    let start = i + c + "color:".len();
    let end = svg[start..].find([';', '}']).map_or(svg.len(), |e| start + e);
    format!("{}{hex}{}", &svg[..start], &svg[end..])
}

/// An image file, its format found from its content (downloaded covers have no reliable
/// extension).
pub fn open_image(path: &std::path::Path) -> Option<image::DynamicImage> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

/// Straight RGBA image → tiny-skia pixmap (premultiplied).
pub fn to_pixmap(img: image::RgbaImage) -> Option<Pixmap> {
    let (w, h) = img.dimensions();
    let mut data = img.into_raw();
    for p in data.as_chunks_mut::<4>().0 {
        let a = p[3] as u16;
        for v in &mut p[..3] {
            *v = ((*v as u16 * a + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h)?)
}

/// Text colour per column (or per row when `vertical`) of the widget box; one entry = uniform.
struct Colors<'a> {
    lut: &'a [Color],
    vertical: bool,
}

impl Colors<'_> {
    fn at(&self, px: i32, py: i32, clip: &Widget) -> Color {
        let i = if self.vertical {
            py - clip.y
        } else {
            px - clip.x
        };
        self.lut[(i.max(0) as usize).min(self.lut.len() - 1)]
    }
}

/// Alpha-blend a coverage bitmap at `(x, y)` of size `(gw, gh)` (clipped to the widget box and
/// the pixmap: the screen, or a widget's own one).
fn blend_coverage(
    pm: &mut Pixmap,
    (x, y): (i32, i32),
    (gw, gh): (usize, usize),
    cov: &[u8],
    colors: &Colors,
    clip: &Widget,
) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let (x0, y0) = (clip.x.max(0), clip.y.max(0));
    let (x1, y1) = ((clip.x + clip.w).min(pw), (clip.y + clip.h).min(ph));
    let data = pm.data_mut();
    for gy in 0..gh as i32 {
        let py = y + gy;
        if py < y0 || py >= y1 {
            continue;
        }
        for gx in 0..gw as i32 {
            let px = x + gx;
            if px < x0 || px >= x1 {
                continue;
            }
            let a = cov[gy as usize * gw + gx as usize] as u32;
            if a == 0 {
                continue;
            }
            let i = (py * pw + px) as usize * 4;
            let c = colors.at(px, py, clip);
            for (k, s) in [c.0, c.1, c.2].into_iter().enumerate() {
                data[i + k] = ((s as u32 * a + data[i + k] as u32 * (255 - a)) / 255) as u8;
            }
            data[i + 3] = 255;
        }
    }
}

/// Deck tiles: background, edge, secondary texts.
const DECK_FILL: Color = Color(0x12, 0x18, 0x22);
const DECK_EDGE: Color = Color(0x26, 0x30, 0x40);
const DECK_DIM: Color = Color(0x8a, 0x96, 0x96);

/// `a` → `b` by `k` (0..1).
fn mix(a: Color, b: Color, k: f32) -> Color {
    let k = k.clamp(0.0, 1.0);
    let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
    Color(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// `img` scaled to the box (x, y, w, h), its corners rounded by `radius` (half the box or more:
/// a disc).
fn draw_image(pm: &mut Pixmap, img: &Pixmap, (x, y, w, h): (f32, f32, f32, f32), radius: f32) {
    let t = Transform::from_row(w / img.width() as f32, 0.0, 0.0, h / img.height() as f32, x, y);
    if radius < 0.5 {
        pm.draw_pixmap(0, 0, img.as_ref(), &PixmapPaint::default(), t, None);
        return;
    }
    // the image as the paint of the rounded box
    let shape = if radius >= w.min(h) / 2.0 {
        Rect::from_xywh(x, y, w, h).and_then(PathBuilder::from_oval)
    } else {
        round_rect(x, y, w, h, radius)
    };
    let shader = tiny_skia::Pattern::new(
        img.as_ref(),
        tiny_skia::SpreadMode::Pad,
        tiny_skia::FilterQuality::Bilinear,
        1.0,
        t,
    );
    let sk = SkPaint { shader, anti_alias: true, ..SkPaint::default() };
    if let Some(shape) = shape {
        pm.fill_path(&shape, &sk, FillRule::Winding, Transform::identity(), None);
    }
}

/// The box (x, y, w, h) grown by `grow` on each side, corners `r + grow`, filled in `c` at
/// opacity `alpha` (layers of it make a glow; a zero box: a disc of radius `grow`).
fn glow_round_rect(
    pm: &mut Pixmap,
    (x, y, w, h): (f32, f32, f32, f32),
    grow: f32,
    r: f32,
    (c, alpha): (Color, u8),
) {
    let mut sk = SkPaint::default();
    sk.set_color_rgba8(c.0, c.1, c.2, alpha);
    sk.anti_alias = true;
    let (bw, bh) = (w + 2.0 * grow, h + 2.0 * grow);
    if let Some(path) = round_rect(x - grow, y - grow, bw, bh, r + grow) {
        pm.fill_path(&path, &sk, FillRule::Winding, Transform::identity(), None);
    }
}

pub fn fill_round_rect(pm: &mut Pixmap, x: i32, y: i32, w: i32, h: i32, r: f32, c: Color) {
    fill_shape(pm, x, y, w, h, r, &paint(c));
}

/// Rounded rectangle filled with `sk` (a colour or a gradient).
fn fill_shape(pm: &mut Pixmap, x: i32, y: i32, w: i32, h: i32, r: f32, sk: &SkPaint) {
    if w <= 0 || h <= 0 {
        return;
    }
    if let Some(path) = round_rect(x as f32, y as f32, w as f32, h as f32, r) {
        pm.fill_path(&path, sk, FillRule::Winding, Transform::identity(), None);
    }
}

/// Border of width `width` along the inside of the rounded box (x, y, w, h).
fn stroke_shape(
    pm: &mut Pixmap,
    (x, y, w, h): (i32, i32, i32, i32),
    r: f32,
    width: f32,
    sk: &SkPaint,
) {
    let half = width / 2.0;
    let (x, y, w, h) = (
        x as f32 + half,
        y as f32 + half,
        w as f32 - width,
        h as f32 - width,
    );
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    if let Some(path) = round_rect(x, y, w, h, (r - half).max(0.0)) {
        let stroke = tiny_skia::Stroke {
            width,
            ..Default::default()
        };
        pm.stroke_path(&path, sk, &stroke, Transform::identity(), None);
    }
}

fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    round_rect_corners(x, y, w, h, [r; 4])
}

/// Rectangle with its own radius per corner: top left, top right, bottom right, bottom left.
fn round_rect_corners(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> Option<tiny_skia::Path> {
    let max = (w / 2.0).min(h / 2.0);
    let [tl, tr, br, bl] = r.map(|r| if r < 0.5 { 0.0 } else { r.min(max) });
    if tl + tr + br + bl == 0.0 {
        return Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect);
    }
    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
    pb.line_to(x + w - tr, y);
    if tr > 0.0 {
        pb.quad_to(x + w, y, x + w, y + tr);
    }
    pb.line_to(x + w, y + h - br);
    if br > 0.0 {
        pb.quad_to(x + w, y + h, x + w - br, y + h);
    }
    pb.line_to(x + bl, y + h);
    if bl > 0.0 {
        pb.quad_to(x, y + h, x, y + h - bl);
    }
    pb.line_to(x, y + tl);
    if tl > 0.0 {
        pb.quad_to(x, y, x + tl, y);
    }
    pb.close();
    pb.finish()
}

/// Border of width `width` inside the box, on the shown `sides` (top, right, bottom, left)
/// only: a corner between two shown sides is rounded, a side next to a hidden one runs to the
/// edge of the box.
fn stroke_sides(
    pm: &mut Pixmap,
    (x, y, w, h): (i32, i32, i32, i32),
    r: f32,
    width: f32,
    sides: [bool; 4],
    sk: &SkPaint,
) {
    if sides.iter().all(|&s| s) {
        return stroke_shape(pm, (x, y, w, h), r, width, sk);
    }
    let half = width / 2.0;
    let (x0, y0, x1, y1) = (x as f32, y as f32, (x + w) as f32, (y + h) as f32);
    // centre line of the border
    let (cx0, cy0, cx1, cy1) = (x0 + half, y0 + half, x1 - half, y1 - half);
    if cx1 <= cx0 || cy1 <= cy0 {
        return;
    }
    let rc = (r - half).max(0.0).min((cx1 - cx0) / 2.0).min((cy1 - cy0) / 2.0);
    // clockwise: side k runs from start[k] to end[k] (edge to edge), then turns at corner[k]
    let start = [(x0, cy0), (cx1, y0), (x1, cy1), (cx0, y1)];
    let end = [(x1, cy0), (cx1, y1), (x0, cy1), (cx0, y0)];
    let corner = [(cx1, cy0), (cx1, cy1), (cx0, cy1), (cx0, cy0)];
    let dir = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
    let mut pb = PathBuilder::new();
    // one stroke per run of consecutive shown sides (there is at least one hidden side)
    for s in (0..4).filter(|&s| sides[s] && !sides[(s + 3) % 4]) {
        pb.move_to(start[s].0, start[s].1);
        let mut k = s;
        while sides[(k + 1) % 4] {
            let (next, (cx, cy)) = ((k + 1) % 4, corner[k]);
            if rc >= 0.5 {
                pb.line_to(cx - rc * dir[k].0, cy - rc * dir[k].1);
                pb.quad_to(cx, cy, cx + rc * dir[next].0, cy + rc * dir[next].1);
            } else {
                pb.line_to(cx, cy);
            }
            k = next;
        }
        pb.line_to(end[k].0, end[k].1);
    }
    if let Some(path) = pb.finish() {
        let stroke = tiny_skia::Stroke {
            width,
            ..Default::default()
        };
        pm.stroke_path(&path, sk, &stroke, Transform::identity(), None);
    }
}

/// Line across the widget box, centred: one piece, dashes or dots (both ends always drawn, the
/// gaps stretched to fit); the paint runs along it, `fade` makes both ends fade out.
fn separator(
    pm: &mut Pixmap,
    w: &Widget,
    color: &Paint,
    thickness: f32,
    vertical: bool,
    style: LineStyle,
    fade: bool,
) {
    let t = thickness.clamp(0.5, 64.0);
    // along the line: from `a0`, `len` long; across: centre `c`
    let (a0, len, c) = if vertical {
        (w.y as f32, w.h as f32, w.x as f32 + w.w as f32 / 2.0)
    } else {
        (w.x as f32, w.w as f32, w.y as f32 + w.h as f32 / 2.0)
    };
    if len < 1.0 {
        return;
    }
    let pieces = match style {
        LineStyle::Solid => vec![(0.0, len)],
        LineStyle::Dashed => evenly(len, (3.0 * t).max(4.0), (2.0 * t).max(3.0)),
        LineStyle::Dotted => evenly(len, t, t.max(2.0)),
    };
    let mut pb = PathBuilder::new();
    for (s, l) in pieces {
        let a = a0 + s;
        if style == LineStyle::Dotted {
            let (px, py) = if vertical { (c, a + l / 2.0) } else { (a + l / 2.0, c) };
            pb.push_circle(px, py, t / 2.0);
        } else {
            let r = if vertical {
                Rect::from_xywh(c - t / 2.0, a, t, l)
            } else {
                Rect::from_xywh(a, c - t / 2.0, l, t)
            };
            if let Some(r) = r {
                pb.push_rect(r);
            }
        }
    }
    let Some(path) = pb.finish() else { return };
    let (start, end) = if vertical {
        ((c, a0), (c, a0 + len))
    } else {
        ((a0, c), (a0 + len, c))
    };
    let sk = if fade {
        faded_shader(color, start, end)
    } else {
        shader(color, start, end)
    };
    pm.fill_path(&path, &sk, FillRule::Winding, Transform::identity(), None);
}

/// (start, length) of pieces `on` long spread over `len`, gaps of about `off`: the first piece
/// starts at 0, the last one ends at `len` (whole pixels).
fn evenly(len: f32, on: f32, off: f32) -> Vec<(f32, f32)> {
    if len <= on {
        return vec![(0.0, len)];
    }
    let n = ((len + off) / (on + off)).floor().max(1.0) as usize;
    if n == 1 {
        return vec![(((len - on) / 2.0).round(), on)];
    }
    let step = (len - on) / (n - 1) as f32;
    (0..n).map(|i| ((i as f32 * step).round(), on)).collect()
}

/// `shader` with both ends fading out to transparent (first and last quarter, smooth).
fn faded_shader(fill: &Paint, start: (f32, f32), end: (f32, f32)) -> SkPaint<'static> {
    const N: usize = 32;
    let stops: Vec<GradientStop> = (0..=N)
        .map(|i| {
            let t = i as f32 / N as f32;
            let c = fill.at(t);
            let a = (t.min(1.0 - t) / 0.25).min(1.0);
            let a = a * a * (3.0 - 2.0 * a);
            let alpha = (a * 255.0).round() as u8;
            GradientStop::new(t, tiny_skia::Color::from_rgba8(c.0, c.1, c.2, alpha))
        })
        .collect();
    let gradient = LinearGradient::new(
        Point::from_xy(start.0, start.1),
        Point::from_xy(end.0, end.1),
        stops,
        SpreadMode::Pad,
        Transform::identity(),
    );
    match gradient {
        Some(shader) => SkPaint {
            shader,
            anti_alias: true,
            ..Default::default()
        },
        None => paint(fill.at(0.5)),
    }
}

/// Filled fraction `f` of the widget, from the bottom (vertical) or the left, with `sk` (whose
/// gradient spans the whole widget: the colour at a level does not depend on the fill).
pub fn bar(pm: &mut Pixmap, w: &Widget, f: f32, vertical: bool, radius: f32, sk: &SkPaint) {
    let f = f.clamp(0.0, 1.0);
    if vertical {
        let h = (w.h as f32 * f).round() as i32;
        fill_shape(pm, w.x, w.y + w.h - h, w.w, h, radius, sk);
    } else {
        let len = (w.w as f32 * f).round() as i32;
        fill_shape(pm, w.x, w.y, len, w.h, radius, sk);
    }
}

const FRAME_COLOR: Color = Color(0x00, 0xc8, 0xff);
const SELECTED_COLOR: Color = Color(0xff, 0xe0, 0x00);

/// Outline rectangles (x, y, w, h, colour) of the page's widgets for the editor: 1 px cyan for
/// every widget (`frames`), 2 px yellow for the selected one (last, on top).
pub fn outline_rects(page: &Page, ov: &Overlay) -> Vec<(i32, i32, i32, i32, Color)> {
    let mut out = Vec::new();
    for (i, w) in page.widgets.iter().enumerate() {
        let (t, c) = if ov.selected.contains(&i) {
            (2, SELECTED_COLOR)
        } else if ov.frames {
            (1, FRAME_COLOR)
        } else {
            continue;
        };
        out.push((w.x, w.y, w.w, t, c));
        out.push((w.x, w.y + w.h - t, w.w, t, c));
        out.push((w.x, w.y, t, w.h, c));
        out.push((w.x + w.w - t, w.y, t, w.h, c));
    }
    out.sort_by_key(|r| r.4 == SELECTED_COLOR);
    out
}

pub fn fill_rect(pm: &mut Pixmap, r: (i32, i32, i32, i32, Color)) {
    fill_round_rect(pm, r.0, r.1, r.2, r.3, 0.0, r.4);
}

/// Colour correction tables of the screen (`[calibration]`), one per channel.
#[derive(Clone)]
pub struct Lut([[u8; 256]; 3]);

impl Lut {
    pub fn new(c: &Calibration) -> Self {
        let mut t = [[0u8; 256]; 3];
        for (ch, table) in t.iter_mut().enumerate() {
            let (gain, gamma) = (c.gain[ch].clamp(0.0, 2.0), c.gamma[ch].clamp(0.2, 5.0));
            for (v, out) in table.iter_mut().enumerate() {
                let y = gain * (v as f32 / 255.0).powf(1.0 / gamma);
                *out = (y * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        Lut(t)
    }

    pub fn rgb565(&self, r: u8, g: u8, b: u8) -> u16 {
        let [tr, tg, tb] = &self.0;
        hsm01_proto::free::rgb565(tr[r as usize], tg[g as usize], tb[b as usize])
    }

    pub fn color565(&self, c: Color) -> u16 {
        self.rgb565(c.0, c.1, c.2)
    }
}

/// RGBA (opaque) → RGB565, through the colour correction.
pub fn to_rgb565(pm: &Pixmap, out: &mut [u16], lut: &Lut) {
    for (o, p) in out.iter_mut().zip(pm.data().as_chunks::<4>().0) {
        *o = lut.rgb565(p[0], p[1], p[2]);
    }
}

/// Calibration test pattern: ramps (grey, red, green, blue in 16 steps), reference swatches
/// (white, mid grey, the logo teals, skin, sky, yellow), near-black and near-white steps.
pub fn test_pattern(pm: &mut Pixmap) {
    pm.fill(tiny_skia::Color::BLACK);
    let ramps: [fn(u8) -> Color; 4] = [
        |v| Color(v, v, v),
        |v| Color(v, 0, 0),
        |v| Color(0, v, 0),
        |v| Color(0, 0, v),
    ];
    for (row, ramp) in ramps.iter().enumerate() {
        for i in 0..16 {
            let v = (i * 255 / 15) as u8;
            fill_round_rect(pm, i * 30, row as i32 * 34, 30, 34, 0.0, ramp(v));
        }
    }
    let swatches = [
        Color(255, 255, 255),
        Color(128, 128, 128),
        Color(0x00, 0xb4, 0xa8),
        Color(0x00, 0x6c, 0x68),
        Color(0x9a, 0xd8, 0xd0),
        Color(0xe0, 0xac, 0x8c),
        Color(0x5a, 0x9b, 0xd5),
        Color(0xe0, 0xc0, 0x20),
    ];
    for (i, c) in swatches.into_iter().enumerate() {
        fill_round_rect(pm, i as i32 * 60, 136, 60, 68, 0.0, c);
    }
    for i in 0..8 {
        let dark = (i * 4) as u8;
        let light = 255 - (i * 4) as u8;
        fill_round_rect(pm, i * 60, 204, 60, 34, 0.0, Color(dark, dark, dark));
        fill_round_rect(pm, i * 60, 238, 60, 34, 0.0, Color(light, light, light));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// Picture of the confirmation over the default page: `TMPDIR=… cargo test -- --ignored
    /// confirm_snapshot` writes `$TMPDIR/hsm01-confirm.png`.
    #[test]
    #[ignore]
    fn confirm_snapshot() {
        let mut r = Renderer::new().expect("system fonts");
        let mut pm = Renderer::new_pixmap();
        let view = View {
            page_name: "Audio".into(),
            ..Default::default()
        };
        r.render(&Config::default().pages[0], &view, &mut pm);
        let c = ConfirmView {
            name: "Starship/Matisse HD Audio Controller Stéréo analogique".into(),
            yes: false,
        };
        r.confirm(&mut pm, &c);
        pm.save_png(std::env::temp_dir().join("hsm01-confirm.png"))
            .unwrap();
    }

    /// Separators, frames with some sides, notification popup: `TMPDIR=… cargo test --
    /// --ignored lines_snapshot` → `$TMPDIR/hsm01-lines.png`, `$TMPDIR/hsm01-notice.png`.
    #[test]
    #[ignore]
    fn lines_snapshot() {
        use crate::config::{Frame, Sides};
        let mut r = Renderer::new().expect("system fonts");
        let mut page = Config::default().pages[0].clone();
        page.wallpaper = None;
        let teal = Paint::Gradient(vec![Color(0, 0x6c, 0x68), Color(0x9a, 0xd8, 0xd0)]);
        let sep = |y, style, fade, color: Paint| Widget {
            x: 16,
            y,
            w: 200,
            h: 12,
            frame: None,
            kind: WidgetKind::Separator {
                color,
                thickness: 2.0,
                vertical: false,
                style,
                fade,
            },
        };
        let grey = Paint::Solid(Color(0x5a, 0x66, 0x66));
        page.widgets = vec![
            sep(16, LineStyle::Solid, false, grey.clone()),
            sep(40, LineStyle::Dashed, false, grey.clone()),
            sep(64, LineStyle::Dotted, false, Paint::Solid(Color(0xd0, 0xd0, 0xd0))),
            sep(88, LineStyle::Solid, true, teal.clone()),
            Widget {
                x: 232,
                y: 8,
                w: 12,
                h: 104,
                frame: None,
                kind: WidgetKind::Separator {
                    color: teal.clone(),
                    thickness: 3.0,
                    vertical: true,
                    style: LineStyle::Dashed,
                    fade: false,
                },
            },
        ];
        let sides = [
            [true, false, true, false],
            [false, false, true, false],
            [false, true, false, true],
            [true, true, true, false],
            [true; 4],
            [false, true, true, false],
        ];
        for (i, s) in sides.into_iter().enumerate() {
            let (x, y) = (16 + (i as i32 % 3) * 152, 128 + (i as i32 / 3) * 72);
            let mut text = WidgetKind::template("text").unwrap();
            if let WidgetKind::Text { text: t, knob, .. } = &mut text {
                *t = format!("cadre {}", i + 1);
                *knob = None;
            }
            page.widgets.push(Widget {
                x,
                y,
                w: 136,
                h: 56,
                frame: Some(Frame {
                    width: 3.0,
                    radius: 12.0,
                    fill: Some(Paint::Solid(Color(0x10, 0x20, 0x20))),
                    sides: Sides::from_array(s),
                    ..Default::default()
                }),
                kind: text,
            });
        }
        let view = View::default();
        let mut pm = Renderer::new_pixmap();
        r.render(&page, &view, &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-lines.png")).unwrap();

        let mut pm = Renderer::new_pixmap();
        r.render(&Config::default().pages[0], &view, &mut pm);
        let n = crate::notify::Notification {
            app: "Discord".into(),
            summary: "Alice (#général, Serveur des amis)".into(),
            body: "Est-ce que quelqu'un a testé le nouveau firmware ? Les molettes répondent \
                   enfin correctement même pendant les GIF plein écran, c'est top."
                .into(),
            icon: None,
            duration: None,
            wake: None,
            led: None,
            desktop: true,
            origin: None,
            style: Default::default(),
        };
        r.notice(&mut pm, &n, (2.4, 6.0));
        pm.save_png(std::env::temp_dir().join("hsm01-notice.png")).unwrap();
        // pill alerts: $TMPDIR/hsm01-pill.png (green as the model, then red outline, white dots)
        let mut pm = Renderer::new_pixmap();
        r.render(&Config::default().pages[0], &view, &mut pm);
        let green = Color(0x3d, 0xdc, 0x84);
        let white = Color(255, 255, 255);
        let pill = |summary: &str, border, dot| crate::notify::Notification {
            summary: summary.into(),
            style: crate::notify::NoticeStyle::Pill { border, dot, color: white },
            ..n.clone()
        };
        r.notice(&mut pm, &pill("ACTION ACTIVÉE : EN DIRECT", green, green), (1.0, 3.0));
        pm.save_png(std::env::temp_dir().join("hsm01-pill.png")).unwrap();
        let mut pm = Renderer::new_pixmap();
        r.render(&Config::default().pages[0], &view, &mut pm);
        r.notice(&mut pm, &pill("MICRO COUPÉ", Color(0xf2, 0x3f, 0x43), white), (0.1, 3.0));
        pm.save_png(std::env::temp_dir().join("hsm01-pill-rising.png")).unwrap();
    }

    /// Gradients: `TMPDIR=… cargo test -- --ignored gradient_snapshot` → `$TMPDIR/hsm01-gradients.png`.
    #[test]
    #[ignore]
    fn gradient_snapshot() {
        use crate::config::Paint;
        let mut r = Renderer::new().expect("system fonts");
        let mut page = Config::default().pages[0].clone();
        let teal = || Paint::Gradient(vec![Color(0, 0x6c, 0x68), Color(0x9a, 0xd8, 0xd0)]);
        for w in &mut page.widgets {
            match &mut w.kind {
                WidgetKind::Text { color, .. } => {
                    *color = Paint::Gradient(vec![Color(0xff, 0x80, 0x40), Color(0x60, 0xa0, 0xff)])
                }
                WidgetKind::Volume { color, .. } => *color = teal(),
                _ => {}
            }
        }
        let mut view = View {
            page_name: "Audio – un titre de page assez long pour défiler".into(),
            time: 2.5,
            ..Default::default()
        };
        for (k, t) in view.knobs.iter_mut().enumerate() {
            *t = TargetState {
                name: ["Sortie", "Micro", "Vivaldi", "Spotify"][k].into(),
                present: true,
                volume: 0.3 + 0.2 * k as f32,
                muted: k == 1,
                level: 0.45 + 0.17 * k as f32,
                peak_hold: 0.6 + 0.12 * k as f32,
                icon: [
                    "audio-speakers",
                    "audio-input-microphone",
                    "vivaldi",
                    "spotify",
                ][k]
                    .into(),
                device: k < 2,
            };
        }
        let mut pm = Renderer::new_pixmap();
        r.render(&page, &view, &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-gradients.png"))
            .unwrap();
        // the four source icons (second knob muted: dimmed)
        let icons = crate::config::Page {
            widgets: (0..4)
                .map(|k| Widget {
                    x: 24 + k * 116,
                    y: 100,
                    w: 72,
                    h: 72,
                    frame: None,
                    kind: WidgetKind::Icon {
                        knob: k as u8 + 1,
                        style: IconStyle::Auto,
                        tint: Color(0xde, 0xde, 0xde),
                    },
                })
                .collect(),
            ..page
        };
        let mut pm = Renderer::new_pixmap();
        r.render(&icons, &view, &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-icons-sources.png"))
            .unwrap();
    }

    #[test]
    fn calibration_tables() {
        let same = Lut::new(&Calibration::default());
        for v in [0u8, 1, 37, 128, 254, 255] {
            assert_eq!(same.rgb565(v, v, v), hsm01_proto::free::rgb565(v, v, v));
        }
        let half_red = Lut::new(&Calibration {
            gain: [0.5, 1.0, 1.0],
            gamma: [1.0; 3],
        });
        assert_eq!(half_red.0[0][255], 128);
        assert_eq!(half_red.0[1][255], 255);
        let lighter = Lut::new(&Calibration {
            gain: [1.0; 3],
            gamma: [2.0; 3],
        });
        assert!(lighter.0[0][64] > 64 && lighter.0[0][255] == 255);
    }

    /// `TMPDIR=… cargo test -- --ignored pattern_snapshot` → `$TMPDIR/hsm01-pattern.png`.
    #[test]
    #[ignore]
    fn pattern_snapshot() {
        let mut pm = Renderer::new_pixmap();
        test_pattern(&mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-pattern.png"))
            .unwrap();
    }

    #[test]
    fn marquee_goes_to_the_end_and_back() {
        assert_eq!(marquee(60.0, 0.0), 0.0);
        assert_eq!(marquee(60.0, 1.5 + 1.0), 30.0);
        assert_eq!(marquee(60.0, 1.5 + 2.0 + 1.0), 60.0);
        assert_eq!(marquee(60.0, 2.0 * 1.5 + 2.0 + 1.0), 30.0);
        assert_eq!(marquee(60.0, 2.0 * (1.5 + 2.0)), 0.0);
    }

    #[test]
    fn template_expansion() {
        let t = TargetState {
            name: "Speakers".into(),
            present: true,
            volume: 0.42,
            muted: true,
            ..Default::default()
        };
        assert_eq!(
            expand("{name}: {volume}{mute}", &t, "P"),
            "Speakers: 42 % MUTE"
        );
        assert_eq!(
            expand("{name} {volume}", &TargetState::default(), "P"),
            "— –"
        );
    }

    /// A `tool` widget draws the tool's page fitted into its box (scaled down, centred), nothing
    /// while the tool is not there.
    #[test]
    fn tool_widget_draws_the_page_fitted() {
        let Ok(mut r) = Renderer::new() else { return }; // no system font in this environment
        let mut inner = Config::default().pages[0].clone();
        inner.wallpaper = None;
        inner.background = Color(200, 0, 0);
        inner.widgets.clear();
        let mut page = inner.clone();
        page.background = Color(0, 0, 0);
        page.widgets = vec![Widget {
            x: 200,
            y: 100,
            w: 240,
            h: 200, // wider page than the box: 240 × 136, centred vertically (y 132..268)
            frame: None,
            kind: WidgetKind::Tool {
                tool: "Menu".into(),
                page: String::new(),
                inputs: Vec::new(),
                overflow: Default::default(),
            },
        }];
        let red = |pm: &Pixmap, x, y| pm.pixel(x, y).unwrap().red() > 150;
        let mut pm = Renderer::new_pixmap();
        r.render(&page, &View::default(), &mut pm);
        assert!(!red(&pm, 320, 200), "no tool page: nothing");
        let mut view = View::default();
        view.embedded.insert(0, (inner, (SCREEN_W, SCREEN_H)));
        r.render(&page, &view, &mut pm);
        assert!(red(&pm, 320, 200) && red(&pm, 202, 134) && red(&pm, 437, 265));
        assert!(!red(&pm, 320, 120) && !red(&pm, 320, 270) && !red(&pm, 190, 200));
    }

    /// Emoji in a text: drawn in their own colours (a white text, a yellow face U+1F600), modifiers
    /// (skin tone, variation selector) not drawn, the text measured with them; characters the
    /// text font lacks (→) from a fallback font. `TMPDIR=… cargo test -- --ignored
    /// emoji_snapshot` → `$TMPDIR/hsm01-emoji.png`.
    #[test]
    fn emoji_in_texts() {
        let Ok(mut r) = Renderer::new() else { return }; // no system font in this environment
        if r.emoji_font.is_none() {
            return; // no colour emoji font installed
        }
        let mut page = Config::default().pages[0].clone();
        (page.wallpaper, page.background) = (None, Color(0, 0, 0));
        page.widgets = vec![Widget {
            x: 0,
            y: 100,
            w: 480,
            h: 40,
            frame: None,
            kind: WidgetKind::Text {
                text: "a\u{1F600}b".into(),
                size: 30.0,
                color: Paint::Solid(Color(255, 255, 255)),
                align: Align::Left,
                bold: false,
                knob: None,
                scroll: false,
            },
        }];
        let mut pm = Renderer::new_pixmap();
        r.render(&page, &View::default(), &mut pm);
        let yellow = (0..480).flat_map(|x| (100..140).map(move |y| (x, y))).any(|(x, y)| {
            let c = pm.pixel(x, y).unwrap();
            c.red() > 180 && c.green() > 150 && c.blue() < 90
        });
        assert!(yellow, "the emoji in its colours");
        let (thumb, toned) = ("\u{1F44D}", "\u{1F44D}\u{1F3FD}");
        assert_eq!(r.text_width(toned, 30.0, false), r.text_width(thumb, 30.0, false), "skin tone");
        let (heart, styled) = ("\u{2764}", "\u{2764}\u{FE0F}");
        assert_eq!(r.text_width(styled, 30.0, false), r.text_width(heart, 30.0, false));
        if r.regular.lookup_glyph_index('→') == 0 {
            let from = r.fallback.iter().find(|f| f.lookup_glyph_index('→') != 0);
            let want = from.expect("a fallback font with →").rasterize('→', 30.0).1;
            assert_eq!(r.glyph('→', 30.0, false).1, want, "→ from the fallback font");
        }
    }

    #[test]
    #[ignore]
    fn emoji_snapshot() {
        let mut r = Renderer::new().expect("system fonts");
        let mut page = Config::default().pages[0].clone();
        (page.wallpaper, page.background) = (None, Color(10, 14, 18));
        let line = |y: i32, text: &str, size: f32, bold: bool| Widget {
            x: 12,
            y,
            w: 456,
            h: 40,
            frame: None,
            kind: WidgetKind::Text {
                text: text.into(),
                size,
                color: Paint::Solid(Color(232, 238, 238)),
                align: Align::Left,
                bold,
                knob: None,
                scroll: false,
            },
        };
        page.widgets = vec![
            // emoji as Unicode escapes (wave, game pad, fire; heart, thumb + skin tone, laugh,
            // headphones, check box, then symbols from the fallback font; fox, turtle, blossom;
            // smileys at 13 px; rockets in a text cut with an ellipsis)
            line(20, "Salut \u{1F44B} on joue ? \u{1F3AE}\u{1F525}", 26.0, true),
            line(70, "\u{2764}\u{FE0F} \u{1F44D}\u{1F3FD} \u{1F602} \u{1F3A7} \u{2705} \u{2192} \u{2713} \u{2605} \u{266B}", 24.0, false),
            line(120, "Alice \u{1F98A} · Bob \u{1F422} · Chloé \u{1F338}", 20.0, false),
            line(170, "Petit texte 13 px \u{1F600}\u{1F642}\u{1F60E}", 13.0, false),
            line(210, "Très long texte avec emoji \u{1F680}\u{1F680}\u{1F680} coupé proprement à la fin du cadre \u{1F680}", 18.0, false),
        ];
        let mut pm = Renderer::new_pixmap();
        r.render(&page, &View::default(), &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-emoji.png")).unwrap();
    }

    /// A module widget taller than its box (120×600: red top half, blue bottom half, in a
    /// 120×200 box): `shrink` shows it whole, narrower; `scroll` keeps its width, shows the top
    /// first, then scrolls down to the bottom and back.
    #[test]
    fn module_widgets_shrink_or_scroll() {
        let Ok(mut r) = Renderer::new() else { return }; // no system font in this environment
        let mut inner = Config::default().pages[0].clone();
        (inner.wallpaper, inner.background) = (None, Color(200, 0, 0));
        inner.widgets = vec![Widget {
            x: 0,
            y: 300,
            w: 120,
            h: 300,
            frame: None,
            kind: WidgetKind::Rect { color: Paint::Solid(Color(0, 0, 200)), radius: 0.0 },
        }];
        let mut page = inner.clone();
        (page.background, page.widgets) = (Color(0, 0, 0), Vec::new());
        let tool = |overflow| Widget {
            x: 100,
            y: 40,
            w: 120,
            h: 200,
            frame: None,
            kind: WidgetKind::Tool { tool: "M".into(), page: "w".into(), inputs: Vec::new(), overflow },
        };
        let shot = |r: &mut Renderer, overflow, time: f32| {
            let mut p = page.clone();
            p.widgets = vec![tool(overflow)];
            let mut view = View { time, ..Default::default() };
            view.embedded.insert(0, (inner.clone(), (120, 600)));
            let mut pm = Renderer::new_pixmap();
            r.render(&p, &view, &mut pm);
            let at = |x, y| {
                let c = pm.pixel(x, y).unwrap();
                (c.red() > 150, c.blue() > 150)
            };
            [at(160, 45), at(160, 235), at(101, 140), at(130, 140)]
        };
        // shrink: 40×200, centred: red above, blue below, nothing at the box's left edge
        let s = shot(&mut r, Overflow::Shrink, 0.0);
        assert_eq!(s, [(true, false), (false, true), (false, false), (false, false)]);
        // scroll: 1:1, the top (red) first, the bottom (blue) once scrolled (2.5 s + 400 px at
        // 20 px/s, then 2.5 s at the bottom)
        let top = shot(&mut r, Overflow::Scroll, 1.0);
        assert_eq!([top[0], top[1], top[2]], [(true, false), (true, false), (true, false)]);
        let bottom = shot(&mut r, Overflow::Scroll, 2.5 + 20.0 + 1.0);
        assert_eq!([bottom[0], bottom[1]], [(false, true), (false, true)]);
        let back = shot(&mut r, Overflow::Scroll, 2.0 * (2.5 + 20.0) + 1.0);
        assert_eq!(back[0], (true, false), "back to the top");
    }

    /// A module widget wider than its box (600×120: red left half, blue right half, in a
    /// 200×120 box): `scroll_sideways` keeps its height, shows the left first, then scrolls to
    /// the right end and back.
    #[test]
    fn module_widgets_scroll_sideways() {
        let Ok(mut r) = Renderer::new() else { return }; // no system font in this environment
        let mut inner = Config::default().pages[0].clone();
        (inner.wallpaper, inner.background) = (None, Color(200, 0, 0));
        inner.widgets = vec![Widget {
            x: 300,
            y: 0,
            w: 300,
            h: 120,
            frame: None,
            kind: WidgetKind::Rect { color: Paint::Solid(Color(0, 0, 200)), radius: 0.0 },
        }];
        let mut page = inner.clone();
        (page.background, page.widgets) = (Color(0, 0, 0), Vec::new());
        let shot = |r: &mut Renderer, time: f32| {
            let mut p = page.clone();
            p.widgets = vec![Widget {
                x: 100,
                y: 40,
                w: 200,
                h: 120,
                frame: None,
                kind: WidgetKind::Tool {
                    tool: "M".into(),
                    page: "w".into(),
                    inputs: Vec::new(),
                    overflow: Overflow::ScrollSideways,
                },
            }];
            let mut view = View { time, ..Default::default() };
            view.embedded.insert(0, (inner.clone(), (600, 120)));
            let mut pm = Renderer::new_pixmap();
            r.render(&p, &view, &mut pm);
            let at = |x, y| {
                let c = pm.pixel(x, y).unwrap();
                (c.red() > 150, c.blue() > 150)
            };
            [at(105, 100), at(295, 100), at(200, 45), at(200, 165)]
        };
        let left = shot(&mut r, 1.0);
        assert_eq!(left, [(true, false), (true, false), (true, false), (false, false)]);
        let right = shot(&mut r, 2.5 + 20.0 + 1.0);
        assert_eq!([right[0], right[1]], [(false, true), (false, true)]);
        let back = shot(&mut r, 2.0 * (2.5 + 20.0) + 1.0);
        assert_eq!(back[0], (true, false), "back to the left");
    }

    /// A pill alert: centred at the top, as wide as its text, outline and dots in their own
    /// colours, only in `PILL_BAND`.
    #[test]
    fn pill_alerts() {
        let Ok(mut r) = Renderer::new() else { return }; // no system font in this environment
        let (red, blue) = (Color(250, 0, 0), Color(0, 0, 250));
        let n = crate::notify::Notification {
            app: String::new(),
            summary: "EN DIRECT".into(),
            body: String::new(),
            icon: None,
            duration: None,
            wake: None,
            led: None,
            desktop: false,
            origin: None,
            style: crate::notify::NoticeStyle::Pill { border: red, dot: blue, color: red },
        };
        assert_eq!(notice_box(&n), PILL_BAND);
        let mut blank = |age: f32| {
            let mut pm = Pixmap::new(SCREEN_W as u32, SCREEN_H as u32).unwrap();
            r.notice(&mut pm, &n, (age, 3.0));
            pm.data().chunks(4).all(|p| p[3] == 0)
        };
        assert!(blank(0.0) && blank(3.0), "below the screen as it comes and once gone");
        assert!(!blank(0.1) && !blank(2.9), "sliding");
        let mut pm = Pixmap::new(SCREEN_W as u32, SCREEN_H as u32).unwrap();
        r.notice(&mut pm, &n, (0.1, 3.0));
        let rising_top = (0..SCREEN_H).find(|&y| pm.pixel(SCREEN_W as u32 / 2, y as u32).unwrap().red() > 200);
        assert!(rising_top.is_some_and(|y| y > PILL_Y), "still rising: {rising_top:?}");
        let mut pm = Pixmap::new(SCREEN_W as u32, SCREEN_H as u32).unwrap();
        r.notice(&mut pm, &n, (1.0, 3.0));
        let px = |x: i32, y: i32| pm.pixel(x as u32, y as u32).unwrap();
        let top = px(SCREEN_W / 2, PILL_Y);
        assert!(top.red() > 200 && top.blue() < 50, "outline at the top centre: {top:?}");
        let row: Vec<_> = (0..SCREEN_W).map(|x| px(x, PILL_Y + PILL_H / 2)).collect();
        let dots: Vec<_> = (0..SCREEN_W).filter(|&x| row[x as usize].blue() > 200).collect();
        let (l, r_) = (dots[0], *dots.last().unwrap());
        assert!(l > 0 && r_ < SCREEN_W - 1 && (l + r_ - SCREEN_W).abs() <= 2, "centred dots: {l} {r_}");
        assert!(r_ - l < SCREEN_W / 2, "as wide as the text");
        let (bx, by, bw, bh) = PILL_BAND;
        let outside = (0..SCREEN_H).flat_map(|y| (0..SCREEN_W).map(move |x| (x, y)))
            .filter(|&(x, y)| !(bx..bx + bw).contains(&x) || !(by..by + bh).contains(&y));
        assert!(outside.into_iter().all(|(x, y)| px(x, y).alpha() == 0), "inside its band");
    }

    /// An image widget: scaled to its box; `radius` of half the box or more makes it a disc.
    #[test]
    fn images_rounded_to_a_disc() {
        let Ok(mut r) = Renderer::new() else { return }; // no system font in this environment
        let path = std::env::temp_dir().join(format!("hsm01-disc-{}.png", std::process::id()));
        let mut green = Pixmap::new(8, 8).unwrap();
        green.fill(tiny_skia::Color::from_rgba8(0, 200, 0, 255));
        green.save_png(&path).unwrap();
        let mut page = Config::default().pages[0].clone();
        page.wallpaper = None;
        page.background = Color(0, 0, 0);
        let image = |radius| Widget {
            x: 100,
            y: 50,
            w: 100,
            h: 100,
            frame: None,
            kind: WidgetKind::Image { path: path.display().to_string(), radius },
        };
        let is_green = |pm: &Pixmap, x, y| pm.pixel(x, y).unwrap().green() > 150;
        let mut pm = Renderer::new_pixmap();
        page.widgets = vec![image(0.0)];
        r.render(&page, &View::default(), &mut pm);
        assert!(is_green(&pm, 150, 100) && is_green(&pm, 101, 51) && is_green(&pm, 198, 148));
        page.widgets = vec![image(50.0)];
        r.render(&page, &View::default(), &mut pm);
        assert!(is_green(&pm, 150, 100) && is_green(&pm, 102, 100) && is_green(&pm, 150, 147));
        assert!(!is_green(&pm, 103, 53) && !is_green(&pm, 196, 146), "corners outside the disc");
        let _ = std::fs::remove_file(path);
    }

    /// The modules menu over the default page, slid in and half-way: `TMPDIR=… cargo test --
    /// --ignored menu_snapshot` → `$TMPDIR/hsm01-menu.png`, `$TMPDIR/hsm01-menu-half.png`.
    #[test]
    #[ignore]
    fn menu_snapshot() {
        let mut r = Renderer::new().expect("system fonts");
        let mut page = Config::default().pages[0].clone();
        page.wallpaper = None;
        let items = [("Lecteur média", "Média"), ("Stream deck", "Stream deck"), ("Système", "Système"), ("Mon module", "Mon module")];
        let mut m = crate::state::MenuView {
            items: items.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            selected: 1,
            shown: 1.0,
        };
        let mut pm = Renderer::new_pixmap();
        let view = View { page_name: "Audio".into(), ..Default::default() };
        for (shown, file) in [(1.0, "hsm01-menu.png"), (0.5, "hsm01-menu-half.png")] {
            m.shown = shown;
            r.render(&page, &view, &mut pm);
            r.module_menu(&mut pm, &m);
            pm.save_png(std::env::temp_dir().join(file)).unwrap();
        }
    }

    /// Deck widgets: four columns (one mid-slide, one just clicked) and a row, with real theme
    /// icons: `TMPDIR=… cargo test -- --ignored deck_snapshot` → `$TMPDIR/hsm01-deck.png` and
    /// `$TMPDIR/hsm01-deck-row.png`.
    #[test]
    #[ignore]
    fn deck_snapshot() {
        use crate::config::{Action, Tile};
        use crate::state::DeckView;
        let mut r = Renderer::new().expect("system fonts");
        let mut page = Config::default().pages[0].clone();
        page.wallpaper = None;
        let cmd = |t: &str, s: &str, i: &str| Tile::new(t, s, i, Action::None);
        let tracks: [(Color, Vec<Tile>); 4] = [
            (Color(0x00, 0xf0, 0xff), vec![
                cmd("Muet micro", "couper", "microphone-sensitivity-muted"),
                cmd("Gain +3 dB", "voix", "audio-volume-high"),
                cmd("Casque", "retour", "audio-headphones"),
            ]),
            (Color(0xff, 0xaa, 0x00), Tile::media()),
            (Color(0x00, 0xff, 0x88), vec![
                cmd("Discord", "muet", "discord"),
                cmd("Terminal", "konsole", "utilities-terminal"),
                cmd("Fichiers", "dolphin", "system-file-manager"),
            ]),
            (Color(0xb0, 0x26, 0xff), vec![
                cmd("Capture", "écran", "spectacle"),
                cmd("Navigateur", "web", "firefox"),
            ]),
        ];
        let mut view = View::default();
        page.widgets = tracks
            .iter()
            .enumerate()
            .map(|(k, (color, tiles))| Widget {
                x: k as i32 * 120 + 4,
                y: 12,
                w: 112,
                h: 248,
                frame: None,
                kind: WidgetKind::Deck {
                    knob: k as u8 + 1,
                    vertical: true,
                    color: *color,
                    tiles: tiles.clone(),
                },
            })
            .collect();
        view.decks.insert(1, DeckView { selected: 1, offset: 0.4, flash: 0.0 });
        view.decks.insert(2, DeckView { selected: 0, offset: 0.0, flash: 0.8 });
        let mut pm = Renderer::new_pixmap();
        r.render(&page, &view, &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-deck.png")).unwrap();
        page.widgets = vec![Widget {
            x: 0,
            y: 100,
            w: 480,
            h: 72,
            frame: None,
            kind: WidgetKind::Deck {
                knob: 1,
                vertical: false,
                color: tracks[1].0,
                tiles: Tile::media(),
            },
        }];
        r.render(&page, &View::default(), &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-deck-row.png")).unwrap();
    }

    #[test]
    fn default_page_renders() {
        let Ok(mut r) = Renderer::new() else { return }; // no system font in this environment
        let cfg = Config::default();
        let mut pm = Renderer::new_pixmap();
        let mut view = View {
            page_name: "Audio".into(),
            ..Default::default()
        };
        view.knobs[0] = TargetState {
            name: "Out".into(),
            present: true,
            volume: 0.5,
            level: 0.8,
            ..Default::default()
        };
        r.render(&cfg.pages[0], &view, &mut pm);
        let mut px = vec![0u16; (SCREEN_W * SCREEN_H) as usize];
        to_rgb565(&pm, &mut px, &Lut::new(&Calibration::default()));
        assert!(px.iter().any(|&p| p != 0), "something was drawn");
    }
}

