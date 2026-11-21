//! PipeWire audio through the PulseAudio client API (pipewire-pulse), like pavucontrol:
//! inventory of sinks / sources / application streams kept up to date by subscription events,
//! volume + mute control, and peak-detect record streams for the meters (one per displayed
//! target, an application being metered through its own stream with `set_monitor_stream`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libpulse_binding as pa;
use pa::callbacks::ListResult;
use pa::context::subscribe::InterestMaskSet;
use pa::context::{Context, FlagSet as ContextFlags, State as ContextState};
use pa::def::BufferAttr;
use pa::mainloop::threaded::Mainloop;
use pa::proplist::properties;
use pa::sample::{Format, Spec};
use pa::stream::{FlagSet as StreamFlags, PeekResult, Stream};
use pa::volume::{ChannelVolumes, Volume};

use crate::audio::{Audio, Inventory, PCM_RATE};
use crate::config::Target;
use crate::state::{TargetState, level_from_peak};

/// Meter samples per second requested from the server.
const METER_RATE: u32 = 60;
/// Frames a meter stream survives without being displayed.
const METER_GRACE_FRAMES: u32 = 120;

#[derive(Debug, Clone)]
struct Device {
    index: u32,
    name: String,
    description: String,
    volume: ChannelVolumes,
    muted: bool,
    /// Sinks: index of their monitor source.
    monitor: u32,
    /// Sources: true for "monitor of …" sources (hidden from targets).
    is_monitor: bool,
    /// `device.icon_name`.
    icon: String,
}

#[derive(Debug, Clone)]
struct AppStream {
    index: u32,
    app: String,
    binary: String,
    /// `media.name` – the only name of loopback streams (e.g. "Spotify Playback output").
    media: String,
    sink: u32,
    volume: ChannelVolumes,
    muted: bool,
    has_volume: bool,
    /// `application.icon_name`.
    icon: String,
    /// `application.process.id` (0 if unknown).
    pid: u32,
}

#[derive(Debug, Default)]
struct Model {
    default_sink: String,
    default_source: String,
    sinks: Vec<Device>,
    sources: Vec<Device>,
    apps: Vec<AppStream>,
}

/// What a target currently resolves to.
enum Resolved {
    Sink(Device),
    Source(Device),
    Apps(Vec<AppStream>),
}

/// Meter source: a source (sink monitor or input) or one application stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MeterKey {
    Source(u32),
    App { stream: u32, monitor: u32 },
}

struct Meter {
    stream: Rc<RefCell<Stream>>,
    unused_frames: u32,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    /// Peak-detect stream (meters).
    Peak,
    /// Raw PCM (spectrum analyser).
    Pcm,
}

/// PCM samples kept per spectrum source.
const PCM_KEEP: usize = 8192;

/// What a record stream does with each chunk of samples.
type DataSink = Box<dyn FnMut(&[u8])>;

pub struct PulseAudio {
    mainloop: Mainloop,
    context: Context,
    model: Arc<Mutex<Model>>,
    dirty: Arc<AtomicBool>,
    peaks: Arc<Mutex<HashMap<MeterKey, f32>>>,
    meters: HashMap<MeterKey, Meter>,
    wanted: HashSet<MeterKey>,
    pcm: Arc<Mutex<HashMap<MeterKey, VecDeque<f32>>>>,
    pcm_streams: HashMap<MeterKey, Meter>,
    pcm_wanted: HashSet<MeterKey>,
    /// Keys whose stream could not be opened (retried after the next inventory change).
    failed: HashSet<MeterKey>,
    /// Full-screen window (KWin) and its icon.
    fullscreen: Option<(crate::fullscreen::FullscreenApp, String)>,
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    !needle.is_empty() && hay.to_lowercase().contains(&needle.to_lowercase())
}

fn fraction(cv: &ChannelVolumes) -> f32 {
    cv.avg().0 as f32 / Volume::NORMAL.0 as f32
}

fn with_fraction(cv: &ChannelVolumes, f: f32) -> ChannelVolumes {
    let mut out = *cv;
    let v = Volume((f.clamp(0.0, 1.5) * Volume::NORMAL.0 as f32) as u32);
    out.set(cv.len().max(1), v);
    out
}

