//! Freedesktop icon lookup for the audio sources' icons (`application.icon_name`,
//! `device.icon_name`): the desktop's icon theme (`kdeglobals` `[Icons] Theme`, Breeze by
//! default) and the themes it inherits, then hicolor (where applications install theirs),
//! Adwaita and `/usr/share/pixmaps`. SVG preferred, then the larger PNG sizes.

use std::path::PathBuf;

const SIZES: [&str; 10] = [
    "scalable", "64", "48", "96", "128", "256", "32", "24", "22", "16",
];
/// Searched in this order (the first contexts win for names found in several).
const CONTEXTS: [&str; 8] = [
    "apps",
    "devices",
    "status",
    "places",
    "categories",
    "mimetypes",
    // previous / play / next, go-previous… (pages of the API, menus)
    "actions",
    "emblems",
];

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Directories holding icon themes, most specific first.
fn roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(h) = home() {
        out.push(h.join(".local/share/icons"));
        out.push(h.join(".icons"));
        out.push(h.join(".local/share/flatpak/exports/share/icons"));
    }
    let data =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    out.extend(
        data.split(':')
            .filter(|d| !d.is_empty())
            .map(|d| PathBuf::from(d).join("icons")),
    );
    out.push(PathBuf::from("/var/lib/flatpak/exports/share/icons"));
    out
}

/// `[Icons] Theme` of kdeglobals, Breeze otherwise.
fn current_theme() -> String {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".config")));
    let text = config
        .and_then(|c| std::fs::read_to_string(c.join("kdeglobals")).ok())
        .unwrap_or_default();
    let mut in_icons = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_icons = line == "[Icons]";
        } else if in_icons && let Some(theme) = line.strip_prefix("Theme=") {
            return theme.trim().to_owned();
        }
    }
    "breeze".into()
}

/// The theme (its dark variant first: the device screen is usually dark), the themes it
/// inherits (`index.theme`), then hicolor and Adwaita.
fn themes(roots: &[PathBuf]) -> Vec<String> {
    let theme = current_theme();
    let dark = format!("{theme}-dark");
    let mut out = Vec::new();
    if !theme.ends_with("-dark") && roots.iter().any(|r| r.join(&dark).is_dir()) {
        out.push(dark);
    }
    out.push(theme);
    let mut i = 0;
    while i < out.len() && out.len() < 8 {
        let index = roots
            .iter()
            .map(|r| r.join(&out[i]).join("index.theme"))
            .find_map(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        if let Some(line) = index
            .lines()
            .find_map(|l| l.trim().strip_prefix("Inherits="))
        {
            for t in line.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                if !out.iter().any(|o| o == t) {
                    out.push(t.to_owned());
                }
            }
        }
        i += 1;
    }
    for t in ["hicolor", "Adwaita"] {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_owned());
        }
    }
    out
}

/// Icon of a window: `Icon=` of its desktop file (`desktop`, or one whose name or
/// `StartupWMClass` is `class`), `steam_icon_N` for Steam games (`steam_app_N`), else the class.
pub fn window_icon(class: &str, desktop: &str) -> String {
    if let Some(id) = class.strip_prefix("steam_app_") {
        let steam = format!("steam_icon_{id}");
        if find(&steam).is_some() {
            return steam;
        }
        if let Some(p) = id.parse().ok().and_then(steam_game_icon) {
            return p.to_string_lossy().into_owned();
        }
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(h) = home() {
        dirs.push(h.join(".local/share/applications"));
        dirs.push(h.join(".local/share/flatpak/exports/share/applications"));
    }
    let data =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(
        data.split(':')
            .filter(|d| !d.is_empty())
            .map(|d| PathBuf::from(d).join("applications")),
    );
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    let icon_of = |text: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix("Icon="))
            .map(|i| i.trim().to_owned())
    };
    let wanted: Vec<String> = [desktop, class]
        .iter()
        .filter(|n| !n.is_empty())
        .map(|n| format!("{}.desktop", n.to_lowercase()))
        .collect();
    let class_line = format!("StartupWMClass={class}");
    for dir in &dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in entries.filter_map(Result::ok) {
            let file = e.file_name().to_string_lossy().to_lowercase();
            if !file.ends_with(".desktop") {
                continue;
            }
            let by_name = wanted.contains(&file);
            let text = match std::fs::read_to_string(e.path()) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let by_class = !class.is_empty() && text.lines().any(|l| l.trim() == class_line);
            if (by_name || by_class)
                && let Some(icon) = icon_of(&text)
            {
                return icon;
            }
        }
    }
    class.to_lowercase()
}

/// Steam installations: native, `~/.steam/steam`, Flatpak.
fn steam_roots() -> Vec<PathBuf> {
    let Some(h) = home() else { return Vec::new() };
    [
        ".local/share/Steam",
        ".steam/steam",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
    ]
    .iter()
    .map(|d| h.join(d))
    .filter(|d| d.join("appcache").is_dir())
    .collect()
}

