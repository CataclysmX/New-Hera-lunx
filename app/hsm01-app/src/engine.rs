//! Device engine (its own thread): connects to the Stream 100 (custom firmware), reads buttons and
//! knobs, runs actions, renders the current page at 60 fps and sends only the changed tiles.
//! Shares the config, the current page, a preview of the screen and a status with the GUI.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};

use hsm01_proto::free::{self, InputReport, knob};
use hsm01_usb::free::FreeDevice;

use crate::i18n::tr;
use crate::audio::{Audio, Inventory};
use crate::config::{
    Action, Color, Config, HOLD_MS, Led, MIN_BRIGHTNESS, Page, SCREEN_H, SCREEN_W,
    STANDBY_MIN_SECS, Target, WidgetKind,
};
use crate::keys::Keys;
use crate::api::ClientPage;
use crate::notify::Notification;
use serde_json::{Value, json};
use crate::render::{
    Lut, MENU_BOX, Renderer, notice_box, fill_rect, outline_rects, test_pattern, to_rgb565,
};
use crate::screen::Screen;
use crate::state::{ConfirmView, Overlay, View};
use crate::visual::{Analyzer, FFT_SIZE, bar_columns, column_delta, column_state};

const FRAME: Duration = Duration::from_micros(16_667);
/// Tile data sent per frame at most (~60 × 12 KiB ≈ the USB throughput): bigger changes are
/// spread over the next frames.
const SEND_BUDGET: usize = 12 * 1024;
/// Peak-hold marker fall speed (meter lengths per second).
const PEAK_FALL: f32 = 0.6;
const LED_ON: u8 = 100;

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub connected: bool,
    pub firmware: (u8, u8),
    pub message: String,
    pub fps: f32,
    pub kib_per_s: f32,
    /// Screen in standby (backlight off, nothing drawn).
    pub asleep: bool,
}

pub struct Shared {
    pub config: RwLock<Config>,
    /// Incremented when the engine reloads the config file (edited outside the GUI).
    pub config_version: AtomicU64,
    /// Index of the page shown on the device.
    pub page: AtomicUsize,
    /// Last rendered frame, RGBA 480×272, for the GUI preview.
    pub preview: Mutex<Vec<u8>>,
    pub preview_frame: AtomicU64,
    pub status: Mutex<Status>,
    pub quit: AtomicBool,
    /// Tray icon: open (or bring forward) the window.
    pub show_window: AtomicBool,
    /// Widget outlines drawn on the device (set by the window while it is open).
    pub overlay: Mutex<Overlay>,
    /// Full-screen window reported by KWin (`fullscreen.rs`).
    pub fullscreen: Mutex<Option<crate::fullscreen::FullscreenApp>>,
    /// Audio objects present (GUI target lists), refreshed twice a second.
    pub inventory: Mutex<Inventory>,
    /// Desktop notifications and API alerts waiting to be shown (`notify.rs`, `api.rs`).
    pub notifications: Mutex<VecDeque<Notification>>,
    /// Open API: clients, their pages (`api.rs`).
    pub api: crate::api::Hub,
    /// Programs run by the app (`tools.rs`): name → status.
    pub tools: crate::tools::Statuses,
    /// Media player followed (`media.rs`).
    pub media: Mutex<Option<crate::media::MediaState>>,
}

impl Shared {
    pub fn new(config: Config) -> Self {
        Shared {
            config: RwLock::new(config),
            config_version: AtomicU64::new(0),
            page: AtomicUsize::new(0),
            preview: Mutex::new(vec![0; (SCREEN_W * SCREEN_H * 4) as usize]),
            preview_frame: AtomicU64::new(0),
            status: Mutex::new(Status::default()),
            quit: AtomicBool::new(false),
            show_window: AtomicBool::new(false),
            overlay: Mutex::new(Overlay::default()),
            fullscreen: Mutex::new(None),
            inventory: Mutex::new(Inventory::default()),
            notifications: Mutex::new(VecDeque::new()),
            api: Default::default(),
            tools: Default::default(),
            media: Mutex::new(None),
        }
    }

    pub fn set_message(&self, msg: impl Into<String>) {
        let msg = msg.into();
        let mut s = self.status.lock().unwrap();
        if s.message != msg {
            eprintln!("hsm01-app: {msg}");
            s.message = msg;
        }
    }
}

struct Engine {
    shared: Arc<Shared>,
    audio: Box<dyn Audio>,
    dev: Option<Arc<FreeDevice>>,
    /// Input reports, read continuously by their own thread while connected.
    input: Option<Reader>,
    decoders: [knob::Decoder; 4],
    carry: [i32; 4],
    buttons: u16,
    peaks: [f32; 4],
    leds: Option<[u8; 4]>,
    spectra: Spectra,
    /// Redraw the spectrum areas completely (new connection, page change, config reload).
    force_direct: bool,
    /// Virtual keyboard for `Action::Keys`, started on first use.
    keys: Option<Keys>,
    /// Backlight last sent to the device.
    brightness: Option<u8>,
    /// Backlight chosen with the buttons (brightness actions), until the configured one changes.
    user_brightness: Option<u8>,
    /// Diagnostics printed every second with `HSM01_STATS=1`.
    stats: Stats,
    /// Last user action (standby timer).
    last_activity: Instant,
    /// Inputs that wake the screen up are not executed until then.
    wake_grace: Option<Instant>,
    /// Question shown before a volume goes above 100 %.
    confirm: Option<BoostConfirm>,
    /// Press start of the buttons (report bits) that have a hold action.
    pressed_at: [Option<Instant>; 9],
    /// Buttons whose hold action already ran during the current press.
    hold_done: u16,
    /// Buttons pressed while a program takes them, whose hold still opens the modules menu
    /// (`menu_keys`).
    menu_hold: u16,
    /// Deck knobs pressed whose hold opens the modules menu: their tile runs on release.
    deck_armed: u16,
    /// The module page shown (module name, page id), to come back to it after a restart.
    shown_tool: Option<(String, String)>,
    /// A module page left because its module went away (restarted…): shown again if it comes
    /// back soon.
    comeback: Option<Comeback>,
    /// Targets allowed above 100 % (until they come back to 100 % or less).
    boost_ok: std::collections::HashSet<Target>,
    /// Notification shown, since when.
    notice: Option<(Notification, Instant)>,
    /// Pages of the configuration + of the API clients.
    n_pages: usize,
    /// Pages of API clients: knob counts not sent yet, press start of the buttons, buttons
    /// whose hold was sent.
    client_carry: [i32; 4],
    client_pressed: [Option<Instant>; 9],
    client_held: u16,
    /// Client taking each button's current press (`None` inside: nobody, the bindings), and the
    /// page its events name.
    client_target: [Option<(Option<u64>, String)>; 9],
    /// Inputs taken by the tools shown in `tool` widgets of the configured page shown: input
    /// name → (client, its page id).
    routes: HashMap<String, (u64, String)>,
    /// Page a jump (`page`, `tool` actions) came from: where `back` returns.
    jump_from: Option<usize>,
    /// Last `screen_epoch` of the input reports, and a change seen: the firmware took the screen
    /// back (screen saver after 3 s without us), redraw everything.
    screen_epoch: Option<u8>,
    redraw: bool,
    /// `deck` widgets: (page, widget index) → selection, slide, flash.
    decks: HashMap<(usize, usize), DeckState>,
    /// Knobs aimed at a tool's command: counts not sent yet.
    tool_carry: [i32; 4],
    /// Pages of the configuration (the modules' come after them), and the last one shown:
    /// `next_page` / `prev_page` cycle through them only, power leaves a module's page for it.
    n_cfg: usize,
    last_cfg: usize,
    /// The modules menu, while it is open (or sliding out).
    menu: Option<ModuleMenu>,
}

/// The modules menu slides in, out, and closes by itself after a while without input.
const MENU_OPEN: Duration = Duration::from_millis(200);
const MENU_CLOSE: Duration = Duration::from_millis(150);
const MENU_IDLE: Duration = Duration::from_secs(10);

/// A module with a page, in the menu.
struct MenuEntry {
    owner: u64,
    page: String,
    name: String,
    title: String,
}

/// The modules menu: its entries (refreshed each frame), selection, opening and closing.
struct ModuleMenu {
    entries: Vec<MenuEntry>,
    selected: usize,
    carry: i32,
    opened: Instant,
    closing: Option<Instant>,
    used: Instant,
}

impl ModuleMenu {
    fn new() -> Self {
        ModuleMenu {
            entries: Vec::new(),
            selected: 0,
            carry: 0,
            opened: Instant::now(),
            closing: None,
            used: Instant::now(),
        }
    }

    /// 0 hidden → 1 shown: sliding in (eased out), or out.
    fn shown(&self) -> f32 {
        let k = |t: Instant, d: Duration| (t.elapsed().as_secs_f32() / d.as_secs_f32()).min(1.0);
        match self.closing {
            Some(t) => 1.0 - k(t, MENU_CLOSE),
            None => 1.0 - (1.0 - k(self.opened, MENU_OPEN)).powi(3),
        }
    }

    fn view(&self) -> crate::state::MenuView {
        crate::state::MenuView {
            items: self.entries.iter().map(|e| (e.name.clone(), e.title.clone())).collect(),
            selected: self.selected,
            shown: self.shown(),
        }
    }
}

/// The modules that have a page (their first one), by name.
fn menu_entries(api: &crate::api::Hub, pages: &[ClientPage]) -> Vec<MenuEntry> {
    let mut out: Vec<MenuEntry> = Vec::new();
    for p in pages.iter().filter(|p| p.size.is_none()) {
        if out.iter().any(|e| e.owner == p.owner) {
            continue;
        }
        out.push(MenuEntry {
            owner: p.owner,
            page: p.id.clone(),
            name: api.client_name(p.owner).unwrap_or_else(|| p.id.clone()),
            title: p.page.name.clone(),
        });
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    out
}

/// Knob counts (512 per turn) of one step of a `deck` (~13 tiles per turn), duration of its
/// slide to the next tile and of the flash of a clicked tile.
const DECK_STEP: i32 = 40;
const DECK_SLIDE: Duration = Duration::from_millis(160);
const DECK_FLASH: Duration = Duration::from_millis(250);

/// Selection of a `deck` widget, its slide and its flash.
#[derive(Default)]
struct DeckState {
    selected: usize,
    /// Knob counts not turned into steps yet.
    carry: i32,
    /// Slide: offset (tiles) when it started, and when.
    slide: (f32, Option<Instant>),
    flash: Option<Instant>,
}

impl DeckState {
    /// Slide still to go (tiles), easing out to 0.
    fn offset(&self) -> f32 {
        let (from, Some(t)) = self.slide else {
            return 0.0;
        };
        let k = (t.elapsed().as_secs_f32() / DECK_SLIDE.as_secs_f32()).min(1.0);
        from * (1.0 - k).powi(3)
    }

    /// `steps` tiles further (endless among `n`): the new selection slides in from where it is.
    fn step(&mut self, steps: i32, n: usize) {
        self.selected = (self.selected as i64 + steps as i64).rem_euclid(n as i64) as usize;
        self.slide = ((self.offset() + steps as f32).clamp(-2.0, 2.0), Some(Instant::now()));
    }

    fn view(&self, n: usize) -> crate::state::DeckView {
        let flash = self.flash.map_or(0.0, |t| {
            (1.0 - t.elapsed().as_secs_f32() / DECK_FLASH.as_secs_f32()).max(0.0)
        });
        crate::state::DeckView {
            selected: self.selected % n.max(1),
            offset: self.offset(),
            flash,
        }
    }
}

/// Knob counts (512 per turn) of one `turn` step sent to API clients (~21 per turn).
const CLIENT_STEP: i32 = 24;

/// A tool's page shown by the `tool` widget `index` of a configured page, and the inputs the
/// widget gives the tool.
struct Embed<'a> {
    index: usize,
    page: &'a ClientPage,
    inputs: &'a [String],
}

/// The `tool` widgets of `page` whose tool is running and has the page asked (among `pages`).
fn embedded<'a>(api: &crate::api::Hub, page: &'a Page, pages: &'a [ClientPage]) -> Vec<Embed<'a>> {
    let found = |(index, w): (usize, &'a crate::config::Widget)| {
        let WidgetKind::Tool { tool, page: id, inputs, .. } = &w.kind else {
            return None;
        };
        let (owner, id) = api.page_of_tool(tool, id)?;
        let page = pages.iter().find(|p| p.owner == owner && p.id == id)?;
        Some(Embed { index, page, inputs })
    };
    page.widgets.iter().enumerate().filter_map(found).collect()
}

