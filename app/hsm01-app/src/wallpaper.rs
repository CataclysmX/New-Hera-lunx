//! Page background images (`[page.wallpaper]`): PNG, JPEG, WebP or GIF, animated or not. Every
//! frame is decoded, fitted to the 480×272 screen and darkened once, then the renderer only picks
//! the frame of the moment. Animation speed is real time, capped by `fps`; the engine holds the
//! clock while the previous frame is still being sent (USB), so slow links drop frames instead of
//! slowing the animation down or blocking the inputs.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, ImageFormat, RgbaImage};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Rect, Transform};

use crate::i18n::tr;
use crate::config::{Color, Fit, SCREEN_H, SCREEN_W};

/// Longer animations keep one frame out of k (memory: 522 KB per frame).
const MAX_FRAMES: usize = 150;
/// Frame delay used when a GIF says 0.
const DEFAULT_DELAY_MS: u32 = 100;

pub struct Frames {
    /// Screen-sized frames and how long each is shown (ms).
    frames: Vec<(Pixmap, u32)>,
    total_ms: u32,
}

impl Frames {
    /// Frame shown `t` seconds into the animation, time stepped at `fps` frames per second.
    pub fn at(&self, t: f32, fps: u8) -> &Pixmap {
        if self.frames.len() == 1 || self.total_ms == 0 {
            return &self.frames[0].0;
        }
        let step = 1000 / u64::from(fps.max(1));
        let ms = ((t.max(0.0) * 1000.0) as u64 / step * step % u64::from(self.total_ms)) as u32;
        let mut acc = 0;
        for (frame, delay) in &self.frames {
            acc += delay;
            if ms < acc {
                return frame;
            }
        }
        &self.frames[self.frames.len() - 1].0
    }
}

/// Decoded frames of `path` with their delays (ms).
fn decode(path: &str) -> Result<Vec<(RgbaImage, u32)>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let err = |e: image::ImageError| format!("{path}: {e}");
    let frames = |frames: image::Frames<'_>| -> Result<Vec<(RgbaImage, u32)>, String> {
        Ok(frames
            .collect_frames()
            .map_err(err)?
            .into_iter()
            .map(|f| {
                let (n, d) = f.delay().numer_denom_ms();
                let ms = if n == 0 {
                    DEFAULT_DELAY_MS
                } else {
                    n / d.max(1)
                };
                (f.into_buffer(), ms.max(10))
            })
            .collect())
    };
    match image::guess_format(&bytes).map_err(err)? {
        ImageFormat::Gif => frames(
            GifDecoder::new(Cursor::new(&bytes))
                .map_err(err)?
                .into_frames(),
        ),
        ImageFormat::WebP => {
            let d = WebPDecoder::new(Cursor::new(&bytes)).map_err(err)?;
            if d.has_animation() {
                frames(d.into_frames())
            } else {
                Ok(vec![(
                    image::load_from_memory(&bytes).map_err(err)?.into_rgba8(),
                    0,
                )])
            }
        }
        _ => Ok(vec![(
            image::load_from_memory(&bytes).map_err(err)?.into_rgba8(),
            0,
        )]),
    }
}

/// Load `path`, fitted to the screen on `bg` and darkened by `dim` percent.
pub fn load(path: &str, fit: Fit, dim: u8, bg: Color) -> Result<Frames, String> {
    let mut decoded = decode(path)?;
    if decoded.is_empty() {
        return Err(tf!("{path}: no image", path = path));
    }
    if decoded.len() > MAX_FRAMES {
        // keep one frame out of k, each shown for the k merged delays
        let k = decoded.len().div_ceil(MAX_FRAMES);
        decoded = decoded
            .chunks(k)
            .map(|c| (c[0].0.clone(), c.iter().map(|f| f.1).sum()))
            .collect();
    }
    let total_ms = decoded.iter().map(|f| f.1).sum();
    let frames = decoded
        .into_iter()
        .map(|(img, ms)| Ok((compose(img, fit, dim, bg)?, ms)))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Frames { frames, total_ms })
}

fn compose(img: RgbaImage, fit: Fit, dim: u8, bg: Color) -> Result<Pixmap, String> {
    let src = crate::render::to_pixmap(img).ok_or(tr("empty image"))?;
    let mut pm = Pixmap::new(SCREEN_W as u32, SCREEN_H as u32).ok_or(tr("screen"))?;
    pm.fill(tiny_skia::Color::from_rgba8(bg.0, bg.1, bg.2, 255));
    let (sw, sh) = (SCREEN_W as f32, SCREEN_H as f32);
    let (w, h) = (src.width() as f32, src.height() as f32);
    let (kx, ky) = match fit {
        Fit::Cover => {
            let k = (sw / w).max(sh / h);
            (k, k)
        }
        Fit::Contain => {
            let k = (sw / w).min(sh / h);
            (k, k)
        }
        Fit::Stretch => (sw / w, sh / h),
        Fit::Center => (1.0, 1.0),
    };
    let (dx, dy) = ((sw - w * kx) / 2.0, (sh - h * ky) / 2.0);
    let paint = PixmapPaint {
        quality: FilterQuality::Bicubic,
        ..Default::default()
    };
    pm.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &paint,
        Transform::from_row(kx, 0.0, 0.0, ky, dx, dy),
        None,
    );
    if dim > 0
        && let Some(r) = Rect::from_xywh(0.0, 0.0, sw, sh)
    {
        let mut shade = tiny_skia::Paint::default();
        shade.set_color_rgba8(0, 0, 0, (u32::from(dim.min(95)) * 255 / 100) as u8);
        pm.fill_rect(r, &shade, Transform::identity(), None);
    }
    Ok(pm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(delays: &[u32]) -> Frames {
        let mut v = Vec::new();
        for (i, &d) in delays.iter().enumerate() {
            let mut pm = Pixmap::new(1, 1).unwrap();
            pm.fill(tiny_skia::Color::from_rgba8(i as u8, 0, 0, 255));
            v.push((pm, d));
        }
        Frames {
            total_ms: delays.iter().sum(),
            frames: v,
        }
    }

    #[test]
    fn frame_timing() {
        let f = frames(&[100, 100, 200]);
        let red = |pm: &Pixmap| pm.pixel(0, 0).unwrap().red();
        assert_eq!(red(f.at(0.0, 60)), 0);
        assert_eq!(red(f.at(0.15, 60)), 1);
        assert_eq!(red(f.at(0.35, 60)), 2);
        assert_eq!(red(f.at(0.45, 60)), 0); // loops after 400 ms
        // 2 fps: time stepped by 500 ms
        assert_eq!(red(f.at(0.45, 2)), 0);
        assert_eq!(red(f.at(0.55, 2)), 1);
    }
}
