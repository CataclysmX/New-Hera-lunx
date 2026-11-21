//! Desktop notifications shown on the device (Discord…). The app watches the session bus as a
//! monitor (`BecomeMonitor`, like `dbus-monitor`) for the `org.freedesktop.Notifications.Notify`
//! calls that applications send to the desktop's notification server, and queues those of the
//! chosen applications for the engine. The desktop still shows them; nothing is sent on that
//! connection (a monitor may not); their content is never written to the logs.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tiny_skia::Pixmap;
use zbus::zvariant::{OwnedValue, Value};

use crate::i18n::tr;
use crate::config::{Color, Notifications};
use crate::engine::Shared;

/// Side of the picture in the popup (px).
pub const ICON: u32 = 56;
/// Notifications waiting at most (the oldest are dropped).
const QUEUE: usize = 4;
/// Longest title / message kept (characters).
const MAX_CHARS: usize = 300;

#[derive(Clone)]
pub struct Notification {
    /// Application name ("Discord", "Vesktop"…).
    pub app: String,
    /// Title (for Discord: sender, and channel / server).
    pub summary: String,
    pub body: String,
    /// Picture of the notification (sender's avatar, round) or the application's icon,
    /// `ICON`×`ICON` at most.
    pub icon: Option<Pixmap>,
    /// Display time (s), else the `[notifications]` one.
    pub duration: Option<f32>,
    /// Wake the screen up from standby, else as `[notifications]` says.
    pub wake: Option<bool>,
    /// LED (1-4) blinking while it is shown.
    pub led: Option<u8>,
    /// From the desktop (D-Bus): shown only while `[notifications]` is enabled.
    pub desktop: bool,
    /// Alert of an API client: (client, alert id), told when it closes.
    pub origin: Option<(u64, u64)>,
    /// How it looks: card (default) or pill.
    pub style: NoticeStyle,
}

/// How a notification looks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum NoticeStyle {
    /// Card at the top: picture, application, title, message, remaining time.
    #[default]
    Card,
    /// Short banner (`summary` only): a rounded pill at the top, outlined in `border` (with its
    /// glow), a `dot` at each end, the text in `color`.
    Pill { border: Color, dot: Color, color: Color },
}

impl Notification {
    /// A desktop notification (the other settings from `[notifications]`).
    fn desktop(app: String, summary: String, body: String, icon: Option<Pixmap>) -> Self {
        Notification {
            app,
            summary,
            body,
            icon,
            duration: None,
            wake: None,
            led: None,
            desktop: true,
            origin: None,
            style: NoticeStyle::Card,
        }
    }

    /// Who it comes from, for « one per application »: its application's name (case ignored),
    /// else the API client that sent it; None: not grouped.
    pub fn source(&self) -> Option<String> {
        let app = self.app.trim().to_lowercase();
        if !app.is_empty() {
            return Some(app);
        }
        self.origin.map(|(client, _)| format!("#{client}"))
    }
}

