//! Which application is full screen (the "application en plein écran" target, for games): a small
//! KWin script, loaded through KWin's D-Bus scripting interface, reports the full-screen window
//! (`fullScreen`, or a borderless window covering its whole output) to this app's D-Bus service
//! `org.hsm01.App`. The last such window is kept while it stays full screen, even when another
//! window takes the focus. The script is unloaded when the app stops (`Watcher` dropped).

use std::sync::Arc;

use zbus::blocking::{Connection, Proxy, connection};

use crate::i18n::tr;
use crate::engine::Shared;

/// The full-screen window as KWin reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FullscreenApp {
    pub pid: u32,
    /// X11 / Wayland window class (`steam_app_1234` for Steam games under Proton).
    pub class: String,
    /// Desktop file name (without `.desktop`), often empty for games.
    pub desktop: String,
    pub caption: String,
}

const SCRIPT_NAME: &str = "hsm01-fullscreen";
const SCRIPT: &str = r#"
// hsm01-app: reports the full-screen window to org.hsm01.App (see fullscreen.rs)
let last = null;
function covers(w) {
    try {
        const g = w.frameGeometry, o = w.output.geometry;
        return g.width >= o.width && g.height >= o.height && g.x <= o.x && g.y <= o.y;
    } catch (e) {
        return false;
    }
}
function isFull(w) {
    return !!w && w.normalWindow && (w.fullScreen || covers(w));
}
function send() {
    const w = workspace.activeWindow;
    if (isFull(w)) {
        last = w;
    } else if (last && (workspace.windowList().indexOf(last) < 0 || !isFull(last))) {
        last = null;
    }
    callDBus("org.hsm01.App", "/org/hsm01/App", "org.hsm01.App", "Fullscreen",
             last ? String(last.pid) : "0",
             last ? String(last.resourceClass || "") : "",
             last ? String(last.desktopFileName || "") : "",
             last ? String(last.caption || "") : "");
}
function watch(w) {
    if (!w) return;
    w.fullScreenChanged.connect(send);
    w.frameGeometryChanged.connect(send);
}
workspace.windowList().forEach(watch);
workspace.windowAdded.connect(watch);
workspace.windowActivated.connect(send);
workspace.windowRemoved.connect(send);
send();
"#;

struct Service {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.hsm01.App")]
impl Service {
    /// Called by the KWin script; `pid` "0" = no full-screen window.
    fn fullscreen(&self, pid: String, class: String, desktop: String, caption: String) {
        let pid: u32 = pid.parse().unwrap_or(0);
        let app = (pid != 0).then_some(FullscreenApp {
            pid,
            class,
            desktop,
            caption,
        });
        let mut now = self.shared.fullscreen.lock().unwrap();
        if *now != app {
            let msg = match &app {
                Some(a) => tf!("full screen: {app}", app = if a.caption.is_empty() { &a.class } else { &a.caption }),
                None => tr("no full-screen application anymore").into(),
            };
            *now = app;
            drop(now);
            self.shared.set_message(msg);
        }
    }
}

/// Keeps the D-Bus service and the KWin script alive.
pub struct Watcher {
    conn: Connection,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Ok(p) = scripting(&self.conn) {
            let _: zbus::Result<bool> = p.call("unloadScript", &(SCRIPT_NAME,));
        }
    }
}

fn scripting(conn: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(conn, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting")
}

/// Start watching (KDE Plasma only); `None` with a message elsewhere.
pub fn start(shared: Arc<Shared>) -> Option<Watcher> {
    let run = || -> Result<Watcher, String> {
        let conn = connection::Builder::session()
            .and_then(|b| b.name("org.hsm01.App"))
            .and_then(|b| b.serve_at("/org/hsm01/App", Service { shared }))
            .and_then(|b| b.build())
            .map_err(|e| format!("D-Bus : {e}"))?;
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = dir.join("hsm01-fullscreen.js");
        std::fs::write(&path, SCRIPT).map_err(|e| format!("{}: {e}", path.display()))?;
        let p = scripting(&conn).map_err(|e| format!("KWin : {e}"))?;
        let _: zbus::Result<bool> = p.call("unloadScript", &(SCRIPT_NAME,)); // a stale copy
        let id: i32 = p
            .call(
                "loadScript",
                &(path.to_string_lossy().as_ref(), SCRIPT_NAME),
            )
            .map_err(|e| format!("KWin (script) : {e}"))?;
        let script = Proxy::new(
            &conn,
            "org.kde.KWin",
            format!("/Scripting/Script{id}"),
            "org.kde.kwin.Script",
        )
        .map_err(|e| format!("KWin : {e}"))?;
        script
            .call::<_, _, ()>("run", &())
            .map_err(|e| format!("KWin (run) : {e}"))?;
        drop((script, p));
        Ok(Watcher { conn })
    };
    match run() {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("hsm01-app: {}", tf!("full screen not followed ({e})", e = e));
            None
        }
    }
}

/// `pid` or one of its ancestors is `root` (a game's audio may come from a child process).
pub fn descends_from(pid: u32, root: u32) -> bool {
    let mut p = pid;
    for _ in 0..16 {
        if p == root {
            return true;
        }
        if p <= 1 {
            return false;
        }
        // /proc/<pid>/stat: "pid (comm) state ppid …" – comm may contain spaces/parentheses
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{p}/stat")) else {
            return false;
        };
        let Some(rest) = stat.rfind(')').map(|i| &stat[i + 1..]) else {
            return false;
        };
        match rest.split_whitespace().nth(1).and_then(|s| s.parse().ok()) {
            Some(ppid) => p = ppid,
            None => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_ancestry() {
        let me = std::process::id();
        assert!(descends_from(me, me));
        assert!(!descends_from(1, me));
    }
}