/// Icon of Steam game `appid` (Proton games have no desktop file): its client icon
/// (`steam/games/<hash>.ico`, several sizes up to 256 px; the hash is in `appcache/appinfo.vdf`),
/// else the 32×32 library icon (`appcache/librarycache/<appid>/<hash>.jpg`, or `<appid>_icon.jpg`
/// in the older layout).
pub fn steam_game_icon(appid: u32) -> Option<PathBuf> {
    let roots = steam_roots();
    for root in &roots {
        if let Some(p) = steam_client_icon(root, appid) {
            return Some(p);
        }
    }
    for root in &roots {
        let cache = root.join("appcache/librarycache");
        let old = cache.join(format!("{appid}_icon.jpg"));
        if old.is_file() {
            return Some(old);
        }
        let Ok(dir) = std::fs::read_dir(cache.join(appid.to_string())) else {
            continue;
        };
        let entries: Vec<_> = dir.filter_map(Result::ok).collect();
        for e in &entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.len() == 44 && name.ends_with(".jpg") && is_hash(&name[..40]) {
                return Some(e.path());
            }
        }
        // no icon cached: the cover art (in the game's folder or one of its hash sub-folders)
        for art in [
            "library_capsule.jpg",
            "library_600x900.jpg",
            "logo.png",
            "header.jpg",
            "library_header.jpg",
        ] {
            let found = std::iter::once(cache.join(appid.to_string()))
                .chain(entries.iter().map(|e| e.path()))
                .map(|d| d.join(art))
                .find(|p| p.is_file());
            if found.is_some() {
                return found;
            }
        }
    }
    None
}

fn is_hash(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `.ico` named by one of the 40-hex strings of the game's `appinfo.vdf` entry (the client
/// icon; the other hashes are library images).
fn steam_client_icon(root: &std::path::Path, appid: u32) -> Option<PathBuf> {
    let data = std::fs::read(root.join("appcache/appinfo.vdf")).ok()?;
    let entry = appinfo_entry(&data, appid)?;
    let games = root.join("steam/games");
    entry
        .split(|&b| b == 0)
        .filter_map(|s| std::str::from_utf8(s).ok())
        .filter_map(|s| s.get(s.len().saturating_sub(40)..))
        .filter(|s| is_hash(s))
        .map(|hash| games.join(format!("{hash}.ico")))
        .find(|p| p.is_file())
}

/// Bytes of `appid`'s entry in `appinfo.vdf` (header: magic, universe, + string-table offset
/// since v29; entries: appid u32, size u32, then `size` bytes; appid 0 ends the list).
fn appinfo_entry(data: &[u8], appid: u32) -> Option<&[u8]> {
    let u32_at = |i: usize| -> Option<u32> {
        Some(u32::from_le_bytes(data.get(i..i + 4)?.try_into().ok()?))
    };
    let mut i = match u32_at(0)? {
        0x0756_4429 => 16,
        0x0756_4427 | 0x0756_4428 => 8,
        _ => return None,
    };
    loop {
        let id = u32_at(i)?;
        if id == 0 {
            return None;
        }
        let size = u32_at(i + 4)? as usize;
        let (start, end) = (i + 8, i + 8 + size);
        if id == appid {
            return data.get(start..end);
        }
        i = end;
    }
}

/// File of icon `name` (or `name` itself when it is an absolute path to a file, as a desktop
/// file's `Icon=` can be), or of a more generic name (`audio-card-analog-pci` → `audio-card-analog` →
/// `audio-card`, as the freedesktop spec falls back), if installed.
pub fn find(name: &str) -> Option<PathBuf> {
    if name.starts_with('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    if name.is_empty() || name.contains('/') {
        return None;
    }
    let mut n = name;
    loop {
        if let Some(p) = find_exact(n) {
            return Some(p);
        }
        n = &n[..n.rfind('-')?];
    }
}

/// Monochrome `<name>-symbolic` icon (same generic fallback on `name`).
pub fn find_symbolic(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    let mut n = name;
    loop {
        if let Some(p) = find_exact(&format!("{n}-symbolic")) {
            return Some(p);
        }
        n = &n[..n.rfind('-')?];
    }
}

fn find_exact(name: &str) -> Option<PathBuf> {
    let roots = roots();
    for theme in themes(&roots) {
        for root in &roots {
            let dir = root.join(&theme);
            if !dir.is_dir() {
                continue;
            }
            for size in SIZES {
                let ext = if size == "scalable" { "svg" } else { "png" };
                for ctx in CONTEXTS {
                    // hicolor layout `48x48/apps`, Breeze layout `apps/48`
                    let sized = if size == "scalable" {
                        size.to_owned()
                    } else {
                        format!("{size}x{size}")
                    };
                    let candidates = [
                        dir.join(&sized).join(ctx).join(format!("{name}.{ext}")),
                        dir.join(ctx).join(size).join(format!("{name}.svg")),
                        dir.join(ctx).join(size).join(format!("{name}.png")),
                    ];
                    if let Some(p) = candidates.into_iter().find(|p| p.is_file()) {
                        return Some(p);
                    }
                }
            }
        }
    }
    ["svg", "png"]
        .iter()
        .map(|ext| PathBuf::from(format!("/usr/share/pixmaps/{name}.{ext}")))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_fallback() {
        // only meaningful where Breeze or hicolor is installed
        if find("audio-card").is_some() {
            assert!(find("audio-card-analog-pci-xyz").is_some());
        }
    }

    /// `cargo test --release -- --ignored --nocapture steam_icons`: icons of the cached games.
    #[test]
    #[ignore]
    fn steam_icons() {
        for root in steam_roots() {
            let ids = std::fs::read_dir(root.join("appcache/librarycache")).unwrap();
            for id in ids.filter_map(Result::ok).take(12) {
                let Ok(appid) = id.file_name().to_string_lossy().parse::<u32>() else {
                    continue;
                };
                let icon = steam_game_icon(appid);
                let size = icon
                    .as_ref()
                    .and_then(|p| image::open(p).ok())
                    .map(|i| (i.width(), i.height()));
                println!("{appid}: {size:?} {icon:?}");
            }
        }
    }

    #[test]
    fn rejects_paths() {
        assert_eq!(find("../../etc/passwd"), None);
        assert_eq!(find("/nonexistent/icon.png"), None);
        assert_eq!(find(""), None);
    }
}