impl PulseAudio {
    pub fn new() -> Result<Self, String> {
        let mut mainloop = Mainloop::new().ok_or("cannot create the PulseAudio main loop")?;
        let mut context =
            Context::new(&mainloop, "hsm01-app").ok_or("cannot create the PulseAudio context")?;
        context
            .connect(None, ContextFlags::NOFLAGS, None)
            .map_err(|e| format!("PulseAudio/PipeWire connection: {e}"))?;
        mainloop.lock();
        mainloop
            .start()
            .map_err(|e| format!("PulseAudio main loop: {e}"))?;
        loop {
            match context.get_state() {
                ContextState::Ready => break,
                ContextState::Failed | ContextState::Terminated => {
                    mainloop.unlock();
                    mainloop.stop();
                    return Err("PulseAudio/PipeWire connection failed".into());
                }
                _ => {
                    mainloop.unlock();
                    std::thread::sleep(Duration::from_millis(10));
                    mainloop.lock();
                }
            }
        }
        let dirty = Arc::new(AtomicBool::new(true));
        let d = dirty.clone();
        context.set_subscribe_callback(Some(Box::new(move |_, _, _| {
            d.store(true, Ordering::Release)
        })));
        context.subscribe(
            InterestMaskSet::SINK
                | InterestMaskSet::SOURCE
                | InterestMaskSet::SINK_INPUT
                | InterestMaskSet::SERVER,
            |_| {},
        );
        mainloop.unlock();
        Ok(PulseAudio {
            mainloop,
            context,
            model: Arc::default(),
            dirty,
            peaks: Arc::default(),
            meters: HashMap::new(),
            wanted: HashSet::new(),
            pcm: Arc::default(),
            pcm_streams: HashMap::new(),
            pcm_wanted: HashSet::new(),
            failed: HashSet::new(),
            fullscreen: None,
        })
    }

    /// Ask the server for the whole inventory again (answers arrive asynchronously).
    fn refresh(&mut self) {
        self.mainloop.lock();
        let intro = self.context.introspect();
        let m = self.model.clone();
        intro.get_server_info(move |i| {
            let mut m = m.lock().unwrap();
            m.default_sink = i.default_sink_name.as_deref().unwrap_or("").to_owned();
            m.default_source = i.default_source_name.as_deref().unwrap_or("").to_owned();
        });
        let (m, mut acc) = (self.model.clone(), Vec::new());
        intro.get_sink_info_list(move |r| match r {
            ListResult::Item(i) => acc.push(Device {
                index: i.index,
                name: i.name.as_deref().unwrap_or("").to_owned(),
                description: i.description.as_deref().unwrap_or("").to_owned(),
                volume: i.volume,
                muted: i.mute,
                monitor: i.monitor_source,
                is_monitor: false,
                icon: i
                    .proplist
                    .get_str(properties::DEVICE_ICON_NAME)
                    .unwrap_or_default(),
            }),
            ListResult::End => m.lock().unwrap().sinks = std::mem::take(&mut acc),
            ListResult::Error => {}
        });
        let (m, mut acc) = (self.model.clone(), Vec::new());
        intro.get_source_info_list(move |r| match r {
            ListResult::Item(i) => acc.push(Device {
                index: i.index,
                name: i.name.as_deref().unwrap_or("").to_owned(),
                description: i.description.as_deref().unwrap_or("").to_owned(),
                volume: i.volume,
                muted: i.mute,
                monitor: i.index,
                is_monitor: i.monitor_of_sink.is_some(),
                icon: i
                    .proplist
                    .get_str(properties::DEVICE_ICON_NAME)
                    .unwrap_or_default(),
            }),
            ListResult::End => m.lock().unwrap().sources = std::mem::take(&mut acc),
            ListResult::Error => {}
        });
        let (m, mut acc) = (self.model.clone(), Vec::new());
        intro.get_sink_input_info_list(move |r| match r {
            ListResult::Item(i) => acc.push(AppStream {
                index: i.index,
                app: i
                    .proplist
                    .get_str(properties::APPLICATION_NAME)
                    .unwrap_or_default(),
                binary: i
                    .proplist
                    .get_str(properties::APPLICATION_PROCESS_BINARY)
                    .unwrap_or_default(),
                media: i
                    .proplist
                    .get_str(properties::MEDIA_NAME)
                    .unwrap_or_default(),
                sink: i.sink,
                volume: i.volume,
                muted: i.mute,
                has_volume: i.has_volume,
                icon: i
                    .proplist
                    .get_str(properties::APPLICATION_ICON_NAME)
                    .unwrap_or_default(),
                pid: i
                    .proplist
                    .get_str(properties::APPLICATION_PROCESS_ID)
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(0),
            }),
            ListResult::End => m.lock().unwrap().apps = std::mem::take(&mut acc),
            ListResult::Error => {}
        });
        self.mainloop.unlock();
    }

