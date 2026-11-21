//! `hsm01-app` – « new hera-nux », companion app for the Hercules Stream 100 running the custom
//! firmware.
//! Window with live preview and editor (`gui`), the device engine in its own thread
//! (docs/app/companion.md). Options: `--headless` (no window), `--minimized` (started in the system
//! tray: no window until its icon or a new start asks for it; for the session's autostart),
//! `--page N` (start page),
//! `--sim` (simulated audio), `--print-default-config`, `--snapshot <png> [--page N]`,
//! `--keys <combo>` (test a shortcut).

#[macro_use]
mod i18n;
mod api;
mod audio;
mod config;
mod deckfile;
mod engine;
mod fullscreen;
mod gauge;
mod gui;
mod icon;
mod icons;
mod keys;
mod layout;
mod media;
mod notify;
mod pulse;
mod render;
mod screen;
mod settingsform;
mod single;
mod state;
mod tools;
mod tray;
mod visual;
mod wallpaper;

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use config::Config;
use crate::i18n::tr;

/// Name shown in the window title, the system tray and the messages.
pub const APP_NAME: &str = "new hera-nux";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--print-default-config") {
        print!(
            "{}",
            toml::to_string_pretty(&Config::default()).expect("default config")
        );
        return ExitCode::SUCCESS;
    }
    if let Some(i) = args.iter().position(|a| a == "--keys") {
        let spec = args.get(i + 1).map_or("", String::as_str);
        return match keys::press_once(spec) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("hsm01-app: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let cfg = match Config::load_or_default() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hsm01-app: {e}");
            return ExitCode::FAILURE;
        }
    };
    i18n::set_language(&cfg.language);
    eprintln!("hsm01-app: config {}", Config::path().display());
    let simulate = args.iter().any(|a| a == "--sim");
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let Some(path) = args.get(i + 1) else {
            eprintln!("usage: hsm01-app --snapshot <file.png> [--page N] [--sim]");
            return ExitCode::from(2);
        };
        let page = args
            .iter()
            .position(|a| a == "--page")
            .and_then(|i| args.get(i + 1)?.parse::<usize>().ok())
            .map_or(0, |p| p.saturating_sub(1));
        return match engine::snapshot(&cfg, page, audio::open(simulate), path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("hsm01-app: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let start_page = args
        .iter()
        .position(|a| a == "--page")
        .and_then(|i| args.get(i + 1)?.parse::<usize>().ok())
        .map_or(0, |p| p.saturating_sub(1));
    let headless = args.iter().any(|a| a == "--headless");
    let minimized = args.iter().any(|a| a == "--minimized");
    if !headless && minimized && single::running() {
        eprintln!("hsm01-app: {}", tr("new hera-nux is already running"));
        return ExitCode::SUCCESS;
    }
    if !headless && !minimized && single::ask_running_instance() {
        eprintln!("hsm01-app: {}", tr("new hera-nux is already running – its window is shown"));
        return ExitCode::SUCCESS;
    }
    let shared = Arc::new(engine::Shared::new(cfg));
    quit_on_signals(&shared);
    shared.page.store(start_page, Ordering::Relaxed);
    let _fullscreen = fullscreen::start(shared.clone());
    notify::start(shared.clone());
    api::start(shared.clone());
    let ws = shared.config.read().unwrap().api.clone();
    api::set_websocket(&shared, &ws);
    media::start(shared.clone());
    tools::start(shared.clone());
    if headless {
        engine::run(shared, || audio::open(simulate));
        return ExitCode::SUCCESS;
    }
    let engine = {
        let shared = shared.clone();
        std::thread::Builder::new()
            .name("hsm01-engine".into())
            .spawn(move || engine::run(shared, move || audio::open(simulate)))
    };
    let engine = match engine {
        Ok(h) => h,
        Err(e) => {
            eprintln!("hsm01-app: engine thread: {e}");
            return ExitCode::FAILURE;
        }
    };
    single::listen(shared.clone());
    // with a tray icon, closing the window keeps the app running; the icon reopens it.
    // `--minimized`: no window until the icon or a new start (single instance) asks for one
    let tray = tray::spawn(shared.clone());
    let mut window = !minimized;
    loop {
        if window {
            if let Err(e) = gui::run(shared.clone()) {
                eprintln!("hsm01-app: {}", tf!("{e} – without a window", e = e));
                break;
            }
            if !tray {
                shared.quit.store(true, Ordering::Relaxed);
            }
        }
        window = true;
        while !shared.quit.load(Ordering::Relaxed)
            && !shared.show_window.swap(false, Ordering::Relaxed)
        {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if shared.quit.load(Ordering::Relaxed) {
            break;
        }
    }
    let _ = engine.join();
    ExitCode::SUCCESS
}

/// SIGTERM (end of the session, shutdown, `kill`), SIGINT (Ctrl+C), SIGHUP: quit as from the
/// tray, so that the device goes to standby on the way out (the engine's `Drop`). Exit anyway
/// `QUIT_DEADLINE` after any quit request: a minimised window may never see it.
fn quit_on_signals(shared: &Arc<engine::Shared>) {
    const QUIT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let s = shared.clone();
        // SAFETY: the handler only stores into an atomic (async-signal-safe)
        let r = unsafe {
            signal_hook_registry::register(sig, move || s.quit.store(true, Ordering::Relaxed))
        };
        if let Err(e) = r {
            eprintln!("hsm01-app: signal {sig}: {e}");
        }
    }
    let s = shared.clone();
    std::thread::spawn(move || {
        while !s.quit.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        std::thread::sleep(QUIT_DEADLINE);
        eprintln!("hsm01-app: {}", tr("forced exit (the window did not answer)"));
        std::process::exit(0);
    });
}