/// The inputs the embedded tools take: input name → (client, its page id).
fn routes(embeds: &[Embed]) -> HashMap<String, (u64, String)> {
    let mut routes = HashMap::new();
    for x in embeds {
        for i in x.inputs {
            routes.insert(i.clone(), (x.page.owner, x.page.id.clone()));
        }
    }
    routes
}

/// "Volume au-dessus de 100 % ?" on the device: turning the knob that asked chooses (left « Oui »,
/// right « Non », « Non » first), clicking it validates; any other button or `CONFIRM_TIMEOUT`
/// without action answers « Non ».
struct BoostConfirm {
    knob: usize,
    target: Target,
    name: String,
    yes: bool,
    /// Counts turned since the last choice change.
    turn: i32,
    since: Instant,
}

const CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);
/// Knob counts (512 per turn) to move the choice.
const CONFIRM_STEP: i32 = 24;
/// Volume treated as 100 % (rounding of the server's volumes).
const FULL_VOLUME: f32 = 0.995;

/// After waking up from standby, knob turns and clicks are ignored this long.
const WAKE_GRACE: Duration = Duration::from_millis(500);

/// Reads every input report (~90/s) as soon as it arrives: polling from the 60 Hz loop with a
/// short timeout got only ~20/s, so short clicks and power presses were missed.
struct Reader {
    rx: mpsc::Receiver<Result<InputReport, String>>,
    stop: Arc<AtomicBool>,
}

impl Reader {
    fn spawn(dev: Arc<FreeDevice>) -> Self {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = move || {
            while !flag.load(Ordering::Relaxed) {
                let msg = match dev.read_input(Duration::from_millis(100)) {
                    Ok(None) => continue,
                    Ok(Some(r)) => Ok(r),
                    Err(e) => Err(e.to_string()),
                };
                let failed = msg.is_err();
                if tx.send(msg).is_err() || failed {
                    break;
                }
            }
        };
        if let Err(e) = std::thread::Builder::new()
            .name("hsm01-input".into())
            .spawn(worker)
        {
            eprintln!("hsm01-app: input thread: {e}");
        }
        Reader { rx, stop }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Stats {
    reports: u32,
    /// Reports skipped according to their sequence number.
    gaps: u32,
    last_seq: Option<u8>,
    read_time: Duration,
    send_time: Duration,
    send_max: Duration,
    frame_max: Duration,
    /// Knob counts received (per knob) and volume steps / clicks applied.
    turns: [i32; 4],
    steps: u32,
    clicks: u32,
    /// Notification popups opened.
    notices: u32,
}

/// Spectrum analyser state per (page, widget).
#[derive(Default)]
pub struct Spectra {
    states: HashMap<(usize, usize), SpectrumState>,
    pcm: Vec<f32>,
}

struct SpectrumState {
    analyzer: Analyzer,
    /// Column pixel states last sent to the device.
    sent: Vec<(i32, i32)>,
    last: Instant,
}

impl Spectra {
    /// Analyse the audio of every spectrum widget of `page` into `view.spectra`; returns their
    /// boxes (drawn with fills, left out of the tile diff).
    fn update(
        &mut self,
        audio: &mut dyn Audio,
        page: &Page,
        page_idx: usize,
        view: &mut View,
    ) -> Vec<(i32, i32, i32, i32)> {
        let mut boxes = Vec::new();
        for (i, w) in page.widgets.iter().enumerate() {
            let WidgetKind::Spectrum { target, bars, .. } = &w.kind else {
                continue;
            };
            let st = self
                .states
                .entry((page_idx, i))
                .or_insert_with(|| SpectrumState {
                    analyzer: Analyzer::default(),
                    sent: Vec::new(),
                    last: Instant::now(),
                });
            let dt = st.last.elapsed().as_secs_f32().min(0.1);
            st.last = Instant::now();
            let rate = audio.pcm(target, FFT_SIZE, &mut self.pcm);
            let levels = st
                .analyzer
                .update(rate.map(|r| (&self.pcm[..], r)), *bars as usize, dt);
            view.spectra.insert(i, levels);
            // inside its frame: the frame itself goes with the tiles
            let c = w.inner();
            boxes.push((c.x, c.y, c.w, c.h));
        }
        boxes
    }

    /// FILL commands for the spectrum columns that changed since the last frame sent.
    fn fills(
        &mut self,
        (page, page_idx): (&Page, usize),
        view: &View,
        (force, hole): (bool, Option<Rect>),
        lut: &Lut,
        out: &mut Vec<u8>,
    ) {
        let c565 = |c: Color| lut.color565(c);
        // nothing drawn in `hole` (sent as tiles)
        let fill = |r: Rect, c: u16, out: &mut Vec<u8>| {
            for (x, y, w, h) in minus(r, hole) {
                free::fill(x as u16, y as u16, w as u16, h as u16, c, out);
            }
        };
        for (i, w) in page.widgets.iter().enumerate() {
            let w = &w.inner(); // the spectrum is drawn inside its frame
            let WidgetKind::Spectrum {
                bars,
                gap,
                colors,
                peak,
                background,
                ..
            } = &w.kind
            else {
                continue;
            };
            let (Some(levels), Some(st)) =
                (view.spectra.get(&i), self.states.get_mut(&(page_idx, i)))
            else {
                continue;
            };
            let cols = bar_columns(w.x, w.w, *bars as usize, *gap);
            if force || st.sent.len() != cols.len() {
                fill((w.x, w.y, w.w, w.h), c565(*background), out);
                st.sent = vec![(0, -1); cols.len()]; // = empty column, already drawn by the fill
            }
            for (c, (&(x, bw), &(level, cap))) in cols.iter().zip(levels).enumerate() {
                let state = column_state(w.h, level, cap, peak.is_some());
                if st.sent[c] == state {
                    continue;
                }
                let old = std::mem::replace(&mut st.sent[c], state);
                for (y, h, col) in column_delta(
                    w.y,
                    w.h,
                    old,
                    state,
                    colors,
                    peak.unwrap_or(*background),
                    *background,
                ) {
                    fill((x, y, bw, h), c565(col), out);
                }
            }
        }
    }
}

/// Render `page` with live audio for ~1 s (no device) and save the last frame as PNG.
pub fn snapshot(
    cfg: &Config,
    page: usize,
    mut audio: Box<dyn Audio>,
    path: &str,
) -> Result<(), String> {
    let page_idx = page;
    let page = cfg.pages.get(page_idx).ok_or("no such page")?;
    let mut renderer = Renderer::new()?;
    let mut pm = Renderer::new_pixmap();
    let mut peaks = [0f32; 4];
    let mut spectra = Spectra::default();
    for _ in 0..60 {
        audio.begin_frame();
        let mut view = View {
            page_name: page.name.clone(),
            ..Default::default()
        };
        for (k, p) in peaks.iter_mut().enumerate() {
            let mut s = audio.state(knob_target(page, k as u8 + 1));
            *p = s.level.max(*p - PEAK_FALL * FRAME.as_secs_f32());
            s.peak_hold = *p;
            view.knobs[k] = s;
        }
        spectra.update(audio.as_mut(), page, page_idx, &mut view);
        renderer.render(page, &view, &mut pm);
        std::thread::sleep(FRAME);
    }
    pm.save_png(path).map_err(|e| format!("{path}: {e}"))
}

/// Runs until `shared.quit`; `make_audio` is called here, in the engine thread.
pub fn run(shared: Arc<Shared>, make_audio: impl FnOnce() -> Box<dyn Audio>) {
    let audio = make_audio();
    let mut renderer = match Renderer::new() {
        Ok(r) => r,
        Err(e) => return shared.set_message(tf!("rendering: {e}", e = e)),
    };
    let mut e = Engine::new(shared.clone(), audio);
    let mut covered_was = (false, None, false);
    let (mut api_seen, mut api_pages) = (u64::MAX, Vec::<ClientPage>::new());
    let mut last_client: Option<(u64, String)> = None;
    // WebSocket settings in force (started by main) and a change waiting to settle
    let mut ws_applied = shared.config.read().unwrap().api.clone();
    let mut ws_pending: Option<(crate::config::ApiSettings, Instant)> = None;
    let t0 = Instant::now();
    // animation clock of the wallpapers: advances only when the screen is up to date
    let (mut anim_time, mut screen_done, mut last_frame) = (0.0f32, true, Instant::now());
    let print_stats = std::env::var_os("HSM01_STATS").is_some();
    let mut last_page = usize::MAX;
    let mut pm = Renderer::new_pixmap();
    let mut px = vec![0u16; (SCREEN_W * SCREEN_H) as usize];
    let mut screen = Screen::default();
    let (mut retry_at, mut next) = (Instant::now(), Instant::now());
    let (mut stat_t, mut stat_frames, mut stat_bytes) = (Instant::now(), 0u32, 0usize);
    let (mut cfg_mtime, mut cfg_check) = (Config::mtime(), Instant::now());
    let mut prev_cfg = shared.config.read().unwrap().clone();
    let mut prev_overlay = Overlay::default();
    let mut lut = Lut::new(&prev_cfg.calibration);
    let mut inventory_t = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);

    while !shared.quit.load(Ordering::Relaxed) {
        if e.dev.is_none() && Instant::now() >= retry_at {
            retry_at = Instant::now() + Duration::from_secs(1);
            if e.connect() {
                screen.invalidate();
                // scrolling sent as COPY + what appeared (firmware 240.21+)
                screen.copy = shared.status.lock().unwrap().firmware.1 >= free::COPY_SINCE;
                e.force_direct = true;
            }
        }
        if inventory_t.elapsed() >= Duration::from_millis(500) {
            inventory_t = Instant::now();
            let inv = e.audio.inventory();
            let mut shown = shared.inventory.lock().unwrap();
            if *shown != inv {
                *shown = inv;
            }
        }
        if cfg_check.elapsed() >= Duration::from_secs(1) {
            cfg_check = Instant::now();
            let m = Config::mtime();
            if m != cfg_mtime {
                cfg_mtime = m;
                match Config::load() {
                    Ok(c) => {
                        if *shared.config.read().unwrap() != c {
                            crate::i18n::set_language(&c.language);
                            *shared.config.write().unwrap() = c;
                            shared.set_message(tr("configuration reloaded"));
                        }
                        shared.config_version.fetch_add(1, Ordering::Release);
                    }
                    Err(err) => shared.set_message(tf!("configuration not reloaded: {err}", err = err)),
                }
            }
        }
        let cfg = shared.config.read().unwrap().clone();
        if prev_cfg.brightness != cfg.brightness {
            e.user_brightness = None; // the configured brightness was changed: it applies again
        }
        if prev_cfg.calibration != cfg.calibration {
            lut = Lut::new(&cfg.calibration);
        }
        // the local WebSocket follows its settings once they stay the same for a moment (the
        // port is typed or dragged)
        if cfg.api != ws_applied {
            match &ws_pending {
                Some((s, t)) if *s == cfg.api && t.elapsed() >= Duration::from_millis(600) => {
                    crate::api::set_websocket(&shared, &cfg.api);
                    ws_applied = cfg.api.clone();
                    ws_pending = None;
                }
                Some((s, _)) if *s == cfg.api => {}
                _ => ws_pending = Some((cfg.api.clone(), Instant::now())),
            }
        }
        if prev_cfg != cfg {
            // reloaded or edited in the GUI: the spectrum columns are redrawn completely
            e.force_direct = true;
            e.last_activity = Instant::now();
            prev_cfg = cfg.clone();
        }
        // pages of the API clients come after the configuration's
        let show = shared.api.show.lock().unwrap().take();
        let find = |pages: &[ClientPage], (owner, id): &(u64, String)| {
            pages.iter().position(|p| p.owner == *owner && p.id == *id)
        };
        let v = shared.api.version.load(Ordering::Acquire);
        // a page shown just after its creation may be newer than the copy: copy again
        if v != api_seen || show.as_ref().is_some_and(|s| find(&api_pages, s).is_none()) {
            api_seen = v;
            api_pages = shared.api.pages();
        }
        let n_cfg = cfg.pages.len();
        e.n_cfg = n_cfg;
        e.n_pages = n_cfg + api_pages.len();
        if e.n_pages == 0 {
            shared.set_message(tr("no page in the configuration"));
            std::thread::sleep(FRAME);
            continue;
        }
        let now_at = shared.page.load(Ordering::Relaxed);
        if let Some(to) = follow_shown(now_at, (last_page, last_client.as_ref()), &api_pages, (n_cfg, e.last_cfg)) {
            if to < n_cfg {
                // its module left: the main page meanwhile, its page again if it comes back
                e.comeback = e.shown_tool.take().map(|(tool, page)| Comeback {
                    tool,
                    page,
                    since: Instant::now(),
                    at: to,
                    jump_from: e.jump_from.take(),
                });
            }
            shared.page.store(to, Ordering::Relaxed);
        }
        if let Some(to) = e.come_back(shared.page.load(Ordering::Relaxed), &api_pages, n_cfg) {
            shared.page.store(to, Ordering::Relaxed);
        }
        if let Some(i) = show.as_ref().and_then(|s| find(&api_pages, s)) {
            shared.page.store(n_cfg + i, Ordering::Relaxed);
        }
        let page_idx = shared.page.load(Ordering::Relaxed).min(e.n_pages - 1);
        let client = page_idx.checked_sub(n_cfg).map(|i| &api_pages[i]);
        let page = match client {
            Some(c) => &c.page,
            None => &cfg.pages[page_idx], // page_idx < n_cfg
        };
        let shown = client.map(|c| (c.owner, c.id.clone()));
        if client.is_none() {
            e.last_cfg = page_idx;
        }
        e.shown_tool = client.and_then(|c| Some((shared.api.client_name(c.owner)?, c.id.clone())));
        e.menu_tick(&api_pages, shown.as_ref());
        // `tool` widgets of a configured page: the tools' pages drawn inside, the inputs they take
        let embeds = match client {
            None => embedded(&shared.api, page, &api_pages),
            Some(_) => Vec::new(),
        };
        e.routes = routes(&embeds);

        if page_idx != last_page || shown != last_client {
            for (who, on) in [(&last_client, false), (&shown, true)] {
                if let Some((owner, id)) = who {
                    let msg = json!({"event": "page_shown", "page": id, "shown": on});
                    shared.api.send(*owner, &msg);
                }
            }
            last_client = shown;
            last_page = page_idx;
            e.force_direct = true;
            e.last_activity = Instant::now();
        }
        let frame_t0 = Instant::now();
        e.next_notice(&cfg);
        e.audio
            .set_fullscreen(shared.fullscreen.lock().unwrap().clone());
        e.audio.begin_frame();
        let t = Instant::now();
        // a module that keeps the device awake (« Empêche la veille ») while it is on screen (its
        // page shown, or its page / widget in the page shown): as if in use, the standby delay
        // starts again when it goes
        let mut on_screen: Vec<String> = e.shown_tool.iter().map(|(name, _)| name.clone()).collect();
        on_screen.extend(embeds.iter().filter_map(|em| shared.api.client_name(em.page.owner)));
        if kept_awake(&cfg, &on_screen) {
            e.last_activity = Instant::now();
        }
        let (_, was_asleep) = standby(&cfg, e.last_activity.elapsed());
        e.read_inputs(&cfg, (page, page_idx, client), was_asleep);
        e.check_holds(&cfg, page, page_idx);
        // actions asked by local programs (API `action`)
        let asked = std::mem::take(&mut *shared.api.actions.lock().unwrap());
        for a in asked {
            e.run_action(&a, &cfg, page, page_idx);
        }
        if std::mem::take(&mut e.redraw) {
            // the firmware's screen saver drew over our picture (we were silent for 3 s)
            screen.invalidate();
            e.force_direct = true;
            (e.brightness, e.leds) = (None, None);
        }
        e.stats.read_time += t.elapsed();
        let (level, asleep) = standby(&cfg, e.last_activity.elapsed());
        if shared.status.lock().unwrap().asleep != asleep {
            shared.status.lock().unwrap().asleep = asleep;
            shared.set_message(if asleep { tr("standby") } else { tr("awake") });
            shared
                .api
                .broadcast(&json!({"event": "standby", "asleep": asleep}));
        }
        // asleep: nothing is drawn or sent; the device keeps the last frame (backlight off), so
        // the tile diff stays valid for the wake-up
        let mut view = View::default();
        let mut spectrum_boxes = Vec::new();
        if e.confirm
            .as_ref()
            .is_some_and(|c| c.since.elapsed() > CONFIRM_TIMEOUT)
        {
            e.confirm = None;
        }
        let dialog = e.confirm.as_ref().map(|c| ConfirmView {
            name: c.name.clone(),
            yes: c.yes,
        });
        let overlay = shared.overlay.lock().unwrap().clone();
        let notice_rect = e.notice.as_ref().filter(|_| !overlay.pattern).map(|(n, _)| notice_box(n));
        let notice = notice_rect.is_some();
        let menu = e.menu.as_ref().map(ModuleMenu::view).filter(|_| !overlay.pattern);
        let covered = (dialog.is_some(), notice_rect, menu.is_some());
        if covered != covered_was {
            // the question / the popup cover spectrum areas (drawn by tiles meanwhile): full
            // redraw after
            e.force_direct = true;
            covered_was = covered;
        }
        // the spectra keep running around the popup and the menu: their tiles leave their fills
        let hole = union(
            notice_rect.map(tile_aligned),
            menu.as_ref().map(|_| tile_aligned(MENU_BOX)),
        );
        if overlay != prev_overlay {
            e.force_direct = true; // repaint the spectrum areas without the old outlines
            prev_overlay = overlay.clone();
        }
        // the window's outlines are those of its pages, not of the clients'
        let outlines = if overlay.pattern || client.is_some() {
            Vec::new()
        } else {
            outline_rects(page, &overlay)
        };
        if !asleep {
            view = e.view(page);
            view.embedded = embeds
                .iter()
                .map(|x| (x.index, (x.page.page.clone(), x.page.size.unwrap_or((SCREEN_W, SCREEN_H)))))
                .collect();
            view.decks = e.deck_views(page, page_idx);
            view.time = t0.elapsed().as_secs_f32();
            let dt = last_frame.elapsed().as_secs_f32().min(0.1);
            last_frame = Instant::now();
            if screen_done {
                anim_time += dt;
            }
            view.anim_time = anim_time;
            if dialog.is_none() && !overlay.pattern {
                spectrum_boxes = e
                    .spectra
                    .update(e.audio.as_mut(), page, page_idx, &mut view)
                    .into_iter()
                    .flat_map(|b| minus(b, hole))
                    .collect();
            }
            if overlay.pattern {
                test_pattern(&mut pm);
            } else {
                renderer.render(page, &view, &mut pm);
            }
            for &r in &outlines {
                fill_rect(&mut pm, r);
            }
            if let Some((n, since)) = e.notice.as_ref().filter(|_| notice) {
                let total = n.duration.unwrap_or(cfg.notifications.duration_secs);
                renderer.notice(&mut pm, n, (since.elapsed().as_secs_f32(), total));
            }
            if let Some(m) = &menu {
                renderer.module_menu(&mut pm, m);
            }
            if let Some(d) = &dialog {
                renderer.confirm(&mut pm, d);
            }
            to_rgb565(&pm, &mut px, &lut);
        }

        if let Some(dev) = &e.dev {
            let mut out = Vec::new();
            if !asleep {
                screen_done = screen.update(&px, &spectrum_boxes, &mut out, SEND_BUDGET).1;
                e.spectra
                    .fills((page, page_idx), &view, (e.force_direct, hole), &lut, &mut out);
                e.force_direct = false;
                // outlines over spectrum areas (left out of the tile diff): redrawn every frame
                // on top of the bar fills
                for &(x, y, w, h, c) in &outlines {
                    let over_spectrum = spectrum_boxes.iter().any(|&(bx, by, bw, bh)| {
                        x < bx + bw && bx < x + w && y < by + bh && by < y + h
                    });
                    if over_spectrum && let Some((x, y, w, h)) = clip_to_screen(x, y, w, h) {
                        free::fill(x, y, w, h, lut.color565(c), &mut out);
                    }
                }
            }
            let base = e.user_brightness.unwrap_or(cfg.brightness);
            let brightness = (base as f32 * level).round() as u8;
            if e.brightness != Some(brightness) {
                free::backlight(brightness, &mut out);
                e.brightness = Some(brightness);
            }
            let mut leds = match client {
                _ if asleep => [0; 4],
                Some(c) => c.leds,
                None => led_values(page, &view, &shared.api),
            };
            // an alert's LED blinks (2 Hz) while it is shown
            if !asleep
                && let Some(l) = e.notice.as_ref().and_then(|(n, _)| n.led)
            {
                let on = (t0.elapsed().as_millis() / 250).is_multiple_of(2);
                leds[(l as usize - 1).min(3)] = if on { LED_ON } else { 0 };
            }
            if e.leds != Some(leds) {
                free::leds(leds, &mut out);
                e.leds = Some(leds);
            }
            if !out.is_empty() {
                stat_bytes += out.len();
                let t = Instant::now();
                let r = dev.send(&out);
                let dt = t.elapsed();
                e.stats.send_time += dt;
                e.stats.send_max = e.stats.send_max.max(dt);
                if let Err(err) = r {
                    e.disconnect(tf!("send: {err}", err = err));
                }
            }
        }
        if !asleep {
            shared.preview.lock().unwrap().copy_from_slice(pm.data());
            shared.preview_frame.fetch_add(1, Ordering::Release);
        }

        stat_frames += 1;
        e.stats.frame_max = e.stats.frame_max.max(frame_t0.elapsed());
        let dt = stat_t.elapsed().as_secs_f32();
        if dt >= 1.0 {
            if print_stats {
                let st = std::mem::take(&mut e.stats);
                let ms = |d: Duration| d.as_secs_f32() * 1000.0;
                eprintln!(
                    "stats page {}: {:.0} fps, {:.0} KiB/s, reports {} (gaps {}), read {:.0} ms, send {:.0} ms (max {:.1}), frame max {:.1} ms",
                    page_idx + 1,
                    stat_frames as f32 / dt,
                    stat_bytes as f32 / 1024.0 / dt,
                    st.reports,
                    st.gaps,
                    ms(st.read_time),
                    ms(st.send_time),
                    ms(st.send_max),
                    ms(st.frame_max)
                );
                eprintln!(
                    "      inputs: knob counts {:?}, volume steps {}, clicks {}, notifications {}, confirm {}, grace {}, asleep {}",
                    st.turns,
                    st.steps,
                    st.clicks,
                    st.notices,
                    e.confirm.is_some(),
                    e.wake_grace.is_some(),
                    shared.status.lock().unwrap().asleep,
                );
                e.stats.last_seq = st.last_seq;
            }
            let mut s = shared.status.lock().unwrap();
            s.fps = stat_frames as f32 / dt;
            s.kib_per_s = stat_bytes as f32 / 1024.0 / dt;
            (stat_t, stat_frames, stat_bytes) = (Instant::now(), 0, 0);
        }
        next += FRAME;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now; // late: do not try to catch up
        }
    }
}

