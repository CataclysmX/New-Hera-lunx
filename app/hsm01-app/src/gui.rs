//! Desktop window (egui): live preview of the device screen and an editor for pages, knobs,
//! buttons and widgets. Edits apply at once (the engine reads `shared.config` every frame);
//! « Enregistrer » (Ctrl+S) writes the TOML. Widgets are selected and moved on the preview.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Button, Color32, ColorImage, ComboBox, DragValue, IdSalt, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, TextEdit, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};

use crate::audio::Inventory;
use crate::config::{
    Action, Align, Color, Config, Fit, Frame, Gauge, IconStyle, Led, LineStyle, Overflow, Page,
    Paint, SCREEN_H, SCREEN_W, STANDBY_MIN_SECS, Sides, Target, Tile, Wallpaper, Widget, WidgetKind,
};
use crate::engine::Shared;
use crate::i18n::{tr, tx};
use crate::layout::{self, Edge};
use crate::state::Overlay;

const W: f32 = SCREEN_W as f32;
/// Hover text of the « Hold » settings.
fn hold_help() -> &'static str {
    tr("press held 0.5 s; when there is a hold action, the click goes on the release of a short press")
}
const H: f32 = SCREEN_H as f32;
const ACCENT: Color32 = Color32::from_rgb(0x00, 0xb4, 0xa8);
const WARN: Color32 = Color32::from_rgb(0xe0, 0xb0, 0x40);
const ERROR: Color32 = Color32::from_rgb(0xd0, 0x60, 0x50);

pub fn run(shared: Arc<Shared>) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("{} – Hercules Stream 100", crate::APP_NAME))
            .with_app_id("hsm01-app")
            .with_inner_size([1360.0, 780.0])
            .with_min_inner_size([960.0, 540.0])
            .with_icon(Arc::new(egui::IconData {
                rgba: crate::icon::rgba(crate::icon::WINDOW_SVG, 128, crate::icon::WINDOW_COLOR)
                    .unwrap_or_default(),
                width: 128,
                height: 128,
            })),
        ..Default::default()
    };
    eframe::run_native(
        "hsm01-app",
        options,
        Box::new(move |_cc| Ok(Box::new(Gui::new(shared)))),
    )
    .map_err(|e| format!("window: {e}"))
}

fn normalised(mut cfg: Config) -> Config {
    cfg.pages.iter_mut().for_each(Page::normalise);
    cfg
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Widgets,
    Controls,
    Device,
    Tools,
    /// The settings of the enabled tool `Gui::settings_tab` (its `[settings]`).
    Settings,
}

/// A module's sensitive data being typed in its tab: one value per `[[secret]]` field, as last
/// read or written, and which hidden ones are shown.
struct SecretsEdit {
    dir: std::path::PathBuf,
    values: Vec<String>,
    saved: Vec<String>,
    shown: Vec<bool>,
}

/// The settings file of a tool, being edited in its tab.
struct SettingsEdit {
    path: std::path::PathBuf,
    /// Text of the file as last read or written ("" while it does not exist).
    saved: String,
    form: SettingsForm,
    /// Last problem (reading, checking, writing).
    error: Option<String>,
    /// Last look at the file on disk (changed by hand: read again if nothing is being edited).
    checked: Instant,
}

enum SettingsForm {
    /// The stream deck's file: as read, and as edited.
    Deck {
        read: crate::deckfile::DeckFile,
        edit: crate::deckfile::DeckFile,
    },
    /// A form of the module's fields (`editor = "formulaire"`): the values as read and edited.
    Fields {
        fields: Vec<crate::tools::Field>,
        read: toml::Table,
        edit: toml::Table,
    },
    /// Any other file: its TOML text.
    Text(String),
}

impl SettingsEdit {
    /// The file of `m` (`deck` editor, or its text — also for a deck file that does not read).
    fn open(m: &crate::tools::Module) -> Option<SettingsEdit> {
        let path = m.settings_path()?;
        let deck = m.settings.as_ref().is_some_and(|s| s.editor == "deck");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let fields = m.settings.as_ref().filter(|s| s.editor == "formulaire" && !s.fields.is_empty());
        if let Some(s) = fields {
            let (form, error) = match toml::from_str::<toml::Table>(&text) {
                Ok(t) => (SettingsForm::Fields { fields: s.fields.clone(), read: t.clone(), edit: t }, None),
                Err(e) => (SettingsForm::Text(text.clone()), Some(tf!("{e} (edited as text)", e = e))),
            };
            return Some(SettingsEdit { path, saved: text, form, error, checked: Instant::now() });
        }
        let (form, error) = match deck.then(|| crate::deckfile::load(&path)) {
            Some(Ok((read, _))) => (
                SettingsForm::Deck {
                    edit: read.clone(),
                    read,
                },
                None,
            ),
            Some(Err(e)) => (SettingsForm::Text(text.clone()), Some(tf!("{e} (edited as text)", e = e))),
            None => (SettingsForm::Text(text.clone()), None),
        };
        Some(SettingsEdit {
            path,
            saved: text,
            form,
            error,
            checked: Instant::now(),
        })
    }

    fn changed(&self) -> bool {
        match &self.form {
            SettingsForm::Deck { read, edit } => read != edit,
            SettingsForm::Fields { read, edit, .. } => read != edit,
            SettingsForm::Text(text) => *text != self.saved,
        }
    }

    /// Write the file (text: only if it is valid TOML).
    fn save(&mut self) -> Result<(), String> {
        let text = match &self.form {
            SettingsForm::Deck { edit, .. } => crate::deckfile::to_toml(edit, &self.saved)?,
            SettingsForm::Fields { edit, .. } => crate::settingsform::to_toml(edit, &self.saved)?,
            SettingsForm::Text(text) => {
                toml::from_str::<toml::Table>(text).map_err(|e| tf!("invalid TOML: {e}", e = e))?;
                text.clone()
            }
        };
        std::fs::write(&self.path, &text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        match &mut self.form {
            SettingsForm::Deck { read, edit } => *read = edit.clone(),
            SettingsForm::Fields { read, edit, .. } => *read = edit.clone(),
            SettingsForm::Text(_) => {}
        }
        self.saved = text;
        self.error = None;
        Ok(())
    }
}

/// Mouse action on the preview (device pixels).
enum Drag {
    /// The selection: box of each selected widget at the start, pointer movement since.
    Move {
        start: Vec<(usize, (i32, i32, i32, i32))>,
        moved: Vec2,
    },
    /// One widget by one of its handles.
    Resize {
        index: usize,
        start: (i32, i32, i32, i32),
        handle: Handle,
        moved: Vec2,
    },
    /// Selection rectangle from `from` to `to`, added to `base` (Shift).
    Band {
        base: Vec<usize>,
        from: (f32, f32),
        to: (f32, f32),
    },
}

/// Resize handle: the edges it moves.
#[derive(Clone, Copy, Debug)]
struct Handle {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Handle {
    fn cursor(self) -> egui::CursorIcon {
        use egui::CursorIcon as C;
        match (self.left || self.right, self.top || self.bottom) {
            (true, false) => C::ResizeHorizontal,
            (false, true) => C::ResizeVertical,
            _ if (self.left && self.top) || (self.right && self.bottom) => C::ResizeNwSe,
            _ => C::ResizeNeSw,
        }
    }
}

/// The 8 handles (corners, middle of the sides) of a widget, in screen coordinates.
fn handles_of(w: &Widget, to_screen: &impl Fn(f32, f32) -> Pos2) -> Vec<(Pos2, Handle)> {
    let (x0, y0) = (w.x as f32, w.y as f32);
    let (x1, y1) = (x0 + w.w as f32, y0 + w.h as f32);
    let (xm, ym) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let h = |left, right, top, bottom| Handle {
        left,
        right,
        top,
        bottom,
    };
    vec![
        (to_screen(x0, y0), h(true, false, true, false)),
        (to_screen(xm, y0), h(false, false, true, false)),
        (to_screen(x1, y0), h(false, true, true, false)),
        (to_screen(x0, ym), h(true, false, false, false)),
        (to_screen(x1, ym), h(false, true, false, false)),
        (to_screen(x0, y1), h(true, false, false, true)),
        (to_screen(xm, y1), h(false, false, false, true)),
        (to_screen(x1, y1), h(false, true, false, true)),
    ]
}

fn boxed(w: &Widget) -> (i32, i32, i32, i32) {
    (w.x, w.y, w.w, w.h)
}

fn snap16(v: i32) -> i32 {
    (v as f32 / 16.0).round() as i32 * 16
}

/// Topmost widget under the device point (px, py).
fn hit_widget(ws: &[Widget], px: f32, py: f32) -> Option<usize> {
    ws.iter().rposition(|w| {
        px >= w.x as f32 && px < (w.x + w.w) as f32 && py >= w.y as f32 && py < (w.y + w.h) as f32
    })
}

/// Shift+click: add `i` to the selection, or remove it when already there.
fn toggle_in(sel: &mut Vec<usize>, i: usize) {
    if let Some(p) = sel.iter().position(|&s| s == i) {
        sel.remove(p);
    } else {
        sel.push(i);
    }
}

/// Magnet along one axis: the start, centre or end of a box at `pos` of length `len` snaps to
/// a start, centre or end of the `others` (pos, len) or of the screen (0, middle, `screen`)
/// within `MAGNET_PX`. Returns the shift and the guide position.
fn magnet(
    pos: i32,
    len: i32,
    others: impl Iterator<Item = (i32, i32)>,
    screen: i32,
) -> (i32, Option<i32>) {
    const MAGNET_PX: i32 = 6;
    let mut lines = vec![0, screen / 2, screen];
    for (p, l) in others {
        lines.extend([p, p + l / 2, p + l]);
    }
    let mut best: Option<(i32, i32)> = None;
    for line in lines {
        for edge in [pos, pos + len / 2, pos + len] {
            let d = line - edge;
            if d.abs() <= MAGNET_PX && best.is_none_or(|(bd, _)| d.abs() < bd.abs()) {
                best = Some((d, line));
            }
        }
    }
    best.map_or((0, None), |(d, line)| (d, Some(line)))
}

/// Titled block of the right panel.
fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).strong().size(15.0).color(ACCENT));
    ui.separator();
}

/// Framed group (one knob, one button…).
fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong());
        add(ui);
    });
    ui.add_space(4.0);
}

/// Window preferences kept between launches (`$XDG_STATE_HOME/hsm01/window.toml`, by default
/// `~/.local/state/hsm01/window.toml`): not part of the device configuration.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Prefs {
    /// « Cadres des widgets »
    frames: bool,
    magnets: bool,
    /// « Grille 16 px »
    grid: bool,
    /// Calibration: « Lier rouge, vert et bleu »
    link_rgb: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            frames: true,
            magnets: true,
            grid: false,
            link_rgb: false,
        }
    }
}

impl Prefs {
    fn path() -> std::path::PathBuf {
        let home = || std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        std::env::var_os("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home().join(".local/state"))
            .join("hsm01")
            .join("window.toml")
    }

    /// Saved preferences, defaults if none (always the defaults in tests).
    fn load() -> Self {
        if cfg!(test) {
            return Prefs::default();
        }
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Written at each change (never from the tests).
    fn save(&self) {
        if cfg!(test) {
            return;
        }
        let path = Self::path();
        let text = toml::to_string(self).unwrap_or_default();
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, text));
        if let Err(e) = written {
            eprintln!("hsm01-app: {}: {e}", path.display());
        }
    }
}

struct Gui {
    shared: Arc<Shared>,
    /// Preferences as last saved.
    prefs: Prefs,
    /// Modules shipped with the app (tab « Modules »).
    modules: Vec<crate::tools::Module>,
    /// Fingerprint of their descriptions when read, and when it was last checked (2 s).
    modules_stamp: (u64, Instant),
    /// Languages offered (code, name), read again when the list opens.
    languages: Vec<(String, String)>,
    /// Text of the arguments field of each tool (by folder) while it is edited.
    args_text: std::collections::HashMap<String, String>,
    /// Name typed for a new tool (« + Ajouter un module »).
    new_tool: String,
    /// Config being edited (applied live) and the last one saved or loaded.
    draft: Config,
    saved: Config,
    seen_version: u64,
    /// The file changed on disk while the draft had unsaved edits.
    external_change: bool,
    page: usize,
    /// Selected widgets of the page, in selection order: the last one is shown in the inspector
    /// and is the reference of « same size ».
    selection: Vec<usize>,
    texture: Option<TextureHandle>,
    seen_frame: u64,
    /// Mouse action in progress on the preview.
    drag: Option<Drag>,
    snap: bool,
    /// Magnets: a dragged widget snaps to the edges / centres of the others and of the screen.
    magnets: bool,
    /// Alignment guides shown while dragging (vertical x, horizontal y; device pixels).
    guides: (Option<i32>, Option<i32>),
    show_frames: bool,
    /// Calibration section open (in the « Appareil » tab): the device shows the test pattern.
    calibrating: bool,
    show_pattern: bool,
    /// Calibration: moving one channel moves the other two by the same amount.
    link_rgb: bool,
    /// Tab of the right panel.
    tab: Tab,
    /// Tool (folder) whose settings tab is chosen, and its file being edited.
    settings_tab: String,
    settings_edit: Option<SettingsEdit>,
    /// Its sensitive data being typed (`[[secret]]`, its `.env`).
    secrets_edit: Option<SecretsEdit>,
    /// File chosen in the « Parcourir… » dialog (runs in its own thread): (page, path).
    picked: Arc<Mutex<Option<(usize, String)>>>,
    note: String,
    /// Close requested with unsaved changes: ask first.
    confirm_close: bool,
    /// Undo / redo (Ctrl+Z, Ctrl+Maj+Z / Ctrl+Y): the drafts before each edit.
    undo: Vec<Config>,
    redo: Vec<Config>,
    /// Draft as of the previous frame.
    seen: Config,
    /// Edit in progress: the draft before it and its last change (one drag, one typing burst…
    /// make one undo step).
    gesture: Option<(Config, Instant)>,
}

/// Undo steps kept.
const UNDO_DEPTH: usize = 100;
/// An edit is complete after this pause (mouse buttons up).
const GESTURE_PAUSE: Duration = Duration::from_millis(400);

impl Gui {
    fn new(shared: Arc<Shared>) -> Self {
        let draft = normalised(shared.config.read().unwrap().clone());
        let prefs = Prefs::load();
        Gui {
            prefs,
            modules: crate::tools::modules(),
            modules_stamp: (crate::tools::descriptions_stamp(), Instant::now()),
            languages: crate::i18n::available(),
            args_text: Default::default(),
            new_tool: String::new(),
            seen_version: shared.config_version.load(Ordering::Acquire),
            page: shared.page.load(Ordering::Relaxed),
            saved: draft.clone(),
            seen: draft.clone(),
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: None,
            draft,
            shared,
            external_change: false,
            selection: Vec::new(),
            texture: None,
            seen_frame: u64::MAX,
            drag: None,
            snap: prefs.grid,
            magnets: prefs.magnets,
            guides: (None, None),
            show_frames: prefs.frames,
            calibrating: false,
            show_pattern: true,
            link_rgb: prefs.link_rgb,
            tab: Tab::Widgets,
            settings_tab: String::new(),
            settings_edit: None,
            secrets_edit: None,
            picked: Arc::default(),
            note: String::new(),
            confirm_close: false,
        }
    }

    /// Record edits for undo: a change opens a gesture (with the draft before it); the gesture
    /// becomes one undo step once the mouse buttons are up and nothing changed for a moment.
    fn track_edits(&mut self, busy: bool) {
        if self.draft != self.seen {
            let before = std::mem::replace(&mut self.seen, self.draft.clone());
            match &mut self.gesture {
                Some((_, last)) => *last = Instant::now(),
                None => self.gesture = Some((before, Instant::now())),
            }
        }
        if !busy
            && self
                .gesture
                .as_ref()
                .is_some_and(|(_, t)| t.elapsed() >= GESTURE_PAUSE)
        {
            self.commit_gesture();
        }
    }

