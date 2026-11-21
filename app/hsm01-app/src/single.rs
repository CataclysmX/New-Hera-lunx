//! One running app per user: the first instance listens on a Unix socket in `$XDG_RUNTIME_DIR`;
//! starting the app again only asks the running one to show its window.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::engine::Shared;

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("hsm01-app.sock")
}

/// `true` if another instance is running (nothing asked of it).
pub fn running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// `true` if another instance is running (it has been asked to show its window).
pub fn ask_running_instance() -> bool {
    ask_at(&socket_path())
}

fn ask_at(path: &std::path::Path) -> bool {
    match UnixStream::connect(path) {
        Ok(mut s) => {
            let _ = s.write_all(b"show\n");
            true
        }
        Err(_) => false,
    }
}

/// Become the running instance: a "show" line from a later start opens the window (a connection
/// without it only checked that the app runs).
pub fn listen(shared: Arc<Shared>) {
    listen_at(socket_path(), shared);
}

fn listen_at(path: PathBuf, shared: Arc<Shared>) {
    let _ = std::fs::remove_file(&path); // left by a crashed instance (nobody answered above)
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return eprintln!("hsm01-app: {}: {e}", path.display()),
    };
    let serve = move || {
        for conn in listener.incoming().flatten() {
            let _ = conn.set_read_timeout(Some(std::time::Duration::from_secs(1)));
            let mut line = String::new();
            if BufReader::new(conn).read_line(&mut line).is_ok() && line.trim() == "show" {
                shared.show_window.store(true, Ordering::Relaxed);
            }
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("hsm01-single".into())
        .spawn(serve)
    {
        eprintln!("hsm01-app: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A start that only checks that the app runs (`--minimized`) opens nothing; a normal start
    /// asks the running instance to show its window.
    #[test]
    fn show_only_when_asked() {
        let path = std::env::temp_dir().join(format!("hsm01-single-{}.sock", std::process::id()));
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        listen_at(path.clone(), shared.clone());
        let wait = |want: bool| {
            for _ in 0..100 {
                if shared.show_window.load(Ordering::Relaxed) == want {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            false
        };
        assert!(UnixStream::connect(&path).is_ok(), "running");
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!shared.show_window.load(Ordering::Relaxed), "a check shows nothing");
        assert!(ask_at(&path));
        assert!(wait(true), "a start shows the window");
        let _ = std::fs::remove_file(&path);
    }
}
