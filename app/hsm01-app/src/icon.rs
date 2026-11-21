//! App icons, embedded at build time and rasterised in a chosen colour (monochrome SVGs drawn with
//! `currentColor`): `public/tray.svg` (detailed: window icon) and `public/tray-symbolic.svg`
//! (simplified, bold: system tray at 16-48 px, where the detailed drawing is unreadable).

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

pub const WINDOW_SVG: &str = include_str!("../../../public/tray.svg");
pub const TRAY_SVG: &str = include_str!("../../../public/tray-symbolic.svg");

/// Light grey of the Breeze dark panel icons (when the KDE colour scheme cannot be read).
pub const TRAY_COLOR: &str = "#fcfcfc";
/// Teal of the boot logo.
pub const WINDOW_COLOR: &str = "#00b4a8";

/// Text colour of the KDE colour scheme (`kdeglobals`, `[Colors:Window] ForegroundNormal`), so
/// that the tray icon matches the panel; `TRAY_COLOR` otherwise.
pub fn tray_color() -> String {
    let path = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .map(|d| d.join("kdeglobals"));
    let text = path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let mut in_window = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_window = line == "[Colors:Window]";
        } else if in_window && let Some(rgb) = line.strip_prefix("ForegroundNormal=") {
            let v: Vec<u8> = rgb
                .split(',')
                .filter_map(|c| c.trim().parse().ok())
                .collect();
            if let [r, g, b] = v[..] {
                return format!("#{r:02x}{g:02x}{b:02x}");
            }
        }
    }
    TRAY_COLOR.to_owned()
}

/// `svg` drawn in `color` into `size`×`size` unpremultiplied RGBA, centred with a small margin.
pub fn rgba(svg: &str, size: u32, color: &str) -> Option<Vec<u8>> {
    let svg = svg.replace("currentColor", color);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    let mut pm = Pixmap::new(size, size)?;
    let s = tree.size();
    let inner = size as f32 * 0.94;
    let k = (inner / s.width()).min(inner / s.height());
    let (dx, dy) = (
        (size as f32 - s.width() * k) / 2.0,
        (size as f32 - s.height() * k) / 2.0,
    );
    resvg::render(
        &tree,
        Transform::from_row(k, 0.0, 0.0, k, dx, dy),
        &mut pm.as_mut(),
    );
    Some(
        pm.pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Icon sheet on dark and light backgrounds at tray sizes and large:
    /// `TMPDIR=… cargo test -- --ignored icon_sheet` → `$TMPDIR/hsm01-icons.png`.
    #[test]
    #[ignore]
    fn icon_sheet() {
        let sizes = [16u32, 22, 32, 48, 256];
        let (w, h) = (
            sizes.iter().sum::<u32>() + 12 * sizes.len() as u32 + 12,
            2 * 268,
        );
        let mut sheet = Pixmap::new(w, h).unwrap();
        for (row, bg) in [(0, [0x23u8, 0x26, 0x29]), (1, [0xef, 0xf0, 0xf1])] {
            let mut x = 12;
            for &size in &sizes {
                let color = if row == 0 { TRAY_COLOR } else { "#232629" };
                let svg = if size >= 64 { WINDOW_SVG } else { TRAY_SVG };
                let px = rgba(svg, size, color).unwrap();
                for yy in 0..268 {
                    for xx in 0..size + 12 {
                        let i = ((row * 268 + yy) * w + x - 6 + xx) as usize * 4;
                        sheet.data_mut()[i..i + 4].copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
                    }
                }
                for yy in 0..size {
                    for xx in 0..size {
                        let s = ((yy * size + xx) * 4) as usize;
                        let a = px[s + 3] as u32;
                        let i = ((row * 268 + 6 + yy) * w + x + xx) as usize * 4;
                        for k in 0..3 {
                            let d = sheet.data()[i + k] as u32;
                            sheet.data_mut()[i + k] =
                                ((px[s + k] as u32 * a + d * (255 - a)) / 255) as u8;
                        }
                    }
                }
                x += size + 12;
            }
        }
        sheet
            .save_png(std::env::temp_dir().join("hsm01-icons.png"))
            .unwrap();
    }

    #[test]
    fn the_icon_renders() {
        let px = rgba(TRAY_SVG, 32, TRAY_COLOR).expect("valid SVG");
        assert_eq!(px.len(), 32 * 32 * 4);
        let opaque = px.chunks(4).filter(|p| p[3] > 128).count();
        assert!(
            opaque > 32 && opaque < 32 * 32 / 2,
            "{opaque} opaque pixels"
        );
    }
}