/// The app stops (or the engine panics): the device goes to standby – backlight and LEDs off –
/// until the next start (`connect` lights it again); nothing crosses the USB meanwhile.
impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(dev) = &self.dev {
            let mut out = Vec::new();
            free::backlight(0, &mut out);
            free::leds([0; 4], &mut out);
            let _ = dev.send(&out);
        }
    }
}

impl Engine {
    fn new(shared: Arc<Shared>, audio: Box<dyn Audio>) -> Self {
        Engine {
            shared,
            audio,
            dev: None,
            input: None,
            decoders: Default::default(),
            carry: [0; 4],
            buttons: 0,
            peaks: [0.0; 4],
            leds: None,
            spectra: Spectra::default(),
            force_direct: true,
            keys: None,
            brightness: None,
            user_brightness: None,
            stats: Stats::default(),
            last_activity: Instant::now(),
            wake_grace: None,
            confirm: None,
            pressed_at: [None; 9],
            hold_done: 0,
            menu_hold: 0,
            deck_armed: 0,
            shown_tool: None,
            comeback: None,
            boost_ok: Default::default(),
            notice: None,
            n_pages: 0,
            client_carry: [0; 4],
            client_pressed: [None; 9],
            client_held: 0,
            client_target: Default::default(),
            routes: HashMap::new(),
            jump_from: None,
            screen_epoch: None,
            redraw: false,
            decks: HashMap::new(),
            tool_carry: [0; 4],
            n_cfg: 0,
            last_cfg: 0,
            menu: None,
        }
    }