    fn resolve(&self, t: &Target) -> Option<Resolved> {
        let m = self.model.lock().unwrap();
        let dev = |list: &[Device], pred: &dyn Fn(&Device) -> bool| {
            list.iter().find(|d| pred(d)).cloned()
        };
        match t {
            Target::None | Target::Tool { .. } => None,
            Target::DefaultSink => dev(&m.sinks, &|d| d.name == m.default_sink).map(Resolved::Sink),
            Target::DefaultSource => {
                dev(&m.sources, &|d| d.name == m.default_source).map(Resolved::Source)
            }
            Target::Sink { name } => dev(&m.sinks, &|d| {
                contains_ci(&d.name, name) || contains_ci(&d.description, name)
            })
            .map(Resolved::Sink),
            Target::Source { name } => dev(&m.sources, &|d| {
                !d.is_monitor && (contains_ci(&d.name, name) || contains_ci(&d.description, name))
            })
            .map(Resolved::Source),
            Target::App { name } => {
                let apps: Vec<_> = m
                    .apps
                    .iter()
                    .filter(|a| contains_ci(&a.app, name) || contains_ci(&a.binary, name))
                    .cloned()
                    .collect();
                (!apps.is_empty()).then_some(Resolved::Apps(apps))
            }
            Target::Fullscreen => {
                // streams of the window's process or of its children, else by name
                let (fs, _) = self.fullscreen.as_ref()?;
                let by_pid: Vec<_> = m
                    .apps
                    .iter()
                    .filter(|a| a.pid != 0 && crate::fullscreen::descends_from(a.pid, fs.pid))
                    .cloned()
                    .collect();
                let apps = if by_pid.is_empty() {
                    let class = fs.class.to_lowercase();
                    m.apps
                        .iter()
                        .filter(|a| {
                            !class.is_empty()
                                && (contains_ci(&a.binary, &class) || contains_ci(&a.app, &class))
                        })
                        .cloned()
                        .collect()
                } else {
                    by_pid
                };
                (!apps.is_empty()).then_some(Resolved::Apps(apps))
            }
        }
    }

    fn meter_key(&self, r: &Resolved) -> Option<MeterKey> {
        match r {
            Resolved::Sink(d) => Some(MeterKey::Source(d.monitor)),
            Resolved::Source(d) => Some(MeterKey::Source(d.index)),
            Resolved::Apps(apps) => {
                let a = apps.first()?;
                let m = self.model.lock().unwrap();
                let monitor = m.sinks.iter().find(|s| s.index == a.sink)?.monitor;
                Some(MeterKey::App {
                    stream: a.index,
                    monitor,
                })
            }
        }
    }