    fn commit_gesture(&mut self) {
        if let Some((before, _)) = self.gesture.take()
            && before != self.draft
        {
            self.undo.push(before);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    fn undo(&mut self) {
        self.commit_gesture();
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.draft, prev));
            self.after_history();
        }
    }

    fn redo(&mut self) {
        self.commit_gesture();
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.draft, next));
            self.after_history();
        }
    }

    /// After undo / redo: not an edit itself; keep the page and the selection valid.
    fn after_history(&mut self) {
        self.seen = self.draft.clone();
        let n = self.draft.pages.len();
        if n > 0 && self.page >= n {
            self.set_page(n - 1);
        }
        let widgets = self
            .draft
            .pages
            .get(self.page)
            .map_or(0, |p| p.widgets.len());
        self.selection.retain(|&i| i < widgets);
        self.note = tf!("{undo} steps to undo, {redo} to redo", undo = self.undo.len(), redo = self.redo.len());
    }

    /// Follow the config file (edited elsewhere) and the page shown on the device.
    fn sync(&mut self) {
        let v = self.shared.config_version.load(Ordering::Acquire);
        if v != self.seen_version {
            self.seen_version = v;
            let disk = normalised(self.shared.config.read().unwrap().clone());
            if disk != self.saved {
                if self.draft == self.saved {
                    self.draft = disk.clone();
                    self.saved = disk;
                } else {
                    self.external_change = true;
                }
            }
        }
        // a page of the configuration shown on the device: the editor follows; a module's page
        // (they come after the configuration's): the editor stays on its page
        let (shown, n) = (self.shared.page.load(Ordering::Relaxed), self.draft.pages.len());
        let to = if shown < n { shown } else { self.page.min(n.saturating_sub(1)) };
        if to != self.page {
            self.page = to;
            self.selection.clear();
        }
    }

    fn set_page(&mut self, i: usize) {
        self.page = i;
        self.selection.clear();
        self.shared.page.store(i, Ordering::Relaxed);
    }

    /// Live: the engine draws the draft from the next frame on.
    fn apply(&self) {
        crate::i18n::set_language(&self.draft.language);
        if *self.shared.config.read().unwrap() != self.draft {
            *self.shared.config.write().unwrap() = self.draft.clone();
        }
    }

    /// Tray requests (quit, bring forward) and closing with unsaved changes.
    fn window_requests(&mut self, ctx: &egui::Context) {
        let close = |ctx: &egui::Context| ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        if self.shared.quit.load(Ordering::Relaxed) {
            close(ctx);
            return;
        }
        if self.shared.show_window.swap(false, Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.draft != self.saved {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        if !self.confirm_close {
            return;
        }
        egui::Modal::new(egui::Id::unique("confirm_close")).show(ctx, |ui| {
            ui.label(tr("Changes are not saved."));
            ui.horizontal(|ui| {
                if ui.button(tr("Save")).clicked() {
                    self.save();
                    self.confirm_close = false;
                    if self.draft == self.saved {
                        close(ctx);
                    }
                }
                if ui.button(tr("Discard")).clicked() {
                    self.draft = self.saved.clone();
                    self.apply();
                    self.confirm_close = false;
                    close(ctx);
                }
                if ui.button(tr("Cancel")).clicked() {
                    self.confirm_close = false;
                }
            });
        });
    }

    fn save(&mut self) {
        match self.draft.save() {
            Ok(()) => {
                self.saved = self.draft.clone();
                self.external_change = false;
                self.note = tf!("saved: {path}", path = Config::path().display());
            }
            Err(e) => self.note = tf!("saving failed: {e}", e = e),
        }
    }

    fn revert(&mut self) {
        match Config::load() {
            Ok(c) => {
                self.draft = normalised(c);
                self.saved = self.draft.clone();
                self.external_change = false;
                self.selection.clear();
                self.note = tr("file reloaded").into();
            }
            Err(e) => self.note = e,
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let s = self.shared.status.lock().unwrap().clone();
        ui.horizontal(|ui| {
            // the default fonts have no "●": paint the status dot
            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            let colour = if s.connected { ACCENT } else { ERROR };
            ui.painter().circle_filled(dot.center(), 4.5, colour);
            if s.connected {
                ui.label(tf!("Stream 100 – firmware {version} – {fps} fps – {kib} KiB/s", version = format!("{}.{:02}", s.firmware.0, s.firmware.1), fps = format!("{:.0}", s.fps), kib = format!("{:.0}", s.kib_per_s)));
            } else {
                ui.label(tr("disconnected"));
            }
            if s.asleep {
                ui.colored_label(WARN, tr("standby"));
            }
            ui.separator();
            ui.label(s.message.as_str());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = self.draft != self.saved;
                if ui
                    .add_enabled(dirty, Button::new(tr("Save")))
                    .on_hover_text("Ctrl+S")
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .add_enabled(dirty || self.external_change, Button::new(tr("Reload")))
                    .on_hover_text(tr("discard the changes: go back to the saved file (Ctrl+Z undoes this)"))
                    .clicked()
                {
                    self.revert();
                }
                let can_redo = !self.redo.is_empty();
                if ui
                    .add_enabled(can_redo, Button::new(tr("Redo")))
                    .on_hover_text(tr("Ctrl+Shift+Z or Ctrl+Y"))
                    .clicked()
                {
                    self.redo();
                }
                let can_undo = !self.undo.is_empty() || self.gesture.is_some();
                if ui
                    .add_enabled(can_undo, Button::new(tr("Undo")))
                    .on_hover_text("Ctrl+Z")
                    .clicked()
                {
                    self.undo();
                }
                if self.external_change {
                    ui.colored_label(WARN, tr("file changed elsewhere"));
                } else if dirty {
                    ui.colored_label(WARN, tr("unsaved changes"));
                } else if !self.note.is_empty() {
                    ui.weak(self.note.as_str());
                }
            });
        });
    }

    fn preview(&mut self, ui: &mut Ui) {
        let frame = self.shared.preview_frame.load(Ordering::Acquire);
        if frame != self.seen_frame || self.texture.is_none() {
            self.seen_frame = frame;
            let img = ColorImage::from_rgba_unmultiplied(
                [SCREEN_W as usize, SCREEN_H as usize],
                &self.shared.preview.lock().unwrap(),
            );
            match &mut self.texture {
                Some(t) => t.set(img, TextureOptions::NEAREST),
                None => {
                    self.texture = Some(ui.ctx().load_texture(
                        "preview",
                        img,
                        TextureOptions::NEAREST,
                    ));
                }
            }
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_frames, tr("Widget frames"));
            ui.checkbox(&mut self.magnets, tr("Magnets")).on_hover_text(
                tr("while dragging, edges and centres snap to those of the other widgets and of the screen (hold Alt to move freely)"),
            );
            ui.checkbox(&mut self.snap, tr("16 px grid"))
                .on_hover_text(tr("advised for the spectrum (lightest sending)"));
            ui.weak(tr("click: select – drag: move"));
        });
        let avail = ui.available_rect_before_wrap();
        let fit = ((avail.width() - 24.0) / W)
            .min((avail.height() - 24.0) / H)
            .max(0.25);
        // integer zoom when it fits: sharp pixels
        let scale = if fit >= 1.0 { fit.floor() } else { fit };
        let rect = Rect::from_center_size(avail.center(), vec2(W * scale, H * scale));
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(avail);
        painter.rect_filled(rect.expand(8.0), 10.0, Color32::from_gray(28));
        if let Some(t) = &self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(t.id(), rect, uv, Color32::WHITE);
        }
        let Some(page) = self.draft.pages.get_mut(self.page) else {
            return;
        };
        let n = page.widgets.len();
        self.selection.retain(|&i| i < n);
        // the outlines are drawn by the engine, on the device itself (and so in this preview)
        let overlay = Overlay {
            frames: self.show_frames,
            selected: self.selection.clone(),
            pattern: self.calibrating && self.show_pattern,
        };
        {
            let mut shown = self.shared.overlay.lock().unwrap();
            if *shown != overlay {
                *shown = overlay;
            }
        }

        let to_dev = |p: Pos2| ((p.x - rect.min.x) / scale, (p.y - rect.min.y) / scale);
        let to_screen = |x: f32, y: f32| pos2(rect.min.x + x * scale, rect.min.y + y * scale);
        let (shift, free) = ui.ctx().input(|i| (i.modifiers.shift, i.modifiers.alt));
        let single = |sel: &[usize]| (sel.len() == 1).then(|| sel[0]);
        let handles = |ws: &[Widget], sel: &[usize]| {
            single(sel)
                .and_then(|i| ws.get(i))
                .map_or_else(Vec::new, |w| handles_of(w, &to_screen))
        };
        let grab = |ws: &[Widget], sel: &[usize], p: Pos2| {
            handles(ws, sel)
                .into_iter()
                .find(|(h, _)| (h.x - p.x).abs() <= 6.0 && (h.y - p.y).abs() <= 6.0)
                .map(|(_, h)| h)
        };

        // click: select (Shift: add / remove); empty area: deselect
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let (px, py) = to_dev(p);
            let hit = hit_widget(&page.widgets, px, py);
            match (hit, shift) {
                (Some(i), true) => toggle_in(&mut self.selection, i),
                (Some(i), false) => self.selection = vec![i],
                (None, false) => self.selection.clear(),
                (None, true) => {}
            }
            if hit.is_some() {
                self.tab = Tab::Widgets;
            }
        }
        // drag: resize handle, move the selection, or a selection rectangle on empty space
        if resp.drag_started()
            && let Some(p) = ui.ctx().input(|i| i.pointer.press_origin())
        {
            let (px, py) = to_dev(p);
            self.drag = if let (Some(handle), Some(i)) = (
                grab(&page.widgets, &self.selection, p),
                single(&self.selection),
            ) {
                Some(Drag::Resize {
                    index: i,
                    start: boxed(&page.widgets[i]),
                    handle,
                    moved: Vec2::ZERO,
                })
            } else if let Some(i) = hit_widget(&page.widgets, px, py) {
                if !self.selection.contains(&i) {
                    if shift {
                        self.selection.push(i);
                    } else {
                        self.selection = vec![i];
                    }
                }
                self.tab = Tab::Widgets;
                let start = self
                    .selection
                    .iter()
                    .map(|&j| (j, boxed(&page.widgets[j])))
                    .collect();
                Some(Drag::Move {
                    start,
                    moved: Vec2::ZERO,
                })
            } else {
                let base = if shift {
                    self.selection.clone()
                } else {
                    Vec::new()
                };
                self.selection.clear();
                Some(Drag::Band {
                    base,
                    from: (px, py),
                    to: (px, py),
                })
            };
        }
        if resp.dragged() {
            let delta = resp.drag_delta() / scale;
            let (snap, magnets) = (self.snap, self.magnets && !free && !self.snap);
            // boxes of the widgets that stay put, for the magnets
            let moving: Vec<usize> = match &self.drag {
                Some(Drag::Move { start, .. }) => start.iter().map(|s| s.0).collect(),
                Some(Drag::Resize { index, .. }) => vec![*index],
                _ => Vec::new(),
            };
            let others: Vec<(i32, i32, i32, i32)> = page
                .widgets
                .iter()
                .enumerate()
                .filter(|(j, _)| !moving.contains(j))
                .map(|(_, o)| boxed(o))
                .collect();
            let xs = || others.iter().map(|o| (o.0, o.2));
            let ys = || others.iter().map(|o| (o.1, o.3));
            let mut guides = (None, None);
            match &mut self.drag {
                Some(Drag::Move { start, moved }) => {
                    *moved += delta;
                    let (bx0, by0, bx1, by1) = start.iter().fold(
                        (i32::MAX, i32::MAX, i32::MIN, i32::MIN),
                        |(a, b, c, d), (_, (x, y, w, h))| {
                            (a.min(*x), b.min(*y), c.max(x + w), d.max(y + h))
                        },
                    );
                    let (bw, bh) = (bx1 - bx0, by1 - by0);
                    let mut x = bx0 + moved.x.round() as i32;
                    let mut y = by0 + moved.y.round() as i32;
                    if snap {
                        (x, y) = (snap16(x), snap16(y));
                    } else if magnets {
                        let (dx, gx) = magnet(x, bw, xs(), SCREEN_W);
                        let (dy, gy) = magnet(y, bh, ys(), SCREEN_H);
                        (x, y) = (x + dx, y + dy);
                        guides = (gx, gy);
                    }
                    // keep at least 8 px of the selection on the screen
                    let x = x.clamp(8 - bw, SCREEN_W - 8);
                    let y = y.clamp(8 - bh, SCREEN_H - 8);
                    for &(j, (sx, sy, _, _)) in start.iter() {
                        if let Some(w) = page.widgets.get_mut(j) {
                            (w.x, w.y) = (sx + x - bx0, sy + y - by0);
                        }
                    }
                }
                Some(Drag::Resize {
                    index,
                    start,
                    handle,
                    moved,
                }) => {
                    *moved += delta;
                    let (sx, sy, sw, sh) = *start;
                    let (dx, dy) = (moved.x.round() as i32, moved.y.round() as i32);
                    let mut edge = |v: i32, horizontal: bool| -> i32 {
                        if snap {
                            return snap16(v);
                        }
                        if !magnets {
                            return v;
                        }
                        let (d, g) = if horizontal {
                            magnet(v, 0, xs(), SCREEN_W)
                        } else {
                            magnet(v, 0, ys(), SCREEN_H)
                        };
                        if g.is_some() {
                            if horizontal {
                                guides.0 = g;
                            } else {
                                guides.1 = g;
                            }
                        }
                        v + d
                    };
                    let (mut x0, mut y0, mut x1, mut y1) = (sx, sy, sx + sw, sy + sh);
                    if handle.left {
                        x0 = edge(sx + dx, true);
                    }
                    if handle.right {
                        x1 = edge(sx + sw + dx, true);
                    }
                    if handle.top {
                        y0 = edge(sy + dy, false);
                    }
                    if handle.bottom {
                        y1 = edge(sy + sh + dy, false);
                    }
                    const MIN: i32 = 4;
                    if x1 - x0 < MIN {
                        if handle.left {
                            x0 = x1 - MIN;
                        } else {
                            x1 = x0 + MIN;
                        }
                    }
                    if y1 - y0 < MIN {
                        if handle.top {
                            y0 = y1 - MIN;
                        } else {
                            y1 = y0 + MIN;
                        }
                    }
                    if let Some(w) = page.widgets.get_mut(*index) {
                        (w.x, w.y, w.w, w.h) = (x0, y0, x1 - x0, y1 - y0);
                    }
                }
                Some(Drag::Band { base, from, to }) => {
                    if let Some(p) = resp.interact_pointer_pos() {
                        *to = to_dev(p);
                    }
                    let r = (
                        from.0.min(to.0).floor() as i32,
                        from.1.min(to.1).floor() as i32,
                        from.0.max(to.0).ceil() as i32,
                        from.1.max(to.1).ceil() as i32,
                    );
                    let mut sel = base.clone();
                    for i in layout::in_rect(&page.widgets, r) {
                        if !sel.contains(&i) {
                            sel.push(i);
                        }
                    }
                    self.selection = sel;
                }
                None => {}
            }
            self.guides = guides;
        }
        if resp.drag_stopped() {
            self.drag = None;
            self.guides = (None, None);
        }
        // preview only: selection rectangle, resize handles, magnet guides
        if let Some(Drag::Band { from, to, .. }) = &self.drag {
            let r = Rect::from_two_pos(to_screen(from.0, from.1), to_screen(to.0, to.1));
            painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(0, 200, 255, 30));
            let stroke = Stroke::new(1.0, Color32::from_rgb(0, 200, 255));
            painter.rect_stroke(r, 0.0, stroke, StrokeKind::Inside);
        }
        for (h, kind) in handles(&page.widgets, &self.selection) {
            let r = Rect::from_center_size(h, vec2(7.0, 7.0));
            painter.rect_filled(r, 1.0, Color32::WHITE);
            let stroke = Stroke::new(1.0, Color32::from_gray(40));
            painter.rect_stroke(r, 1.0, stroke, StrokeKind::Inside);
            if resp
                .hover_pos()
                .is_some_and(|p| (h.x - p.x).abs() <= 6.0 && (h.y - p.y).abs() <= 6.0)
            {
                ui.ctx().set_cursor_icon(kind.cursor());
            }
        }
        let guide = Stroke::new(1.0, Color32::from_rgb(0xff, 0x40, 0xd0));
        if let Some(gx) = self.guides.0 {
            let px = rect.min.x + gx as f32 * scale;
            painter.line_segment([pos2(px, rect.min.y), pos2(px, rect.max.y)], guide);
        }
        if let Some(gy) = self.guides.1 {
            let py = rect.min.y + gy as f32 * scale;
            painter.line_segment([pos2(rect.min.x, py), pos2(rect.max.x, py)], guide);
        }
        if let Some(p) = resp.hover_pos() {
            let (px, py) = ((p.x - rect.min.x) / scale, (p.y - rect.min.y) / scale);
            painter.text(
                rect.left_bottom() + vec2(0.0, 12.0),
                egui::Align2::LEFT_TOP,
                format!("{:.0}, {:.0}", px.floor(), py.floor()),
                egui::FontId::monospace(12.0),
                Color32::GRAY,
            );
        }
    }

    /// The enabled tools that have a settings file: (folder, name) — one tab each.
    fn settings_tabs(&self) -> Vec<(String, String)> {
        let enabled = |m: &crate::tools::Module| {
            self.draft
                .tools
                .iter()
                .find(|t| t.enabled && t.command.trim().is_empty() && t.module == m.id)
                .map(|t| t.name.clone())
        };
        self.modules
            .iter()
            .filter(|m| m.settings.is_some() || !m.secrets.is_empty())
            .filter_map(|m| Some((m.id.clone(), enabled(m)?)))
            .collect()
    }

    /// « Données sensibles » of a module's tab: its `[[secret]]` fields (hidden ones as dots,
    /// shown on demand), written to its `.env` (owner only) by « Enregistrer », which restarts it.
    fn secrets_section(&mut self, ui: &mut Ui, m: &crate::tools::Module) {
        if self.secrets_edit.as_ref().is_none_or(|s| s.dir != m.dir) {
            let env = crate::tools::read_env(&m.dir);
            let value = |n: &str| env.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
            let values: Vec<String> = m.secrets.iter().map(|s| value(&s.name).unwrap_or_default()).collect();
            self.secrets_edit = Some(SecretsEdit {
                dir: m.dir.clone(),
                saved: values.clone(),
                values,
                shown: vec![false; m.secrets.len()],
            });
        }
        let Some(se) = &mut self.secrets_edit else {
            return;
        };
        ui.label(RichText::new(tr("Sensitive data")).strong());
        ui.weak(tf!("Kept in {path} (readable by you only), never in the config; given to the module when it starts. Do not share this file.", path = m.dir.join(crate::tools::ENV_FILE).display()));
        egui::Grid::new(("secrets", &m.id)).num_columns(3).show(ui, |ui| {
            for (i, s) in m.secrets.iter().enumerate() {
                let label = if s.label.is_empty() { s.name.as_str() } else { s.label.as_str() };
                let r = ui.label(label);
                if !s.description.is_empty() {
                    r.on_hover_text(&s.description);
                }
                let hide = s.hidden && !se.shown[i];
                ui.add(TextEdit::singleline(&mut se.values[i]).password(hide).desired_width(260.0));
                if s.hidden {
                    let eye = if se.shown[i] { "masquer" } else { "afficher" };
                    if ui.small_button(eye).clicked() {
                        se.shown[i] = !se.shown[i];
                    }
                }
                ui.end_row();
            }
        });
        let changed = se.values != se.saved;
        let mut saved = false;
        ui.horizontal(|ui| {
            if ui.add_enabled(changed, Button::new(tr("Save the sensitive data"))).clicked() {
                let pairs: Vec<(String, String)> = m
                    .secrets
                    .iter()
                    .zip(&se.values)
                    .map(|(s, v)| (s.name.clone(), v.trim().to_owned()))
                    .collect();
                match crate::tools::write_env(&m.dir, &pairs) {
                    Ok(()) => {
                        se.saved = se.values.clone();
                        saved = true;
                    }
                    Err(e) => self.note = format!("{}: {e}", m.dir.join(".env").display()),
                }
            }
            if changed {
                ui.colored_label(WARN, tr("changed, not saved yet"));
            }
        });
        if saved {
            // the module reads them at start: restart it
            let mut statuses = self.shared.tools.lock().unwrap();
            for t in self.draft.tools.iter().filter(|t| t.module == m.id && t.enabled) {
                statuses.entry(t.name.clone()).or_default().restart = true;
            }
            self.note = tf!("{module}: sensitive data saved, module restarted", module = m.name);
        }
        ui.add_space(8.0);
    }

    /// Right panel: tabs « Widgets » (page, widget list, inspector), « Commandes » (knobs,
    /// buttons, power of the page), « Appareil » (device-wide settings), « Modules », then one
    /// per enabled tool with a settings file (`[settings]` of its `module.toml`).
    fn side_panel(&mut self, ui: &mut Ui) {
        self.calibrating = false; // set again below while the calibration section is open
        let tool_tabs = self.settings_tabs();
        if self.tab == Tab::Settings && !tool_tabs.iter().any(|(id, _)| *id == self.settings_tab) {
            self.tab = Tab::Tools; // its tool was switched off
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in [
                (Tab::Widgets, tr("Widgets")),
                (Tab::Controls, tr("Controls")),
                (Tab::Device, tr("Device")),
                (Tab::Tools, tr("Modules")),
            ] {
                let text = RichText::new(label).size(16.0);
                if ui.selectable_label(self.tab == tab, text).clicked() {
                    self.tab = tab;
                }
            }
            for (id, name) in &tool_tabs {
                let on = self.tab == Tab::Settings && self.settings_tab == *id;
                let text = RichText::new(name).size(16.0).color(ACCENT);
                if ui
                    .selectable_label(on, text)
                    .on_hover_text(tr("module settings (file in its folder)"))
                    .clicked()
                {
                    self.tab = Tab::Settings;
                    self.settings_tab.clone_from(id);
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| match self.tab {
                Tab::Widgets => {
                    self.page_bar(ui);
                    self.tab_widgets(ui);
                }
                Tab::Controls => {
                    self.page_bar(ui);
                    self.tab_controls(ui);
                }
                Tab::Device => self.tab_device(ui),
                Tab::Tools => self.tab_tools(ui),
                Tab::Settings => self.tab_settings(ui),
            });
        // a list of the modules' commands or widgets was open: the modules « Load only when
        // shown » load meanwhile to offer theirs
        if ui.ctx().data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(WANT_MODULES))).is_some() {
            self.shared.want_all(Duration::from_secs(20));
        }
    }

    /// Tab of an enabled tool: its settings file (stream deck editor, or TOML text), written by
    /// « Enregistrer » (the tool then restarts by itself: its files changed).
    fn tab_settings(&mut self, ui: &mut Ui) {
        let Some(m) = self.modules.iter().find(|m| m.id == self.settings_tab).cloned() else {
            return;
        };
        let title = self
            .draft
            .tools
            .iter()
            .find(|t| t.module == m.id)
            .map_or(m.name.clone(), |t| t.name.clone());
        section(ui, &title);
        // « Load only when shown »: loaded while its settings are open (the lists it gives them)
        self.shared.want(&title, Duration::from_secs(2));
        if let Some(g) = m.settings.as_ref().map(|s| s.guide.as_str()).filter(|g| !g.is_empty()) {
            crate::settingsform::guide(ui, g);
        }
        if !m.secrets.is_empty() {
            self.secrets_section(ui, &m);
        }
        let Some(path) = m.settings_path() else {
            return;
        };
        let stale = |s: &SettingsEdit| {
            // the file changed by hand while nothing is being edited here: read it again
            !s.changed()
                && s.checked.elapsed() > Duration::from_secs(1)
                && std::fs::read_to_string(&s.path).unwrap_or_default() != s.saved
        };
        if self.settings_edit.as_ref().is_none_or(|s| s.path != path || stale(s)) {
            self.settings_edit = SettingsEdit::open(&m);
        }
        let ctx = self.action_ctx();
        let Some(se) = &mut self.settings_edit else {
            return;
        };
        if se.checked.elapsed() > Duration::from_secs(1) {
            se.checked = Instant::now();
        }
        ui.label(RichText::new(tr("Settings")).strong());
        ui.weak(tf!("Module settings: {path}. \"Save\" writes it, then the module restarts by itself.", path = path.display()));
        match &mut se.form {
            SettingsForm::Deck { edit, .. } => deck_file_edit(ui, edit, &ctx),
            SettingsForm::Fields { fields, edit, .. } => {
                let choices = self.shared.api.tool_choices(&title);
                crate::settingsform::form(ui, "settings", fields, edit, &choices);
            }
            SettingsForm::Text(text) => {
                if let Err(e) = toml::from_str::<toml::Table>(text) {
                    ui.colored_label(ERROR, tf!("invalid TOML: {e}", e = e));
                }
                ui.add(
                    TextEdit::multiline(text)
                        .code_editor()
                        .desired_rows(24)
                        .desired_width(f32::INFINITY),
                );
            }
        }
        ui.add_space(6.0);
        let changed = se.changed();
        let mut note = None;
        ui.horizontal(|ui| {
            if ui.add_enabled(changed, Button::new(tr("Save"))).clicked() {
                match se.save() {
                    Ok(()) => note = Some(tf!("{title}: settings saved", title = title)),
                    Err(e) => se.error = Some(e),
                }
            }
            if ui
                .add_enabled(changed, Button::new(tr("Discard the changes")))
                .clicked()
            {
                *se = SettingsEdit::open(&m).unwrap_or_else(|| unreachable!("it has settings"));
            }
            if ui.button(tr("Open the folder")).clicked() {
                open_path(&m.dir);
            }
            if changed {
                ui.colored_label(WARN, tr("changed, not saved yet"));
            }
        });
        if let Some(e) = &se.error {
            ui.colored_label(ERROR, e);
        }
        if let Some(n) = note {
            self.note = n;
        }
    }

    /// Tools (tab « Modules »): the sub-folders of the tools folder, read again with « Rafraîchir
    /// les modules » — switch, state, arguments, folder, output — and the commands of the config.
    fn tab_tools(&mut self, ui: &mut Ui) {
        use crate::config::Tool;
        section(ui, tr("Modules"));
        let dir = crate::tools::user_dir();
        ui.weak(tf!("A module = a subfolder of {dir} (module.toml + program), read on the fly; changing its files restarts the module. Make your own: docs/app/modules.md.", dir = dir.display()));
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(tr("⟳ Refresh the modules"))
                .on_hover_text(tr("read the folder again: modules added, changed or removed"))
                .clicked()
            {
                self.modules = crate::tools::modules();
            }
            if ui.button(tr("Open the folder")).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                open_path(&dir);
            }
            if ui
                .button(tr("Reinstall the shipped modules"))
                .on_hover_text(
                    tr("puts the shipped modules back in their original version (changes made in their folders are lost)"),
                )
                .clicked()
            {
                self.note = match crate::tools::install_shipped(true) {
                    Ok(n) => tf!("{n} shipped modules reinstalled", n = n),
                    Err(e) => tf!("shipped modules: {e}", e = e),
                };
                self.modules = crate::tools::modules();
            }
        });
        let unique = |base: &str, tools: &[Tool]| {
            (1..)
                .map(|n| if n == 1 { base.to_owned() } else { format!("{base} {n}") })
                .find(|name| tools.iter().all(|t| &t.name != name))
                .unwrap_or_default()
        };
        let statuses = self.shared.tools.lock().unwrap().clone();
        let modules = self.modules.clone();
        // one card per folder of the tools folder
        for m in &modules {
            let entry = self
                .draft
                .tools
                .iter()
                .position(|t| t.command.trim().is_empty() && t.module == m.id);
            card(ui, &m.name, |ui| {
                ui.horizontal(|ui| {
                    let mut on = entry.is_some_and(|i| self.draft.tools[i].enabled);
                    if switch(ui, &mut on).on_hover_text(tr("start / stop")).changed() {
                        match entry {
                            Some(i) => self.draft.tools[i].enabled = on,
                            None => {
                                let name = unique(&m.name, &self.draft.tools);
                                self.draft.tools.push(Tool {
                                    name,
                                    enabled: true,
                                    module: m.id.clone(),
                                    args: m.args.clone(),
                                    command: String::new(),
                                    keep_awake: false,
                                    pause_hidden: true,
                                });
                            }
                        }
                    }
                    let st = entry.and_then(|i| statuses.get(&self.draft.tools[i].name));
                    tool_state(ui, st, on);
                });
                if !m.description.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&m.description).weak()).wrap());
                }
                egui::Grid::new(("tool", &m.id)).num_columns(2).show(ui, |ui| {
                    if let Some(i) = entry {
                        let t = &mut self.draft.tools[i];
                        ui.label(tr("Arguments")).on_hover_text(tr("separated by spaces"));
                        let text = self
                            .args_text
                            .entry(m.id.clone())
                            .or_insert_with(|| t.args.join(" "));
                        if ui.text_edit_singleline(text).changed() {
                            t.args = text.split_whitespace().map(str::to_owned).collect();
                        }
                        ui.end_row();
                        keep_awake_row(ui, &mut t.keep_awake);
                        pause_hidden_row(ui, &mut t.pause_hidden);
                    }
                    ui.label(tr("Folder"));
                    ui.horizontal(|ui| {
                        ui.monospace(m.dir.display().to_string());
                        if ui.small_button(tr("Open")).clicked() {
                            open_path(&m.dir);
                        }
                    });
                    ui.end_row();
                });
                if let Some(i) = entry {
                    tool_log(ui, &m.id, statuses.get(&self.draft.tools[i].name));
                }
            });
        }
        // config entries without a folder: commands, or folders that are gone
        let mut remove = None;
        for (i, t) in self.draft.tools.iter_mut().enumerate() {
            let command = !t.command.trim().is_empty();
            if !command && modules.iter().any(|m| m.id == t.module) {
                continue;
            }
            card(ui, &t.name.clone(), |ui| {
                ui.horizontal(|ui| {
                    switch(ui, &mut t.enabled).on_hover_text(tr("start / stop"));
                    tool_state(ui, statuses.get(&t.name), t.enabled);
                    if ui.small_button(tr("Remove")).clicked() {
                        remove = Some(i);
                    }
                });
                if command {
                    egui::Grid::new(("command", i)).num_columns(2).show(ui, |ui| {
                        ui.label(tr("Name"));
                        ui.text_edit_singleline(&mut t.name);
                        ui.end_row();
                        ui.label(tr("Command")).on_hover_text(tr("run by sh -c"));
                        ui.add(TextEdit::singleline(&mut t.command).desired_width(f32::INFINITY));
                        ui.end_row();
                        keep_awake_row(ui, &mut t.keep_awake);
                        pause_hidden_row(ui, &mut t.pause_hidden);
                    });
                } else {
                    ui.colored_label(
                        Color32::from_rgb(0xe0, 0x50, 0x40),
                        tf!("folder \"{folder}\" not found (Refresh the modules?)", folder = t.module),
                    );
                }
                tool_log(ui, &t.name, statuses.get(&t.name));
            });
        }
        if let Some(i) = remove {
            self.draft.tools.remove(i);
        }
        if modules.is_empty() {
            ui.weak(tr("No module in the folder: \"Reinstall the shipped modules\" or make one."));
        }
        ui.add_space(6.0);
        let mut created = None;
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.new_tool)
                    .hint_text(tr("name of the new module"))
                    .desired_width(180.0),
            );
            let name = self.new_tool.trim().to_owned();
            if ui
                .add_enabled(!name.is_empty(), Button::new(tr("Create a module")))
                .on_hover_text(tr("a subfolder with module.toml and main.py (a template to change), opened afterwards"))
                .clicked()
            {
                created = Some(name);
            }
            if ui
                .button(tr("+ Command"))
                .on_hover_text(tr("any program already installed, run by sh -c"))
                .clicked()
            {
                let name = unique(tr("Command"), &self.draft.tools);
                self.draft.tools.push(Tool {
                    name,
                    enabled: false,
                    module: String::new(),
                    args: Vec::new(),
                    command: tr("python3 /path/to/my_module.py").into(),
                    keep_awake: false,
                    pause_hidden: false, // any program: it may have no page to show
                });
            }
        });
        if let Some(name) = created {
            match crate::tools::create_tool(&name) {
                Ok(id) => {
                    self.modules = crate::tools::modules();
                    let unique_name = unique(&name, &self.draft.tools);
                    self.draft.tools.push(Tool {
                        name: unique_name,
                        enabled: true,
                        module: id.clone(),
                        args: Vec::new(),
                        command: String::new(),
                        keep_awake: false,
                        pause_hidden: true,
                    });
                    self.new_tool.clear();
                    open_path(&dir.join(id));
                }
                Err(e) => self.note = tf!("module: {e}", e = e),
            }
        }
        ui.weak(tr("Changes apply at once (remember to Save to keep them)."));
    }

    fn tab_device(&mut self, ui: &mut Ui) {
        section(ui, tr("Language"));
        let code = &mut self.draft.language;
        let name = |code: &str, langs: &[(String, String)]| {
            langs.iter().find(|(c, _)| c == code).map_or(code.to_owned(), |(_, n)| n.clone())
        };
        let shown = name(code, &self.languages);
        let languages = &mut self.languages;
        egui::ComboBox::from_id_salt("language")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                *languages = crate::i18n::available(); // a file added meanwhile
                for (c, n) in languages.iter() {
                    ui.selectable_value(code, c.clone(), n);
                }
            })
            .response
            .on_hover_text(tr(
                "window, messages, texts drawn on the device and modules; other languages: a \
                 translation file in ~/.config/hsm01/i18n/ (docs/app/companion.md)",
            ));
        section(ui, tr("Screen"));
        egui::Grid::new("screen").num_columns(2).show(ui, |ui| {
            ui.label(tr("Brightness"));
            ui.add(egui::Slider::new(&mut self.draft.brightness, 0..=100).suffix(" %"));
            ui.end_row();
        });
        section(ui, tr("Knobs"));
        egui::Grid::new("knobs_speed")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label(tr("Sensitivity"));
                ui.add(
                    egui::Slider::new(&mut self.draft.counts_per_percent, 1..=64)
                        .suffix(tr(" steps / %")),
                )
                .on_hover_text(tr("knob steps for 1 % of volume (512 steps a turn)"));
                ui.end_row();
            });
        section(ui, tr("Standby"));
        let standby = &mut self.draft.standby;
        ui.checkbox(&mut standby.enabled, tr("Turn the screen off when idle"));
        egui::Grid::new("standby").num_columns(2).show(ui, |ui| {
            ui.label(tr("After"));
            let delay = DragValue::new(&mut standby.after_secs)
                .range(STANDBY_MIN_SECS..=4 * 3600)
                .speed(1.0)
                .custom_formatter(|v, _| {
                    let v = v.round() as u32;
                    match (v / 60, v % 60) {
                        (0, s) => format!("{s} s"),
                        (m, 0) => format!("{m} min"),
                        (m, s) => format!("{m} min {s} s"),
                    }
                });
            ui.add_enabled(standby.enabled, delay)
                .on_hover_text(tr("in seconds, 10 s minimum"));
            ui.end_row();
            ui.label(tr("Fade"));
            let fade = DragValue::new(&mut standby.fade_secs)
                .range(0.0..=30.0)
                .speed(0.1)
                .max_decimals(1)
                .suffix(" s");
            ui.add_enabled(standby.enabled, fade);
            ui.end_row();
        });
        ui.weak(
            tr("An action (knob, button, change here) turns the screen back on; that first action is not run."),
        );

        section(ui, tr("Notifications"));
        let n = &mut self.draft.notifications;
        ui.checkbox(&mut n.enabled, tr("Show desktop notifications on the screen"))
            .on_hover_text(tr("in a window at the top of the screen; the desktop still shows them"));
        ui.add_enabled_ui(n.enabled, |ui| {
            egui::Grid::new("notifications")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label(tr("Applications"))
                        .on_hover_text(tr("part of the name is enough, any case; none = all"));
                    ui.horizontal_wrapped(|ui| {
                        let mut remove = None;
                        for (i, app) in n.apps.iter_mut().enumerate() {
                            ui.add(TextEdit::singleline(app).desired_width(90.0));
                            if ui.small_button("×").on_hover_text(tr("remove")).clicked() {
                                remove = Some(i);
                            }
                        }
                        if let Some(i) = remove {
                            n.apps.remove(i);
                        }
                        if ui.small_button("+").on_hover_text(tr("add an application")).clicked() {
                            n.apps.push(String::new());
                        }
                    });
                    ui.end_row();
                    ui.label(tr("Duration"));
                    ui.add(
                        DragValue::new(&mut n.duration_secs)
                            .range(2.0..=60.0)
                            .speed(0.1)
                            .max_decimals(1)
                            .suffix(" s"),
                    );
                    ui.end_row();
                    ui.label(tr("In standby"));
                    ui.checkbox(&mut n.wake, tr("turn the screen on (otherwise the notification is ignored)"));
                    ui.end_row();
                });
            if ui.button(tr("Test")).clicked() {
                crate::notify::test(&self.shared);
            }
            if ui
                .button(tr("Test a pill"))
                .on_hover_text(tr("short alert of the modules (\"pill\" style: outline, dots, text)"))
                .clicked()
            {
                crate::notify::test_pill(&self.shared);
            }
        });
        ui.weak(
            tr("A click on a button or a knob closes the window without triggering anything; turning a knob sets the volume as usual. The content is never saved."),
        );

        section(ui, tr("Open API"));
        ui.weak(
            tr("Programs (Python, shell, web page…) show alerts and pages on the Stream 100 and receive knobs and buttons. Protocol: docs/app/api.md; examples: app/clients/python/examples."),
        );
        egui::Grid::new("api").num_columns(2).show(ui, |ui| {
            ui.label(tr("Socket"));
            ui.monospace(crate::api::socket_path().display().to_string());
            ui.end_row();
            let api = &mut self.draft.api;
            ui.label("WebSocket");
            ui.horizontal(|ui| {
                switch(ui, &mut api.websocket)
                    .on_hover_text(tr("Local WebSocket (127.0.0.1), switched on or off at once"));
                ui.label("port");
                ui.add_enabled(api.websocket, DragValue::new(&mut api.port).range(1024..=65535));
            });
            ui.end_row();
            ui.label("");
            let running = self
                .shared
                .api
                .websocket
                .lock()
                .unwrap()
                .as_ref()
                .map(|w| (w.port, w.token.clone()));
            match &running {
                Some((port, _)) => ui.colored_label(
                    Color32::from_rgb(0x00, 0xb4, 0xa8),
                    tf!("active on 127.0.0.1:{port}", port = port),
                ),
                None => ui.weak(tr("stopped")),
            };
            ui.end_row();
            if let Some((port, token)) = running {
                ui.label("");
                ui.horizontal(|ui| {
                    if ui
                        .button(tr("Copy the address"))
                        .on_hover_text(tr("holds the secret token: do not share it"))
                        .clicked()
                    {
                        ui.ctx()
                            .copy_text(format!("ws://127.0.0.1:{port}/?token={token}"));
                    }
                    if ui
                        .button(tr("New token"))
                        .on_hover_text(
                            tr("the old address will no longer work (open connections stay)"),
                        )
                        .clicked()
                        && let Err(e) = crate::api::new_token(&self.shared)
                    {
                        self.note = tf!("token: {e}", e = e);
                    }
                });
                ui.end_row();
            }
        });

        section(ui, tr("Colour calibration"));
        let open = egui::CollapsingHeader::new(tr("Adjust"))
            .default_open(false)
            .show(ui, |ui| self.calibration(ui))
            .body_returned
            .is_some();
        self.calibrating = open;
    }

    /// Per-channel gain and gamma, compared on the test pattern: the Stream 100 shows it corrected,
    /// the preview shows the reference.
    fn calibration(&mut self, ui: &mut Ui) {
        ui.weak(
            tr("Compare the Stream 100's test chart with the preview (reference). Gain: white balance (lower the channel in excess). Gamma: midtones (> 1 brightens)."),
        );
        ui.checkbox(&mut self.show_pattern, tr("Test chart on the Stream 100"));
        ui.checkbox(&mut self.link_rgb, tr("Link red, green and blue"));
        let link = self.link_rgb;
        let cal = &mut self.draft.calibration;
        let before = cal.clone();
        egui::Grid::new("calibration")
            .num_columns(4)
            .show(ui, |ui| {
                ui.label("");
                ui.colored_label(Color32::from_rgb(0xe0, 0x50, 0x50), tr("Red"));
                ui.colored_label(Color32::from_rgb(0x50, 0xc0, 0x50), tr("Green"));
                ui.colored_label(Color32::from_rgb(0x60, 0x90, 0xf0), tr("Blue"));
                ui.end_row();
                ui.label(tr("Gain"));
                for v in &mut cal.gain {
                    ui.add(
                        DragValue::new(v)
                            .range(0.3..=1.5)
                            .speed(0.005)
                            .max_decimals(3),
                    );
                }
                ui.end_row();
                ui.label(tr("Gamma"));
                for v in &mut cal.gamma {
                    ui.add(
                        DragValue::new(v)
                            .range(0.4..=2.5)
                            .speed(0.005)
                            .max_decimals(3),
                    );
                }
                ui.end_row();
            });
        if link {
            let follow = |now: &mut [f32; 3], old: &[f32; 3], range: (f32, f32)| {
                if let Some(c) = (0..3).find(|&c| now[c] != old[c]) {
                    let delta = now[c] - old[c];
                    for (k, v) in now.iter_mut().enumerate() {
                        if k != c {
                            *v = (old[k] + delta).clamp(range.0, range.1);
                        }
                    }
                }
            };
            follow(&mut cal.gain, &before.gain, (0.3, 1.5));
            follow(&mut cal.gamma, &before.gamma, (0.4, 2.5));
        }
        if ui.button(tr("Reset")).clicked() {
            *cal = Default::default();
        }
    }

    /// What the action editors list: pages, tools, commands of the running tools.
    fn action_ctx(&self) -> ActionCtx {
        ActionCtx {
            n_pages: self.draft.pages.len(),
            tools: self.draft.tools.iter().map(|t| t.name.clone()).collect(),
            commands: self.shared.api.tool_commands(),
            tool_pages: self.shared.api.tool_pages(),
        }
    }

    fn tab_controls(&mut self, ui: &mut Ui) {
        let inv = self.shared.inventory.lock().unwrap().clone();
        let ctx = self.action_ctx();
        let Some(page) = self.draft.pages.get_mut(self.page) else {
            return;
        };
        section(ui, tr("Knobs"));
        for (k, knob) in page.knobs.iter_mut().enumerate() {
            let id = IdSalt::new(("knob", k));
            card(ui, &tf!("Knob {n}", n = k + 1), |ui| {
                egui::Grid::new(("knob_grid", k))
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(tr("Turn"));
                        ui.horizontal(|ui| {
                            let cmds = Some(ctx.commands.as_slice());
                            target_edit(ui, IdSalt::new((id, "target")), &mut knob.target, &inv, cmds)
                        });
                        ui.end_row();
                        ui.label(tr("Click"));
                        ui.horizontal(|ui| {
                            let k = k as u8 + 1;
                            action_edit(ui, IdSalt::new((id, "click")), &mut knob.click, k, &ctx)
                        });
                        ui.end_row();
                        ui.label(tr("Hold")).on_hover_text(hold_help());
                        ui.horizontal(|ui| {
                            let k = k as u8 + 1;
                            action_edit(ui, IdSalt::new((id, "hold")), &mut knob.hold, k, &ctx)
                        });
                        ui.end_row();
                    });
            });
        }
        section(ui, tr("Buttons"));
        for (b, button) in page.buttons.iter_mut().enumerate() {
            let id = IdSalt::new(("button", b));
            card(ui, &tf!("Button {n}", n = b + 1), |ui| {
                egui::Grid::new(("button_grid", b))
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(tr("Click"));
                        ui.horizontal(|ui| {
                            action_edit(
                                ui,
                                IdSalt::new((id, "action")),
                                &mut button.action,
                                1,
                                &ctx,
                            )
                        });
                        ui.end_row();
                        ui.label(tr("Hold")).on_hover_text(hold_help());
                        ui.horizontal(|ui| {
                            action_edit(ui, IdSalt::new((id, "hold")), &mut button.hold, 1, &ctx)
                        });
                        ui.end_row();
                        ui.label("LED");
                        ui.horizontal(|ui| led_edit(ui, IdSalt::new((id, "led")), &mut button.led, &ctx.commands));
                        ui.end_row();
                    });
            });
        }
        card(ui, tr("Power button"), |ui| {
            egui::Grid::new("power_grid").num_columns(2).show(ui, |ui| {
                ui.label(tr("Click"));
                ui.horizontal(|ui| {
                    action_edit(ui, IdSalt::new("power"), &mut page.power, 1, &ctx)
                });
                ui.end_row();
                ui.label(tr("Hold")).on_hover_text(hold_help());
                ui.horizontal(|ui| {
                    action_edit(
                        ui,
                        IdSalt::new("power_hold"),
                        &mut page.power_hold,
                        1,
                        &ctx,
                    )
                });
                ui.end_row();
            });
            ui.weak(tr("Keep \"next page\" on at least one control to change pages."));
        });
    }

    fn tab_widgets(&mut self, ui: &mut Ui) {
        let inv = self.shared.inventory.lock().unwrap().clone();
        let actx = self.action_ctx();
        let Some(page) = self.draft.pages.get_mut(self.page) else {
            return;
        };
        egui::Grid::new("page_props").num_columns(2).show(ui, |ui| {
            ui.label(tr("Name"));
            ui.text_edit_singleline(&mut page.name);
            ui.end_row();
            ui.label(tr("Background"));
            color_edit(ui, &mut page.background);
            ui.end_row();
        });
        let page_idx = self.page;
        wallpaper_edit(ui, page, page_idx, &self.picked);

        section(ui, tr("Widgets"));
        let shift = ui.ctx().input(|i| i.modifiers.shift);
        for (i, w) in page.widgets.iter().enumerate() {
            let label = format!("{:>2}.  {}", i + 1, describe(w));
            let picked = ui
                .selectable_label(self.selection.contains(&i), label)
                .on_hover_text(tr("Shift+click: add to / remove from the selection"));
            if picked.clicked() {
                if shift {
                    toggle_in(&mut self.selection, i);
                } else {
                    self.selection = vec![i];
                }
            }
        }
        ui.add_space(4.0);
        ui.menu_button(tr("+ Add a widget"), |ui| {
            for k in WidgetKind::NAMES {
                if ui.button(kind_label(k)).clicked() {
                    if let Some(mut kind) = WidgetKind::template(k) {
                        if let (WidgetKind::Tool { tool, .. }, Some(first)) =
                            (&mut kind, actx.tools.first())
                        {
                            tool.clone_from(first);
                        }
                        let (w, h) = default_size(&kind);
                        let (mut x, y) = ((SCREEN_W - w) / 2, (SCREEN_H - h) / 2);
                        // a deck: the first knob without one, its column right above it
                        if let WidgetKind::Deck { knob, .. } = &mut kind {
                            let used = |k: u8| {
                                page.widgets.iter().any(|w| {
                                    matches!(w.kind, WidgetKind::Deck { knob, .. } if knob == k)
                                })
                            };
                            *knob = (1..=4).find(|&k| !used(k)).unwrap_or(1);
                            x = (*knob as i32 - 1) * (SCREEN_W / 4) + (SCREEN_W / 4 - w) / 2;
                        }
                        page.widgets.push(Widget {
                            x,
                            y,
                            w,
                            h,
                            frame: None,
                            kind,
                        });
                        self.selection = vec![page.widgets.len() - 1];
                    }
                    ui.close();
                }
            }
        });
        ui.weak(
            tr("Preview: click = select, Shift+click = add, drag in empty space = selection box, handles = size. Keyboard: arrows (Shift = 10 px), Delete, Ctrl+D (duplicate), Ctrl+A (all), Esc."),
        );

        self.selection.retain(|&i| i < page.widgets.len());
        if self.selection.len() > 1 {
            let sel = self.selection.clone();
            let primary = *sel.last().unwrap();
            section(ui, &tf!("{n} widgets selected", n = sel.len()));
            egui::Grid::new("multi").num_columns(2).show(ui, |ui| {
                ui.label(tr("Align"));
                ui.horizontal_wrapped(|ui| {
                    for (edge, label) in [
                        (Edge::Left, "gauche"),
                        (Edge::CenterX, "centre"),
                        (Edge::Right, "droite"),
                        (Edge::Top, "haut"),
                        (Edge::CenterY, "milieu"),
                        (Edge::Bottom, "bas"),
                    ] {
                        if ui.button(label).clicked() {
                            layout::align(&mut page.widgets, &sel, edge);
                        }
                    }
                })
                .response
                .on_hover_text(tr("on the box around the selection"));
                ui.end_row();
                ui.label(tr("Distribute"));
                ui.horizontal(|ui| {
                    let ok = sel.len() >= 3;
                    if ui.add_enabled(ok, Button::new(tr("horizontally"))).clicked() {
                        layout::distribute(&mut page.widgets, &sel, true);
                    }
                    if ui.add_enabled(ok, Button::new(tr("vertically"))).clicked() {
                        layout::distribute(&mut page.widgets, &sel, false);
                    }
                })
                .response
                .on_hover_text(tr("equal gaps between the widgets (3 at least)"));
                ui.end_row();
                ui.label(tr("Same size"));
                ui.horizontal(|ui| {
                    if ui.button(tr("width")).clicked() {
                        layout::same_size(&mut page.widgets, &sel, primary, true);
                    }
                    if ui.button(tr("height")).clicked() {
                        layout::same_size(&mut page.widgets, &sel, primary, false);
                    }
                })
                .response
                .on_hover_text(tr("as the last widget chosen"));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button(tr("Duplicate")).clicked() {
                    self.selection = layout::duplicate(&mut page.widgets, &sel);
                }
                if ui.button(RichText::new(tr("Delete")).color(ERROR)).clicked() {
                    layout::delete(&mut page.widgets, &sel);
                    self.selection.clear();
                }
            });
            return;
        }
        let Some(i) = self.selection.first().copied() else {
            ui.add_space(8.0);
            ui.weak(tr("Select a widget in the list or on the preview."));
            return;
        };
        section(ui, &describe(&page.widgets[i]));
        ui.horizontal_wrapped(|ui| {
            let n = page.widgets.len();
            if ui.button(tr("Duplicate")).clicked() {
                self.selection = layout::duplicate(&mut page.widgets, &[i]);
            } else if ui
                .add_enabled(i > 0, Button::new(tr("Back")))
                .on_hover_text(tr("drawn earlier (below)"))
                .clicked()
            {
                page.widgets.swap(i, i - 1);
                self.selection = vec![i - 1];
            } else if ui
                .add_enabled(i + 1 < n, Button::new(tr("Forward")))
                .on_hover_text(tr("drawn later (above)"))
                .clicked()
            {
                page.widgets.swap(i, i + 1);
                self.selection = vec![i + 1];
            } else if ui.button(RichText::new(tr("Delete")).color(ERROR)).clicked() {
                page.widgets.remove(i);
                self.selection.clear();
            }
        });
        if let Some(w) = self
            .selection
            .first()
            .and_then(|&i| page.widgets.get_mut(i))
        {
            widget_edit(ui, w, &inv, &actx);
        }
    }

    /// Keyboard editing of the selection (when no text field has the keyboard): arrows move by
    /// 1 px (Shift: 10), Delete / Backspace remove, Ctrl+D duplicates, Ctrl+A selects all,
    /// Escape deselects.
    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.tab != Tab::Widgets {
            return;
        }
        let Some(page) = self.draft.pages.get_mut(self.page) else {
            return;
        };
        use egui::{Key, Modifiers as M};
        let key = |m: M, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if key(M::COMMAND, Key::A) {
            self.selection = (0..page.widgets.len()).collect();
        }
        if self.selection.is_empty() {
            return;
        }
        if key(M::NONE, Key::Escape) {
            self.selection.clear();
            return;
        }
        if key(M::NONE, Key::Delete) || key(M::NONE, Key::Backspace) {
            layout::delete(&mut page.widgets, &self.selection);
            self.selection.clear();
            return;
        }
        if key(M::COMMAND, Key::D) {
            self.selection = layout::duplicate(&mut page.widgets, &self.selection);
            return;
        }
        for (k, dx, dy) in [
            (Key::ArrowLeft, -1, 0),
            (Key::ArrowRight, 1, 0),
            (Key::ArrowUp, 0, -1),
            (Key::ArrowDown, 0, 1),
        ] {
            if key(M::SHIFT, k) {
                layout::nudge(&mut page.widgets, &self.selection, dx * 10, dy * 10);
            } else if key(M::NONE, k) {
                layout::nudge(&mut page.widgets, &self.selection, dx, dy);
            }
        }
    }

    fn page_bar(&mut self, ui: &mut Ui) {
        let n = self.draft.pages.len();
        let mut chosen = self.page;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Page")).strong());
            let title = |i: usize, p: &Page| format!("{}. {}", i + 1, p.name);
            let current = self
                .draft
                .pages
                .get(self.page)
                .map_or_else(String::new, |p| title(self.page, p));
            ComboBox::from_id_salt("page")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, p) in self.draft.pages.iter().enumerate() {
                        ui.selectable_value(&mut chosen, i, title(i, p));
                    }
                });
            if ui.button(tr("New")).clicked() {
                // same knob and button bindings as the current page, no widget
                let mut p = self.draft.pages.get(self.page).cloned().unwrap_or(Page {
                    name: String::new(),
                    background: Color(0, 0, 0),
                    knobs: Vec::new(),
                    buttons: Vec::new(),
                    power: Action::NextPage,
                    power_hold: Action::None,
                    wallpaper: None,
                    widgets: Vec::new(),
                });
                p.name = tf!("Page {n}", n = n + 1);
                p.widgets.clear();
                p.normalise();
                chosen = (self.page + 1).min(n);
                self.draft.pages.insert(chosen, p);
            }
            if ui.add_enabled(n > 0, Button::new(tr("Duplicate"))).clicked() {
                let mut p = self.draft.pages[self.page].clone();
                p.name.push_str(" (copie)");
                chosen = self.page + 1;
                self.draft.pages.insert(chosen, p);
            }
            if ui.add_enabled(n > 1, Button::new(tr("Delete"))).clicked() {
                self.draft.pages.remove(self.page);
                chosen = self.page.min(n - 2);
            }
            if ui
                .add_enabled(self.page > 0, Button::new("<"))
                .on_hover_text(tr("move the page to the left"))
                .clicked()
            {
                self.draft.pages.swap(self.page, self.page - 1);
                chosen = self.page - 1;
            }
            if ui
                .add_enabled(self.page + 1 < n, Button::new(">"))
                .on_hover_text(tr("move the page to the right"))
                .clicked()
            {
                self.draft.pages.swap(self.page, self.page + 1);
                chosen = self.page + 1;
            }
        });
        if chosen != self.page || n != self.draft.pages.len() {
            self.set_page(chosen);
        }
    }
}

