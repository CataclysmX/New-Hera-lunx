//! What the screen shows: live state of the 4 knob targets on the current page.

/// State of one knob's audio target.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TargetState {
    /// Display name ("" = no target / not found).
    pub name: String,
    pub present: bool,
    /// 0.0 ..= 1.5 (100 % = 1.0).
    pub volume: f32,
    pub muted: bool,
    /// Meter level 0.0 ..= 1.0 (dB-scaled, see `level_from_peak`).
    pub level: f32,
    /// Peak-hold marker 0.0 ..= 1.0.
    pub peak_hold: f32,
    /// Freedesktop icon name (application or device), "" if unknown.
    pub icon: String,
    /// Output / input device (not an application stream).
    pub device: bool,
}

#[derive(Debug, Clone, Default)]
pub struct View {
    pub page_name: String,
    pub knobs: [TargetState; 4],
    /// Spectrum widgets of the page: widget index → (bar level, peak cap) per bar, 0..1.
    pub spectra: std::collections::HashMap<usize, Vec<(f32, f32)>>,
    /// Seconds since the engine started (scrolling texts).
    pub time: f32,
    /// Animation clock of the wallpapers (s): held while the screen is still being sent.
    pub anim_time: f32,
    /// `tool` widgets of the page: widget index → the tool's page they show.
    pub embedded: std::collections::HashMap<usize, (crate::config::Page, (i32, i32))>,
    /// `deck` widgets of the page: widget index → selection and animation.
    pub decks: std::collections::HashMap<usize, DeckView>,
}

/// The modules menu at this frame: (module name, page name) of each entry, the selected one,
/// how far it has slid in (0 hidden → 1 shown).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MenuView {
    pub items: Vec<(String, String)>,
    pub selected: usize,
    pub shown: f32,
}

/// What a `deck` widget shows at this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DeckView {
    /// Index of the selected tile.
    pub selected: usize,
    /// Slide still to go, in tiles (towards 0): the tiles are drawn `offset` tiles further.
    pub offset: f32,
    /// Flash of the selected tile after a click, 1 → 0.
    pub flash: f32,
}

/// Editing aids drawn on the device itself while the window is open: outline of every widget
/// (`frames`) and of the selected one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    pub frames: bool,
    pub selected: Vec<usize>,
    /// Calibration test pattern instead of the page.
    pub pattern: bool,
}

/// Question shown on the device before a volume goes above 100 %.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmView {
    /// Name of the target whose volume would go above 100 %.
    pub name: String,
    /// "Oui" selected (otherwise "Non").
    pub yes: bool,
}

/// Linear sample peak (0..1) → meter position: -60 dBFS .. 0 dBFS mapped to 0..1.
pub fn level_from_peak(peak: f32) -> f32 {
    if peak <= 0.001 {
        return 0.0;
    }
    ((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_scale() {
        assert_eq!(level_from_peak(0.0), 0.0);
        assert!((level_from_peak(1.0) - 1.0).abs() < 1e-6);
        assert!((level_from_peak(0.1) - 2.0 / 3.0).abs() < 1e-3); // -20 dBFS
    }
}