    fn connect(&mut self) -> bool {
        match FreeDevice::open() {
            Ok(dev) => {
                let brightness = self.shared.config.read().unwrap().brightness;
                let mut out = Vec::new();
                free::backlight(brightness, &mut out);
                if let Err(err) = dev.send(&out) {
                    self.shared.set_message(tf!("send: {err}", err = err));
                    return false;
                }
                let v = dev.version();
                {
                    let mut s = self.shared.status.lock().unwrap();
                    s.connected = true;
                    s.firmware = v;
                }
                self.shared
                    .set_message(tf!("connected, firmware {version}", version = format!("{}.{:02}", v.0, v.1)));
                self.decoders = Default::default();
                self.carry = [0; 4];
                self.leds = None;
                self.brightness = Some(brightness);
                self.screen_epoch = None;
                let dev = Arc::new(dev);
                self.input = Some(Reader::spawn(dev.clone()));
                self.dev = Some(dev);
                true
            }
            Err(hsm01_usb::Error::WrongFirmware(maj, min)) => {
                self.shared.set_message(tf!("the Stream 100 runs firmware {version}: the app needs the custom firmware (240.xx)", version = format!("{maj}.{min:02}")));
                false
            }
            Err(err) => {
                self.shared
                    .set_message(tf!("waiting for the Stream 100 ({err})", err = err));
                false
            }
        }
    }

    fn disconnect(&mut self, why: String) {
        self.dev = None;
        self.input = None;
        self.shared.status.lock().unwrap().connected = false;
        self.shared.set_message(tf!("disconnected: {why}", why = why));
    }