impl Drop for Gui {
    /// Window closed: no more outlines on the device.
    fn drop(&mut self) {
        *self.shared.overlay.lock().unwrap() = Overlay::default();
    }
}

impl eframe::App for Gui {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

impl Gui {
    fn show(&mut self, ui: &mut Ui) {
        self.sync();
        // a module.toml changed (module updated in place): its card and settings form again
        if self.modules_stamp.1.elapsed() > Duration::from_secs(2) {
            let stamp = crate::tools::descriptions_stamp();
            if stamp != self.modules_stamp.0 {
                self.modules = crate::tools::modules();
            }
            self.modules_stamp = (stamp, Instant::now());
        }
        self.window_requests(ui.ctx());
        if ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))
        {
            self.save();
        }
        // undo / redo, unless a text field has the keyboard (it has its own Ctrl+Z)
        let ctx = ui.ctx().clone();
        if !ctx.egui_wants_keyboard_input() {
            let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
            if ctx.input_mut(|i| {
                i.consume_key(shift, egui::Key::Z)
                    || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
            }) {
                self.redo();
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                self.undo();
            }
        }
        self.keyboard(&ctx);
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::right("editor")
            .resizable(true)
            .default_size(480.0)
            .min_size(380.0)
            .show(ui, |ui| self.side_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| self.preview(ui));
        self.track_edits(ctx.input(|i| i.pointer.any_down()));
        self.apply();
        let prefs = Prefs {
            frames: self.show_frames,
            magnets: self.magnets,
            grid: self.snap,
            link_rgb: self.link_rgb,
        };
        if prefs != self.prefs {
            prefs.save();
            self.prefs = prefs;
        }
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

/// Background image of a page: file (typed or chosen with the desktop's file dialog), fit,
/// darkening, frame rate of animations.
fn wallpaper_edit(
    ui: &mut Ui,
    page: &mut Page,
    page_idx: usize,
    picked: &Arc<Mutex<Option<(usize, String)>>>,
) {
    if let Some((idx, path)) = picked.lock().unwrap().take_if(|(i, _)| *i == page_idx) {
        debug_assert_eq!(idx, page_idx);
        match &mut page.wallpaper {
            Some(w) => w.path = path,
            None => page.wallpaper = Some(Wallpaper::new(path)),
        }
    }
    ui.add_space(4.0);
    ui.label(RichText::new(tr("Background image")).strong())
        .on_hover_text(tr("PNG, JPEG, WebP or GIF (animated or not), under the widgets"));
    ui.horizontal(|ui| {
        if ui.button(tr("Browse…")).clicked() {
            let picked = picked.clone();
            let _ = std::thread::Builder::new()
                .name("hsm01-file".into())
                .spawn(move || {
                    let file = rfd::FileDialog::new()
                        .set_title(tr("Background image"))
                        .add_filter(tr("Images"), &["png", "jpg", "jpeg", "gif", "webp"])
                        .pick_file();
                    if let Some(f) = file {
                        *picked.lock().unwrap() =
                            Some((page_idx, f.to_string_lossy().into_owned()));
                    }
                });
        }
        if page.wallpaper.is_some() && ui.button(tr("Remove")).clicked() {
            page.wallpaper = None;
        }
    });
    let Some(wp) = &mut page.wallpaper else {
        return;
    };
    egui::Grid::new("wallpaper").num_columns(2).show(ui, |ui| {
        ui.label(tr("File"));
        ui.add(TextEdit::singleline(&mut wp.path).desired_width(260.0));
        ui.end_row();
        ui.label(tr("Framing"));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut wp.fit, Fit::Cover, tr("fill"))
                .on_hover_text(tr("the whole surface, the excess is cropped"));
            ui.selectable_value(&mut wp.fit, Fit::Contain, tr("contain"))
                .on_hover_text(tr("the whole image, bands of the background colour"));
            ui.selectable_value(&mut wp.fit, Fit::Stretch, tr("stretch"));
            ui.selectable_value(&mut wp.fit, Fit::Center, tr("centred"))
                .on_hover_text(tr("actual size"));
        });
        ui.end_row();
        ui.label(tr("Darken"));
        ui.add(egui::Slider::new(&mut wp.dim, 0..=90).suffix(" %"))
            .on_hover_text(tr("to keep the widgets readable"));
        ui.end_row();
        ui.label(tr("Animation"));
        ui.add(
            DragValue::new(&mut wp.fps)
                .range(1..=30)
                .suffix(tr(" frames/s max")),
        )
        .on_hover_text(
            tr("animated GIF / WebP; USB limits a full-screen background that changes completely to about 3-4 frames/s anyway"),
        );
        ui.end_row();
    });
}

