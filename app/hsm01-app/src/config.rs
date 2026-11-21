//! Configuration (`~/.config/hsm01/config.toml`): pages of freely placed widgets, and what each
//! knob / knob click / button does on that page. See docs/app/companion.md.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use crate::i18n::tr;

pub const SCREEN_W: i32 = 480;
pub const SCREEN_H: i32 = 272;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Language of the window, the messages and the modules: `en` (default) or a language with a
    /// translation file (`fr`…, i18n.rs).
    #[serde(default = "default_language", skip_serializing_if = "is_default_language")]
    pub language: String,
    /// Backlight while the app drives the device (percent).
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    /// Knob counts (512 per turn) for one volume percent.
    #[serde(default = "default_counts_per_percent")]
    pub counts_per_percent: u32,
    /// Automatic standby of the screen (backlight off, no drawing).
    #[serde(default)]
    pub standby: Standby,
    /// Colour correction of the device screen.
    #[serde(default)]
    pub calibration: Calibration,
    /// Desktop notifications shown as a popup on the device.
    #[serde(default)]
    pub notifications: Notifications,
    /// Open API (docs/app/api.md).
    #[serde(default)]
    pub api: ApiSettings,
    /// Programs run by the app while it runs (modules of the open API): `[[module]]` (formerly
    /// `[[module]]`, still read).
    #[serde(rename = "module", alias = "tool", default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
    #[serde(rename = "page", default)]
    pub pages: Vec<Page>,
}

/// After `after_secs` (at least `STANDBY_MIN_SECS`) without user action (knob, button, edit in the
/// window), the backlight fades to 0 over `fade_secs` (perceptually even), then nothing is drawn
/// or sent; any action wakes the screen up (that first action is not executed).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Standby {
    pub enabled: bool,
    pub after_secs: u32,
    pub fade_secs: f32,
}

pub const STANDBY_MIN_SECS: u32 = 10;

/// Desktop notifications (Discord…) of the chosen applications, shown on the device for
/// `duration_secs` (a click on a button or a knob closes them sooner, without acting).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Notifications {
    pub enabled: bool,
    /// Part of the application's name, any case ("discord" also matches "Discord Canary");
    /// empty = all applications.
    pub apps: Vec<String>,
    pub duration_secs: f32,
    /// A notification wakes the screen up from standby (otherwise it is not shown).
    pub wake: bool,
}

/// Open API: the Unix socket is always on; `websocket` adds 127.0.0.1:`port` (token required),
/// for programs that cannot use a Unix socket (web pages…). Read at start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ApiSettings {
    pub websocket: bool,
    pub port: u16,
}

/// A program the app starts, restarts if it stops and stops when it quits (`[[module]]`): a module
/// shipped with the app (`module = "media"`, see `tools.rs`) with its `args`, or any `command`
/// (run by `sh -c`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub module: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// No standby of the device while it runs (« Empêche la veille » in the Modules tab).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_awake: bool,
}

impl Default for ApiSettings {
    fn default() -> Self {
        ApiSettings {
            websocket: true,
            port: 9137,
        }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Notifications {
            enabled: true,
            apps: vec!["discord".into(), "vesktop".into()],
            duration_secs: 6.0,
            wake: true,
        }
    }
}

/// Per-channel (red, green, blue) correction applied to everything sent to the screen:
/// `out = gain × in^(1/gamma)` (values 0..1). Gain < 1 tames a channel (white balance),
/// gamma > 1 lightens its mid-tones.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Calibration {
    pub gain: [f32; 3],
    pub gamma: [f32; 3],
}

impl Default for Calibration {
    fn default() -> Self {
        Calibration {
            gain: [1.0; 3],
            gamma: [1.0; 3],
        }
    }
}

impl Default for Standby {
    fn default() -> Self {
        Standby {
            enabled: true,
            after_secs: 300,
            fade_secs: 6.0,
        }
    }
}

fn next_page() -> Action {
    Action::NextPage
}

fn default_brightness() -> u8 {
    80
}

fn default_language() -> String {
    crate::i18n::DEFAULT.to_owned()
}

fn is_default_language(code: &String) -> bool {
    code == crate::i18n::DEFAULT
}

fn default_counts_per_percent() -> u32 {
    8
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Page {
    pub name: String,
    #[serde(default)]
    pub background: Color,
    /// Knobs 1..4: target turned by the knob, action on click.
    #[serde(default)]
    pub knobs: Vec<Knob>,
    /// Buttons 1..4: action and what their LED shows.
    #[serde(default)]
    pub buttons: Vec<Button>,
    /// Power button.
    #[serde(default = "next_page")]
    pub power: Action,
    /// Long press of the power button.
    #[serde(default, skip_serializing_if = "Action::is_none")]
    pub power_hold: Action,
    /// Background image (PNG, JPEG, WebP, GIF – animated or not), over `background`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
    #[serde(rename = "widget", default)]
    pub widgets: Vec<Widget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Wallpaper {
    pub path: String,
    #[serde(default)]
    pub fit: Fit,
    /// Darkening (percent) so that the widgets stay readable.
    #[serde(default)]
    pub dim: u8,
    /// Animations: at most this many frames per second.
    #[serde(default = "default_wallpaper_fps")]
    pub fps: u8,
}

fn default_wallpaper_fps() -> u8 {
    12
}

/// What a `module` widget does with a page or widget of the module bigger than its box:
/// `shrink` it whole into the box (default), or `scroll`: its width fitted, the rest scrolling
/// down and back up by itself (a list of names keeps readable letters), or `scroll_sideways`:
/// its height fitted, the rest scrolling right and back (a row of names).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Overflow {
    #[default]
    Shrink,
    Scroll,
    ScrollSideways,
}

impl Overflow {
    fn is_shrink(&self) -> bool {
        *self == Overflow::Shrink
    }
}

/// How an image fills the screen: `cover` (crop), `contain` (whole, bands of the background
/// colour), `stretch` (distort), `center` (actual size).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    #[default]
    Cover,
    Contain,
    Stretch,
    Center,
}