    /// Apply the pending input reports. Any knob turn or click is user activity; while `asleep`
    /// (and for `WAKE_GRACE` after) it only wakes the screen up.
    fn read_inputs(
        &mut self,
        cfg: &Config,
        (page, page_idx, client): (&Page, usize, Option<&ClientPage>),
        asleep: bool,
    ) {
        loop {
            let Some(input) = &self.input else { return };
            let report = match input.rx.try_recv() {
                Ok(Ok(r)) => r,
                Ok(Err(err)) => return self.disconnect(tf!("read: {err}", err = err)),
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return self.disconnect(tr("input thread stopped").into());
                }
            };
            self.stats.reports += 1;
            if let Some(prev) = self.stats.last_seq {
                self.stats.gaps += u32::from(report.seq.wrapping_sub(prev).wrapping_sub(1));
            }
            self.stats.last_seq = Some(report.seq);
            let epoch = Some(report.screen_epoch);
            if self.screen_epoch.is_some() && self.screen_epoch != epoch {
                self.redraw = true;
            }
            self.screen_epoch = epoch;
            let counts: [i32; 4] = std::array::from_fn(|k| {
                self.decoders[k].update(report.wipers[2 * k], report.wipers[2 * k + 1])
            });
            let mut pressed = report.buttons & !self.buttons;
            let released = self.buttons & !report.buttons;
            self.buttons = report.buttons;
            for (t, c) in self.stats.turns.iter_mut().zip(counts) {
                *t += c;
            }
            self.stats.clicks += pressed.count_ones();
            if counts.iter().any(|&c| c != 0) || pressed != 0 {
                let now = Instant::now();
                self.last_activity = now;
                if asleep && self.wake_grace.is_none() {
                    self.wake_grace = Some(now + WAKE_GRACE);
                }
            }
            if let Some(until) = self.wake_grace {
                if Instant::now() < until {
                    self.carry = [0; 4];
                    continue;
                }
                self.wake_grace = None;
            }
            // a click closes the notification popup, and does nothing else
            if pressed != 0 && self.confirm.is_none() && self.notice.is_some() {
                self.close_notice("dismissed");
                pressed = 0;
            }
            // the modules menu, while open, takes every input
            if self.menu.as_ref().is_some_and(|m| m.closing.is_none()) {
                self.menu_inputs(counts, pressed, page_idx);
                continue;
            }
            // `deck` widgets take their knob first (on a tool's page too), then the API: every
            // input reaches the clients as events; the client of the page shown, or the one that
            // grabbed the input, takes it (no configured binding runs for it)
            let raw = (pressed, released);
            let (counts, (pressed, released)) = if self.confirm.is_none() {
                self.deck_inputs(cfg, (page, page_idx, client), counts, (pressed, released))
            } else {
                (counts, (pressed, released))
            };
            let deck_taken = (raw.0 & !pressed) | (raw.1 & !released);
            let (knobs_taken, buttons_taken) =
                self.api_inputs(page, client, counts, (pressed, released));
            self.keep_menu_keys(cfg, (page, client), raw, buttons_taken | deck_taken);
            let counts: [i32; 4] =
                std::array::from_fn(|k| if knobs_taken[k] { 0 } else { counts[k] });
            let (pressed, released) = (pressed & !buttons_taken, released & !buttons_taken);
            if let Some(c) = &mut self.confirm {
                // only the question: nothing else happens until it is answered
                self.carry = [0; 4];
                c.turn += counts[c.knob];
                if c.turn.abs() >= CONFIRM_STEP {
                    c.yes = c.turn < 0; // left: « Oui », right: « Non »
                    c.turn = 0;
                    c.since = Instant::now();
                }
                if pressed & (1 << c.knob) != 0 {
                    if c.yes {
                        self.shared.set_message(tf!("{name}: volume above 100% allowed", name = c.name));
                        self.boost_ok.insert(c.target.clone());
                    }
                    self.confirm = None;
                } else if pressed != 0 {
                    self.confirm = None;
                }
                continue;
            }
            let counts = self.tool_turns(page, counts);
            for (k, &counts) in counts.iter().enumerate() {
                self.carry[k] += counts;
                let per = cfg.counts_per_percent.max(1) as i32;
                let pct = self.carry[k] / per;
                if pct != 0 {
                    self.carry[k] -= pct * per;
                    self.turn_knob(k, page, pct);
                }
            }
            // without a hold action, a click acts at once; with one, on a short press's release
            for bit in 0..9 {
                let m = 1u16 << bit;
                let (click, hold) = bindings(page, bit);
                if pressed & m != 0 {
                    if hold.is_none() {
                        self.run_action(&click, cfg, page, page_idx);
                    } else {
                        self.pressed_at[bit] = Some(Instant::now());
                        self.hold_done &= !m;
                    }
                }
                if released & m != 0 {
                    if self.pressed_at[bit].take().is_some() && self.hold_done & m == 0 {
                        self.run_action(&click, cfg, page, page_idx);
                    }
                    self.hold_done &= !m;
                }
            }
        }
    }

    /// `deck` widgets of the page: their knob moves their selection and its click runs the
    /// selected tile, instead of the knob's own bindings or the page's client (the first deck of
    /// a knob only; a knob grabbed by a program stays its). On a tool's page, the tool is told
    /// (`deck` event). Returns the counts and clicks left for the others.
    fn deck_inputs(
        &mut self,
        cfg: &Config,
        (page, page_idx, client): (&Page, usize, Option<&ClientPage>),
        mut counts: [i32; 4],
        (pressed, released): (u16, u16),
    ) -> ([i32; 4], (u16, u16)) {
        let grabs = self.shared.api.grabs.lock().unwrap().clone();
        // a knob whose hold opens the modules menu runs its tile on a short press's release
        let menu_keys = self.menu_keys(cfg, page, client);
        let (mut taken, mut run) = (0u16, Vec::new());
        for (index, w) in page.widgets.iter().enumerate() {
            let WidgetKind::Deck { knob, tiles, .. } = &w.kind else {
                continue;
            };
            let k = ((*knob).clamp(1, 4) - 1) as usize;
            let bit = 1u16 << k;
            if taken & bit != 0 || grabs.contains_key(&format!("knob{}", k + 1)) {
                continue;
            }
            taken |= bit;
            let fire = self.deck_fires(bit, menu_keys, (pressed, released));
            let st = self.decks.entry((page_idx, index)).or_default();
            st.carry += std::mem::take(&mut counts[k]);
            let steps = st.carry / DECK_STEP;
            st.carry -= steps * DECK_STEP;
            if steps != 0 && !tiles.is_empty() {
                st.step(steps, tiles.len());
            }
            if fire && !tiles.is_empty() {
                st.flash = Some(Instant::now());
                let i = st.selected % tiles.len();
                run.push(tiles[i].action.clone());
                if let Some((c, (id, _))) = client.and_then(|c| Some((c, c.widgets.get(index)?))) {
                    let ev = json!({"event": "deck", "page": c.id, "widget": id, "tile": i,
                                    "title": tiles[i].title});
                    self.shared.api.send(c.owner, &ev);
                }
            }
        }
        for a in run {
            self.run_action(&a, cfg, page, page_idx);
        }
        (counts, (pressed & !taken, released & !taken))
    }

    /// Whether the click of deck knob `bit` runs its tile now: at the press, or, when its hold
    /// opens the modules menu (`menu_keys`), at the release of a short press (a hold opens the
    /// menu, which then takes the release).
    fn deck_fires(&mut self, bit: u16, menu_keys: u16, (pressed, released): (u16, u16)) -> bool {
        if menu_keys & bit == 0 {
            return pressed & bit != 0;
        }
        let armed = released & bit != 0 && self.deck_armed & bit != 0;
        self.deck_armed = (self.deck_armed & !(bit & released)) | (bit & pressed);
        armed
    }

    /// The modules menu: its entries again (modules come and go), the selection on the module
    /// shown when it opened; closing by itself after `MENU_IDLE`, gone once slid out.
    fn menu_tick(&mut self, pages: &[ClientPage], shown: Option<&(u64, String)>) {
        let Some(m) = &mut self.menu else {
            return;
        };
        let first = m.entries.is_empty();
        m.entries = menu_entries(&self.shared.api, pages);
        if first && let Some((owner, _)) = shown {
            m.selected = m.entries.iter().position(|e| e.owner == *owner).unwrap_or(0);
        }
        m.selected = m.selected.min(m.entries.len().saturating_sub(1));
        match m.closing {
            Some(t) if t.elapsed() >= MENU_CLOSE => self.menu = None,
            None if m.used.elapsed() >= MENU_IDLE => m.closing = Some(Instant::now()),
            _ => {}
        }
    }

    /// While the modules menu is open: knob 1 turns choose, its click opens the chosen module's
    /// page, any other click closes the menu.
    fn menu_inputs(&mut self, counts: [i32; 4], pressed: u16, page_idx: usize) {
        let Some(m) = &mut self.menu else {
            return;
        };
        if counts.iter().any(|&c| c != 0) || pressed != 0 {
            m.used = Instant::now();
        }
        m.carry += counts[0];
        let steps = m.carry / DECK_STEP;
        m.carry -= steps * DECK_STEP;
        let n = m.entries.len() as i64;
        if steps != 0 && n > 0 {
            m.selected = (m.selected as i64 + steps as i64).clamp(0, n - 1) as usize;
        }
        if pressed == 0 {
            return;
        }
        let chosen = (pressed & 1 != 0)
            .then(|| m.entries.get(m.selected))
            .flatten()
            .map(|e| (e.owner, e.page.clone()));
        m.closing = Some(Instant::now());
        if let Some(show) = chosen {
            self.jump_from = Some(page_idx);
            *self.shared.api.show.lock().unwrap() = Some(show);
        }
    }

    /// Knobs whose target is a tool's command: their turns go to the tool (`command` events,
    /// steps of `CLIENT_STEP` counts) instead of a volume. Returns the counts left for the audio.
    fn tool_turns(&mut self, page: &Page, mut counts: [i32; 4]) -> [i32; 4] {
        for (k, c) in counts.iter_mut().enumerate() {
            let Target::Tool { tool, command } = knob_target(page, k as u8 + 1) else {
                self.tool_carry[k] = 0;
                continue;
            };
            self.tool_carry[k] += std::mem::take(c);
            let steps = self.tool_carry[k] / CLIENT_STEP;
            if steps != 0 {
                self.tool_carry[k] -= steps * CLIENT_STEP;
                if !self.shared.api.run_command(tool, command, Some(steps)) {
                    self.shared.set_message(tf!("module \"{tool}\": not running", tool = tool));
                }
            }
        }
        counts
    }

    /// What the `deck` widgets of `page` show now.
    fn deck_views(&self, page: &Page, page_idx: usize) -> HashMap<usize, crate::state::DeckView> {
        let view = |(i, w): (usize, &crate::config::Widget)| {
            let WidgetKind::Deck { tiles, .. } = &w.kind else {
                return None;
            };
            let st = self.decks.get(&(page_idx, i));
            Some((i, st.map_or_else(Default::default, |s| s.view(tiles.len()))))
        };
        page.widgets.iter().enumerate().filter_map(view).collect()
    }

    /// Input events for the API clients (`turn` in steps of `CLIENT_STEP` counts, `press`,
    /// `release`, `click`; `hold` from `check_holds`), sent to every client: `mine` is true for
    /// the one that takes the input – it grabbed it, or its page is shown (power only if the page
    /// grabs it), or a `tool` widget of the page shown gives it the input (`routes`; `page` then
    /// names the tool's page). Returns the knobs and the buttons (report bits) taken.
    fn api_inputs(
        &mut self,
        page: &Page,
        client: Option<&ClientPage>,
        counts: [i32; 4],
        (pressed, released): (u16, u16),
    ) -> ([bool; 4], u16) {
        let shared = self.shared.clone();
        let grabs = shared.api.grabs.lock().unwrap().clone();
        let routes = self.routes.clone();
        let label = client.map_or(page.name.as_str(), |c| c.id.as_str());
        // who takes an input, and the page its events name
        let taker = |name: &str, power: bool| -> (Option<u64>, String) {
            if let Some(&id) = grabs.get(name) {
                return (Some(id), label.to_string());
            }
            match (client, routes.get(name)) {
                (Some(c), _) => ((!power || c.grab_power).then_some(c.owner), label.to_string()),
                (None, Some((owner, id))) => (Some(*owner), id.clone()),
                (None, None) => (None, label.to_string()),
            }
        };
        let emit = |owner: Option<u64>, at: &str, mut msg: Value| {
            msg["page"] = json!(at);
            shared.api.emit(owner, msg);
        };
        let mut knobs = [false; 4];
        for (k, taken) in knobs.iter_mut().enumerate() {
            let (owner, at) = taker(&format!("knob{}", k + 1), false);
            *taken = owner.is_some();
            self.client_carry[k] += counts[k];
            let steps = self.client_carry[k] / CLIENT_STEP;
            if steps != 0 {
                self.client_carry[k] -= steps * CLIENT_STEP;
                emit(owner, &at, json!({"event": "turn", "knob": k + 1, "delta": steps}));
            }
        }
        let mut buttons = 0u16;
        for bit in 0..9 {
            let m = 1u16 << bit;
            let name = crate::api::button_name(bit);
            if pressed & m != 0 {
                let (owner, at) = taker(&name, bit == 8);
                self.client_pressed[bit] = Some(Instant::now());
                self.client_held &= !m;
                emit(owner, &at, json!({"event": "press", "button": name}));
                if owner.is_some() {
                    buttons |= m;
                }
                self.client_target[bit] = Some((owner, at));
            }
            if released & m != 0
                && let Some((owner, at)) = self.client_target[bit].take()
            {
                self.client_pressed[bit] = None;
                emit(owner, &at, json!({"event": "release", "button": name}));
                if self.client_held & m == 0 {
                    emit(owner, &at, json!({"event": "click", "button": name}));
                }
                self.client_held &= !m;
                if owner.is_some() {
                    buttons |= m;
                }
            }
        }
        (knobs, buttons)
    }

    /// Close the popup; an API client's alert tells its client why.
    fn close_notice(&mut self, reason: &str) {
        if let Some((n, _)) = self.notice.take()
            && let Some((client, id)) = n.origin
        {
            let msg = json!({"event": "alert_closed", "id": id, "reason": reason});
            self.shared.api.send(client, &msg);
        }
    }

    /// Notification popup: closed after its time, the next one once none is shown; it may wake
    /// the screen up (otherwise one arriving during standby is dropped). Desktop notifications
    /// only while `[notifications]` is enabled; API alerts always.
    fn next_notice(&mut self, cfg: &Config) {
        let n = &cfg.notifications;
        let expired = |(x, t): &(Notification, Instant)| {
            t.elapsed().as_secs_f32() >= x.duration.unwrap_or(n.duration_secs)
        };
        if !n.enabled && self.notice.as_ref().is_some_and(|(x, _)| x.desktop) {
            self.close_notice("disabled");
        }
        if self.notice.as_ref().is_some_and(expired) {
            self.close_notice("timeout");
        }
        // one per application: a newer one of the application shown takes its place at once
        let src = self.notice.as_ref().and_then(|(x, _)| x.source());
        if let Some(src) = src {
            let newer = {
                let mut q = self.shared.notifications.lock().unwrap();
                let at = q.iter().position(|x| x.source().as_ref() == Some(&src));
                at.and_then(|i| q.remove(i))
            };
            if let Some(next) = newer {
                self.close_notice("replaced");
                self.notice = Some((next, Instant::now()));
                return;
            }
        }
        if self.notice.is_some() {
            return;
        }
        let Some(next) = self.shared.notifications.lock().unwrap().pop_front() else {
            return;
        };
        if next.desktop && !n.enabled {
            return;
        }
        let asleep = standby(cfg, self.last_activity.elapsed()).1;
        let wake = next.wake.unwrap_or(n.wake);
        if wake {
            self.last_activity = Instant::now();
        }
        self.notice = Some((next, Instant::now()));
        if wake || !asleep {
            self.stats.notices += 1;
        } else {
            self.close_notice("standby"); // not shown
        }
    }

    /// The page index to show when the module of the page left (`comeback`) is back with that
    /// page, if the page shown (`now_at`) is still the one fallen back to and it is not too late.
    fn come_back(&mut self, now_at: usize, api_pages: &[ClientPage], n_cfg: usize) -> Option<usize> {
        let c = self.comeback.as_ref()?;
        if now_at != c.at || c.since.elapsed() > COMEBACK {
            self.comeback = None; // something else chosen meanwhile, or too late
            return None;
        }
        let (owner, id) = self.shared.api.page_of_tool(&c.tool, &c.page)?;
        let i = api_pages.iter().position(|p| p.owner == owner && p.id == id)?;
        self.jump_from = self.comeback.take()?.jump_from;
        Some(n_cfg + i)
    }

    /// Buttons (report bits) whose hold opens the modules menu on the configured page (the last
    /// one shown, under a module's page), unless a program grabbed that input for itself: the
    /// menu stays reachable from a module's page, or with a `module` widget taking the input.
    fn menu_keys(&self, cfg: &Config, page: &Page, client: Option<&ClientPage>) -> u16 {
        let base = match client {
            Some(_) => cfg.pages.get(self.last_cfg),
            None => Some(page),
        };
        let Some(base) = base else { return 0 };
        let grabs = self.shared.api.grabs.lock().unwrap();
        (0..9)
            .filter(|&bit| bindings(base, bit).1 == Action::ModuleMenu)
            .filter(|&bit| !grabs.contains_key(&crate::api::button_name(bit)))
            .fold(0, |keys, bit| keys | 1 << bit)
    }

    /// The modules menu's buttons (`menu_keys`) that a program takes: their press is timed for
    /// the hold (`check_holds`), their click stays the program's.
    fn keep_menu_keys(
        &mut self,
        cfg: &Config,
        (page, client): (&Page, Option<&ClientPage>),
        (pressed, released): (u16, u16),
        taken: u16,
    ) {
        if (pressed | released) == 0 {
            return;
        }
        let keys = self.menu_keys(cfg, page, client) & taken;
        for bit in 0..9 {
            let m = 1u16 << bit;
            if pressed & m != 0 {
                self.menu_hold = (self.menu_hold & !m) | (keys & m);
                if keys & m != 0 {
                    self.pressed_at[bit] = Some(Instant::now());
                    self.hold_done &= !m;
                }
            }
            if released & m != 0 && self.menu_hold & m != 0 {
                self.pressed_at[bit] = None;
                self.menu_hold &= !m;
            }
        }
    }

    /// Hold actions of the buttons pressed for `HOLD_MS` (once per press), and the API's `hold`
    /// events (every page).
    fn check_holds(&mut self, cfg: &Config, page: &Page, page_idx: usize) {
        let hold = Duration::from_millis(HOLD_MS);
        for bit in 0..9 {
            let held = self.client_pressed[bit].is_some_and(|t| t.elapsed() >= hold);
            if held && self.client_held & (1 << bit) == 0 {
                self.client_held |= 1 << bit;
                if let Some((owner, at)) = &self.client_target[bit] {
                    let name = crate::api::button_name(bit);
                    let msg = json!({"event": "hold", "page": at, "button": name});
                    self.shared.api.emit(*owner, msg);
                }
            }
        }
        for bit in 0..9 {
            let held =
                self.pressed_at[bit].is_some_and(|t| t.elapsed() >= Duration::from_millis(HOLD_MS));
            if held && self.hold_done & (1 << bit) == 0 {
                self.hold_done |= 1 << bit;
                let hold = if self.menu_hold & (1 << bit) != 0 {
                    Action::ModuleMenu
                } else {
                    bindings(page, bit).1
                };
                self.run_action(&hold, cfg, page, page_idx);
            }
        }
    }

    /// Knob `k` turned by `pct` volume percent. Going above 100 % needs a confirmation on the
    /// device first (the volume stops at 100 % until then).
    fn turn_knob(&mut self, k: usize, page: &Page, pct: i32) {
        let target = knob_target(page, k as u8 + 1).clone();
        let mut delta = pct as f32 / 100.0;
        if delta > 0.0 && !self.boost_ok.contains(&target) {
            let now = self.audio.state(&target);
            if !now.present {
                return;
            }
            if now.volume >= FULL_VOLUME {
                self.confirm = Some(BoostConfirm {
                    knob: k,
                    target,
                    name: now.name,
                    yes: false,
                    turn: 0,
                    since: Instant::now(),
                });
                self.carry = [0; 4];
                return;
            }
            delta = delta.min(1.0 - now.volume);
        }
        self.audio.change_volume(&target, delta);
        self.stats.steps += 1;
        if delta < 0.0
            && self.boost_ok.contains(&target)
            && self.audio.state(&target).volume < FULL_VOLUME
        {
            self.boost_ok.remove(&target); // back under 100 %: ask again next time
        }
    }

    fn run_action(&mut self, a: &Action, cfg: &Config, page: &Page, page_idx: usize) {
        let n = self.n_pages.max(1);
        // the configuration's pages: the ones next / previous / go to / back cycle through
        let n_cfg = self.n_cfg.clamp(1, n);
        let from = if page_idx < n_cfg { page_idx } else { self.last_cfg.min(n_cfg - 1) };
        match a {
            Action::None => {}
            Action::ModuleMenu => {
                if self.menu.as_ref().is_some_and(|m| m.closing.is_none()) {
                    self.menu.as_mut().unwrap().closing = Some(Instant::now());
                } else {
                    self.menu = Some(ModuleMenu::new());
                    // the press that opened it (a hold) must not act once released
                    self.pressed_at = [None; 9];
                    self.deck_armed = 0;
                }
            }
            Action::Mute { knob } => {
                let target = knob_target(page, *knob).clone();
                self.audio.toggle_mute(&target);
            }
            Action::Command { cmd } => spawn_detached("sh", &["-c", cmd]),
            Action::Url { url } => spawn_detached("xdg-open", &[url]),
            Action::Keys { keys } => {
                let shared = self.shared.clone();
                let kb = self
                    .keys
                    .get_or_insert_with(|| Keys::spawn(move |e| shared.set_message(e)));
                if let Err(e) = kb.press(keys) {
                    self.shared.set_message(e);
                }
            }
            Action::NextPage => self
                .shared
                .page
                .store((from + 1) % n_cfg, Ordering::Relaxed),
            Action::PrevPage => self
                .shared
                .page
                .store((from + n_cfg - 1) % n_cfg, Ordering::Relaxed),
            Action::Page { index } => {
                self.jump_from = Some(page_idx);
                self.shared
                    .page
                    .store(index.saturating_sub(1).min(n_cfg - 1), Ordering::Relaxed);
            }
            Action::Tool { name } => match self.shared.api.page_of_tool(name, "") {
                Some(shown) => {
                    self.jump_from = Some(page_idx);
                    *self.shared.api.show.lock().unwrap() = Some(shown);
                }
                None => self.shared.set_message(tf!("module \"{name}\": no page (stopped, or still starting?)", name = name)),
            },
            Action::ToolCommand { tool, command } => {
                if !self.shared.api.run_command(tool, command, None) {
                    self.shared.set_message(tf!("module \"{tool}\": command \"{command}\" unavailable (module stopped?)", tool = tool, command = command));
                }
            }
            Action::Media { action } => {
                // a D-Bus call (up to 2 s): out of the engine's thread
                let (shared, req) = (self.shared.clone(), json!({ "action": action }));
                std::thread::spawn(move || {
                    if let Err(e) = crate::media::request(&shared, &req) {
                        shared.set_message(tf!("media: {e}", e = e));
                    }
                });
            }
            // back where the jump came from; else a module's page goes back to the last page
            // of the configuration, one of these to the next one
            Action::Back => match self.jump_from.take() {
                Some(j) if j != page_idx => self.shared.page.store(j.min(n - 1), Ordering::Relaxed),
                _ if page_idx >= n_cfg => self.shared.page.store(from, Ordering::Relaxed),
                _ => self.shared.page.store((from + 1) % n_cfg, Ordering::Relaxed),
            },
            Action::BrightnessCycle { levels } => {
                let now = self.user_brightness.unwrap_or(cfg.brightness);
                let closest = levels
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, l)| (**l as i32 - now as i32).abs())
                    .map(|(i, _)| i);
                if let Some(i) = closest {
                    self.set_brightness(levels[(i + 1) % levels.len()], 0);
                }
            }
            Action::BrightnessSet { level } => self.set_brightness(*level, 0),
            Action::BrightnessUp { step } => {
                let now = self.user_brightness.unwrap_or(cfg.brightness);
                self.set_brightness(now.saturating_add(*step), MIN_BRIGHTNESS);
            }
            Action::BrightnessDown { step } => {
                let now = self.user_brightness.unwrap_or(cfg.brightness);
                self.set_brightness(now.saturating_sub(*step), MIN_BRIGHTNESS);
            }
        }
    }

    /// Backlight chosen with an action, at least `min` percent.
    fn set_brightness(&mut self, pct: u8, min: u8) {
        let pct = pct.clamp(min, 100);
        self.user_brightness = Some(pct);
        self.shared.set_message(tf!("brightness {pct} %", pct = pct));
    }

    fn view(&mut self, page: &Page) -> View {
        let mut view = View {
            page_name: page.name.clone(),
            ..Default::default()
        };
        for k in 0..4 {
            let target = knob_target(page, k as u8 + 1);
            if let Target::Tool { tool, command } = target {
                view.knobs[k] = tool_state(&self.shared.api, tool, command);
                continue;
            }
            let mut s = self.audio.state(target);
            self.peaks[k] = s.level.max(self.peaks[k] - PEAK_FALL * FRAME.as_secs_f32());
            s.peak_hold = self.peaks[k];
            view.knobs[k] = s;
        }
        view
    }
}