/// Decorative frame of a widget: border (colour / gradient, width), background, rounded corners,
/// padding of the content.
fn frame_edit(ui: &mut Ui, frame: &mut Option<Frame>) {
    ui.add_space(6.0);
    let mut on = frame.is_some();
    if ui
        .checkbox(&mut on, RichText::new(tr("Frame")).strong())
        .on_hover_text(tr("border, background and rounded corners around the widget, content inset"))
        .changed()
    {
        *frame = on.then(Frame::default);
    }
    let Some(f) = frame else {
        return;
    };
    egui::Grid::new("frame").num_columns(2).show(ui, |ui| {
        ui.label(tr("border"));
        paint_edit(ui, &mut f.border, true);
        ui.end_row();
        ui.label(tr("edges"));
        sides_edit(ui, &mut f.sides);
        ui.end_row();
        ui.label(tr("thickness"));
        ui.add(
            DragValue::new(&mut f.width)
                .range(0.0..=20.0)
                .speed(0.1)
                .suffix(" px"),
        )
        .on_hover_text(tr("0 = no border"));
        ui.end_row();
        ui.label(tr("background"));
        ui.horizontal(|ui| {
            let mut filled = f.fill.is_some();
            if ui.checkbox(&mut filled, "").changed() {
                f.fill = filled.then_some(Paint::Solid(Color(0x10, 0x18, 0x18)));
            }
            if let Some(fill) = &mut f.fill {
                paint_edit(ui, fill, true);
            }
        });
        ui.end_row();
        ui.label(tr("rounding"));
        ui.add(
            DragValue::new(&mut f.radius)
                .range(0.0..=60.0)
                .speed(0.2)
                .suffix(" px"),
        );
        ui.end_row();
        ui.label(tr("margin"));
        ui.add(DragValue::new(&mut f.padding).range(0..=40).suffix(" px"))
            .on_hover_text(tr("inset of the content inside the border"));
        ui.end_row();
    });
}

