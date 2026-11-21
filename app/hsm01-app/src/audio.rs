//! Audio back-end interface: PipeWire (`crate::pulse`) or `SimAudio` (volumes and mutes in memory,
//! meters following a synthetic signal – for tests and machines without PipeWire).
//! Back-ends are created inside the engine thread (libpulse objects are not `Send`).

use std::collections::HashMap;
use std::time::Instant;

use crate::i18n::tr;
use crate::config::Target;
use crate::state::TargetState;

pub trait Audio {
    /// Called once per frame before `state()` (refresh inventory, open/close meters).
    fn begin_frame(&mut self) {}
    /// Name, presence, volume, mute and meter level (0..1, dB-scaled) of `t`.
    fn state(&mut self, t: &Target) -> TargetState;
    /// Change the volume by `delta` (1.0 = 100 %), clamped to 0..=1.5.
    fn change_volume(&mut self, t: &Target, delta: f32);
    fn toggle_mute(&mut self, t: &Target);
    /// Latest `n` mono samples of `t`'s signal (oldest first) for the spectrum analyser; returns
    /// the sample rate, or `None` while nothing is available yet.
    fn pcm(&mut self, _t: &Target, _n: usize, _out: &mut Vec<f32>) -> Option<u32> {
        None
    }
    /// The full-screen window (KWin), for `Target::Fullscreen`.
    fn set_fullscreen(&mut self, _app: Option<crate::fullscreen::FullscreenApp>) {}
    /// What can be chosen as a target right now (suggestions in the GUI).
    fn inventory(&mut self) -> Inventory {
        Inventory::default()
    }
}

/// Audio objects present now, for the GUI's target lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inventory {
    /// Output devices: (name, description).
    pub sinks: Vec<(String, String)>,
    /// Input devices (monitors excluded): (name, description).
    pub sources: Vec<(String, String)>,
    /// Playing applications: (application name, binary), without duplicates.
    pub apps: Vec<(String, String)>,
}

/// Sample rate of the PCM captured for the spectrum analyser.
pub const PCM_RATE: u32 = 48_000;

/// PipeWire if reachable (unless `simulate`), otherwise the simulation.
pub fn open(simulate: bool) -> Box<dyn Audio> {
    if !simulate {
        match crate::pulse::PulseAudio::new() {
            Ok(p) => return Box::new(p),
            Err(e) => eprintln!("hsm01-app: {}", tf!("{e} – simulated audio", e = e)),
        }
    }
    Box::<SimAudio>::default()
}

pub struct SimAudio {
    t0: Instant,
    values: HashMap<Target, (f32, bool)>,
}

impl Default for SimAudio {
    fn default() -> Self {
        SimAudio {
            t0: Instant::now(),
            values: HashMap::new(),
        }
    }
}

fn label(t: &Target) -> Option<String> {
    Some(match t {
        Target::None | Target::Fullscreen | Target::Tool { .. } => return None,
        Target::DefaultSink => tr("Output").into(),
        Target::DefaultSource => tr("Mic").into(),
        Target::Sink { name } | Target::Source { name } | Target::App { name } => name.clone(),
    })
}

impl Audio for SimAudio {
    fn state(&mut self, t: &Target) -> TargetState {
        let Some(name) = label(t) else {
            return TargetState::default();
        };
        let (volume, muted) = *self.values.entry(t.clone()).or_insert((0.5, false));
        let seed = name.bytes().map(u32::from).sum::<u32>() as f32;
        let x = self.t0.elapsed().as_secs_f32() * (1.3 + seed % 7.0 * 0.2) + seed;
        let signal =
            (0.55 + 0.3 * x.sin() + 0.15 * (x * 3.7).sin()).clamp(0.0, 1.0) * volume.min(1.0);
        let icon = match t {
            Target::DefaultSink | Target::Sink { .. } => "audio-speakers".into(),
            Target::DefaultSource | Target::Source { .. } => "audio-input-microphone".into(),
            _ => name.to_lowercase(),
        };
        TargetState {
            name,
            present: true,
            volume,
            muted,
            level: if muted { 0.0 } else { signal },
            peak_hold: 0.0,
            icon,
            device: !matches!(t, Target::App { .. }),
        }
    }

    fn change_volume(&mut self, t: &Target, delta: f32) {
        if let Some(v) = self.values.get_mut(t) {
            v.0 = (v.0 + delta).clamp(0.0, 1.5);
        }
    }

    fn inventory(&mut self) -> Inventory {
        let pair = |a: &str, b: &str| (a.to_owned(), b.to_owned());
        Inventory {
            sinks: vec![pair("sim_sortie", tr("Simulated output"))],
            sources: vec![pair("sim_micro", tr("Simulated mic"))],
            apps: vec![pair(tr("Music"), "musique"), pair(tr("Browser"), "navigateur")],
        }
    }

    fn toggle_mute(&mut self, t: &Target) {
        if let Some(v) = self.values.get_mut(t) {
            v.1 = !v.1;
        }
    }

    /// Synthetic music-like signal: bass pulse, a few tones drifting in level, some noise.
    fn pcm(&mut self, t: &Target, n: usize, out: &mut Vec<f32>) -> Option<u32> {
        label(t)?;
        let t_end = self.t0.elapsed().as_secs_f64();
        let mut noise = (t_end * 1e6) as u32 | 1;
        out.clear();
        for i in 0..n {
            let s = t_end - (n - i) as f64 / PCM_RATE as f64;
            let beat = (1.0 - (s * 2.0).fract()).powi(3);
            let tones = [
                (55.0, beat),
                (220.0, 0.3 + 0.3 * (s * 0.7).sin()),
                (880.0, 0.2 + 0.2 * (s * 1.3).sin()),
                (3500.0, 0.1),
            ];
            let mut v: f64 = tones
                .iter()
                .map(|(f, a)| a * (s * f * std::f64::consts::TAU).sin())
                .sum();
            noise ^= noise << 13;
            noise ^= noise >> 17;
            noise ^= noise << 5;
            v += (noise as f64 / u32::MAX as f64 - 0.5) * 0.05;
            out.push((v * 0.4) as f32);
        }
        Some(PCM_RATE)
    }
}
