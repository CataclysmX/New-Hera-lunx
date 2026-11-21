//! Keyboard shortcuts through a virtual keyboard (uinput): "ctrl+alt+t", "media_play_pause",
//! "super+e", "f13"… Keys are pressed in order and released in reverse order.
//! Names are evdev key codes, i.e. physical key positions of a US (QWERTY) keyboard: on AZERTY,
//! "a" is the key labelled Q. Access: /dev/uinput writable (udev rule in packaging/udev).

use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};
use crate::i18n::tr;

/// Friendly names → evdev names (without the `KEY_` prefix).
const ALIASES: &[(&str, &str)] = &[
    ("ctrl", "LEFTCTRL"),
    ("control", "LEFTCTRL"),
    ("shift", "LEFTSHIFT"),
    ("alt", "LEFTALT"),
    ("altgr", "RIGHTALT"),
    ("super", "LEFTMETA"),
    ("meta", "LEFTMETA"),
    ("win", "LEFTMETA"),
    ("escape", "ESC"),
    ("return", "ENTER"),
    ("del", "DELETE"),
    ("pgup", "PAGEUP"),
    ("pgdn", "PAGEDOWN"),
    ("print", "SYSRQ"),
    ("printscreen", "SYSRQ"),
    ("media_play_pause", "PLAYPAUSE"),
    ("play_pause", "PLAYPAUSE"),
    ("media_next", "NEXTSONG"),
    ("next", "NEXTSONG"),
    ("media_previous", "PREVIOUSSONG"),
    ("previous", "PREVIOUSSONG"),
    ("prev", "PREVIOUSSONG"),
    ("media_stop", "STOPCD"),
    ("stop", "STOPCD"),
    ("volume_up", "VOLUMEUP"),
    ("volume_down", "VOLUMEDOWN"),
    ("mic_mute", "MICMUTE"),
];

/// Parse "ctrl+alt+t" into key codes.
pub fn parse(spec: &str) -> Result<Vec<KeyCode>, String> {
    spec.split('+')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            let lower = t.to_ascii_lowercase();
            let name = ALIASES.iter().find(|(a, _)| *a == lower).map_or_else(
                || {
                    t.trim_start_matches("KEY_")
                        .trim_start_matches("key_")
                        .to_ascii_uppercase()
                },
                |(_, n)| (*n).to_owned(),
            );
            KeyCode::from_str(&format!("KEY_{name}"))
                .map_err(|_| tf!("unknown key {key} in {spec}", key = format!("{t:?}"), spec = format!("{spec:?}")))
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|k| {
            if k.is_empty() {
                Err(tf!("empty combination {spec}", spec = format!("{spec:?}")))
            } else {
                Ok(k)
            }
        })
}

/// A new input device is used by the compositor only once udev has processed it (~1.5 s on KDE
/// Wayland); keys sent earlier are lost. Wait for udev, then `SETTLE` for the compositor, at most
/// `SETTLE_MAX` in all.
const SETTLE: Duration = Duration::from_millis(200);
const SETTLE_MAX: Duration = Duration::from_secs(4);

/// Shortcuts pressed from their own thread: creating the device and the key timing must not
/// stall the 60 Hz loop. The device is created on the first press and then kept.
pub struct Keys {
    tx: mpsc::Sender<Vec<KeyCode>>,
}

impl Keys {
    pub fn spawn(on_error: impl Fn(String) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel::<Vec<KeyCode>>();
        let worker = move || {
            let mut kb = None;
            for keys in rx {
                if let Err(e) = press_on(&mut kb, &keys) {
                    on_error(e);
                }
            }
        };
        if let Err(e) = thread::Builder::new()
            .name("hsm01-keys".into())
            .spawn(worker)
        {
            eprintln!("hsm01-app: keyboard thread: {e}");
        }
        Keys { tx }
    }

    pub fn press(&self, spec: &str) -> Result<(), String> {
        let keys = parse(spec)?;
        self.tx
            .send(keys)
            .map_err(|_| tr("virtual keyboard thread stopped").to_owned())
    }
}