/// Row « Veille » of a module's card: no standby of the device while the module is on screen.
fn keep_awake_row(ui: &mut Ui, on: &mut bool) {
    ui.label(tr("Standby"));
    ui.checkbox(on, tr("no standby while it is shown")).on_hover_text(
        tr("the screen does not turn off while the module's page (or its widget in the page) is shown; the standby delay starts again when the page changes"),
    );
    ui.end_row();
}

/// Row « Chargement » of a module's card: it runs only while its page or its widget is on screen
/// (frozen otherwise, `pause_hidden`).
fn pause_hidden_row(ui: &mut Ui, on: &mut bool) {
    ui.label(tr("Load"));
    ui.checkbox(on, tr("load only when shown")).on_hover_text(
        tr("the module starts only when the screen needs it (its page, its widget in the page, a knob, LED or button of the page tied to one of its commands); a few seconds after it leaves the screen it is frozen (no processor time), and stopped after 30 s. Meanwhile it does nothing else: no alerts, no pills, no relayed data. Untick it for a module that must work in the background"),
    );
    ui.end_row();
}

/// State of a tool: running (pid, restarts) or paused, switched off, stopped (why), starting.
fn tool_state(ui: &mut Ui, st: Option<&crate::tools::ToolStatus>, enabled: bool) {
    match st {
        Some(s) if s.running => {
            let mut text = if s.paused {
                tf!("paused, hidden (pid {pid})", pid = s.pid.unwrap_or(0))
            } else {
                tf!("running (pid {pid})", pid = s.pid.unwrap_or(0))
            };
            if s.restarts > 0 {
                text.push_str(&tf!(", restarted {n} times", n = s.restarts));
            }
            ui.colored_label(Color32::from_rgb(0x00, 0xb4, 0xa8), text);
        }
        _ if !enabled => {
            ui.weak(tr("disabled"));
        }
        Some(s) if s.waiting => {
            ui.weak(tr("not loaded: loads when shown"));
        }
        Some(s) if s.last_exit.is_some() => {
            let why = s.last_exit.clone().unwrap_or_default();
            ui.colored_label(Color32::from_rgb(0xe0, 0xb0, 0x40), tf!("stopped: {why}", why = why));
        }
        _ => {
            ui.weak(tr("starting…"));
        }
    }
}

/// Last output lines of a tool, folded.
fn tool_log(ui: &mut Ui, id: &str, st: Option<&crate::tools::ToolStatus>) {
    let Some(s) = st.filter(|s| !s.log.is_empty()) else {
        return;
    };
    egui::CollapsingHeader::new(tf!("Log ({n} lines)", n = s.log.len()))
        .id_salt(("log", id))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &s.log {
                        ui.monospace(line);
                    }
                });
        });
}

/// Open a folder or a file with the desktop (file manager…), without waiting.
fn open_path(p: &std::path::Path) {
    match std::process::Command::new("xdg-open").arg(p).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("hsm01-app: xdg-open {}: {e}", p.display()),
    }
}

/// On / off switch (a sliding knob, teal when on).
fn switch(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = vec2(2.0, 1.0) * ui.spacing().interact_size.y;
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let r = rect.height() / 2.0;
    let off = ui.visuals().widgets.inactive.bg_fill;
    let fill = off.lerp_to_gamma(Color32::from_rgb(0x00, 0xb4, 0xa8), t);
    ui.painter().rect_filled(rect, r, fill);
    let x = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    ui.painter()
        .circle_filled(pos2(x, rect.center().y), r * 0.78, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Sides of a frame that have the border: a small box where a click on a side shows or hides
/// it (in the centre: all / none), and presets.
fn sides_edit(ui: &mut Ui, sides: &mut Sides) {
    let mut s = sides.array();
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(vec2(60.0, 42.0), Sense::click());
        // side under the pointer: the nearest edge, within 12 px
        let side_at = |p: Pos2| {
            let d = [
                p.y - rect.top(),
                rect.right() - p.x,
                rect.bottom() - p.y,
                p.x - rect.left(),
            ];
            let (k, m) = d
                .into_iter()
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(&b.1))?;
            (m <= 12.0).then_some(k)
        };
        let hover = resp.hover_pos().and_then(side_at);
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            match side_at(p) {
                Some(k) => s[k] = !s[k],
                None => s = [!s.iter().all(|&v| v); 4],
            }
        }
        ui.painter()
            .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
        paint_sides(ui, rect.shrink(6.0), s, hover, 4.0);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("click on an edge: show or hide it · in the centre: all / none"));
        ui.add_space(6.0);
        for (preset, tip) in [
            ([true; 4], tr("all the edges")),
            ([true, false, true, false], tr("top and bottom")),
            ([false, true, false, true], tr("left and right")),
            ([false, false, true, false], tr("bottom only (underline)")),
        ] {
            let (r, resp) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
            let v = ui.visuals();
            let bg = if s == preset {
                v.selection.bg_fill
            } else if resp.hovered() {
                v.widgets.hovered.bg_fill
            } else {
                v.widgets.inactive.bg_fill
            };
            ui.painter().rect_filled(r, 3.0, bg);
            paint_sides(ui, r.shrink(6.0), preset, None, 2.0);
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tip)
                .clicked()
            {
                s = preset;
            }
        }
    });
    *sides = Sides::from_array(s);
}

/// A box whose shown sides (top, right, bottom, left) are thick teal lines and the hidden ones
/// dashed; the hovered side is drawn as it would be after a click.
fn paint_sides(ui: &Ui, r: Rect, s: [bool; 4], hover: Option<usize>, thick: f32) {
    const ON: Color32 = Color32::from_rgb(0x00, 0xb4, 0xa8);
    let off = ui.visuals().weak_text_color();
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()];
    let p = ui.painter();
    for k in 0..4 {
        let seg = [corners[k], corners[(k + 1) % 4]];
        let shown = s[k] != (hover == Some(k));
        let color = match (shown, hover == Some(k)) {
            (true, true) => ON.gamma_multiply(0.6),
            (true, false) => ON,
            (false, _) => off,
        };
        if shown {
            p.line_segment(seg, Stroke::new(thick, color));
        } else {
            p.extend(egui::Shape::dashed_line(&seg, Stroke::new(1.0, color), 3.0, 3.0));
        }
    }
}

/// Box to give a meter after a style change.
#[derive(Clone, Copy)]
enum Reshape {
    /// Arc, needle: square.
    Round,
    /// Back to a bar / segments from a round or image style: the usual gauge size.
    Bar,
}

/// Style of a level meter / volume gauge and its settings; the box change the new style needs.
fn gauge_edit(ui: &mut Ui, g: &mut Gauge) -> Option<Reshape> {
    const STYLES: [(&str, &str); 5] = [
        ("bar", tx("bar")),
        ("segments", tx("segments (LED)")),
        ("arc", tx("arc")),
        ("needle", tx("needle")),
        ("image", tx("images (free shape)")),
    ];
    let mut reshape = None;
    ui.vertical(|ui| {
        let mut chosen = g.name();
        ComboBox::from_id_salt("gauge_style")
            .selected_text(label_of(&STYLES, chosen))
            .show_ui(ui, |ui| {
                for (k, l) in STYLES {
                    ui.selectable_value(&mut chosen, k, tr(l));
                }
            });
        if chosen != g.name() {
            let was = g.name();
            *g = Gauge::template(chosen);
            reshape = match chosen {
                "arc" | "needle" => Some(Reshape::Round),
                "bar" | "segments" if matches!(was, "arc" | "needle" | "image") => {
                    Some(Reshape::Bar)
                }
                _ => None,
            };
        }
        let angles = |ui: &mut Ui, start: &mut f32, end: &mut f32| {
            ui.horizontal(|ui| {
                ui.label(tr("from"));
                ui.add(DragValue::new(start).range(-360.0..=360.0).suffix(" °"));
                ui.label(tr("to"));
                ui.add(DragValue::new(end).range(-360.0..=360.0).suffix(" °"));
            })
            .response
            .on_hover_text(tr("0° = top, clockwise; -135 → 135: open at the bottom"));
        };
        match g {
            Gauge::Bar => {}
            Gauge::Segments { count, gap } => {
                ui.horizontal(|ui| {
                    ui.label(tr("count"));
                    ui.add(DragValue::new(count).range(2..=64));
                    ui.label(tr("gap"));
                    ui.add(DragValue::new(gap).range(0..=10).suffix(" px"));
                });
            }
            Gauge::Arc {
                start,
                end,
                thickness,
            }
            | Gauge::Needle {
                start,
                end,
                thickness,
            } => {
                angles(ui, start, end);
                ui.horizontal(|ui| {
                    ui.label(tr("thickness"));
                    ui.add(
                        DragValue::new(thickness)
                            .range(1.0..=60.0)
                            .speed(0.2)
                            .suffix(" px"),
                    );
                });
            }
            Gauge::Image {
                track,
                fill,
                sweep,
                start,
                end,
            } => {
                ui.add(TextEdit::singleline(track).hint_text(tr("track.png (background)")))
                    .on_hover_text(tr("image drawn whole, under the fill"));
                ui.add(TextEdit::singleline(fill).hint_text(tr("fill.png")))
                    .on_hover_text(tr("image revealed by the level"));
                ui.checkbox(sweep, tr("reveal by turning")).on_hover_text(
                    tr("by angle around the centre (round gauge) instead of along the widget"),
                );
                if *sweep {
                    angles(ui, start, end);
                }
            }
        }
    });
    reshape
}