    /// Record stream on `key`'s source: peak-detect (meters) or PCM (spectrum analyser).
    fn open_stream(&mut self, key: MeterKey, kind: Kind) -> Result<Meter, String> {
        let (rate, fragsize, flags, name) = match kind {
            // one float per fragment: a fresh peak as soon as it exists
            Kind::Peak => (
                METER_RATE,
                4,
                StreamFlags::DONT_MOVE | StreamFlags::PEAK_DETECT | StreamFlags::ADJUST_LATENCY,
                "hsm01 meter",
            ),
            // 512 samples (~11 ms) per fragment
            Kind::Pcm => (
                PCM_RATE,
                2048,
                StreamFlags::DONT_MOVE | StreamFlags::ADJUST_LATENCY,
                "hsm01 spectrum",
            ),
        };
        let mut on_data: DataSink = match kind {
            Kind::Peak => {
                let peaks = self.peaks.clone();
                Box::new(move |d| {
                    let peak = d
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|b| f32::from_ne_bytes(*b).abs())
                        .fold(0.0, f32::max);
                    peaks.lock().unwrap().insert(key, peak);
                })
            }
            Kind::Pcm => {
                let bufs = self.pcm.clone();
                Box::new(move |d| {
                    let mut bufs = bufs.lock().unwrap();
                    let ring = bufs.entry(key).or_default();
                    ring.extend(d.as_chunks::<4>().0.iter().map(|b| f32::from_ne_bytes(*b)));
                    let excess = ring.len().saturating_sub(PCM_KEEP);
                    ring.drain(..excess);
                })
            }
        };
        let spec = Spec {
            format: Format::F32le,
            channels: 1,
            rate,
        };
        self.mainloop.lock();
        let result = (|| {
            let mut s = Stream::new(&mut self.context, name, &spec, None).ok_or("Stream::new")?;
            let source = match key {
                MeterKey::Source(src) => src,
                MeterKey::App { stream, monitor } => {
                    s.set_monitor_stream(stream).map_err(|e| format!("{e}"))?;
                    monitor
                }
            };
            let attr = BufferAttr {
                maxlength: u32::MAX,
                tlength: u32::MAX,
                prebuf: u32::MAX,
                minreq: u32::MAX,
                fragsize,
            };
            s.connect_record(Some(&source.to_string()), Some(&attr), flags)
                .map_err(|e| format!("{e}"))?;
            let stream = Rc::new(RefCell::new(s));
            let weak = Rc::downgrade(&stream);
            stream
                .borrow_mut()
                .set_read_callback(Some(Box::new(move |_| {
                    let Some(rc) = weak.upgrade() else { return };
                    let Ok(mut s) = rc.try_borrow_mut() else {
                        return;
                    };
                    loop {
                        match s.peek() {
                            Ok(PeekResult::Data(d)) => {
                                on_data(d);
                                let _ = s.discard();
                            }
                            Ok(PeekResult::Hole(_)) => {
                                let _ = s.discard();
                            }
                            _ => break,
                        }
                    }
                })));
            Ok(Meter {
                stream,
                unused_frames: 0,
            })
        })();
        self.mainloop.unlock();
        result
    }

    /// Open the streams displayed this frame, close those unused for a while.
    fn manage_streams(&mut self, kind: Kind) {
        let (mut streams, wanted) = match kind {
            Kind::Peak => (
                std::mem::take(&mut self.meters),
                std::mem::take(&mut self.wanted),
            ),
            Kind::Pcm => (
                std::mem::take(&mut self.pcm_streams),
                std::mem::take(&mut self.pcm_wanted),
            ),
        };
        for key in &wanted {
            if let Some(m) = streams.get_mut(key) {
                m.unused_frames = 0;
            } else if !self.failed.contains(key) {
                match self.open_stream(*key, kind) {
                    Ok(m) => drop(streams.insert(*key, m)),
                    Err(e) => {
                        eprintln!("hsm01-app: {kind:?} stream {key:?}: {e}");
                        self.failed.insert(*key);
                    }
                }
            }
        }
        let stale: Vec<MeterKey> = streams
            .iter_mut()
            .filter(|(k, _)| !wanted.contains(k))
            .filter_map(|(k, m)| {
                m.unused_frames += 1;
                (m.unused_frames > METER_GRACE_FRAMES).then_some(*k)
            })
            .collect();
        if !stale.is_empty() {
            self.mainloop.lock();
            for k in stale {
                if let Some(m) = streams.remove(&k) {
                    close_stream(&m);
                }
                match kind {
                    Kind::Peak => drop(self.peaks.lock().unwrap().remove(&k)),
                    Kind::Pcm => drop(self.pcm.lock().unwrap().remove(&k)),
                }
            }
            self.mainloop.unlock();
        }
        match kind {
            Kind::Peak => self.meters = streams,
            Kind::Pcm => self.pcm_streams = streams,
        }
    }

    fn set_volume(&mut self, r: &Resolved, f: f32) {
        self.mainloop.lock();
        let mut intro = self.context.introspect();
        match r {
            Resolved::Sink(d) => {
                drop(intro.set_sink_volume_by_index(d.index, &with_fraction(&d.volume, f), None))
            }
            Resolved::Source(d) => {
                drop(intro.set_source_volume_by_index(d.index, &with_fraction(&d.volume, f), None))
            }
            Resolved::Apps(apps) => {
                for a in apps.iter().filter(|a| a.has_volume) {
                    drop(intro.set_sink_input_volume(a.index, &with_fraction(&a.volume, f), None));
                }
            }
        }
        self.mainloop.unlock();
        // optimistic local update: fast knob turns accumulate before the server event comes back
        let mut m = self.model.lock().unwrap();
        match r {
            Resolved::Sink(d) => m
                .sinks
                .iter_mut()
                .filter(|x| x.index == d.index)
                .for_each(|x| x.volume = with_fraction(&x.volume, f)),
            Resolved::Source(d) => m
                .sources
                .iter_mut()
                .filter(|x| x.index == d.index)
                .for_each(|x| x.volume = with_fraction(&x.volume, f)),
            Resolved::Apps(apps) => m
                .apps
                .iter_mut()
                .filter(|x| apps.iter().any(|a| a.index == x.index))
                .for_each(|x| x.volume = with_fraction(&x.volume, f)),
        }
    }
}