impl Wallpaper {
    pub fn new(path: String) -> Self {
        Wallpaper {
            path,
            fit: Fit::Cover,
            dim: 0,
            fps: default_wallpaper_fps(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Knob {
    #[serde(default)]
    pub target: Target,
    #[serde(default)]
    pub click: Action,
    /// Long press (`HOLD_MS`); when set, `click` runs on a short press's release.
    #[serde(default, skip_serializing_if = "Action::is_none")]
    pub hold: Action,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Button {
    #[serde(default)]
    pub action: Action,
    /// Long press (`HOLD_MS`); when set, `action` runs on a short press's release.
    #[serde(default, skip_serializing_if = "Action::is_none")]
    pub hold: Action,
    #[serde(default)]
    pub led: Led,
}

/// Audio object a knob controls (volume) and widgets display (name, volume, mute, level).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Target {
    #[default]
    None,
    /// Default output device.
    DefaultSink,
    /// Default input device (microphone).
    DefaultSource,
    /// Output device whose name or description contains `name`.
    Sink { name: String },
    /// Input device whose name or description contains `name`.
    Source { name: String },
    /// Application stream(s) whose application name / binary contains `name`.
    App { name: String },
    /// Audio of the full-screen application (games): reported by KWin, see `fullscreen.rs`.
    Fullscreen,
    /// Not audio: the turns go to the command `command` (a knob one) of the running tool named
    /// `tool`, whose state (text, value, active) the widgets show. `type = "module"` (formerly
    /// `"tool"`).
    #[serde(rename = "module", alias = "tool")]
    Tool {
        #[serde(rename = "module", alias = "tool")]
        tool: String,
        command: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    #[default]
    None,
    /// Toggle mute of the target of knob `knob` (1..4) on the current page.
    Mute {
        knob: u8,
    },
    /// Run a shell command (`sh -c`).
    Command {
        cmd: String,
    },
    /// Open a URL / file with the desktop's default handler.
    Url {
        url: String,
    },
    /// Key combination through a virtual keyboard, e.g. "ctrl+alt+t", "media_play_pause".
    Keys {
        keys: String,
    },
    NextPage,
    PrevPage,
    /// Go to page `index` (1-based).
    Page {
        index: usize,
    },
    /// Show the page of the module (`[[module]]`) named `name`. `type = "module"` (formerly
    /// `"tool"`).
    #[serde(rename = "module", alias = "tool")]
    Tool {
        name: String,
    },
    /// Media player that plays (the app follows them): `play_pause`, `next`, `previous`, `stop`.
    Media {
        action: String,
    },
    /// Menu of the modules, sliding up from the bottom of the screen: knob 1 chooses, its click
    /// opens the module's page (by default: knob 1 held).
    ModuleMenu,
    /// Command `command` that the running module named `tool` offers (API `commands`).
    /// `type = "module_command"`, `module = …` (formerly `tool_command`, `tool`).
    #[serde(rename = "module_command", alias = "tool_command")]
    ToolCommand {
        #[serde(rename = "module", alias = "tool")]
        tool: String,
        command: String,
    },
    /// Back to the page a jump (`page`, `tool`) came from; without one, next page.
    Back,
    /// Backlight to the next of `levels` (percent, 0 = off), from the one closest to the current
    /// level.
    BrightnessCycle {
        #[serde(default = "default_levels")]
        levels: Vec<u8>,
    },
    /// Backlight to `level` percent (0 = off).
    BrightnessSet {
        #[serde(default = "default_level")]
        level: u8,
    },
    /// Backlight up / down by `step` percent (5 % minimum).
    BrightnessUp {
        #[serde(default = "default_step")]
        step: u8,
    },
    BrightnessDown {
        #[serde(default = "default_step")]
        step: u8,
    },
}

impl Action {
    pub fn is_none(&self) -> bool {
        matches!(self, Action::None)
    }
}

/// A press held this long runs the "hold" action instead of the click.
pub const HOLD_MS: u64 = 500;

pub fn default_levels() -> Vec<u8> {
    vec![100, 60, 30, 10]
}

pub fn default_level() -> u8 {
    50
}

pub fn default_step() -> u8 {
    10
}

/// Lowest backlight reachable with « luminosité + / − » (0 only with « définir » / « cycle »).
pub const MIN_BRIGHTNESS: u8 = 5;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Led {
    #[default]
    Off,
    On,
    /// Lit while the target of knob `knob` is muted.
    Muted {
        knob: u8,
    },
    /// Lit while the target of knob `knob` is not muted.
    Unmuted {
        knob: u8,
    },
    /// Lit while the module `tool` says its command `command` is active (API `command_state`).
    #[serde(rename = "module", alias = "tool")]
    Tool {
        #[serde(rename = "module", alias = "tool")]
        tool: String,
        command: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Widget {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Decorative frame (background, border, padding of the content).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
    #[serde(flatten)]
    pub kind: WidgetKind,
}

/// Decorative frame of a widget (`frame = { border = "#00b4a8", width = 2.0, fill = "#101818",
/// radius = 8.0, padding = 4 }`): background, rounded border drawn over the content, which is
/// inset by `padding` (+ the border width).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Frame {
    pub border: Paint,
    /// Border width (0 = no border).
    pub width: f32,
    /// Background (none = transparent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    pub radius: f32,
    pub padding: i32,
    /// Sides that have the border (all by default). A corner is rounded unless the border
    /// stops there (one side shown, the other not).
    #[serde(skip_serializing_if = "Sides::is_all")]
    pub sides: Sides,
}

impl Default for Frame {
    fn default() -> Self {
        Frame {
            border: Paint::Solid(Color(0x00, 0xb4, 0xa8)),
            width: 2.0,
            fill: None,
            radius: 8.0,
            padding: 4,
            sides: Sides::default(),
        }
    }
}

/// `sides = { top = true, right = false, bottom = true, left = false }`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Sides {
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
    pub left: bool,
}

impl Default for Sides {
    fn default() -> Self {
        Sides::from_array([true; 4])
    }
}

impl Sides {
    /// Top, right, bottom, left (clockwise).
    pub fn array(self) -> [bool; 4] {
        [self.top, self.right, self.bottom, self.left]
    }

    pub fn from_array([top, right, bottom, left]: [bool; 4]) -> Self {
        Sides {
            top,
            right,
            bottom,
            left,
        }
    }

    pub fn is_all(&self) -> bool {
        self.array().iter().all(|&s| s)
    }
}

impl Widget {
    /// The box where the content goes: inside the frame's border and padding.
    pub fn inner(&self) -> Widget {
        // per side: padding, + the border width where there is a border
        let [top, right, bottom, left] = self.frame.as_ref().map_or([0; 4], |f| {
            let width = f.width.max(0.0).ceil() as i32;
            f.sides
                .array()
                .map(|s| f.padding.max(0) + if s { width } else { 0 })
        });
        Widget {
            x: self.x + left,
            y: self.y + top,
            w: (self.w - left - right).max(1),
            h: (self.h - top - bottom).max(1),
            frame: None,
            kind: self.kind.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WidgetKind {
    Rect {
        color: Paint,
        #[serde(default)]
        radius: f32,
    },
    /// Text; `{name}`, `{volume}`, `{mute}` come from knob `knob`'s target, `{page}` = page name.
    Text {
        text: String,
        #[serde(default)]
        knob: Option<u8>,
        #[serde(default = "default_text_size")]
        size: f32,
        /// A gradient spans the widget box (the visible zone), not the text: a scrolling text
        /// moves under it.
        #[serde(default = "white_paint")]
        color: Paint,
        #[serde(default)]
        align: Align,
        #[serde(default)]
        bold: bool,
        /// Too long for the box: scroll back and forth (otherwise cut with "…").
        #[serde(default = "yes")]
        scroll: bool,
    },
    /// Level meter of knob `knob`'s target (green / yellow / red zones by default, + peak hold);
    /// `color` runs along the meter, from empty to full.
    Vu {
        knob: u8,
        #[serde(default = "yes")]
        vertical: bool,
        #[serde(default = "vu_paint")]
        color: Paint,
        #[serde(default, skip_serializing_if = "Gauge::is_bar")]
        gauge: Gauge,
    },
    /// Volume gauge of knob `knob`'s target (greyed out while muted).
    Volume {
        knob: u8,
        /// Runs along the gauge, from empty to full.
        #[serde(default = "accent_paint")]
        color: Paint,
        #[serde(default = "yes")]
        vertical: bool,
        #[serde(default, skip_serializing_if = "Gauge::is_bar")]
        gauge: Gauge,
    },
    /// Straight line across the box, centred in it (horizontal, or vertical); a gradient runs
    /// along it.
    Separator {
        #[serde(default = "separator_paint")]
        color: Paint,
        #[serde(default = "default_thickness")]
        thickness: f32,
        #[serde(default)]
        vertical: bool,
        #[serde(default)]
        style: LineStyle,
        /// Both ends fade out.
        #[serde(default)]
        fade: bool,
    },
    /// The page of a module (`[[module]]` named `module`; `page`: one of its pages, else its
    /// first), scaled into the box; while this page is shown, the `inputs` listed go to the
    /// module (as on its own page), the others keep their bindings. `kind = "module"` (formerly
    /// `"tool"`).
    #[serde(rename = "module", alias = "tool")]
    Tool {
        #[serde(rename = "module", alias = "tool")]
        tool: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        page: String,
        #[serde(default = "default_tool_inputs")]
        inputs: Vec<String>,
        /// Content bigger than the box: `shrink` it (default) or `scroll` it.
        #[serde(default, skip_serializing_if = "Overflow::is_shrink")]
        overflow: Overflow,
    },
    /// « Stream deck » track: `tiles` in an endless carousel, the selected one in the middle;
    /// while this page is shown, turning knob `knob` moves the selection, its click runs the
    /// selected tile's action (the knob's own bindings are off on this page).
    Deck {
        knob: u8,
        #[serde(default = "yes")]
        vertical: bool,
        #[serde(default = "accent")]
        color: Color,
        #[serde(default, rename = "tile")]
        tiles: Vec<Tile>,
    },
    /// PNG image, scaled to the widget box; `radius` rounds its corners (half the box or more:
    /// a disc, like an avatar).
    Image {
        path: String,
        #[serde(default, skip_serializing_if = "is_zero")]
        radius: f32,
    },
    /// Icon of the desktop's theme by name (`audio-input-microphone`, `discord`…) or an image
    /// file (absolute path: SVG, PNG, JPEG…), fitted in the box; `tint` = monochrome version in
    /// that colour when the theme has one.
    ThemeIcon {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tint: Option<Color>,
    },
    /// Icon of knob `knob`'s target (application or device icon of the desktop's icon theme),
    /// fitted in the box, dimmed while muted.
    Icon {
        knob: u8,
        #[serde(default)]
        style: IconStyle,
        /// Colour of the symbolic (monochrome) icons.
        #[serde(default = "light_grey")]
        tint: Color,
    },
    /// Spectrum analyser of `target`'s audio: `bars` log-spaced bands, colour zones from the
    /// bottom (`colors[0]`) up, falling peak caps. Drawn on the device with rectangle fills
    /// (cheap at 60 fps); keep x/y/w/h multiples of 16 for the best efficiency.
    Spectrum {
        #[serde(default = "default_sink")]
        target: Target,
        #[serde(default = "default_bars")]
        bars: u32,
        #[serde(default = "default_gap")]
        gap: i32,
        #[serde(default = "default_spectrum_colors")]
        colors: Vec<Color>,
        #[serde(default)]
        peak: Option<Color>,
        #[serde(default)]
        background: Color,
    },
}

impl WidgetKind {
    /// Kind names as written in the TOML (`kind = "…"`).
    pub const NAMES: [&'static str; 11] = [
        "text",
        "rect",
        "separator",
        "vu",
        "volume",
        "icon",
        "theme_icon",
        "image",
        "spectrum",
        "module",
        "deck",
    ];

    pub fn name(&self) -> &'static str {
        match self {
            WidgetKind::Text { .. } => "text",
            WidgetKind::Rect { .. } => "rect",
            WidgetKind::Separator { .. } => "separator",
            WidgetKind::Vu { .. } => "vu",
            WidgetKind::Volume { .. } => "volume",
            WidgetKind::Image { .. } => "image",
            WidgetKind::ThemeIcon { .. } => "theme_icon",
            WidgetKind::Tool { .. } => "module",
            WidgetKind::Deck { .. } => "deck",
            WidgetKind::Icon { .. } => "icon",
            WidgetKind::Spectrum { .. } => "spectrum",
        }
    }

    /// A new widget of kind `name` with default settings.
    pub fn template(name: &str) -> Option<WidgetKind> {
        Some(match name {
            "text" => WidgetKind::Text {
                text: "{name}".into(),
                knob: Some(1),
                size: default_text_size(),
                color: white().into(),
                align: Align::Center,
                bold: false,
                scroll: true,
            },
            "rect" => WidgetKind::Rect {
                color: accent().into(),
                radius: 6.0,
            },
            "separator" => WidgetKind::Separator {
                color: separator_paint(),
                thickness: default_thickness(),
                vertical: false,
                style: LineStyle::Solid,
                fade: false,
            },
            "vu" => WidgetKind::Vu {
                knob: 1,
                vertical: true,
                color: vu_paint(),
                gauge: Gauge::Bar,
            },
            "volume" => WidgetKind::Volume {
                knob: 1,
                color: accent().into(),
                vertical: true,
                gauge: Gauge::Bar,
            },
            "icon" => WidgetKind::Icon {
                knob: 1,
                style: IconStyle::Auto,
                tint: light_grey(),
            },
            "image" => WidgetKind::Image {
                path: String::new(),
                radius: 0.0,
            },
            "theme_icon" => WidgetKind::ThemeIcon {
                name: "audio-volume-high".into(),
                tint: None,
            },
            "module" => WidgetKind::Tool {
                tool: String::new(),
                page: String::new(),
                inputs: default_tool_inputs(),
                overflow: Overflow::Shrink,
            },
            "deck" => WidgetKind::Deck {
                knob: 1,
                vertical: true,
                color: accent(),
                tiles: Tile::media(),
            },
            "spectrum" => WidgetKind::Spectrum {
                target: Target::DefaultSink,
                bars: default_bars(),
                gap: default_gap(),
                colors: default_spectrum_colors(),
                peak: Some(white()),
                background: Color(0, 0, 0),
            },
            _ => return None,
        })
    }
}

impl Page {
    /// Exactly 4 knobs and 4 buttons (missing ones do nothing), for editing.
    pub fn normalise(&mut self) {
        self.knobs.resize(4, Knob::default());
        self.buttons.resize(4, Button::default());
    }
}

fn default_text_size() -> f32 {
    18.0
}

/// Pattern of a separator line.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

/// A tile of a `deck` widget: what it shows, what the click of its knob does.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Tile {
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
    /// Name in the desktop's icon theme, or an image file (as for `theme_icon`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    #[serde(default)]
    pub action: Action,
}

impl Tile {
    pub fn new(title: &str, subtitle: &str, icon: &str, action: Action) -> Tile {
        Tile {
            title: title.into(),
            subtitle: subtitle.into(),
            icon: icon.into(),
            action,
        }
    }

    /// Play / pause, next, previous, stop of the player that plays.
    pub fn media() -> Vec<Tile> {
        let media = |a: &str| Action::Media { action: a.into() };
        vec![
            Tile::new(tr("Play"), "pause", "media-playback-start", media("play_pause")),
            Tile::new(tr("Next"), tr("next track"), "media-skip-forward", media("next")),
            Tile::new(tr("Previous"), tr("previous track"), "media-skip-backward", media("previous")),
            Tile::new(tr("Stop"), tr("stop"), "media-playback-stop", media("stop")),
        ]
    }
}

/// Inputs of a `tool` widget by default: knob 4 (scrolling a menu) and the 4 buttons.
pub fn default_tool_inputs() -> Vec<String> {
    ["knob4", "button1", "button2", "button3", "button4"]
        .map(String::from)
        .to_vec()
}

fn separator_paint() -> Paint {
    Paint::Solid(Color(0x5a, 0x66, 0x66))
}

fn default_thickness() -> f32 {
    2.0
}

fn white() -> Color {
    Color(255, 255, 255)
}

/// Source icons: `auto` = devices as tinted symbolic icons (their full-colour illustrations are
/// dark on a dark screen), applications in colour; `color` / `symbolic` = all the same way
/// (symbolic falls back to colour when an application has no symbolic icon).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IconStyle {
    #[default]
    Auto,
    Color,
    Symbolic,
}

fn light_grey() -> Color {
    Color(0xde, 0xde, 0xde)
}

/// Look of a level meter / volume gauge (`gauge = { style = "segments", count = 16 }` …): see
/// `gauge.rs`. Angles in degrees, 0 = top, clockwise.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "style", rename_all = "snake_case")]
pub enum Gauge {
    /// Plain bar along the widget (`vertical`).
    #[default]
    Bar,
    /// LED-like blocks along the widget.
    Segments {
        #[serde(default = "default_segments")]
        count: u8,
        #[serde(default = "default_segment_gap")]
        gap: u8,
    },
    /// Arc from `start` to `end`.
    Arc {
        #[serde(default = "arc_start")]
        start: f32,
        #[serde(default = "arc_end")]
        end: f32,
        #[serde(default = "arc_thickness")]
        thickness: f32,
    },
    /// Dial: coloured scale from `start` to `end`, ticks, needle.
    Needle {
        #[serde(default = "arc_start")]
        start: f32,
        #[serde(default = "arc_end")]
        end: f32,
        #[serde(default = "needle_thickness")]
        thickness: f32,
    },
    /// Free shape: the `track` image, and the `fill` image revealed with the level along the
    /// widget, or by angle from `start` to `end` around its centre (`sweep`).
    Image {
        #[serde(default)]
        track: String,
        #[serde(default)]
        fill: String,
        #[serde(default)]
        sweep: bool,
        #[serde(default = "arc_start")]
        start: f32,
        #[serde(default = "arc_end")]
        end: f32,
    },
}

impl Gauge {
    pub fn is_bar(&self) -> bool {
        matches!(self, Gauge::Bar)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Gauge::Bar => "bar",
            Gauge::Segments { .. } => "segments",
            Gauge::Arc { .. } => "arc",
            Gauge::Needle { .. } => "needle",
            Gauge::Image { .. } => "image",
        }
    }

    pub fn template(name: &str) -> Gauge {
        match name {
            "segments" => Gauge::Segments {
                count: default_segments(),
                gap: default_segment_gap(),
            },
            "arc" => Gauge::Arc {
                start: arc_start(),
                end: arc_end(),
                thickness: arc_thickness(),
            },
            "needle" => Gauge::Needle {
                start: arc_start(),
                end: arc_end(),
                thickness: needle_thickness(),
            },
            "image" => Gauge::Image {
                track: String::new(),
                fill: String::new(),
                sweep: false,
                start: arc_start(),
                end: arc_end(),
            },
            _ => Gauge::Bar,
        }
    }
}

fn default_segments() -> u8 {
    16
}

fn default_segment_gap() -> u8 {
    2
}

fn arc_start() -> f32 {
    -135.0
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn arc_end() -> f32 {
    135.0
}

fn arc_thickness() -> f32 {
    10.0
}

fn needle_thickness() -> f32 {
    4.0
}

fn white_paint() -> Paint {
    Paint::Solid(white())
}

fn accent_paint() -> Paint {
    Paint::Solid(accent())
}

/// Green up to 70 %, yellow up to 90 %, then red (sharp zones).
pub fn vu_paint() -> Paint {
    Paint::Stops {
        colors: vec![
            Color(0x10, 0xc0, 0x60),
            Color(0xe0, 0xc0, 0x20),
            Color(0xe0, 0x30, 0x30),
        ],
        stops: vec![0.0, 0.7, 0.9],
        hard: true,
        vertical: false,
    }
}

fn accent() -> Color {
    Color(0x00, 0x6c, 0x68)
}

fn default_sink() -> Target {
    Target::DefaultSink
}

fn default_bars() -> u32 {
    48
}

fn default_gap() -> i32 {
    2
}

/// Bottom → top: the trace teal of the boot logo up to a pale cyan.
fn default_spectrum_colors() -> Vec<Color> {
    vec![
        Color(0x00, 0x6c, 0x68),
        Color(0x00, 0x8c, 0x88),
        Color(0x00, 0xb4, 0xa8),
        Color(0x9a, 0xd8, 0xd0),
    ]
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

/// A colour or a gradient: `"#rrggbb"`, `["#…", "#…"]` (evenly spaced, start → end), or
/// `{ colors = [...], stops = [0.0, 0.7, …], hard = true, vertical = true }` (positions 0..1,
/// `hard` = sharp zones instead of blends, `vertical` = top → bottom for texts and rectangles;
/// gauges and meters always run from empty to full).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Paint {
    Solid(Color),
    Gradient(Vec<Color>),
    Stops {
        colors: Vec<Color>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        stops: Vec<f32>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        hard: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        vertical: bool,
    },
}

impl From<Color> for Paint {
    fn from(c: Color) -> Self {
        Paint::Solid(c)
    }
}

impl Paint {
    /// Colour stops (position 0..1, colour) in order; sharp zones repeat a position.
    pub fn stops(&self) -> Vec<(f32, Color)> {
        let (colors, stops, hard) = match self {
            Paint::Solid(c) => return vec![(0.0, *c), (1.0, *c)],
            Paint::Gradient(colors) => (colors.as_slice(), &[][..], false),
            Paint::Stops {
                colors,
                stops,
                hard,
                ..
            } => (colors.as_slice(), stops.as_slice(), *hard),
        };
        let n = colors.len();
        if n == 0 {
            return vec![(0.0, Color(255, 255, 255)), (1.0, Color(255, 255, 255))];
        }
        let pos = |i: usize| -> f32 {
            if stops.len() == n {
                stops[i].clamp(0.0, 1.0)
            } else if hard {
                i as f32 / n as f32
            } else if n == 1 {
                0.0
            } else {
                i as f32 / (n - 1) as f32
            }
        };
        let mut out = Vec::new();
        for (i, &c) in colors.iter().enumerate() {
            if hard {
                let end = if i + 1 < n { pos(i + 1) } else { 1.0 };
                out.push((pos(i), c));
                out.push((end, c));
            } else {
                out.push((pos(i), c));
            }
        }
        if out.len() == 1 {
            out.push((1.0, out[0].1));
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    /// Colour at position `t` (0..1).
    pub fn at(&self, t: f32) -> Color {
        let s = self.stops();
        let t = t.clamp(0.0, 1.0);
        if t <= s[0].0 {
            return s[0].1;
        }
        for w in s.windows(2) {
            let ((p0, c0), (p1, c1)) = (w[0], w[1]);
            if t <= p1 {
                let k = if p1 > p0 { (t - p0) / (p1 - p0) } else { 1.0 };
                let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
                return Color(mix(c0.0, c1.0), mix(c0.1, c1.1), mix(c0.2, c1.2));
            }
        }
        s[s.len() - 1].1
    }

    /// Texts and rectangles: top → bottom instead of left → right.
    pub fn vertical(&self) -> bool {
        matches!(self, Paint::Stops { vertical: true, .. })
    }
}

/// `#rrggbb` colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Color(pub u8, pub u8, pub u8);

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2))
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let h = s.trim_start_matches('#');
        let v = u32::from_str_radix(h, 16).map_err(serde::de::Error::custom)?;
        if h.len() != 6 {
            return Err(serde::de::Error::custom(format!(
                "colour {s:?}: expected #rrggbb"
            )));
        }
        Ok(Color((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            });
        base.join("hsm01").join("config.toml")
    }

    /// Parse the existing user config.
    pub fn load() -> Result<Config, String> {
        let path = Self::path();
        let s = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Modification time of the config file (hot reload).
    pub fn mtime() -> Option<std::time::SystemTime> {
        std::fs::metadata(Self::path())
            .and_then(|m| m.modified())
            .ok()
    }

    /// Load the user config, or the built-in default (written out on first run).
    pub fn load_or_default() -> Result<Config, String> {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let cfg = Config::default();
                cfg.save().map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(cfg)
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Write the config, keeping the comments and layout of the existing file where values did
    /// not change (the previous file is also kept as `config.toml.bak`).
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let old = std::fs::read_to_string(&path).ok();
        if old.is_some() {
            std::fs::copy(&path, path.with_extension("toml.bak"))?;
        }
        std::fs::write(path, self.to_toml(old.as_deref())?)
    }

    /// TOML text of the config; with `old`, the existing text is updated in place instead.
    fn to_toml(&self, old: Option<&str>) -> std::io::Result<String> {
        let new = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let merged = old.and_then(|old| {
            let mut doc: toml_edit::DocumentMut = old.parse().ok()?;
            let fresh: toml_edit::DocumentMut = new.parse().ok()?;
            keep::table(doc.as_table_mut(), fresh.as_table());
            let text = doc.to_string();
            // never write a file that does not read back as this exact config
            if toml::from_str::<Config>(&text).ok().as_ref() == Some(self) {
                Some(text)
            } else {
                eprintln!("hsm01-app: {}", tr("merge with the old file incorrect, full rewrite"));
                None
            }
        });
        Ok(merged.unwrap_or(new))
    }
}

/// Merge a freshly serialised document into the existing one: unchanged values keep their text,
/// comments and layout; changed values keep their surrounding comments; tables of a `[[…]]`
/// array are matched by position. Tables copied from the fresh document lose their document
/// position: toml_edit then writes them right after their parent (with the fresh document's
/// positions, a new `[page.widget.color]` landed under the next widget → duplicate key).
pub(crate) mod keep {
    use toml_edit::{Item, Table, Value};

    pub fn table(old: &mut Table, new: &Table) {
        let gone: Vec<String> = old
            .iter()
            .map(|(k, _)| k.to_owned())
            .filter(|k| !new.contains_key(k))
            .collect();
        for k in gone {
            old.remove(&k);
        }
        for (k, item) in new.iter() {
            match old.get_mut(k) {
                Some(o) => self::item(o, item),
                None => {
                    old.insert(k, unplaced(item));
                }
            }
        }
    }

    fn item(old: &mut Item, new: &Item) {
        match (old, new) {
            (Item::Table(o), Item::Table(n)) => table(o, n),
            (Item::ArrayOfTables(o), Item::ArrayOfTables(n)) => {
                while o.len() > n.len() {
                    o.remove(o.len() - 1);
                }
                for (i, t) in n.iter().enumerate() {
                    match o.get_mut(i) {
                        Some(ot) => table(ot, t),
                        None => {
                            let mut t = t.clone();
                            unplace_table(&mut t);
                            o.push(t);
                        }
                    }
                }
            }
            (Item::Value(o), Item::Value(n)) => value(o, n.clone()),
            // a value becoming a table (`color = "#…"` → gradient), or a hand-written inline
            // table (`target = { type = "app" }`): stays inline, in place
            (Item::Value(o), Item::Table(n)) => {
                value(o, Value::InlineTable(n.clone().into_inline_table()));
            }
            (o, n) => *o = unplaced(n),
        }
    }

    /// Copy of `item` without document positions (see the module doc).
    fn unplaced(item: &Item) -> Item {
        let mut item = item.clone();
        match &mut item {
            Item::Table(t) => unplace_table(t),
            Item::ArrayOfTables(a) => a.iter_mut().for_each(unplace_table),
            _ => {}
        }
        item
    }

    fn unplace_table(t: &mut Table) {
        t.set_position(None);
        for (_, item) in t.iter_mut() {
            match item {
                Item::Table(t) => unplace_table(t),
                Item::ArrayOfTables(a) => a.iter_mut().for_each(unplace_table),
                _ => {}
            }
        }
    }

    fn value(old: &mut Value, new: Value) {
        if !same(old, &new) {
            let decor = old.decor().clone();
            *old = new;
            *old.decor_mut() = decor;
        }
    }

    fn same(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::String(x), Value::String(y)) => x.value() == y.value(),
            (Value::Integer(x), Value::Integer(y)) => x.value() == y.value(),
            (Value::Float(x), Value::Float(y)) => x.value() == y.value(),
            (Value::Boolean(x), Value::Boolean(y)) => x.value() == y.value(),
            (Value::Datetime(x), Value::Datetime(y)) => x.value() == y.value(),
            (Value::Array(x), Value::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| same(p, q))
            }
            (Value::InlineTable(x), Value::InlineTable(y)) => {
                x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
            }
            _ => false,
        }
    }
}

impl Default for Config {
    /// One "Audio" page: 4 columns like the official software (name, VU + volume, value).
    fn default() -> Self {
        let targets = [
            Target::DefaultSink,
            Target::DefaultSource,
            Target::App {
                name: "firefox".into(),
            },
            Target::App {
                name: "spotify".into(),
            },
        ];
        let mut widgets = vec![Widget {
            x: 0,
            y: 0,
            w: SCREEN_W,
            h: 26,
            frame: None,
            kind: WidgetKind::Text {
                text: "{page}".into(),
                knob: None,
                size: 18.0,
                color: Color(0x9a, 0xd8, 0xd0).into(),
                align: Align::Center,
                bold: true,
                scroll: true,
            },
        }];
        let col_w = SCREEN_W / 4;
        for k in 0..4u8 {
            let x0 = k as i32 * col_w;
            widgets.push(Widget {
                x: x0 + 4,
                y: 30,
                w: col_w - 8,
                h: 22,
                frame: None,
                kind: WidgetKind::Text {
                    text: "{name}".into(),
                    knob: Some(k + 1),
                    size: 16.0,
                    color: Color(255, 255, 255).into(),
                    align: Align::Center,
                    bold: false,
                    scroll: true,
                },
            });
            widgets.push(Widget {
                x: x0 + 30,
                y: 58,
                w: 26,
                h: 170,
                frame: None,
                kind: WidgetKind::Vu {
                    knob: k + 1,
                    vertical: true,
                    color: vu_paint(),
                    gauge: Gauge::Bar,
                },
            });
            widgets.push(Widget {
                x: x0 + 64,
                y: 58,
                w: 26,
                h: 170,
                frame: None,
                kind: WidgetKind::Volume {
                    knob: k + 1,
                    color: accent().into(),
                    vertical: true,
                    gauge: Gauge::Bar,
                },
            });
            widgets.push(Widget {
                x: x0 + 4,
                y: 236,
                w: col_w - 8,
                h: 30,
                frame: None,
                kind: WidgetKind::Text {
                    text: "{volume}{mute}".into(),
                    knob: Some(k + 1),
                    size: 20.0,
                    color: Color(255, 255, 255).into(),
                    align: Align::Center,
                    bold: true,
                    scroll: true,
                },
            });
        }
        let audio = Page {
            name: "Audio".into(),
            background: Color(0, 0, 0),
            knobs: targets
                .into_iter()
                .enumerate()
                .map(|(i, t)| Knob {
                    target: t,
                    click: Action::Mute { knob: i as u8 + 1 },
                    // knob 1 held: the modules menu
                    hold: if i == 0 { Action::ModuleMenu } else { Action::None },
                })
                .collect(),
            buttons: (1..=4)
                .map(|k| Button {
                    action: Action::Mute { knob: k },
                    hold: Action::None,
                    led: Led::Muted { knob: k },
                })
                .collect(),
            power: Action::NextPage,
            power_hold: Action::None,
            wallpaper: None,
            widgets,
        };
        Config {
            language: default_language(),
            brightness: default_brightness(),
            counts_per_percent: default_counts_per_percent(),
            standby: Standby::default(),
            calibration: Calibration::default(),
            notifications: Notifications::default(),
            api: ApiSettings::default(),
            tools: Vec::new(),
            pages: {
                let visualizer = visualizer_page(&audio);
                vec![audio, visualizer]
            },
        }
    }
}

/// "Visualiseur" page: same knobs / buttons as `base`, header + full-screen spectrum analyser
/// (tile-aligned: y 32, h 240).
pub fn visualizer_page(base: &Page) -> Page {
    Page {
        name: tr("Visualizer").into(),
        background: Color(0, 0, 0),
        knobs: base.knobs.clone(),
        buttons: base.buttons.clone(),
        power: base.power.clone(),
        power_hold: base.power_hold.clone(),
        wallpaper: None,
        widgets: vec![
            Widget {
                x: 8,
                y: 4,
                w: SCREEN_W - 120,
                h: 24,
                frame: None,
                kind: WidgetKind::Text {
                    text: "{name}".into(),
                    knob: Some(1),
                    size: 16.0,
                    color: Color(0x9a, 0xd8, 0xd0).into(),
                    align: Align::Left,
                    bold: false,
                    scroll: true,
                },
            },
            Widget {
                x: SCREEN_W - 108,
                y: 4,
                w: 100,
                h: 24,
                frame: None,
                kind: WidgetKind::Text {
                    text: "{volume}{mute}".into(),
                    knob: Some(1),
                    size: 16.0,
                    color: Color(0xff, 0xff, 0xff).into(),
                    align: Align::Right,
                    bold: true,
                    scroll: true,
                },
            },
            Widget {
                x: 0,
                y: 32,
                w: SCREEN_W,
                h: SCREEN_H - 32,
                frame: None,
                kind: WidgetKind::Spectrum {
                    target: Target::DefaultSink,
                    bars: default_bars(),
                    gap: default_gap(),
                    colors: default_spectrum_colors(),
                    peak: Some(white()),
                    background: Color(0, 0, 0),
                },
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips_through_toml() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
        assert!(text.contains("type = \"default_sink\""), "{text}");
    }

    #[test]
    fn colour_parsing() {
        let c: Color = toml::from_str::<toml::Table>("c = \"#006c68\"").unwrap()["c"]
            .clone()
            .try_into()
            .unwrap();
        assert_eq!(c, Color(0, 0x6c, 0x68));
        assert!(
            toml::from_str::<toml::Table>("c = \"#12\"").unwrap()["c"]
                .clone()
                .try_into::<Color>()
                .is_err()
        );
    }

    /// Structural changes (a new sub-table in a widget, a new widget on the first page) must land
    /// in the right place: a VU without `color` in the file got its default gradient as
    /// `[page.widget.color]` under the next widget (duplicate key, config unreadable).
    #[test]
    fn saving_places_new_tables_correctly() {
        let mut cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        // the file as written before the VU had a colour
        let old: String = {
            let mut doc: toml_edit::DocumentMut = text.parse().unwrap();
            let pages = doc["page"].as_array_of_tables_mut().unwrap();
            for page in pages.iter_mut() {
                for w in page["widget"].as_array_of_tables_mut().unwrap().iter_mut() {
                    if w.get("kind").and_then(|k| k.as_str()) == Some("vu") {
                        w.remove("color");
                    }
                }
            }
            doc.to_string()
        };
        assert!(toml::from_str::<Config>(&old).is_ok());
        cfg.pages[0].widgets.push(Widget {
            x: 1,
            y: 2,
            w: 30,
            h: 40,
            frame: None,
            kind: WidgetKind::template("vu").unwrap(),
        });
        if let WidgetKind::Volume { color, .. } = &mut cfg.pages[0].widgets[3].kind {
            *color = Paint::Gradient(vec![Color(1, 2, 3), Color(4, 5, 6)]);
        }
        let saved = cfg.to_toml(Some(&old)).unwrap();
        let back: Config = toml::from_str(&saved).unwrap_or_else(|e| panic!("{e}\n{saved}"));
        assert_eq!(back, cfg);
    }

    /// A file without the newer settings (frame sides, separator, notifications) gets them on
    /// save, comments kept, and reads back the same.
    #[test]
    fn saving_sides_separators_and_notifications() {
        let old = format!(
            "# mon fichier\n{}",
            toml::to_string_pretty(&Config::default()).unwrap()
        );
        let mut cfg: Config = toml::from_str(&old).unwrap();
        cfg.notifications.apps = vec!["discord".into(), "element".into()];
        cfg.notifications.wake = false;
        cfg.tools.push(Tool {
            name: "Lecteur média".into(),
            enabled: true,
            module: "media".into(),
            args: vec!["--alert".into()],
            command: String::new(),
            keep_awake: false,
        });
        let mut sep = WidgetKind::template("separator").unwrap();
        if let WidgetKind::Separator { style, fade, .. } = &mut sep {
            *style = LineStyle::Dotted;
            *fade = true;
        }
        cfg.pages[0].widgets.push(Widget {
            x: 0,
            y: 100,
            w: 200,
            h: 12,
            frame: None,
            kind: sep,
        });
        cfg.pages[0].widgets[0].frame = Some(Frame {
            sides: Sides::from_array([false, false, true, false]),
            ..Default::default()
        });
        let saved = cfg.to_toml(Some(&old)).unwrap();
        assert!(saved.starts_with("# mon fichier"));
        let back: Config = toml::from_str(&saved).unwrap_or_else(|e| panic!("{e}\n{saved}"));
        assert_eq!(back, cfg);
    }

    /// A deck widget (tiles with their actions) is written as `[[page.widget.tile]]` tables, into
    /// an existing file too, and reads back the same; a hand-written one reads with defaults.
    #[test]
    fn deck_widgets_in_toml() {
        let old = toml::to_string_pretty(&Config::default()).unwrap();
        let mut cfg: Config = toml::from_str(&old).unwrap();
        let mut deck = WidgetKind::template("deck").unwrap();
        if let WidgetKind::Deck { tiles, .. } = &mut deck {
            tiles.push(Tile::new("Terminal", "", "utilities-terminal", Action::Command {
                cmd: "konsole".into(),
            }));
        }
        for page in &mut cfg.pages {
            page.widgets.push(Widget {
                x: 4,
                y: 12,
                w: 112,
                h: 248,
                frame: None,
                kind: deck.clone(),
            });
        }
        for saved in [cfg.to_toml(None).unwrap(), cfg.to_toml(Some(&old)).unwrap()] {
            assert!(saved.contains("[[page.widget.tile]]"), "{saved}");
            let back: Config = toml::from_str(&saved).unwrap_or_else(|e| panic!("{e}\n{saved}"));
            assert_eq!(back, cfg);
        }
        let w: Widget = toml::from_str(
            "x = 0\ny = 0\nw = 480\nh = 72\nkind = \"deck\"\nknob = 2\nvertical = false\n\
             [[tile]]\ntitle = \"Suivant\"\naction = { type = \"media\", action = \"next\" }\n",
        )
        .unwrap();
        let WidgetKind::Deck { knob, vertical, color, tiles } = w.kind else {
            panic!("not a deck");
        };
        assert_eq!((knob, vertical, color), (2, false, accent()));
        assert_eq!(tiles, [Tile::new("Suivant", "", "", Action::Media { action: "next".into() })]);
    }

    #[test]
    fn frames_in_toml() {
        let w: Widget = toml::from_str(
            "x = 10\ny = 10\nw = 100\nh = 50\nkind = \"rect\"\ncolor = \"#ffffff\"\n\
             frame = { border = \"#ff0000\", padding = 6 }\n",
        )
        .unwrap();
        let f = w.frame.clone().unwrap();
        assert_eq!(f.border, Paint::Solid(Color(255, 0, 0)));
        assert_eq!(
            (f.width, f.radius, f.padding),
            (2.0, 8.0, 6),
            "defaults for the rest"
        );
        let inner = w.inner();
        assert_eq!((inner.x, inner.y, inner.w, inner.h), (18, 18, 84, 34));
        // no frame: nothing written
        let plain = Widget { frame: None, ..w };
        assert!(!toml::to_string(&plain).unwrap().contains("frame"));
    }

    #[test]
    fn gauge_styles_in_toml() {
        let w: Widget = toml::from_str(
            "x = 0\ny = 0\nw = 100\nh = 100\nkind = \"volume\"\nknob = 1\n\
             gauge = { style = \"arc\", start = -120.0 }\n",
        )
        .unwrap();
        let WidgetKind::Volume { gauge, .. } = &w.kind else {
            panic!("{w:?}")
        };
        assert_eq!(
            *gauge,
            Gauge::Arc {
                start: -120.0,
                end: 135.0,
                thickness: 10.0
            }
        );
        // the default bar is not written
        let bar = WidgetKind::template("volume").unwrap();
        let text = toml::to_string(&Widget {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            frame: None,
            kind: bar,
        })
        .unwrap();
        assert!(!text.contains("gauge"), "{text}");
    }

    #[test]
    fn saving_keeps_comments_of_a_hand_written_file() {
        let old = r##"# my Stream 100
brightness = 80 # evening
counts_per_percent = 8

[[page]]
name = "Audio" # main page
background = "#000000"

# knob 1: speakers
[[page.knobs]]
target = { type = "default_sink" }
click = { type = "mute", knob = 1 }

[[page.knobs]]
target = { type = "app", name = "firefox" } # browser

[[page.widget]]
kind = "text"
x = 0
y = 0
w = 480
h = 30
text = "{page}"
size = 20.0
"##;
        let mut cfg: Config = toml::from_str(old).unwrap();
        cfg.brightness = 60;
        cfg.pages[0].knobs[1].target = Target::App {
            name: "vivaldi".into(),
        };
        cfg.pages[0].widgets[0].y = 4;
        let text = cfg.to_toml(Some(old)).unwrap();
        for kept in [
            "# my Stream 100",
            "# evening",
            "name = \"Audio\" # main page",
            "# knob 1: speakers",
            "target = { type = \"default_sink\" }",
            "# browser",
        ] {
            assert!(text.contains(kept), "{kept:?} lost:\n{text}");
        }
        assert!(text.contains("brightness = 60 # evening"), "{text}");
        assert!(text.contains("name = \"vivaldi\""), "{text}");
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), cfg);
    }
}