fn color_edit(ui: &mut Ui, c: &mut Color) -> egui::Response {
    let mut rgb = [c.0, c.1, c.2];
    let r = ui.color_edit_button_srgb(&mut rgb);
    *c = Color(rgb[0], rgb[1], rgb[2]);
    r
}

/// Colour or gradient. A gradient lists its colours (start → end), optionally their positions
/// (0..1), sharp zones, and for texts and rectangles (`directional`) a vertical direction. The
/// config is only rewritten when something is actually changed here.
fn paint_edit(ui: &mut Ui, p: &mut Paint, directional: bool) {
    let as_stops = |q: &Paint| match q {
        Paint::Gradient(colors) => Paint::Stops {
            colors: colors.clone(),
            stops: Vec::new(),
            hard: false,
            vertical: false,
        },
        other => other.clone(),
    };
    let before = as_stops(p);
    let mut edit = before.clone();
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let solid = matches!(edit, Paint::Solid(_));
            if ui.selectable_label(solid, tr("solid")).clicked() && !solid {
                edit = Paint::Solid(edit.at(0.0));
            }
            if ui.selectable_label(!solid, tr("gradient")).clicked() && solid {
                edit = Paint::Stops {
                    colors: vec![edit.at(0.0), Color(255, 255, 255)],
                    stops: Vec::new(),
                    hard: false,
                    vertical: false,
                };
            }
        });
        match &mut edit {
            Paint::Solid(c) => {
                color_edit(ui, c);
            }
            Paint::Stops {
                colors,
                stops,
                hard,
                vertical,
            } => {
                ui.horizontal_wrapped(|ui| {
                    let n = colors.len();
                    let mut remove = None;
                    for (j, c) in colors.iter_mut().enumerate() {
                        color_edit(ui, c);
                        if let Some(pos) = stops.get_mut(j) {
                            let drag = DragValue::new(pos)
                                .range(0.0..=1.0)
                                .speed(0.01)
                                .max_decimals(2);
                            ui.add(drag);
                        }
                        if n > 2 && ui.small_button("×").clicked() {
                            remove = Some(j);
                        }
                    }
                    if let Some(j) = remove {
                        colors.remove(j);
                        if j < stops.len() {
                            stops.remove(j);
                        }
                    }
                    let add = ui.small_button("+").on_hover_text(tr("add a colour"));
                    if n < 8 && add.clicked() {
                        let last = colors.last().copied().unwrap_or(Color(255, 255, 255));
                        colors.push(last);
                        if !stops.is_empty() {
                            stops.push(1.0);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let mut custom = !stops.is_empty();
                    let toggle = ui
                        .checkbox(&mut custom, "positions")
                        .on_hover_text(tr("start of each colour, from 0 (empty / left) to 1"));
                    if toggle.changed() {
                        let n = colors.len();
                        let d = if *hard { n } else { n.saturating_sub(1).max(1) };
                        *stops = if custom {
                            (0..n).map(|i| i as f32 / d as f32).collect()
                        } else {
                            Vec::new()
                        };
                    }
                    ui.checkbox(hard, tr("sharp zones"))
                        .on_hover_text(tr("each colour flat instead of a blend"));
                    if directional {
                        ui.checkbox(vertical, "vertical")
                            .on_hover_text(tr("top to bottom instead of left to right"));
                    }
                });
            }
            Paint::Gradient(_) => {}
        }
    });
    if edit != before {
        *p = edit;
    }
}

fn knob_edit(ui: &mut Ui, id: IdSalt, knob: &mut u8) {
    ComboBox::from_id_salt(id)
        .width(100.0)
        .selected_text(tf!("knob {knob}", knob = knob))
        .show_ui(ui, |ui| {
            for k in 1..=4 {
                ui.selectable_value(knob, k, tf!("knob {knob}", knob = k));
            }
        });
}

/// Target of a knob: audio, or (`cmds` given: a knob's target) a knob command of a tool.
fn target_edit(
    ui: &mut Ui,
    id: IdSalt,
    t: &mut Target,
    inv: &Inventory,
    cmds: Option<&[(String, crate::api::Command)]>,
) {
    const KINDS: [(&str, &str); 8] = [
        ("none", tx("nothing")),
        ("default_sink", tx("default output")),
        ("default_source", tx("default input")),
        ("sink", tx("output…")),
        ("source", tx("input…")),
        ("app", tx("application…")),
        ("fullscreen", tx("full-screen application")),
        ("tool", tx("module command")),
    ];
    let kind = match t {
        Target::None => "none",
        Target::DefaultSink => "default_sink",
        Target::DefaultSource => "default_source",
        Target::Sink { .. } => "sink",
        Target::Source { .. } => "source",
        Target::App { .. } => "app",
        Target::Fullscreen => "fullscreen",
        Target::Tool { .. } => "tool",
    };
    let mut chosen = kind;
    // a tool's command only for knobs (`cmds` given), or if already chosen
    let tools = cmds.is_some() || matches!(t, Target::Tool { .. });
    let cmds = cmds.unwrap_or_default();
    ComboBox::from_id_salt(id)
        .width(150.0)
        .selected_text(label_of(&KINDS, kind))
        .show_ui(ui, |ui| {
            for (k, l) in KINDS {
                if k != "tool" || tools {
                    ui.selectable_value(&mut chosen, k, tr(l));
                }
            }
        });
    if chosen != kind && chosen == "tool" {
        let knob = crate::api::CommandKind::Knob;
        let first = cmds.iter().find(|(_, c)| c.kind == knob);
        *t = Target::Tool {
            tool: first.map(|(t, _)| t.clone()).unwrap_or_default(),
            command: first.map(|(_, c)| c.id.clone()).unwrap_or_default(),
        };
        return;
    }
    if let Target::Tool { tool, command } = t
        && chosen == kind
    {
        let kind = Some(crate::api::CommandKind::Knob);
        command_pick(ui, IdSalt::new((id, "command")), (tool, command), cmds, kind);
        return;
    }
    if chosen != kind {
        let name = match t {
            Target::Sink { name } | Target::Source { name } | Target::App { name } => {
                std::mem::take(name)
            }
            _ => String::new(),
        };
        *t = match chosen {
            "default_sink" => Target::DefaultSink,
            "default_source" => Target::DefaultSource,
            "sink" => Target::Sink { name },
            "source" => Target::Source { name },
            "app" => Target::App { name },
            "fullscreen" => Target::Fullscreen,
            _ => Target::None,
        };
    }
    if let Target::Sink { name } | Target::Source { name } | Target::App { name } = t {
        ui.add(
            TextEdit::singleline(name)
                .hint_text(tr("name (partial)"))
                .desired_width(130.0),
        )
        .on_hover_text(
            tr("compared with the name / description (devices), the name or binary (applications)"),
        );
    }
    // what is present now: devices by description, applications by name (or binary)
    let choices: Vec<(String, String)> = match t {
        Target::Sink { .. } => inv
            .sinks
            .iter()
            .map(|(_, d)| (d.clone(), d.clone()))
            .collect(),
        Target::Source { .. } => inv
            .sources
            .iter()
            .map(|(_, d)| (d.clone(), d.clone()))
            .collect(),
        Target::App { .. } => inv
            .apps
            .iter()
            .map(|(app, bin)| {
                let value = if app.is_empty() { bin } else { app };
                let label = if bin.is_empty() || bin == app {
                    value.clone()
                } else {
                    format!("{value} ({bin})")
                };
                (label, value.clone())
            })
            .collect(),
        _ => return,
    };
    let (Target::Sink { name } | Target::Source { name } | Target::App { name }) = t else {
        return;
    };
    ui.menu_button(tr("list"), |ui| {
        if choices.is_empty() {
            ui.weak(tr("nothing for now"));
        }
        for (label, value) in choices {
            if ui.selectable_label(*name == value, label).clicked() {
                *name = value;
                ui.close();
            }
        }
    })
    .response
    .on_hover_text(tr("devices present / applications playing sound"));
}

/// The stream deck tool's file: layout, then a card per track (knob k): name, colour, tiles.
fn deck_file_edit(ui: &mut Ui, deck: &mut crate::deckfile::DeckFile, ctx: &ActionCtx) {
    use crate::deckfile::{MAX_TRACKS, Track};
    ui.horizontal(|ui| {
        ui.label(tr("Layout"));
        ui.selectable_value(&mut deck.layout, "colonnes".into(), tr("↕ columns"))
            .on_hover_text(tr("track k above knob k"));
        ui.selectable_value(&mut deck.layout, "lignes".into(), tr("↔ rows"))
            .on_hover_text(tr("one row per knob, its name on the left"));
    });
    ui.add_space(4.0);
    if deck.tracks.is_empty() {
        ui.weak(tr("No track: \"+ track\" (one per knob, in order)."));
    }
    let mut remove = None;
    let n = deck.tracks.len();
    for (k, track) in deck.tracks.iter_mut().enumerate() {
        let title = tf!("Knob {n} · {name}", n = k + 1, name = track.name);
        card(ui, &title, |ui| {
            egui::Grid::new(("deck_track", k)).num_columns(2).show(ui, |ui| {
                ui.label(tr("name"));
                ui.text_edit_singleline(&mut track.name);
                ui.end_row();
                ui.label(tr("colour"));
                color_edit(ui, &mut track.color);
                ui.end_row();
            });
            deck_tiles_edit(ui, ("track", k), &mut track.tiles, ctx);
            if ui
                .small_button(RichText::new(tr("delete the track")).color(ERROR))
                .clicked()
            {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        deck.tracks.remove(k);
    }
    if n < MAX_TRACKS
        && ui
            .button(tr("+ track"))
            .on_hover_text(tf!("for knob {n}", n = n + 1))
            .clicked()
    {
        deck.tracks.push(Track::new(n));
    }
}

/// The tiles of a deck: one foldable entry each (fields, move, remove), then the ways to add;
/// `id` tells several lists apart.
fn deck_tiles_edit(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    tiles: &mut Vec<Tile>,
    ctx: &ActionCtx,
) {
    // (tile, -1 up / +1 down / 0 remove)
    let mut op: Option<(usize, i32)> = None;
    let n = tiles.len();
    for (i, t) in tiles.iter_mut().enumerate() {
        let head = if t.title.is_empty() { tr("(untitled)") } else { t.title.as_str() };
        egui::CollapsingHeader::new(format!("{}. {head}", i + 1))
            .id_salt((id, "tile", i))
            .show(ui, |ui| {
                egui::Grid::new((id, "tile_fields", i))
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(tr("title"));
                        ui.text_edit_singleline(&mut t.title);
                        ui.end_row();
                        ui.label(tr("subtitle"));
                        ui.text_edit_singleline(&mut t.subtitle);
                        ui.end_row();
                        ui.label(tr("icon")).on_hover_text(
                            tr("name in the desktop icon theme, or path of an image (SVG, PNG…)"),
                        );
                        ui.add(TextEdit::singleline(&mut t.icon).hint_text("media-playback-start"));
                        ui.end_row();
                        ui.label("action");
                        ui.horizontal(|ui| {
                            action_edit(ui, IdSalt::new((id, "action", i)), &mut t.action, 1, ctx)
                        });
                        ui.end_row();
                    });
                ui.horizontal(|ui| {
                    if i > 0 && ui.small_button("↑").on_hover_text(tr("move up")).clicked() {
                        op = Some((i, -1));
                    }
                    if i + 1 < n && ui.small_button("↓").on_hover_text(tr("move down")).clicked() {
                        op = Some((i, 1));
                    }
                    if ui.small_button(RichText::new(tr("delete")).color(ERROR)).clicked() {
                        op = Some((i, 0));
                    }
                });
            });
    }
    match op {
        Some((i, 0)) => {
            tiles.remove(i);
        }
        Some((i, d)) => tiles.swap(i, (i as i32 + d) as usize),
        None => {}
    }
    ui.horizontal(|ui| {
        if ui.button(tr("+ tile")).clicked() {
            tiles.push(Tile::new(tr("New"), "", "applications-other", Action::None));
        }
        if ui
            .button(tr("+ media"))
            .on_hover_text(tr("4 tiles: play-pause, next, previous, stop"))
            .clicked()
        {
            tiles.extend(Tile::media());
        }
    });
}

/// What the action editors list: pages, tools, commands of the running tools (tool name).
struct ActionCtx {
    n_pages: usize,
    tools: Vec<String>,
    commands: Vec<(String, crate::api::Command)>,
    /// Pages and widgets of the running modules (`module` widgets).
    tool_pages: Vec<crate::api::ToolPage>,
}

/// The page of module `tool` a `module` widget shows: its page (empty id) or one of the pages and
/// widgets it offers while it runs; one chosen while it does not run stays, marked so. Choosing a
/// widget gives it no input (a list must not take the page's buttons). « sa page » is its first
/// page (`Hub::page_of_tool`), not listed a second time.
fn tool_page_pick(
    ui: &mut Ui,
    (tool, page, inputs): (&String, &mut String, &mut Vec<String>),
    all: &[crate::api::ToolPage],
) {
    let mine: Vec<_> = all.iter().filter(|p| p.tool == *tool).collect();
    let label = |p: &crate::api::ToolPage| match p.size {
        Some((w, h)) => tf!("widget \"{name}\" ({w}×{h})", name = p.name, w = w, h = h),
        None => tf!("page \"{name}\"", name = p.name),
    };
    let first = mine.iter().position(|p| p.size.is_none());
    let main = first.map_or(tr("its page").to_owned(), |i| tf!("its page \"{name}\"", name = mine[i].name));
    let shown = match mine.iter().find(|p| p.id == *page) {
        _ if page.is_empty() => main.clone(),
        Some(p) => label(p),
        None => tf!("{page} (module stopped)", page = page),
    };
    egui::ComboBox::from_id_salt("tool_widget_page")
        .selected_text(shown)
        .show_ui(ui, |ui| {
            want_modules(ui);
            ui.selectable_value(page, String::new(), main);
            for (_, p) in mine.iter().enumerate().filter(|(i, _)| Some(*i) != first) {
                if ui.selectable_label(*page == p.id, label(p)).clicked() {
                    page.clone_from(&p.id);
                    if p.size.is_some() {
                        inputs.clear();
                    }
                }
            }
        });
}

/// Key of the egui flag « a list of the modules' commands or widgets is open » (`want_modules`).
const WANT_MODULES: &str = "hsm01_want_modules";

/// A list of the modules' commands or widgets is open: the modules « Load only when shown » are
/// loaded meanwhile (asked for once the frame is drawn), so that they offer theirs.
fn want_modules(ui: &Ui) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(WANT_MODULES), true));
}

/// Choose a command among those the running tools offer (of `kind` only, if given): « tool ·
/// command ». One chosen while its tool does not run stays, marked so.
fn command_pick(
    ui: &mut Ui,
    id: IdSalt,
    (tool, command): (&mut String, &mut String),
    cmds: &[(String, crate::api::Command)],
    kind: Option<crate::api::CommandKind>,
) {
    let offered: Vec<&(String, crate::api::Command)> =
        cmds.iter().filter(|(_, c)| kind.is_none_or(|k| c.kind == k)).collect();
    let current = offered.iter().find(|(t, c)| t == tool && c.id == *command);
    let shown = match current {
        Some((t, c)) => RichText::new(format!("{t} · {}", c.name)),
        None if command.is_empty() => RichText::new("…"),
        None => RichText::new(tf!("{tool} · {command} (stopped)", tool = tool, command = command)).color(WARN),
    };
    let response = ComboBox::from_id_salt(id)
        .width(190.0)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            want_modules(ui);
            for (t, c) in &offered {
                let on = t == tool && c.id == *command;
                let r = ui.selectable_label(on, format!("{t} · {}", c.name));
                let r = if c.description.is_empty() { r } else { r.on_hover_text(&c.description) };
                if r.clicked() {
                    (*tool, *command) = (t.clone(), c.id.clone());
                }
            }
            if offered.is_empty() {
                ui.weak(tr("none: the running modules add theirs (Modules tab)"));
            }
        })
        .response;
    response.on_hover_text(tr("the commands of the modules switched on and running"));
}