impl Audio for PulseAudio {
    fn set_fullscreen(&mut self, app: Option<crate::fullscreen::FullscreenApp>) {
        if self.fullscreen.as_ref().map(|(a, _)| a) != app.as_ref() {
            self.fullscreen = app.map(|a| {
                let icon = crate::icons::window_icon(&a.class, &a.desktop);
                (a, icon)
            });
        }
    }

    fn inventory(&mut self) -> Inventory {
        let m = self.model.lock().unwrap();
        let devices = |list: &[Device]| {
            list.iter()
                .filter(|d| !d.is_monitor)
                .map(|d| (d.name.clone(), d.description.clone()))
                .collect()
        };
        let mut apps: Vec<(String, String)> = Vec::new();
        for a in m
            .apps
            .iter()
            .filter(|a| !a.app.is_empty() || !a.binary.is_empty())
        {
            let pair = (a.app.clone(), a.binary.clone());
            if !apps.contains(&pair) {
                apps.push(pair);
            }
        }
        Inventory {
            sinks: devices(&m.sinks),
            sources: devices(&m.sources),
            apps,
        }
    }

    fn begin_frame(&mut self) {
        if self.dirty.swap(false, Ordering::AcqRel) {
            self.failed.clear();
            self.refresh();
        }
        self.manage_streams(Kind::Peak);
        self.manage_streams(Kind::Pcm);
    }

    fn pcm(&mut self, t: &Target, n: usize, out: &mut Vec<f32>) -> Option<u32> {
        let key = self.meter_key(&self.resolve(t)?)?;
        self.pcm_wanted.insert(key);
        let bufs = self.pcm.lock().unwrap();
        let ring = bufs.get(&key)?;
        if ring.len() < n {
            return None;
        }
        out.clear();
        out.extend(ring.iter().skip(ring.len() - n));
        Some(PCM_RATE)
    }

    fn state(&mut self, t: &Target) -> TargetState {
        let Some(r) = self.resolve(t) else {
            return TargetState::default();
        };
        let key = self.meter_key(&r);
        if let Some(k) = key {
            self.wanted.insert(k);
        }
        let peak = key
            .and_then(|k| self.peaks.lock().unwrap().get(&k).copied())
            .unwrap_or(0.0);
        let fallback = |icon: &str, default: &str| {
            if icon.is_empty() {
                default.to_owned()
            } else {
                icon.to_owned()
            }
        };
        let icon = match &r {
            Resolved::Sink(d) => fallback(&d.icon, "audio-speakers"),
            Resolved::Source(d) => fallback(&d.icon, "audio-input-microphone"),
            Resolved::Apps(apps) => {
                let a = &apps[0];
                // Spotify announces neither icon nor binary: its application name then
                let named = if a.binary.is_empty() {
                    a.app.to_lowercase()
                } else {
                    a.binary.to_lowercase()
                };
                fallback(&a.icon, &named)
            }
        };
        // the full-screen target shows the window's name and icon
        let (name_override, icon) = match (t, &self.fullscreen) {
            (Target::Fullscreen, Some((fs, fs_icon))) => {
                let name = [&fs.caption, &fs.class]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .cloned();
                let icon = if fs_icon.is_empty() {
                    icon
                } else {
                    fs_icon.clone()
                };
                (name, icon)
            }
            _ => (None, icon),
        };
        let (name, volume, muted) = match &r {
            Resolved::Sink(d) | Resolved::Source(d) => {
                let name = if d.description.is_empty() {
                    &d.name
                } else {
                    &d.description
                };
                (name.clone(), fraction(&d.volume), d.muted)
            }
            Resolved::Apps(apps) => {
                let a = &apps[0];
                let name = [&a.app, &a.media, &a.binary]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .unwrap_or(&a.app);
                (
                    name.clone(),
                    fraction(&a.volume),
                    apps.iter().all(|a| a.muted),
                )
            }
        };
        TargetState {
            name: name_override.unwrap_or(name),
            present: true,
            volume,
            muted,
            level: level_from_peak(peak),
            peak_hold: 0.0,
            icon,
            device: !matches!(r, Resolved::Apps(_)),
        }
    }