/// Listen in a thread of its own for the app's lifetime (reconnects if the bus goes away).
pub fn start(shared: Arc<Shared>) {
    let worker = move || {
        while !shared.quit.load(Ordering::Relaxed) {
            if let Err(e) = listen(&shared) {
                eprintln!("hsm01-app: notifications: {e}");
            }
            std::thread::sleep(Duration::from_secs(10));
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("hsm01-notify".into())
        .spawn(worker)
    {
        eprintln!("hsm01-app: notifications: {e}");
    }
}

/// Example shown by the window's « Tester » button.
pub fn test(shared: &Shared) {
    let n = Notification::desktop(
        "Discord".into(),
        crate::APP_NAME.into(),
        tr("Test notification: it stays on screen for a few seconds; a click on a button or a knob closes it.")
            .into(),
        crate::render::load_icon("discord", ICON, ICON, None),
    );
    push(shared, n, false);
}

/// A pill alert as the modules show them (window's « Tester une pilule »), whatever
/// `[notifications]` says.
pub fn test_pill(shared: &Shared) {
    let green = Color(0x3d, 0xdc, 0x84);
    let n = Notification {
        summary: tr("ACTION ON: LIVE").into(),
        desktop: false,
        duration: Some(3.0),
        style: NoticeStyle::Pill { border: green, dot: green, color: Color(255, 255, 255) },
        ..Notification::desktop(String::new(), String::new(), String::new(), None)
    };
    push(shared, n, false);
}

/// Queue `n` (first if `urgent`): one per application, it replaces the one of its application
/// waiting (track changes in a row: only the last); the oldest waiting ones are dropped beyond
/// `QUEUE`. The one shown is replaced by the engine (`next_notice`).
pub fn push(shared: &Shared, n: Notification, urgent: bool) {
    let mut replaced = Vec::new();
    {
        let mut q = shared.notifications.lock().unwrap();
        if let Some(src) = n.source() {
            replaced.extend(q.iter().filter(|x| x.source().as_ref() == Some(&src)).filter_map(|x| x.origin));
            q.retain(|x| x.source().as_ref() != Some(&src));
        }
        while q.len() >= QUEUE {
            if urgent { q.pop_back() } else { q.pop_front() };
        }
        if urgent {
            q.push_front(n);
        } else {
            q.push_back(n);
        }
    }
    for (client, id) in replaced {
        let msg = serde_json::json!({"event": "alert_closed", "id": id, "reason": "replaced"});
        shared.api.send(client, &msg);
    }
}

/// Picture of an image file (`file://` URI or absolute path): centre square, `ICON`×`ICON`,
/// round if asked.
pub fn picture_file(path: &str, round_it: bool) -> Option<Pixmap> {
    let p = path.strip_prefix("file://").unwrap_or(path);
    let img = cover(crate::render::open_image(p.as_ref())?.to_rgba8());
    crate::render::to_pixmap(if round_it { round(img) } else { img })
}

type Hints = HashMap<String, OwnedValue>;

fn listen(shared: &Shared) -> zbus::Result<()> {
    use zbus::message::Type;
    let conn = zbus::blocking::Connection::session()?;
    let rule = zbus::MatchRule::builder()
        .msg_type(Type::MethodCall)
        .interface("org.freedesktop.Notifications")?
        .member("Notify")?
        .build();
    zbus::blocking::fdo::MonitoringProxy::new(&conn)?.become_monitor(&[rule], 0)?;
    for msg in zbus::blocking::MessageIterator::from(&conn) {
        let msg = msg?;
        let header = msg.header();
        if header.message_type() != Type::MethodCall
            || header.member().is_none_or(|m| m.as_str() != "Notify")
        {
            continue;
        }
        let cfg = shared.config.read().unwrap().notifications.clone();
        if !cfg.enabled {
            continue;
        }
        let body = msg.body();
        type Notify = (String, u32, String, String, String, Vec<String>, Hints, i32);
        let Ok((app, _, app_icon, summary, text, _, hints, _)) = body.deserialize::<Notify>()
        else {
            continue;
        };
        let entry = hint_str(&hints, "desktop-entry").unwrap_or_default();
        if !wanted(&cfg, &app, &entry) {
            continue;
        }
        let name = if app.is_empty() { &entry } else { &app };
        let icon = picture(&hints, &app_icon, &[&app_icon, &entry, &app.to_lowercase()]);
        let n = Notification::desktop(capitalised(name), plain(&summary), plain(&text), icon);
        push(shared, n, false);
    }
    Ok(())
}

/// From one of the chosen applications (part of its name or desktop entry, any case; no
/// application chosen = all).
fn wanted(cfg: &Notifications, app: &str, entry: &str) -> bool {
    let (app, entry) = (app.to_lowercase(), entry.to_lowercase());
    let mut chosen = cfg
        .apps
        .iter()
        .map(|a| a.trim().to_lowercase())
        .filter(|a| !a.is_empty())
        .peekable();
    chosen.peek().is_none() || chosen.any(|a| app.contains(&a) || entry.contains(&a))
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn hint_str(hints: &Hints, key: &str) -> Option<String> {
    match &**hints.get(key)? {
        Value::Str(s) => Some(s.as_str().to_owned()),
        _ => None,
    }
}

/// Text without the markup a notification may carry (`<b>`, `<a href>`…, entities), shortened.
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(4 * MAX_CHARS));
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        // a tag starts with a letter, '/' or '!': "<3" stays text
        if c == '<' && chars.peek().is_some_and(|n| n.is_alphabetic() || *n == '/' || *n == '!') {
            for t in chars.by_ref() {
                if t == '>' {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    out.trim().chars().take(MAX_CHARS).collect()
}

/// The notification's picture (`image-data` hint, then `image-path`, then a file as icon),
/// round; else the icon named by the notification or its application.
fn picture(hints: &Hints, app_icon: &str, names: &[&str]) -> Option<Pixmap> {
    let path_hint = ["image-path", "image_path"]
        .iter()
        .find_map(|k| hint_str(hints, k));
    let file = |p: &str| {
        let p = p.strip_prefix("file://").unwrap_or(p);
        p.starts_with('/')
            .then(|| image::open(p).ok().map(|i| i.to_rgba8()))
            .flatten()
    };
    let img = image_data(hints)
        .or_else(|| path_hint.as_deref().and_then(file))
        .or_else(|| file(app_icon));
    if let Some(img) = img {
        return crate::render::to_pixmap(round(cover(img)));
    }
    path_hint
        .iter()
        .map(String::as_str)
        .chain(names.iter().copied())
        .filter(|n| !n.is_empty() && !n.contains('/'))
        .find_map(|n| crate::render::load_icon(n, ICON, ICON, None))
}

/// Raw image of the `image-data` hint: (width, height, row stride, alpha, bits per sample,
/// channels, bytes).
fn image_data(hints: &Hints) -> Option<image::RgbaImage> {
    let v = ["image-data", "image_data", "icon_data"]
        .iter()
        .find_map(|k| hints.get(*k))?;
    let Value::Structure(s) = &**v else {
        return None;
    };
    let [
        Value::I32(w),
        Value::I32(h),
        Value::I32(stride),
        Value::Bool(alpha),
        Value::I32(8),
        Value::I32(ch),
        Value::Array(data),
    ] = s.fields()
    else {
        return None;
    };
    let (w, h, stride, ch) = (*w as usize, *h as usize, *stride as usize, *ch as usize);
    if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) || !(3..=4).contains(&ch) {
        return None;
    }
    let bytes: Vec<u8> = data
        .iter()
        .filter_map(|v| match v {
            Value::U8(b) => Some(*b),
            _ => None,
        })
        .collect();
    if stride < w * ch || bytes.len() < stride * (h - 1) + w * ch {
        return None;
    }
    Some(image::RgbaImage::from_fn(w as u32, h as u32, |x, y| {
        let i = y as usize * stride + x as usize * ch;
        let a = if *alpha && ch == 4 { bytes[i + 3] } else { 255 };
        image::Rgba([bytes[i], bytes[i + 1], bytes[i + 2], a])
    }))
}

/// Centre square, `ICON`×`ICON`.
fn cover(img: image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    let side = w.min(h);
    let square = image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side);
    image::imageops::resize(
        &square.to_image(),
        ICON,
        ICON,
        image::imageops::FilterType::Triangle,
    )
}

/// Inside the circle only (anti-aliased edge), like the avatars of the chat applications.
fn round(mut img: image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    let (cx, cy, r) = (w as f32 / 2.0, h as f32 / 2.0, w.min(h) as f32 / 2.0);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
        let keep = (r - d + 0.5).clamp(0.0, 1.0);
        p.0[3] = (p.0[3] as f32 * keep).round() as u8;
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_removed_but_not_text() {
        assert_eq!(plain("<b>Alice</b> &amp; Bob : a <3 b"), "Alice & Bob : a <3 b");
        assert_eq!(plain("<a href=\"x\">lien</a>"), "lien");
    }

    /// Real session bus: `cargo test -- --ignored live_notification` (sends one with notify-send).
    #[test]
    #[ignore]
    fn live_notification() {
        use std::time::Instant;
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let s = shared.clone();
        std::thread::spawn(move || listen(&s));
        std::thread::sleep(Duration::from_millis(500));
        std::process::Command::new("notify-send")
            .args(["-a", "discord", "-t", "1500", "hsm01 test", "bonjour <b>tout</b> le monde"])
            .status()
            .expect("notify-send");
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if let Some(n) = shared.notifications.lock().unwrap().pop_front() {
                assert_eq!((n.app.as_str(), n.summary.as_str()), ("Discord", "hsm01 test"));
                assert_eq!(n.body, "bonjour tout le monde");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("notification not seen");
    }

    #[test]
    fn application_filter() {
        let mut cfg = Notifications {
            apps: vec!["Discord".into()],
            ..Default::default()
        };
        assert!(wanted(&cfg, "discord", ""));
        assert!(wanted(&cfg, "", "com.discordapp.Discord"));
        assert!(!wanted(&cfg, "Firefox", "firefox"));
        cfg.apps = vec![" ".into()];
        assert!(wanted(&cfg, "Firefox", ""));
    }
}