fn action_edit(ui: &mut Ui, id: IdSalt, a: &mut Action, own_knob: u8, ctx: &ActionCtx) {
    const KINDS: [(&str, &str); 17] = [
        ("none", tx("nothing")),
        ("mute", tx("mute")),
        ("command", tx("command")),
        ("url", tx("open")),
        ("keys", tx("keyboard shortcut")),
        ("media", tx("media")),
        ("module_menu", tx("module menu")),
        ("tool_command", tx("module command")),
        ("next_page", tx("next page")),
        ("prev_page", tx("previous page")),
        ("page", tx("go to page")),
        ("tool", tx("show a module")),
        ("back", tx("back")),
        ("brightness_set", tx("set brightness")),
        ("brightness_cycle", tx("brightness: cycle")),
        ("brightness_up", tx("brightness +")),
        ("brightness_down", tx("brightness −")),
    ];
    let kind = match a {
        Action::None => "none",
        Action::Mute { .. } => "mute",
        Action::Command { .. } => "command",
        Action::Url { .. } => "url",
        Action::Keys { .. } => "keys",
        Action::Media { .. } => "media",
        Action::ToolCommand { .. } => "tool_command",
        Action::ModuleMenu => "module_menu",
        Action::NextPage => "next_page",
        Action::PrevPage => "prev_page",
        Action::Page { .. } => "page",
        Action::Tool { .. } => "tool",
        Action::Back => "back",
        Action::BrightnessSet { .. } => "brightness_set",
        Action::BrightnessCycle { .. } => "brightness_cycle",
        Action::BrightnessUp { .. } => "brightness_up",
        Action::BrightnessDown { .. } => "brightness_down",
    };
    let mut chosen = kind;
    ComboBox::from_id_salt(id)
        .width(140.0)
        .selected_text(label_of(&KINDS, kind))
        .show_ui(ui, |ui| {
            for (k, l) in KINDS {
                ui.selectable_value(&mut chosen, k, tr(l));
            }
        });
    if chosen != kind {
        *a = match chosen {
            "mute" => Action::Mute { knob: own_knob },
            "command" => Action::Command { cmd: String::new() },
            "url" => Action::Url { url: String::new() },
            "keys" => Action::Keys {
                keys: String::new(),
            },
            "media" => Action::Media {
                action: "play_pause".into(),
            },
            "tool_command" => {
                let button = crate::api::CommandKind::Button;
                let first = ctx.commands.iter().find(|(_, c)| c.kind == button);
                Action::ToolCommand {
                    tool: first.map(|(t, _)| t.clone()).unwrap_or_default(),
                    command: first.map(|(_, c)| c.id.clone()).unwrap_or_default(),
                }
            }
            "module_menu" => Action::ModuleMenu,
            "next_page" => Action::NextPage,
            "prev_page" => Action::PrevPage,
            "page" => Action::Page { index: 1 },
            "tool" => Action::Tool {
                name: ctx.tools.first().cloned().unwrap_or_default(),
            },
            "back" => Action::Back,
            "brightness_set" => Action::BrightnessSet {
                level: crate::config::default_level(),
            },
            "brightness_cycle" => Action::BrightnessCycle {
                levels: crate::config::default_levels(),
            },
            "brightness_up" => Action::BrightnessUp {
                step: crate::config::default_step(),
            },
            "brightness_down" => Action::BrightnessDown {
                step: crate::config::default_step(),
            },
            _ => Action::None,
        };
    }
    match a {
        Action::Mute { knob } => knob_edit(ui, IdSalt::new((id, "knob")), knob),
        Action::Command { cmd } => {
            ui.add(
                TextEdit::singleline(cmd)
                    .hint_text(tr("notify-send hello"))
                    .desired_width(170.0),
            )
            .on_hover_text(tr("run with sh -c"));
        }
        Action::Url { url } => {
            ui.add(
                TextEdit::singleline(url)
                    .hint_text(tr("https://… or file"))
                    .desired_width(170.0),
            );
        }
        Action::Keys { keys } => {
            ui.add(
                TextEdit::singleline(keys)
                    .hint_text("ctrl+alt+t, play_pause…")
                    .desired_width(150.0),
            )
            .on_hover_text(tr("keys at their QWERTY positions (on AZERTY, \"a\" = the Q key)"));
            if !keys.is_empty()
                && let Err(e) = crate::keys::parse(keys)
            {
                ui.colored_label(ERROR, "?").on_hover_text(e);
            }
        }
        Action::Page { index } => {
            ui.add(DragValue::new(index).range(1..=ctx.n_pages.max(1)));
        }
        Action::Tool { name } => {
            if ctx.tools.is_empty() {
                ui.weak(tr("no module (Modules tab)"));
            } else {
                let known = ctx.tools.contains(name);
                ComboBox::from_id_salt((id, "tool"))
                    .width(140.0)
                    .selected_text(if name.is_empty() { "…" } else { name.as_str() })
                    .show_ui(ui, |ui| {
                        for t in &ctx.tools {
                            ui.selectable_value(name, t.clone(), t);
                        }
                    });
                if !known && !name.is_empty() {
                    ui.colored_label(ERROR, "?")
                        .on_hover_text(tr("no module of this name in the Modules tab"));
                }
            }
        }
        Action::ToolCommand { tool, command } => {
            let button = Some(crate::api::CommandKind::Button);
            command_pick(ui, IdSalt::new((id, "tool_command")), (tool, command), &ctx.commands, button);
        }
        Action::Media { action } => {
            const MEDIA: [(&str, &str); 4] = [
                ("play_pause", tx("play / pause")),
                ("next", tx("next")),
                ("previous", tx("previous")),
                ("stop", tx("stop")),
            ];
            ComboBox::from_id_salt((id, "media"))
                .width(120.0)
                .selected_text(label_of(&MEDIA, action))
                .show_ui(ui, |ui| {
                    for (k, l) in MEDIA {
                        ui.selectable_value(action, k.to_owned(), tr(l));
                    }
                })
                .response
                .on_hover_text(tr("the player that plays (Spotify, browser, VLC…)"));
        }
        Action::BrightnessSet { level } => {
            ui.add(DragValue::new(level).range(0..=100).suffix(" %"));
        }
        Action::BrightnessUp { step } | Action::BrightnessDown { step } => {
            ui.add(DragValue::new(step).range(1..=50).suffix(" %"));
        }
        Action::BrightnessCycle { levels } => {
            let n = levels.len();
            let mut remove = None;
            for (j, l) in levels.iter_mut().enumerate() {
                ui.add(DragValue::new(l).range(0..=100).suffix(" %"));
                if n > 2 && ui.small_button("×").clicked() {
                    remove = Some(j);
                }
            }
            if let Some(j) = remove {
                levels.remove(j);
            }
            if n < 8
                && ui
                    .small_button("+")
                    .on_hover_text(tr("add a level"))
                    .clicked()
            {
                levels.push(50);
            }
        }
        Action::Back => {
            ui.weak(tr("to the page we jumped from"))
                .on_hover_text(tr("after \"go to page\" or \"show a module\"; otherwise next page"));
        }
        Action::None | Action::NextPage | Action::PrevPage | Action::ModuleMenu => {}
    }
}

fn led_edit(ui: &mut Ui, id: IdSalt, l: &mut Led, cmds: &[(String, crate::api::Command)]) {
    const KINDS: [(&str, &str); 5] = [
        ("off", tx("off")),
        ("on", tx("on")),
        ("muted", tx("on when muted")),
        ("unmuted", tx("on when active")),
        ("tool", tx("state of a module command")),
    ];
    let kind = match l {
        Led::Off => "off",
        Led::On => "on",
        Led::Muted { .. } => "muted",
        Led::Unmuted { .. } => "unmuted",
        Led::Tool { .. } => "tool",
    };
    let knob = match l {
        Led::Muted { knob } | Led::Unmuted { knob } => *knob,
        _ => 1,
    };
    let mut chosen = kind;
    ComboBox::from_id_salt(id)
        .width(140.0)
        .selected_text(label_of(&KINDS, kind))
        .show_ui(ui, |ui| {
            for (k, label) in KINDS {
                ui.selectable_value(&mut chosen, k, tr(label));
            }
        });
    if chosen != kind {
        let first = cmds.first();
        *l = match chosen {
            "on" => Led::On,
            "muted" => Led::Muted { knob },
            "unmuted" => Led::Unmuted { knob },
            "tool" => Led::Tool {
                tool: first.map(|(t, _)| t.clone()).unwrap_or_default(),
                command: first.map(|(_, c)| c.id.clone()).unwrap_or_default(),
            },
            _ => Led::Off,
        };
    }
    match l {
        Led::Muted { knob } | Led::Unmuted { knob } => {
            knob_edit(ui, IdSalt::new((id, "knob")), knob);
        }
        Led::Tool { tool, command } => {
            command_pick(ui, IdSalt::new((id, "command")), (tool, command), cmds, None);
            ui.weak(tr("on when the module says it is active"));
        }
        _ => {}
    }
}

/// `ctx`: pages and tools (Modules tab), for the `tool` and `deck` widgets.
fn widget_edit(ui: &mut Ui, w: &mut Widget, inv: &Inventory, ctx: &ActionCtx) {
    ui.add_space(4.0);
    ui.label(RichText::new(tr("Position and size")).strong());
    egui::Grid::new("geometry").num_columns(4).show(ui, |ui| {
        ui.label("x");
        ui.add(DragValue::new(&mut w.x).range(-SCREEN_W..=SCREEN_W));
        ui.label("y");
        ui.add(DragValue::new(&mut w.y).range(-SCREEN_H..=SCREEN_H));
        ui.end_row();
        ui.label(tr("width"));
        ui.add(DragValue::new(&mut w.w).range(1..=SCREEN_W));
        ui.label(tr("height"));
        ui.add(DragValue::new(&mut w.h).range(1..=SCREEN_H));
        ui.end_row();
    });
    // a meter switched between vertical and horizontal turns its box too (w ↔ h)
    let mut rotate = false;
    // a deck switched between column and row: back to that shape's box
    let mut deck_shape = false;
    // style change of a meter: square box (arc, needle) or back to the gauge size (vertical?)
    let mut reshape: Option<(Reshape, bool)> = None;
    ui.add_space(6.0);
    ui.label(RichText::new(tr("Settings")).strong());
    egui::Grid::new("properties")
        .num_columns(2)
        .show(ui, |ui| match &mut w.kind {
            WidgetKind::Text {
                text,
                knob,
                size,
                color,
                align,
                bold,
                scroll,
            } => {
                ui.label(tr("text"));
                ui.add(TextEdit::singleline(text).hint_text("{name} {volume} {mute} {page}"))
                    .on_hover_text(
                        tr("{name}, {volume}, {mute}: target of the chosen knob; {page}: name of the page"),
                    );
                ui.end_row();
                ui.label(tr("knob"));
                let text_of = |k: &Option<u8>| k.map_or(tr("none").to_owned(), |k| tf!("knob {knob}", knob = k));
                ComboBox::from_id_salt("text_knob")
                    .selected_text(text_of(knob))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(knob, None, tr("none"));
                        for k in 1..=4 {
                            ui.selectable_value(knob, Some(k), tf!("knob {knob}", knob = k));
                        }
                    });
                ui.end_row();
                ui.label(tr("size"));
                ui.add(DragValue::new(size).range(6.0..=120.0).speed(0.2));
                ui.end_row();
                ui.label(tr("colour"));
                paint_edit(ui, color, true);
                ui.end_row();
                ui.label(tr("alignment"));
                ui.horizontal(|ui| {
                    ui.selectable_value(align, Align::Left, tr("left"));
                    ui.selectable_value(align, Align::Center, tr("centre"));
                    ui.selectable_value(align, Align::Right, tr("right"));
                });
                ui.end_row();
                ui.label(tr("bold"));
                ui.checkbox(bold, "");
                ui.end_row();
                ui.label(tr("too long"));
                ui.checkbox(scroll, tr("scroll (otherwise cut with …)"));
                ui.end_row();
            }
            WidgetKind::Rect { color, radius } => {
                ui.label(tr("colour"));
                paint_edit(ui, color, true);
                ui.end_row();
                ui.label(tr("rounding"));
                ui.add(DragValue::new(radius).range(0.0..=136.0).speed(0.2));
                ui.end_row();
            }
            WidgetKind::Separator {
                color,
                thickness,
                vertical,
                style,
                fade,
            } => {
                ui.label(tr("colour")).on_hover_text(tr("a gradient follows the line"));
                paint_edit(ui, color, false);
                ui.end_row();
                ui.label(tr("thickness"));
                ui.add(
                    DragValue::new(thickness)
                        .range(0.5..=32.0)
                        .speed(0.1)
                        .suffix(" px"),
                );
                ui.end_row();
                ui.label(tr("line"));
                ui.horizontal(|ui| {
                    ui.selectable_value(style, LineStyle::Solid, tr("━ solid"));
                    ui.selectable_value(style, LineStyle::Dashed, tr("╍ dashes"));
                    ui.selectable_value(style, LineStyle::Dotted, tr("··· dots"));
                });
                ui.end_row();
                ui.label("vertical");
                rotate |= ui.checkbox(vertical, "").changed();
                ui.end_row();
                ui.label(tr("ends"));
                ui.checkbox(fade, tr("faded"));
                ui.end_row();
            }
            WidgetKind::Vu {
                knob,
                vertical,
                color,
                gauge,
            } => {
                ui.label(tr("knob"));
                knob_edit(ui, IdSalt::new("vu_knob"), knob);
                ui.end_row();
                ui.label("style");
                if let Some(r) = gauge_edit(ui, gauge) {
                    reshape = Some((r, *vertical));
                }
                ui.end_row();
                ui.label("vertical");
                rotate |= ui.checkbox(vertical, "").changed();
                ui.end_row();
                ui.label(tr("colours"));
                paint_edit(ui, color, false);
                ui.end_row();
            }
            WidgetKind::Volume {
                knob,
                color,
                vertical,
                gauge,
            } => {
                ui.label(tr("knob"));
                knob_edit(ui, IdSalt::new("volume_knob"), knob);
                ui.end_row();
                ui.label("style");
                if let Some(r) = gauge_edit(ui, gauge) {
                    reshape = Some((r, *vertical));
                }
                ui.end_row();
                ui.label(tr("colour"));
                paint_edit(ui, color, false);
                ui.end_row();
                ui.label("vertical");
                rotate |= ui.checkbox(vertical, "").changed();
                ui.end_row();
            }
            WidgetKind::Icon { knob, style, tint } => {
                ui.label(tr("knob"));
                knob_edit(ui, IdSalt::new("icon_knob"), knob);
                ui.end_row();
                ui.label("style");
                ui.vertical(|ui| {
                    ui.selectable_value(style, IconStyle::Auto, tr("auto: devices as symbols"));
                    ui.selectable_value(style, IconStyle::Color, tr("all in colour"));
                    ui.selectable_value(style, IconStyle::Symbolic, tr("all as symbols"));
                });
                ui.end_row();
                ui.label(tr("tint of the symbols"));
                color_edit(ui, tint);
                ui.end_row();
            }
            WidgetKind::ThemeIcon { name, tint } => {
                ui.label(tr("icon"))
                    .on_hover_text(tr("name in the desktop theme, or path of an image (SVG, PNG…)"));
                ui.add(TextEdit::singleline(name).hint_text("audio-input-microphone"));
                ui.end_row();
                ui.label(tr("tint"));
                ui.horizontal(|ui| {
                    let mut on = tint.is_some();
                    if ui
                        .checkbox(&mut on, "")
                        .on_hover_text(tr("monochrome version in this colour, if the theme has one"))
                        .changed()
                    {
                        *tint = on.then_some(Color(0xd0, 0xd8, 0xd8));
                    }
                    if let Some(c) = tint {
                        color_edit(ui, c);
                    }
                });
                ui.end_row();
            }
            WidgetKind::Tool { tool, page, inputs, overflow } => {
                ui.label(tr("module")).on_hover_text(
                    tr("a module of the Modules tab (running): its page, or one of its widgets, shows in the box and updates live"),
                );
                let shown = if tool.is_empty() { tr("(choose)") } else { tool.as_str() };
                egui::ComboBox::from_id_salt("tool_widget")
                    .selected_text(shown.to_owned())
                    .show_ui(ui, |ui| {
                        for t in &ctx.tools {
                            ui.selectable_value(tool, t.clone(), t);
                        }
                    });
                ui.end_row();
                ui.label(tr("show")).on_hover_text(
                    tr("its page, or a widget the module offers (list of the members of a channel…); the list comes from the running module"),
                );
                tool_page_pick(ui, (tool, page, inputs), &ctx.tool_pages);
                ui.end_row();
                ui.label(tr("too big")).on_hover_text(
                    tr("when the content does not fit in the box: shrink it whole, or keep its size and scroll it by itself, top to bottom (width fitted) or left to right (height fitted, for a list in a row)"),
                );
                ui.horizontal(|ui| {
                    ui.selectable_value(overflow, Overflow::Shrink, tr("shrink"));
                    ui.selectable_value(overflow, Overflow::Scroll, tr("scroll ↕"));
                    ui.selectable_value(overflow, Overflow::ScrollSideways, tr("scroll ↔"));
                });
                ui.end_row();
                ui.label(tr("inputs for the module")).on_hover_text(
                    tr("when this page is shown, these inputs go to the module instead of the page's settings (Controls): a knob scrolls the menu, a button activates a tile"),
                );
                ui.vertical(|ui| {
                    for (row, label) in [("knob", "molettes"), ("button", "boutons")] {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            for n in 1..=4 {
                                let name = format!("{row}{n}");
                                let mut on = inputs.contains(&name);
                                if ui.checkbox(&mut on, n.to_string()).changed() {
                                    inputs.retain(|i| *i != name);
                                    if on {
                                        inputs.push(name);
                                        inputs.sort();
                                    }
                                }
                            }
                        });
                    }
                });
                ui.end_row();
            }
            WidgetKind::Deck {
                knob,
                vertical,
                color,
                tiles,
            } => {
                ui.label(tr("knob")).on_hover_text(
                    tr("on this page, it scrolls the tiles (instead of its settings in the Controls tab) and its click runs the middle tile"),
                );
                knob_edit(ui, IdSalt::new("deck_knob"), knob);
                ui.end_row();
                ui.label(tr("direction"));
                ui.horizontal(|ui| {
                    deck_shape |= ui.selectable_value(vertical, true, tr("↕ column")).changed();
                    deck_shape |= ui.selectable_value(vertical, false, tr("↔ row")).changed();
                });
                ui.end_row();
                ui.label(tr("colour"));
                color_edit(ui, color);
                ui.end_row();
                ui.label(tr("tiles"))
                    .on_hover_text(tr("in scrolling order, which loops endlessly"));
                ui.vertical(|ui| deck_tiles_edit(ui, "widget", tiles, ctx));
                ui.end_row();
            }
            WidgetKind::Image { path, radius } => {
                ui.label(tr("PNG file"));
                ui.add(TextEdit::singleline(path).hint_text(tr("/path/image.png")));
                ui.end_row();
                ui.label(tr("rounding"))
                    .on_hover_text(tr("half of the box or more: a disc (avatar)"));
                ui.add(DragValue::new(radius).range(0.0..=136.0).speed(0.2));
                ui.end_row();
            }
            WidgetKind::Spectrum {
                target,
                bars,
                gap,
                colors,
                peak,
                background,
            } => {
                ui.label("source");
                ui.horizontal(|ui| target_edit(ui, IdSalt::new("spectrum_target"), target, inv, None));
                ui.end_row();
                ui.label(tr("bars"));
                ui.add(DragValue::new(bars).range(4..=160));
                ui.end_row();
                ui.label(tr("spacing"));
                ui.add(DragValue::new(gap).range(0..=16).suffix(" px"));
                ui.end_row();
                ui.label(tr("colours"))
                    .on_hover_text(tr("zones from bottom to top"));
                ui.horizontal_wrapped(|ui| {
                    let n = colors.len();
                    let mut remove = None;
                    for (j, c) in colors.iter_mut().enumerate() {
                        color_edit(ui, c);
                        if n > 1 && ui.small_button("×").clicked() {
                            remove = Some(j);
                        }
                    }
                    if let Some(j) = remove {
                        colors.remove(j);
                    }
                    if ui.small_button("+").clicked() {
                        let last = colors.last().copied().unwrap_or(Color(255, 255, 255));
                        colors.push(last);
                    }
                });
                ui.end_row();
                ui.label(tr("peaks"));
                ui.horizontal(|ui| {
                    let mut on = peak.is_some();
                    if ui.checkbox(&mut on, "").changed() {
                        *peak = on.then_some(Color(255, 255, 255));
                    }
                    if let Some(c) = peak {
                        color_edit(ui, c);
                    }
                });
                ui.end_row();
                ui.label(tr("background"));
                color_edit(ui, background);
                ui.end_row();
            }
        });
    frame_edit(ui, &mut w.frame);
    if rotate {
        std::mem::swap(&mut w.w, &mut w.h);
    }
    if deck_shape && let WidgetKind::Deck { knob, vertical, .. } = &w.kind {
        // a column right above its knob, a row across the screen
        (w.w, w.h) = default_size(&w.kind);
        w.x = if *vertical {
            (*knob as i32 - 1).clamp(0, 3) * (SCREEN_W / 4) + (SCREEN_W / 4 - w.w) / 2
        } else {
            0
        };
        w.y = w.y.clamp(0, SCREEN_H - w.h);
    }
    match reshape {
        Some((Reshape::Round, _)) => {
            let side = w.w.max(w.h).min(160);
            (w.w, w.h) = (side, side);
        }
        Some((Reshape::Bar, vertical)) => {
            let (thin, long) = default_size(&w.kind);
            (w.w, w.h) = if vertical { (thin, long) } else { (long, thin) };
        }
        None => {}
    }
}