    fn change_volume(&mut self, t: &Target, delta: f32) {
        if let Some(r) = self.resolve(t) {
            let current = match &r {
                Resolved::Sink(d) | Resolved::Source(d) => fraction(&d.volume),
                Resolved::Apps(apps) => fraction(&apps[0].volume),
            };
            self.set_volume(&r, current + delta);
        }
    }

    fn toggle_mute(&mut self, t: &Target) {
        let Some(r) = self.resolve(t) else { return };
        self.mainloop.lock();
        let mut intro = self.context.introspect();
        match &r {
            Resolved::Sink(d) => drop(intro.set_sink_mute_by_index(d.index, !d.muted, None)),
            Resolved::Source(d) => drop(intro.set_source_mute_by_index(d.index, !d.muted, None)),
            Resolved::Apps(apps) => {
                let mute = !apps.iter().all(|a| a.muted);
                for a in apps {
                    drop(intro.set_sink_input_mute(a.index, mute, None));
                }
            }
        }
        self.mainloop.unlock();
    }
}

/// Disconnect a record stream (mainloop locked). The read callback is removed first: libpulse can
/// still call it until the server confirms the disconnection, after the Rust `Stream` and its
/// closure are gone (use after free → segfault, seen when standby closed every stream at once).
fn close_stream(m: &Meter) {
    let mut s = m.stream.borrow_mut();
    s.set_read_callback(None);
    let _ = s.disconnect();
}

impl Drop for PulseAudio {
    fn drop(&mut self) {
        self.mainloop.lock();
        for (_, m) in self.meters.drain().chain(self.pcm_streams.drain()) {
            close_stream(&m);
        }
        self.context.disconnect();
        self.mainloop.unlock();
        self.mainloop.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens and closes meter + PCM streams many times while audio flows (closing used to free
    /// the read callback while libpulse could still call it: crash on standby).
    /// `cargo test --release -- --ignored --nocapture pulse_stream_churn`
    #[test]
    #[ignore]
    fn pulse_stream_churn() {
        let mut p = PulseAudio::new().expect("PipeWire/PulseAudio reachable");
        let mut pcm = Vec::new();
        for cycle in 0..150 {
            for _ in 0..3 {
                let _ = p.state(&Target::DefaultSink);
                let _ = p.pcm(&Target::DefaultSink, 1024, &mut pcm);
                p.begin_frame();
                std::thread::sleep(Duration::from_millis(10));
            }
            // nothing wanted: the streams close after the grace period
            for _ in 0..=METER_GRACE_FRAMES {
                p.begin_frame();
            }
            assert!(
                p.meters.is_empty() && p.pcm_streams.is_empty(),
                "cycle {cycle}"
            );
        }
    }

    /// Live probe against the running PipeWire: `cargo test -- --ignored --nocapture pulse_probe`.
    #[test]
    #[ignore]
    fn pulse_probe() {
        let mut p = PulseAudio::new().expect("PipeWire/PulseAudio reachable");
        let mut targets = vec![Target::DefaultSink, Target::DefaultSource];
        for _ in 0..30 {
            p.begin_frame();
            std::thread::sleep(Duration::from_millis(16));
        }
        {
            let m = p.model.lock().unwrap();
            println!(
                "default sink {:?}, default source {:?}",
                m.default_sink, m.default_source
            );
            for s in &m.sinks {
                println!(
                    "sink   #{} {:?} {:.0} % mute={}",
                    s.index,
                    s.description,
                    fraction(&s.volume) * 100.0,
                    s.muted
                );
            }
            for a in &m.apps {
                println!(
                    "app    #{} {:?} ({}) sink #{} {:.0} % icon {:?} pid {}",
                    a.index,
                    a.app,
                    a.binary,
                    a.sink,
                    fraction(&a.volume) * 100.0,
                    a.icon,
                    a.pid
                );
                targets.push(Target::App {
                    name: a.binary.clone(),
                });
            }
            assert!(!m.sinks.is_empty(), "inventory received");
        }
        let mut max_level = vec![0f32; targets.len()];
        for _ in 0..60 {
            p.begin_frame();
            for (i, t) in targets.iter().enumerate() {
                max_level[i] = max_level[i].max(p.state(t).level);
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        for (t, l) in targets.iter().zip(&max_level) {
            let s = p.state(t);
            println!(
                "{t:?}: {:?} present={} {:.0} % mute={} max level {l:.2}",
                s.name,
                s.present,
                s.volume * 100.0,
                s.muted
            );
        }
    }
}
