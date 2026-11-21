//! One running app per user: the first instance listens on a Unix socket in `$XDG_RUNTIME_DIR`;
//! starting the app again only asks the running one to show its window.

use std::io::Write;
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

/// `true` if another instance is running (it has been asked to show its window).
pub fn ask_running_instance() -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut s) => {
            let _ = s.write_all(b"show\n");
            true
        }
        Err(_) => false,
    }
}

/// Become the running instance: a "show" from a later start opens the window.
pub fn listen(shared: Arc<Shared>) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path); // left by a crashed instance (nobody answered above)
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return eprintln!("hsm01-app: {}: {e}", path.display()),
    };
    let serve = move || {
        for conn in listener.incoming() {
            if conn.is_ok() {
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