/// (x, y, w, h)
type Rect = (i32, i32, i32, i32);

/// Smallest box around both (either may be missing).
fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some((ax, ay, aw, ah)), Some((bx, by, bw, bh))) => {
            let (x, y) = (ax.min(bx), ay.min(by));
            Some((x, y, (ax + aw).max(bx + bw) - x, (ay + ah).max(by + bh) - y))
        }
        (a, b) => a.or(b),
    }
}

/// Parts of `r` outside `hole` (at most 4 rectangles).
fn minus(r: Rect, hole: Option<Rect>) -> Vec<Rect> {
    let (x, y, w, h) = r;
    let Some((hx, hy, hw, hh)) = hole else {
        return vec![r];
    };
    let (x1, y1, hx1, hy1) = (x + w, y + h, hx + hw, hy + hh);
    if hx >= x1 || hx1 <= x || hy >= y1 || hy1 <= y {
        return vec![r];
    }
    let mut out = Vec::new();
    if hy > y {
        out.push((x, y, w, hy - y));
    }
    if hy1 < y1 {
        out.push((x, hy1, w, y1 - hy1));
    }
    let (my0, my1) = (y.max(hy), y1.min(hy1));
    if hx > x {
        out.push((x, my0, hx - x, my1 - my0));
    }
    if hx1 < x1 {
        out.push((hx1, my0, x1 - hx1, my1 - my0));
    }
    out
}

/// Smallest box of whole tiles around `r`.
fn tile_aligned((x, y, w, h): Rect) -> Rect {
    let t = crate::screen::TILE as i32;
    let (x0, y0) = (x.div_euclid(t) * t, y.div_euclid(t) * t);
    let (x1, y1) = ((x + w + t - 1).div_euclid(t) * t, (y + h + t - 1).div_euclid(t) * t);
    (x0, y0, x1 - x0, y1 - y0)
}

/// Part of a rectangle on the screen, as FILL coordinates.
fn clip_to_screen(x: i32, y: i32, w: i32, h: i32) -> Option<(u16, u16, u16, u16)> {
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + w).min(SCREEN_W), (y + h).min(SCREEN_H));
    (x1 > x0 && y1 > y0).then(|| (x0 as u16, y0 as u16, (x1 - x0) as u16, (y1 - y0) as u16))
}

/// Whether a module of the config that keeps the device awake (`keep_awake`) is on screen
/// (`on_screen`: the names of the modules shown).
fn kept_awake(cfg: &Config, on_screen: &[String]) -> bool {
    cfg.tools.iter().any(|t| t.enabled && t.keep_awake && on_screen.contains(&t.name))
}

/// Backlight factor (1 → 0 during the fade) and whether the screen sleeps, after `idle` without
/// user action.
fn standby(cfg: &Config, idle: Duration) -> (f32, bool) {
    let s = &cfg.standby;
    if !s.enabled {
        return (1.0, false);
    }
    let t = idle.as_secs_f32() - s.after_secs.max(STANDBY_MIN_SECS) as f32;
    if t <= 0.0 {
        (1.0, false)
    } else if t < s.fade_secs {
        // the eye sees brightness about as duty^(1/2.2): fade the perceived brightness linearly
        ((1.0 - t / s.fade_secs).powf(2.2), false)
    } else {
        (0.0, true)
    }
}

/// A module page left because its module went away (`follow_shown`): its module and page, when,
/// the configured page fallen back to (`at`) and where power went back from there.
struct Comeback {
    tool: String,
    page: String,
    since: Instant,
    at: usize,
    jump_from: Option<usize>,
}

/// How long a restarting module has to come back with its page to be shown again.
const COMEBACK: Duration = Duration::from_secs(20);

/// The page to show instead of `now_at` when the module page shown last frame (`last`: its
/// index, its owner and id) moved in the list of the clients' pages (another module left or
/// came), or is gone (its module stopped): back to the configured page it was opened from
/// (`last_cfg`). None: nothing to change (a page change asked meanwhile wins).
fn follow_shown(
    now_at: usize,
    (last_page, last_client): (usize, Option<&(u64, String)>),
    api_pages: &[ClientPage],
    (n_cfg, last_cfg): (usize, usize),
) -> Option<usize> {
    let (owner, id) = last_client?;
    if now_at != last_page {
        return None;
    }
    let to = match api_pages.iter().position(|p| p.owner == *owner && p.id == *id) {
        Some(i) => n_cfg + i,
        None => last_cfg.min(n_cfg.saturating_sub(1)),
    };
    (to != now_at).then_some(to)
}

/// (click, hold) actions of report bit `bit`: knob clicks 0-3, buttons 4-7, power 8.
fn bindings(page: &Page, bit: usize) -> (Action, Action) {
    match bit {
        0..=3 => page
            .knobs
            .get(bit)
            .map(|k| (k.click.clone(), k.hold.clone())),
        4..=7 => page
            .buttons
            .get(bit - 4)
            .map(|b| (b.action.clone(), b.hold.clone())),
        _ => Some((page.power.clone(), page.power_hold.clone())),
    }
    .unwrap_or_default()
}

static NO_TARGET: Target = Target::None;

fn knob_target(page: &Page, knob: u8) -> &Target {
    page.knobs
        .get(knob.saturating_sub(1) as usize)
        .map_or(&NO_TARGET, |k| &k.target)
}

/// What the widgets of a knob aimed at a tool's command show: the tool's text (else the command's
/// name) as `{name}`, its value as the volume, greyed while not active; empty while the tool
/// does not run.
fn tool_state(api: &crate::api::Hub, tool: &str, command: &str) -> crate::state::TargetState {
    match api.command_state(tool, command) {
        Some((c, st)) => crate::state::TargetState {
            name: st.text.unwrap_or(c.name),
            present: true,
            volume: st.value.unwrap_or(0.0),
            muted: st.active == Some(false),
            ..Default::default()
        },
        None => Default::default(),
    }
}

fn led_values(page: &Page, view: &View, api: &crate::api::Hub) -> [u8; 4] {
    let muted = |k: u8| {
        view.knobs
            .get(k.saturating_sub(1) as usize)
            .is_some_and(|t| t.present && t.muted)
    };
    let mut v = [0u8; 4];
    for (i, b) in page.buttons.iter().take(4).enumerate() {
        let active = |tool: &str, command: &str| {
            api.command_state(tool, command).is_some_and(|(_, s)| s.active == Some(true))
        };
        v[i] = match &b.led {
            Led::Off => 0,
            Led::On => LED_ON,
            Led::Muted { knob } if muted(*knob) => LED_ON,
            Led::Unmuted { knob } if !muted(*knob) => LED_ON,
            Led::Tool { tool, command } if active(tool, command) => LED_ON,
            _ => 0,
        };
    }
    v
}