fn label_of(kinds: &[(&str, &'static str)], kind: &str) -> &'static str {
    kinds
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or("?", |(_, l)| tr(l))
}

fn kind_label(name: &str) -> &'static str {
    match name {
        "text" => tr("text"),
        "rect" => tr("rectangle"),
        "separator" => tr("separator"),
        "vu" => tr("VU meter"),
        "volume" => tr("volume gauge"),
        "icon" => tr("icon of the source"),
        "image" => tr("image"),
        "theme_icon" => tr("icon (name or file)"),
        "spectrum" => tr("spectrum"),
        "module" => tr("module (its page or one of its widgets)"),
        "deck" => tr("stream deck (one knob)"),
        _ => "?",
    }
}

fn describe(w: &Widget) -> String {
    match &w.kind {
        WidgetKind::Text { text, .. } => tf!("text \"{text}\"", text = text),
        WidgetKind::Vu { knob, .. } => tf!("VU meter, knob {knob}", knob = knob),
        WidgetKind::Volume { knob, .. } => tf!("volume gauge, knob {knob}", knob = knob),
        WidgetKind::Image { path, .. } => format!("image {path}"),
        WidgetKind::ThemeIcon { name, .. } => tf!("icon \"{name}\"", name = name),
        WidgetKind::Icon { knob, .. } => tf!("icon, knob {knob}", knob = knob),
        WidgetKind::Tool { tool, .. } => tf!("module \"{tool}\"", tool = tool),
        WidgetKind::Deck { knob, tiles, .. } => {
            tf!("stream deck, knob {knob}, {n} tiles", knob = knob, n = tiles.len())
        }
        k => kind_label(k.name()).to_owned(),
    }
}

/// Initial size of a new widget.
fn default_size(kind: &WidgetKind) -> (i32, i32) {
    match kind {
        WidgetKind::Text { .. } => (160, 28),
        WidgetKind::Rect { .. } => (96, 64),
        WidgetKind::Separator { .. } => (240, 12),
        WidgetKind::Vu { .. } | WidgetKind::Volume { .. } => (24, 160),
        WidgetKind::Image { .. } => (96, 96),
        WidgetKind::ThemeIcon { .. } => (48, 48),
        WidgetKind::Icon { .. } => (48, 48),
        WidgetKind::Spectrum { .. } => (SCREEN_W, 240),
        WidgetKind::Tool { .. } => (SCREEN_W / 2, SCREEN_H / 2), // its page at half size
        // a column above its knob, or a row across the screen
        WidgetKind::Deck { vertical: true, .. } => (SCREEN_W / 4 - 8, SCREEN_H - 24),
        WidgetKind::Deck { .. } => (SCREEN_W, 72),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The editor follows the configured page shown on the device; a module's page (after the
    /// configuration's, from the modules menu) leaves it where it was, not on the last page.
    #[test]
    fn editor_follows_the_configured_pages_only() {
        let shared = Arc::new(Shared::new(normalised(Config::default())));
        let mut gui = Gui::new(shared.clone());
        assert_eq!(gui.draft.pages.len(), 2);
        for (device, editor) in [(0, 0), (1, 1), (0, 0), (2, 0), (5, 0), (1, 1), (3, 1)] {
            shared.page.store(device, Ordering::Relaxed);
            gui.sync();
            assert_eq!(gui.page, editor, "device on page index {device}");
        }
    }

    /// The editor must not change anything by itself (a fresh draft stays equal to the file).
    #[test]
    fn editor_leaves_the_config_unchanged() {
        let cfg = normalised(Config::default());
        let mut gui = Gui::new(Arc::new(Shared::new(cfg.clone())));
        for page in 0..cfg.pages.len() {
            gui.page = page;
            gui.selection = vec![0];
            let ctx = egui::Context::default();
            if page == 0 {
                gui.selection = vec![0, 1, 2]; // the multi-selection panel too
            }
            for tab in [Tab::Widgets, Tab::Controls, Tab::Device, Tab::Tools] {
                gui.tab = tab;
                for _ in 0..3 {
                    ctx.run_ui(Default::default(), |ui| gui.side_panel(ui)).textures_delta.clear(); // no renderer here
                }
            }
            assert_eq!(gui.draft, cfg, "page {page}");
        }
    }

    /// The Discord module's tab is a form: opened and drawn (with or without the lists the
    /// running module gives) without writing; a favourite channel saved keeps the file's comment.
    #[test]
    fn discord_settings_form() {
        use crate::config::Tool;
        let shipped = concat!(env!("CARGO_MANIFEST_DIR"), "/../modules/discord");
        let dir = std::env::temp_dir().join(format!("hsm01-form-{}", std::process::id()));
        let d = dir.join("discord");
        std::fs::create_dir_all(&d).unwrap();
        for f in ["module.toml", "discord.toml"] {
            std::fs::copy(format!("{shipped}/{f}"), d.join(f)).unwrap();
        }
        let shared = Arc::new(Shared::new(normalised(Config::default())));
        let mut gui = Gui::new(shared.clone());
        gui.modules = crate::tools::scan_dir(&dir);
        gui.draft.tools = vec![Tool {
            name: "Discord".into(),
            enabled: true,
            module: "discord".into(),
            args: Vec::new(),
            command: String::new(),
            keep_awake: false,
            pause_hidden: false,
        }];
        let ctx = egui::Context::default();
        let frame = |gui: &mut Gui| {
            ctx.run_ui(Default::default(), |ui| gui.side_panel(ui)).textures_delta.clear();
        };
        (gui.tab, gui.settings_tab) = (Tab::Settings, "discord".into());
        for _ in 0..2 {
            frame(&mut gui);
        }
        // the module running gives its lists: drawn again with them
        let (id, _tx, _rx) = crate::api::register(&shared);
        crate::api::process(&shared, id, r#"{"type": "hello", "name": "Discord"}"#);
        let lists = r#"{"type": "choices", "choices": {"serveurs": ["S"], "salons:S": ["Général"]}}"#;
        crate::api::process(&shared, id, lists);
        frame(&mut gui);
        let path = d.join("discord.toml");
        let before = std::fs::read_to_string(&path).unwrap();
        let se = gui.settings_edit.as_mut().expect("opened");
        assert!(matches!(se.form, SettingsForm::Fields { .. }) && !se.changed());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "shown, not written");
        if let SettingsForm::Fields { edit, .. } = &mut se.form {
            let item: toml::Table = toml::from_str("serveur = \"S\"\nsalon = \"Général\"").unwrap();
            crate::settingsform::set(edit, "salon", Some(toml::Value::Array(vec![item.into()])));
        }
        assert!(se.changed());
        se.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Module Discord") && text.contains("[[salon]]"), "{text}");
        let back: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(back["salon"][0]["salon"].as_str(), Some("Général"));
        frame(&mut gui);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An enabled tool with `[settings]` gets its tab (none once switched off); the stream deck
    /// editor shows its file without changing it, and « Enregistrer » writes an edit back into
    /// it (comments kept); a text settings file gets the text editor.
    #[test]
    fn tool_settings_tabs() {
        use crate::config::Tool;
        let shipped = concat!(env!("CARGO_MANIFEST_DIR"), "/../modules");
        let dir = std::env::temp_dir().join(format!("hsm01-tabs-{}", std::process::id()));
        let tool = |id: &str, settings: &str, editor: &str| {
            let d = dir.join(id);
            std::fs::create_dir_all(&d).unwrap();
            let file = format!("{shipped}/streamdeck/deck.toml");
            std::fs::copy(file, d.join(settings)).unwrap();
            std::fs::write(
                d.join("module.toml"),
                format!("name = \"{id}\"\nfile = \"main.py\"\n[settings]\nfile = \"{settings}\"\neditor = \"{editor}\"\n"),
            )
            .unwrap();
        };
        tool("deck", "deck.toml", "deck");
        tool("autre", "reglages.toml", "texte");
        let mut gui = Gui::new(Arc::new(Shared::new(normalised(Config::default()))));
        gui.modules = crate::tools::scan_dir(&dir);
        let on = |module: &str| Tool {
            name: format!("Mon {module}"),
            enabled: true,
            module: module.into(),
            args: Vec::new(),
            command: String::new(),
            keep_awake: false,
            pause_hidden: false,
        };
        gui.draft.tools = vec![on("deck"), on("autre")];
        let tabs = gui.settings_tabs();
        assert_eq!(tabs.len(), 2, "{tabs:?}");
        let ctx = egui::Context::default();
        let frame = |gui: &mut Gui| {
            ctx.run_ui(Default::default(), |ui| gui.side_panel(ui))
                .textures_delta
                .clear(); // no renderer here
        };
        gui.tab = Tab::Settings;
        gui.settings_tab = "deck".into();
        let path = dir.join("deck/deck.toml");
        let before = std::fs::read_to_string(&path).unwrap();
        for _ in 0..3 {
            frame(&mut gui);
        }
        let se = gui.settings_edit.as_mut().expect("opened");
        assert!(matches!(se.form, SettingsForm::Deck { .. }) && !se.changed());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "shown, not written");
        if let SettingsForm::Deck { edit, .. } = &mut se.form {
            edit.layout = "lignes".into();
            edit.tracks[3].name = "OBS".into();
        }
        assert!(se.changed());
        se.save().unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("layout = \"lignes\"") && after.contains("name = \"OBS\""));
        assert!(after.starts_with("# Cases du « Stream deck »"), "comments kept");
        // the text editor for the other one
        gui.settings_tab = "autre".into();
        frame(&mut gui);
        let se = gui.settings_edit.as_ref().unwrap();
        assert!(matches!(se.form, SettingsForm::Text(_)) && !se.changed());
        // switched off: no tab, back to « Modules »
        gui.draft.tools[1].enabled = false;
        frame(&mut gui);
        assert_eq!(gui.settings_tabs().len(), 1);
        assert!(gui.tab == Tab::Tools);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_and_redo() {
        let mut gui = Gui::new(Arc::new(Shared::new(normalised(Config::default()))));
        let original = gui.draft.clone();
        // one gesture (mouse held): two changes, one undo step
        gui.draft.brightness = 10;
        gui.track_edits(true);
        gui.draft.brightness = 20;
        gui.track_edits(true);
        gui.undo();
        assert_eq!(gui.draft, original);
        gui.redo();
        assert_eq!(gui.draft.brightness, 20);
        // a new edit after an undo drops the redo steps
        gui.undo();
        gui.draft.counts_per_percent = 3;
        gui.track_edits(false);
        gui.commit_gesture();
        assert!(gui.redo.is_empty());
        assert_eq!(gui.undo.len(), 1);
        gui.undo();
        assert_eq!(gui.draft, original);
        // removing a widget, then undoing it, keeps the selection valid
        gui.selection = vec![gui.draft.pages[0].widgets.len() - 1];
        gui.draft.pages[0].widgets.pop();
        gui.track_edits(false);
        gui.undo();
        assert_eq!(gui.draft, original);
    }

    #[test]
    fn picking_and_handles() {
        let w = |x, y| Widget {
            x,
            y,
            w: 40,
            h: 20,
            frame: None,
            kind: WidgetKind::template("rect").unwrap(),
        };
        let ws = vec![w(0, 0), w(20, 10)];
        assert_eq!(hit_widget(&ws, 25.0, 15.0), Some(1), "topmost wins");
        assert_eq!(hit_widget(&ws, 5.0, 5.0), Some(0));
        assert_eq!(hit_widget(&ws, 100.0, 100.0), None);
        let hs = handles_of(&ws[0], &|x, y| pos2(x * 2.0, y * 2.0));
        assert_eq!(hs.len(), 8);
        assert_eq!(hs[7].0, pos2(80.0, 40.0), "bottom-right corner, zoom 2");
        assert!(matches!(hs[7].1.cursor(), egui::CursorIcon::ResizeNwSe));
        assert!(matches!(hs[1].1.cursor(), egui::CursorIcon::ResizeVertical));
        let mut sel = vec![0];
        toggle_in(&mut sel, 1);
        toggle_in(&mut sel, 0);
        assert_eq!(sel, vec![1]);
    }

    #[test]
    fn magnets_align_edges_and_centres() {
        // left edge 3 px right of another box's left edge: snaps onto it
        assert_eq!(
            magnet(103, 10, [(100, 50)].into_iter(), 480),
            (-3, Some(100))
        );
        // the closest match wins: left edge 120 → other box's centre 125
        assert_eq!(
            magnet(120, 40, [(100, 50)].into_iter(), 480),
            (5, Some(125))
        );
        // far from everything: no snap
        assert_eq!(magnet(300, 40, [(100, 50)].into_iter(), 480), (0, None));
        // screen centre
        assert_eq!(magnet(218, 40, std::iter::empty(), 480), (2, Some(240)));
    }

    /// The checkbox and the selection reach the engine (outlines drawn on the device).
    #[test]
    fn frames_go_to_the_device_overlay() {
        let shared = Arc::new(Shared::new(normalised(Config::default())));
        let mut gui = Gui::new(shared.clone());
        let ctx = egui::Context::default();
        gui.show_frames = true;
        gui.selection = vec![1, 3];
        ctx.run_ui(Default::default(), |ui| gui.preview(ui)).textures_delta.clear(); // no renderer here
        assert_eq!(
            *shared.overlay.lock().unwrap(),
            Overlay {
                frames: true,
                selected: vec![1, 3],
                pattern: false,
            }
        );
        gui.show_frames = false;
        ctx.run_ui(Default::default(), |ui| gui.preview(ui)).textures_delta.clear(); // no renderer here
        assert!(!shared.overlay.lock().unwrap().frames);
        drop(gui);
        assert_eq!(*shared.overlay.lock().unwrap(), Overlay::default());
    }

    /// A real click (through egui input) on « Cadres des widgets » toggles it.
    #[test]
    fn frames_checkbox_reacts_to_a_click() {
        let mut gui = Gui::new(Arc::new(Shared::new(normalised(Config::default()))));
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1360.0, 780.0));
        let frame = |gui: &mut Gui, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| gui.show(ui)).textures_delta.clear(); // no renderer here
        };
        let pos = pos2(16.0, 16.0);
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut gui, vec![]);
        frame(&mut gui, vec![egui::Event::PointerMoved(pos)]);
        frame(&mut gui, vec![button(true)]);
        frame(&mut gui, vec![button(false)]);
        frame(&mut gui, vec![]);
        assert!(!gui.show_frames, "the click did not toggle the checkbox");
    }
}