/// One-shot press (`--keys`): create, wait, press, keep the device until the events are read.
pub fn press_once(spec: &str) -> Result<(), String> {
    let keys = parse(spec)?;
    let mut kb = None;
    press_on(&mut kb, &keys)?;
    thread::sleep(Duration::from_millis(300));
    Ok(())
}

fn press_on(slot: &mut Option<Keyboard>, keys: &[KeyCode]) -> Result<(), String> {
    let kb = match slot {
        Some(kb) => kb,
        None => {
            let mut kb = Keyboard::new()?;
            kb.wait_ready();
            slot.insert(kb)
        }
    };
    let r = kb.press(keys);
    if r.is_err() {
        *slot = None; // recreate next time
    }
    r
}

struct Keyboard {
    dev: VirtualDevice,
}

impl Keyboard {
    fn new() -> Result<Self, String> {
        let mut keys = AttributeSet::<KeyCode>::new();
        // keyboard keys only: with the BTN_* range (0x100..0x160) udev classifies the device as
        // a mouse/joystick too
        for code in (1..0x100).chain(0x160..0x2ff) {
            keys.insert(KeyCode::new(code));
        }
        let dev = VirtualDevice::builder()
            .and_then(|b| b.name("hsm01 virtual keyboard").with_keys(&keys))
            .and_then(|b| b.build())
            .map_err(|e| tf!("virtual keyboard (/dev/uinput): {e}", e = e))?;
        Ok(Keyboard { dev })
    }

    /// Wait until udev has processed the device (its database entry exists without
    /// `ID_PROCESSING`), or `SETTLE_MAX`.
    fn wait_ready(&mut self) {
        let t0 = Instant::now();
        // /sys/devices/virtual/input/inputN/eventM/dev = "13:95" → /run/udev/data/c13:95
        let db = self.dev.get_syspath().ok().and_then(|sys| {
            let event = std::fs::read_dir(&sys)
                .ok()?
                .filter_map(Result::ok)
                .find(|e| e.file_name().as_encoded_bytes().starts_with(b"event"))?;
            let dev = std::fs::read_to_string(event.path().join("dev")).ok()?;
            Some(format!("/run/udev/data/c{}", dev.trim()))
        });
        let processed = || {
            db.as_ref().is_some_and(|p| {
                std::fs::read_to_string(p).is_ok_and(|s| !s.contains("ID_PROCESSING=1"))
            })
        };
        while t0.elapsed() < SETTLE_MAX - SETTLE && !processed() {
            thread::sleep(Duration::from_millis(50));
        }
        thread::sleep(SETTLE);
    }

    fn press(&mut self, keys: &[KeyCode]) -> Result<(), String> {
        let ev = |k: &KeyCode, v| InputEvent::new(EventType::KEY.0, k.code(), v);
        let io = |e: std::io::Error| tf!("virtual keyboard: {e}", e = e);
        for k in keys {
            self.dev.emit(&[ev(k, 1)]).map_err(io)?;
        }
        thread::sleep(Duration::from_millis(20));
        for k in keys.iter().rev() {
            self.dev.emit(&[ev(k, 0)]).map_err(io)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_combinations_and_aliases() {
        assert_eq!(
            parse("ctrl+alt+t").unwrap(),
            vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTALT, KeyCode::KEY_T]
        );
        assert_eq!(
            parse("media_play_pause").unwrap(),
            vec![KeyCode::KEY_PLAYPAUSE]
        );
        assert_eq!(
            parse("Super + F13").unwrap(),
            vec![KeyCode::KEY_LEFTMETA, KeyCode::KEY_F13]
        );
        assert_eq!(parse("KEY_VOLUMEUP").unwrap(), vec![KeyCode::KEY_VOLUMEUP]);
        assert!(parse("ctrl+nope").is_err());
        assert!(parse("").is_err());
    }
}