/// Start a program without blocking the engine; a helper thread reaps it (no zombie).
fn spawn_detached(prog: &str, args: &[&str]) {
    match std::process::Command::new(prog).args(args).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("hsm01-app: {prog}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The events a client received, as JSON.
    fn events(rx: &mpsc::Receiver<String>) -> Vec<Value> {
        rx.try_iter()
            .map(|l| serde_json::from_str(&l).unwrap())
            .collect()
    }

    /// « Afficher un module » shows the page of the module's client; « retour » (power on a
    /// module's page) goes back where the jump came from, else to the last page of the
    /// configuration; next / previous / back on these cycle through them only.
    #[test]
    fn module_pages_out_of_the_page_cycle() {
        let shared = Arc::new(Shared::new(Config::default()));
        let (id, _tx, _rx) = crate::api::register(&shared);
        crate::api::process(&shared, id, r#"{"type": "hello", "name": "Lecteur média"}"#);
        crate::api::process(&shared, id, r#"{"type": "page", "page": "media"}"#);
        let mut e = Engine::new(shared.clone(), crate::audio::open(true));
        let mut cfg = Config::default();
        cfg.pages.push(cfg.pages[0].clone()); // 2 pages, then the module's (index 2)
        (e.n_cfg, e.n_pages) = (2, 3);
        let page = cfg.pages[0].clone();
        let at = |e: &Engine| e.shared.page.load(Ordering::Relaxed);
        e.run_action(&Action::Tool { name: "Lecteur média".into() }, &cfg, &page, 1);
        assert_eq!(shared.api.show.lock().unwrap().clone(), Some((id, "media".into())));
        let module_page = &shared.api.pages()[0].page;
        assert_eq!(module_page.power, Action::Back, "power on a module's page goes back");
        e.run_action(&Action::Back, &cfg, module_page, 2);
        assert_eq!(at(&e), 1, "back to the page of the jump");
        e.last_cfg = 1;
        e.run_action(&Action::Back, &cfg, module_page, 2);
        assert_eq!(at(&e), 1, "no jump: the last page of the configuration");
        e.run_action(&Action::NextPage, &cfg, &page, 1);
        assert_eq!(at(&e), 0, "page 2 → page 1: never the module's");
        e.run_action(&Action::PrevPage, &cfg, &page, 0);
        assert_eq!(at(&e), 1);
        e.run_action(&Action::Back, &cfg, &page, 1);
        assert_eq!(at(&e), 0, "no jump on a page of the configuration: the next one");
        e.run_action(&Action::Page { index: 9 }, &cfg, &page, 0);
        assert_eq!(at(&e), 1, "« aller à la page »: those of the configuration");
        e.run_action(&Action::Tool { name: "nope".into() }, &cfg, &page, 1);
        assert!(shared.status.lock().unwrap().message.contains("nope"));
        assert_eq!(at(&e), 1);
    }

    /// The modules menu (knob 1 held by default): opens with the modules that have a page,
    /// knob 1 turns choose, its click shows the module's page (back: where it was opened), any
    /// other click just closes it; it slides in and out.
    #[test]
    fn module_menu() {
        let shared = Arc::new(Shared::new(Config::default()));
        for (name, page) in [("Système", "system"), ("Lecteur média", "media")] {
            let (id, _tx, _rx) = crate::api::register(&shared);
            std::mem::forget(_tx); // keep the client registered
            crate::api::process(&shared, id, &format!(r#"{{"type": "hello", "name": "{name}"}}"#));
            crate::api::process(&shared, id, &format!(r#"{{"type": "page", "page": "{page}"}}"#));
        }
        let cfg = Config::default();
        assert_eq!(cfg.pages[0].knobs[0].hold, Action::ModuleMenu, "knob 1 held, by default");
        let mut e = Engine::new(shared.clone(), crate::audio::open(true));
        (e.n_cfg, e.n_pages) = (1, 3);
        let page = cfg.pages[0].clone();
        let pages = shared.api.pages();
        e.run_action(&Action::ModuleMenu, &cfg, &page, 0);
        e.menu_tick(&pages, None);
        let v = e.menu.as_ref().unwrap().view();
        let names: Vec<&str> = v.items.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Lecteur média", "Système"], "by name");
        assert!(v.shown < 0.5, "sliding in");
        e.menu_inputs([0, 0, 0, 0], 0, 0);
        e.menu_inputs([DECK_STEP, 0, 0, 5 * DECK_STEP], 0, 0);
        assert_eq!(e.menu.as_ref().unwrap().selected, 1, "knob 1 only");
        e.menu_inputs([5 * DECK_STEP, 0, 0, 0], 0, 0);
        assert_eq!(e.menu.as_ref().unwrap().selected, 1, "a list: stops at the end");
        e.menu_inputs([0; 4], 1, 0); // knob 1 click
        let shown = shared.api.show.lock().unwrap().clone().map(|(_, p)| p);
        assert_eq!(shown.as_deref(), Some("system"));
        assert_eq!(e.jump_from, Some(0), "back: here");
        assert!(e.menu.as_ref().unwrap().closing.is_some(), "slides out");
        // opened again, a button closes it without showing anything
        *shared.api.show.lock().unwrap() = None;
        e.menu = None;
        e.run_action(&Action::ModuleMenu, &cfg, &page, 0);
        e.menu_inputs([0; 4], 1 << 4, 0);
        assert!(e.menu.as_ref().unwrap().closing.is_some());
        assert_eq!(*shared.api.show.lock().unwrap(), None);
    }

    /// Every input reaches every API client; the client of the page shown, or the one that
    /// grabbed an input, takes it (`mine`, and the configured bindings do not get it).
    #[test]
    fn inputs_reach_the_api_clients() {
        let shared = Arc::new(Shared::new(Config::default()));
        let (me, _tx, rx) = crate::api::register(&shared);
        let (_other, _tx2, rx_other) = crate::api::register(&shared);
        let mut e = Engine::new(shared.clone(), crate::audio::open(true));
        let page = Config::default().pages[0].clone();

        // a page of the configuration: seen by everyone, taken by nobody
        let (knobs, buttons) = e.api_inputs(&page, None, [0, 2 * CLIENT_STEP, 0, 0], (1 << 4, 0));
        assert_eq!((knobs, buttons), ([false; 4], 0));
        let ev = events(&rx);
        assert_eq!(ev.len(), 2, "{ev:?}");
        assert_eq!(ev[0]["event"], "turn");
        assert_eq!((ev[0]["knob"].as_u64(), ev[0]["delta"].as_i64()), (Some(2), Some(2)));
        assert_eq!((ev[0]["mine"].as_bool(), ev[0]["page"].as_str()), (Some(false), Some(page.name.as_str())));
        assert_eq!((ev[1]["event"].as_str(), ev[1]["button"].as_str()), (Some("press"), Some("button1")));
        assert_eq!(events(&rx_other).len(), 2, "the other client sees them too");
        // release of that press: release + click, still nobody's
        let (_, buttons) = e.api_inputs(&page, None, [0; 4], (0, 1 << 4));
        assert_eq!(buttons, 0);
        let ev = events(&rx);
        assert_eq!((ev[0]["event"].as_str(), ev[1]["event"].as_str()), (Some("release"), Some("click")));

        // a grabbed knob is taken on any page
        shared.api.grabs.lock().unwrap().insert("knob2".into(), me);
        let (knobs, _) = e.api_inputs(&page, None, [0, CLIENT_STEP, 0, 0], (0, 0));
        assert!(knobs[1]);
        assert_eq!(events(&rx)[0]["mine"], true);
        assert_eq!(events(&rx_other)[..].last().unwrap()["mine"], false);
        shared.api.grabs.lock().unwrap().clear();

        // the client's page: its buttons and knobs are its own, power stays the app's
        let mine = crate::api::ClientPage {
            owner: me,
            id: "deck".into(),
            widgets: Vec::new(),
            page: page.clone(),
            grab_power: false,
            leds: [0; 4],
            size: None,
        };
        let (knobs, buttons) =
            e.api_inputs(&page, Some(&mine), [CLIENT_STEP, 0, 0, 0], (1 << 5 | 1 << 8, 0));
        assert!(knobs[0]);
        assert_eq!(buttons, 1 << 5, "button2 taken, power not");
        let ev = events(&rx);
        assert!(ev.iter().all(|v| v["page"] == "deck"));
        assert_eq!(ev.iter().filter(|v| v["mine"] == true).count(), 2, "{ev:?}");
        // holding button2: a hold event, to the page's client
        e.client_pressed[5] = Some(Instant::now() - Duration::from_millis(HOLD_MS + 10));
        let cfg = Config::default();
        e.check_holds(&cfg, &page, 0);
        let ev = events(&rx);
        assert_eq!((ev[0]["event"].as_str(), ev[0]["mine"].as_bool()), (Some("hold"), Some(true)));
    }

    /// One alert per application: a newer one replaces the one waiting (several track changes:
    /// only the last) and the one shown, at once; the other applications keep their place.
    #[test]
    fn one_alert_per_application() {
        let shared = Arc::new(Shared::new(Config::default()));
        let mut e = Engine::new(shared.clone(), crate::audio::open(true));
        let cfg = Config::default();
        let n = |app: &str, title: &str| Notification {
            app: app.into(),
            summary: title.into(),
            body: String::new(),
            icon: None,
            duration: None,
            wake: Some(true),
            led: None,
            desktop: false,
            origin: None,
            style: Default::default(),
        };
        let push = |x| crate::notify::push(&shared, x, false);
        let waiting = || {
            let q = shared.notifications.lock().unwrap();
            q.iter().map(|x| x.summary.clone()).collect::<Vec<_>>()
        };
        push(n("Spotify", "morceau 1"));
        push(n("Système", "chaud"));
        push(n("spotify", "morceau 2"));
        push(n("Spotify", "morceau 3"));
        assert_eq!(waiting(), ["chaud", "morceau 3"]);
        e.next_notice(&cfg);
        let shown = |e: &Engine| e.notice.as_ref().map(|(x, _)| x.summary.clone());
        assert_eq!(shown(&e).as_deref(), Some("chaud"));
        push(n("Système", "plus chaud"));
        e.next_notice(&cfg);
        assert_eq!(shown(&e).as_deref(), Some("plus chaud"), "replaced at once");
        assert_eq!(waiting(), ["morceau 3"]);
    }

    /// The module page shown follows its module when the list changes; its module gone (stopped,
    /// unloaded): back to the configured page it was opened from; a page change asked meanwhile
    /// wins.
    #[test]
    fn a_module_leaving_sends_back_to_the_main_page() {
        let page = |owner, id: &str| ClientPage {
            owner,
            id: id.into(),
            widgets: Vec::new(),
            page: Config::default().pages[0].clone(),
            grab_power: false,
            leds: [0; 4],
            size: None,
        };
        let shown = (7, "discord".to_string());
        // 2 configured pages, then media (2) and discord (3, shown); opened from page 2 (index 1)
        let both = [page(5, "media"), page(7, "discord")];
        assert_eq!(follow_shown(3, (3, Some(&shown)), &both, (2, 1)), None, "still there");
        let moved = [page(7, "discord")];
        assert_eq!(follow_shown(3, (3, Some(&shown)), &moved, (2, 1)), Some(2), "media left");
        let gone = [page(5, "media")];
        assert_eq!(follow_shown(3, (3, Some(&shown)), &gone, (2, 1)), Some(1), "back to page 2");
        assert_eq!(follow_shown(0, (3, Some(&shown)), &gone, (2, 1)), None, "asked meanwhile");
        assert_eq!(follow_shown(1, (1, None), &gone, (2, 1)), None, "a configured page");
    }

    /// A module with « Empêche la veille » keeps the device awake while it is on screen (its
    /// page, or inside the page shown: the idle time starts again each frame); another page,
    /// switched off or without the option: not. Written in config.toml only when set.
    #[test]
    fn a_module_keeps_the_device_awake() {
        let mut cfg = Config::default();
        cfg.tools.push(crate::config::Tool {
            name: "Discord".into(),
            enabled: true,
            module: "discord".into(),
            args: Vec::new(),
            command: String::new(),
            keep_awake: true,
        });
        let shown = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert!(kept_awake(&cfg, &shown(&["Discord"])), "its page shown");
        assert!(!kept_awake(&cfg, &shown(&[])), "another page");
        assert!(!kept_awake(&cfg, &shown(&["Spotify"])), "another module");
        cfg.tools[0].enabled = false;
        assert!(!kept_awake(&cfg, &shown(&["Discord"])), "switched off");
        cfg.tools[0].enabled = true;
        assert!(toml::to_string(&cfg.tools[0]).unwrap().contains("keep_awake = true"));
        cfg.tools[0].keep_awake = false;
        assert!(!kept_awake(&cfg, &shown(&["Discord"])), "without the option");
        assert!(!toml::to_string(&cfg.tools[0]).unwrap().contains("keep_awake"));
    }

    /// A module restarted while its page is shown (code or settings changed): the main page
    /// meanwhile, its page again once it is back (power still goes back where it went); not when
    /// another page was chosen meanwhile.
    #[test]
    fn a_restarted_module_is_shown_again() {
        let shared = Arc::new(Shared::new(Config::default()));
        let mut e = Engine::new(shared.clone(), crate::audio::open(true));
        let start = |s: &Arc<Shared>| {
            let (id, _tx, _rx) = crate::api::register(s);
            crate::api::process(s, id, r#"{"type": "hello", "name": "Discord"}"#);
            crate::api::process(s, id, r#"{"type": "page", "page": "discord"}"#);
            id
        };
        let n_cfg = 2;
        let first = start(&shared);
        let shown = (first, "discord".to_string());
        (e.shown_tool, e.jump_from) = (Some(("Discord".into(), "discord".into())), Some(1));
        // it stops: back to the configured page (index 1), waiting for it
        crate::api::unregister(&shared, first);
        let pages = shared.api.pages();
        let to = follow_shown(2, (2, Some(&shown)), &pages, (n_cfg, 1)).unwrap();
        assert_eq!(to, 1);
        e.comeback = e.shown_tool.take().map(|(tool, page)| Comeback {
            tool,
            page,
            since: Instant::now(),
            at: to,
            jump_from: e.jump_from.take(),
        });
        assert_eq!(e.come_back(1, &pages, n_cfg), None, "not back yet");
        start(&shared);
        let pages = shared.api.pages();
        assert_eq!(e.come_back(1, &pages, n_cfg), Some(2), "back: its page again");
        assert_eq!((e.jump_from, e.comeback.is_none()), (Some(1), true));
        // another page chosen meanwhile: stays there
        e.comeback = Some(Comeback {
            tool: "Discord".into(),
            page: "discord".into(),
            since: Instant::now(),
            at: 1,
            jump_from: None,
        });
        assert_eq!(e.come_back(0, &pages, n_cfg), None);
        assert!(e.comeback.is_none());
    }

    /// Knob 1 held on a module's page (the module takes its clicks): the modules menu opens all
    /// the same, a short press stays the module's click, a grabbed knob stays the program's; a
    /// deck on knob 1 runs its tile at a short press's release.
    #[test]
    fn the_modules_menu_from_a_module_page() {
        let shared = Arc::new(Shared::new(Config::default()));
        let mut e = Engine::new(shared.clone(), crate::audio::open(true));
        let cfg = Config::default();
        let n_cfg = cfg.pages.len();
        (e.n_cfg, e.n_pages) = (n_cfg, n_cfg + 1);
        let (me, _tx, rx) = crate::api::register(&shared);
        let mut page = cfg.pages[0].clone();
        (page.knobs, page.buttons, page.widgets) = (Vec::new(), Vec::new(), Vec::new());
        let client = crate::api::ClientPage {
            owner: me,
            id: "p".into(),
            widgets: Vec::new(),
            page,
            grab_power: false,
            leds: [0; 4],
            size: None,
        };
        let at = (&client.page, Some(&client));
        let long_ago = Some(Instant::now() - Duration::from_millis(HOLD_MS + 10));
        let input = |e: &mut Engine, pressed: u16, released: u16| {
            let (_, taken) = e.api_inputs(&client.page, Some(&client), [0; 4], (pressed, released));
            e.keep_menu_keys(&cfg, at, (pressed, released), taken);
            taken
        };
        // knob 1 held: the module takes it (and gets `hold`), the menu opens all the same
        assert_eq!(input(&mut e, 1, 0), 1, "the module's page takes knob 1");
        (e.pressed_at[0], e.client_pressed[0]) = (long_ago, long_ago);
        e.check_holds(&cfg, &client.page, n_cfg);
        assert!(e.menu.is_some(), "knob 1 held on a module's page: the modules menu");
        let kinds: Vec<_> = events(&rx).iter().map(|v| v["event"].as_str().unwrap().to_owned()).collect();
        assert_eq!(kinds, ["press", "hold"]);
        // a short press: the module's click, no menu
        e.menu = None;
        input(&mut e, 1, 0);
        input(&mut e, 0, 1);
        e.check_holds(&cfg, &client.page, n_cfg);
        assert!(e.menu.is_none());
        assert!(events(&rx).iter().any(|v| v["event"] == "click" && v["mine"] == true));
        // knob 1 grabbed by a program: all its
        shared.api.grabs.lock().unwrap().insert("knob1".into(), me);
        input(&mut e, 1, 0);
        (e.pressed_at[0], e.client_pressed[0]) = (long_ago, long_ago);
        e.check_holds(&cfg, &client.page, n_cfg);
        assert!(e.menu.is_none(), "grabbed: no menu");
        input(&mut e, 0, 1);
        shared.api.grabs.lock().unwrap().clear();

        // a deck on knob 1 (whose hold opens the menu): its tile runs at a short press's release
        let mut page = cfg.pages[0].clone();
        let tile = crate::config::Tile::new("2", "", "", Action::Page { index: 2 });
        page.widgets.push(crate::config::Widget {
            x: 0,
            y: 0,
            w: 112,
            h: 248,
            frame: None,
            kind: WidgetKind::Deck {
                knob: 1,
                vertical: true,
                color: Color(0, 200, 200),
                tiles: vec![tile],
            },
        });
        shared.page.store(0, Ordering::Relaxed);
        let (_, (pressed, _)) = e.deck_inputs(&cfg, (&page, 0, None), [0; 4], (1, 0));
        assert_eq!(pressed, 0, "the deck takes knob 1");
        assert_eq!(shared.page.load(Ordering::Relaxed), 0, "not at the press");
        let _ = e.deck_inputs(&cfg, (&page, 0, None), [0; 4], (0, 1));
        assert_eq!(shared.page.load(Ordering::Relaxed), 1, "tile « 2 » at the release");
        // held: the menu, and the release does not run the tile
        shared.page.store(0, Ordering::Relaxed);
        let _ = e.deck_inputs(&cfg, (&page, 0, None), [0; 4], (1, 0));
        e.keep_menu_keys(&cfg, (&page, None), (1, 0), 1);
        e.pressed_at[0] = long_ago;
        e.check_holds(&cfg, &page, 0);
        assert!(e.menu.is_some() && e.deck_armed == 0);
    }

    /// A `deck` widget takes its knob on its page: turning moves the selection (endless, with a
    /// slide), the click runs the selected tile; the other knobs keep their bindings.
    #[test]
    fn a_deck_widget_takes_its_knob() {
        let shared = Arc::new(Shared::new(Config::default()));
        let mut e = Engine::new(shared.clone(), crate::audio::open(true));
        let cfg = Config::default();
        (e.n_cfg, e.n_pages) = (3, 3);
        let mut page = cfg.pages[0].clone();
        let tile = |i: usize| {
            crate::config::Tile::new(&format!("{i}"), "", "", Action::Page { index: i })
        };
        page.widgets.push(crate::config::Widget {
            x: 0,
            y: 0,
            w: 112,
            h: 248,
            frame: None,
            kind: WidgetKind::Deck {
                knob: 3,
                vertical: true,
                color: Color(0, 200, 200),
                tiles: (1..=3).map(tile).collect(),
            },
        });
        let deck = page.widgets.len() - 1;
        let turn = [5, 0, 2 * DECK_STEP + 3, 0];
        let (counts, (pressed, _)) = e.deck_inputs(&cfg, (&page, 0, None), turn, (0, 0));
        assert_eq!(counts, [5, 0, 0, 0], "knob 3 taken, knob 1 left to its binding");
        assert_eq!(pressed, 0);
        let v = e.deck_views(&page, 0)[&deck];
        assert_eq!(v.selected, 2);
        assert!(v.offset > 1.5, "slides in from where it was: {}", v.offset);
        // backwards past the first tile: endless
        let _ = e.deck_inputs(&cfg, (&page, 0, None), [0, 0, -3 * DECK_STEP - 3, 0], (0, 0));
        assert_eq!(e.deck_views(&page, 0)[&deck].selected, 2, "2 - 3 = -1 = 2 among 3");
        let _ = e.deck_inputs(&cfg, (&page, 0, None), [0, 0, -DECK_STEP, 0], (0, 0));
        assert_eq!(e.deck_views(&page, 0)[&deck].selected, 1);
        // its click: the selected tile (« 2 »: page 2), not the knob's mute; knob 1's click stays
        let (_, (pressed, _)) = e.deck_inputs(&cfg, (&page, 0, None), [0; 4], (0b101, 0));
        assert_eq!(pressed, 0b001);
        assert_eq!(shared.page.load(Ordering::Relaxed), 1, "tile « 2 » went to page 2");
        assert!(e.deck_views(&page, 0)[&deck].flash > 0.5);
        // on a tool's page: the deck takes its knob before the tool, which is told of a click;
        // a knob grabbed by a program stays the program's
        let (me, _tx, rx) = crate::api::register(&shared);
        let mut tool_page = crate::api::ClientPage {
            owner: me,
            id: "deck".into(),
            widgets: vec![("deck0".into(), page.widgets[deck].clone())],
            page: page.clone(),
            grab_power: false,
            leds: [0; 4],
            size: None,
        };
        tool_page.page.widgets = vec![page.widgets[deck].clone()];
        let at = (&tool_page.page, 5, Some(&tool_page));
        let (counts, (pressed, _)) = e.deck_inputs(&cfg, at, [0, 0, DECK_STEP, 0], (0b100, 0));
        assert_eq!((counts, pressed), ([0; 4], 0));
        let ev = events(&rx);
        assert_eq!((ev[0]["event"].as_str(), ev[0]["widget"].as_str()), (Some("deck"), Some("deck0")));
        assert_eq!(ev[0]["title"], "2", "one step from the first tile, then the click");
        shared.api.grabs.lock().unwrap().insert("knob3".into(), me);
        let (counts, _) = e.deck_inputs(&cfg, at, [0, 0, DECK_STEP, 0], (0, 0));
        assert_eq!(counts[2], DECK_STEP, "grabbed: left to the program");
    }

    /// A tool's commands, once it offers them: a button action runs one, a knob aimed at one
    /// sends it the turns (not to the audio), its state feeds the knob's widgets and a LED; gone
    /// with the tool.
    #[test]
    fn tool_commands_from_buttons_knobs_and_leds() {
        let shared = Arc::new(Shared::new(Config::default()));
        let (id, _tx, rx) = crate::api::register(&shared);
        let ask = |req: &str| crate::api::process(&shared, id, req).unwrap_or_default();
        ask(r#"{"type": "hello", "name": "Discord"}"#);
        let r = ask(r#"{"type": "commands", "req": 1, "commands": [
            {"id": "mute", "name": "Couper le micro"},
            {"id": "volume", "name": "Volume", "kind": "knob"}]}"#);
        assert!(r.contains("\"ok\""), "{r}");
        let mut e = Engine::new(shared.clone(), crate::audio::open(true));
        let cfg = Config::default();
        let mut page = cfg.pages[0].clone();
        let (tool, command) = ("Discord".to_string(), "volume".to_string());
        page.knobs[1].target = Target::Tool { tool, command };
        e.run_action(&Action::ToolCommand { tool: "Discord".into(), command: "mute".into() }, &cfg, &page, 0);
        let counts = e.tool_turns(&page, [7, 2 * CLIENT_STEP + 1, 0, 0]);
        assert_eq!(counts, [7, 0, 0, 0], "knob 2 to the tool, knob 1 to the audio");
        let ev = events(&rx);
        assert_eq!((ev[0]["event"].as_str(), ev[0]["command"].as_str()), (Some("command"), Some("mute")));
        assert_eq!((ev[1]["command"].as_str(), ev[1]["delta"].as_i64()), (Some("volume"), Some(2)));
        // its state: knob 2's widgets, a LED
        ask(r#"{"type": "command_state", "command": "volume", "value": 0.4, "text": "Alice 40 %"}"#);
        ask(r#"{"type": "command_state", "command": "mute", "active": true}"#);
        let v = e.view(&page);
        assert_eq!((v.knobs[1].name.as_str(), v.knobs[1].volume, v.knobs[1].present), ("Alice 40 %", 0.4, true));
        page.buttons[0].led = Led::Tool { tool: "Discord".into(), command: "mute".into() };
        assert_eq!(led_values(&page, &v, &shared.api)[0], LED_ON);
        // the tool goes: nothing offered, nothing lit
        crate::api::process(&shared, id, r#"{"type": "commands", "commands": []}"#);
        assert_eq!(led_values(&page, &e.view(&page), &shared.api)[0], 0);
        assert!(!shared.api.run_command("Discord", "mute", None));
    }

    /// A `tool` widget on a configured page: the tool's page is found (drawn in the widget) and
    /// the inputs the widget lists go to the tool, named by the tool's page; the others stay the
    /// page's.
    #[test]
    fn a_tool_widget_shows_the_tool_and_takes_its_inputs() {
        let shared = Arc::new(Shared::new(Config::default()));
        let (id, _tx, rx) = crate::api::register(&shared);
        crate::api::process(&shared, id, r#"{"type": "hello", "name": "Menu"}"#);
        crate::api::process(&shared, id, r#"{"type": "page", "page": "deck"}"#);
        let mut page = Config::default().pages[0].clone();
        let tool = |tool: &str, id: &str| crate::config::Widget {
            x: 240,
            y: 136,
            w: 240,
            h: 136,
            frame: None,
            kind: WidgetKind::Tool {
                tool: tool.into(),
                page: id.into(),
                inputs: vec!["button1".into(), "knob4".into()],
                overflow: Default::default(),
            },
        };
        // a tool not running, a page it does not have: nothing shown
        page.widgets.extend([tool("Absent", ""), tool("Menu", "other"), tool("Menu", "")]);
        let pages = shared.api.pages();
        let embeds = embedded(&shared.api, &page, &pages);
        assert_eq!(embeds.len(), 1);
        assert_eq!((embeds[0].index, embeds[0].page.id.as_str()), (page.widgets.len() - 1, "deck"));

        let mut e = Engine::new(shared.clone(), crate::audio::open(true));
        e.routes = routes(&embeds);
        let turns = [CLIENT_STEP, 0, 0, CLIENT_STEP];
        let (knobs, buttons) = e.api_inputs(&page, None, turns, (1 << 4 | 1 << 5, 0));
        assert_eq!(knobs, [false, false, false, true]);
        assert_eq!(buttons, 1 << 4, "button1 to the tool, button2 to the page's bindings");
        let ev = events(&rx);
        let at = |v: &Value| (v["mine"].as_bool().unwrap(), v["page"].as_str().unwrap().to_owned());
        let name = page.name.clone();
        let seen: Vec<_> = ev.iter().map(at).collect();
        assert_eq!(
            seen,
            [(false, name.clone()), (true, "deck".into()), (true, "deck".into()), (false, name)],
            "knob1, knob4, button1, button2: {ev:?}"
        );
        // its hold, release and click: the tool's too
        e.client_pressed[4] = Some(Instant::now() - Duration::from_millis(HOLD_MS + 10));
        e.check_holds(&Config::default(), &page, 0);
        let (_, buttons) = e.api_inputs(&page, None, [0; 4], (0, 1 << 4));
        assert_eq!(buttons, 1 << 4);
        let ev = events(&rx);
        assert!(ev.iter().all(|v| v["mine"] == true && v["page"] == "deck"), "{ev:?}");
        let kinds: Vec<_> = ev.iter().map(|v| v["event"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["hold", "release"], "no click after a hold");
    }
}
