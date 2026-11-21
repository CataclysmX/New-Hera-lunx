//! Media players for the open API: the players on the session bus (MPRIS: Spotify, browsers,
//! VLC…) are read twice a second; the one followed is the one playing (the same one as long as
//! it plays), else the last one followed. Its state goes to every client as a `media` event when
//! it changes (and every second while playing, for the position); the `media` request returns it
//! or controls the player (play / pause, next, previous, seek, volume).

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

use crate::engine::Shared;

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PREFIX: &str = "org.mpris.MediaPlayer2.";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaState {
    /// Bus name of the player (`org.mpris.MediaPlayer2.spotify`).
    pub bus: String,
    /// Its name for people (`Spotify`).
    pub player: String,
    /// `Playing`, `Paused` or `Stopped`.
    pub status: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Cover art URL (`file://…` or `https://…`), "" if none.
    pub art: String,
    /// Seconds (0 if unknown).
    pub length: f64,
    pub position: f64,
    /// 0..1, if the player has one.
    pub volume: Option<f64>,
}

impl MediaState {
    pub fn to_json(&self) -> Json {
        json!({
            "player": self.player, "status": self.status, "title": self.title,
            "artist": self.artist, "album": self.album, "art": self.art,
            "length": self.length, "position": self.position, "volume": self.volume,
        })
    }

    /// The same apart from the position.
    fn same_track_state(&self, o: &MediaState) -> bool {
        MediaState { position: 0.0, ..self.clone() } == MediaState { position: 0.0, ..o.clone() }
    }
}

fn connection() -> zbus::Result<&'static Connection> {
    static CONN: OnceLock<Connection> = OnceLock::new();
    if let Some(c) = CONN.get() {
        return Ok(c);
    }
    let c = zbus::blocking::connection::Builder::session()?
        .method_timeout(Duration::from_secs(2))
        .build()?;
    Ok(CONN.get_or_init(|| c))
}

/// Follow the players in a thread of its own until the app quits.
pub fn start(shared: Arc<Shared>) {
    let run = move || {
        while !shared.quit.load(Ordering::Relaxed) {
            if let Err(e) = watch(&shared) {
                eprintln!("hsm01-app: media: {e}");
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("hsm01-media".into())
        .spawn(run)
    {
        eprintln!("hsm01-app: media: {e}");
    }
}

fn watch(shared: &Shared) -> zbus::Result<()> {
    let conn = connection()?;
    let mut names: HashMap<String, String> = HashMap::new(); // bus → identity
    let mut followed: Option<String> = None;
    let mut sent: Option<(MediaState, Instant)> = None;
    let mut none_sent = false;
    while !shared.quit.load(Ordering::Relaxed) {
        let players = players(conn)?;
        names.retain(|bus, _| players.contains(bus));
        let mut states = Vec::new();
        for bus in &players {
            let identity = names
                .entry(bus.clone())
                .or_insert_with(|| identity(conn, bus))
                .clone();
            if let Ok(s) = read(conn, bus, identity) {
                states.push(s);
            }
        }
        let playing = |s: &&MediaState| s.status == "Playing";
        let is_followed = |s: &&MediaState| Some(&s.bus) == followed.as_ref();
        let pick = states
            .iter()
            .find(|s| playing(s) && is_followed(s))
            .or_else(|| states.iter().find(playing))
            .or_else(|| states.iter().find(is_followed))
            .or_else(|| states.first())
            .cloned();
        followed = pick.as_ref().map(|s| s.bus.clone());
        *shared.media.lock().unwrap() = pick.clone();
        match pick {
            Some(s) => {
                let due = sent.as_ref().is_none_or(|(old, at)| {
                    !old.same_track_state(&s)
                        || (s.status == "Playing" && at.elapsed() >= Duration::from_secs(1))
                });
                if due {
                    let mut ev = s.to_json();
                    ev["event"] = json!("media");
                    shared.api.broadcast(&ev);
                    sent = Some((s, Instant::now()));
                }
                none_sent = false;
            }
            None if !none_sent => {
                shared.api.broadcast(&json!({"event": "media", "player": null}));
                sent = None;
                none_sent = true;
            }
            None => {}
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

fn players(conn: &Connection) -> zbus::Result<Vec<String>> {
    let reply = conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "ListNames",
        &(),
    )?;
    let names: Vec<String> = reply.body().deserialize()?;
    let mut players: Vec<String> = names.into_iter().filter(|n| n.starts_with(PREFIX)).collect();
    players.sort();
    Ok(players)
}

/// `Identity` of the player, else the end of its bus name.
fn identity(conn: &Connection, bus: &str) -> String {
    let fallback = || {
        let n = bus.trim_start_matches(PREFIX);
        let n = n.split('.').next().unwrap_or(n);
        let mut c = n.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    };
    conn.call_method(
        Some(bus),
        PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &("org.mpris.MediaPlayer2", "Identity"),
    )
    .ok()
    .and_then(|m| m.body().deserialize::<OwnedValue>().ok())
    .and_then(|v| match &*v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    })
    .filter(|s| !s.is_empty())
    .unwrap_or_else(fallback)
}

/// All the properties of a player (one call).
fn read(conn: &Connection, bus: &str, player: String) -> zbus::Result<MediaState> {
    let reply = conn.call_method(
        Some(bus),
        PATH,
        Some("org.freedesktop.DBus.Properties"),
        "GetAll",
        &(PLAYER,),
    )?;
    let props: HashMap<String, OwnedValue> = reply.body().deserialize()?;
    Ok(state_of(bus, player, &props))
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().filter_map(text).collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
        _ => None,
    }
}

fn number(v: &Value) -> Option<f64> {
    Some(match v {
        Value::I64(n) => *n as f64,
        Value::U64(n) => *n as f64,
        Value::I32(n) => *n as f64,
        Value::U32(n) => *n as f64,
        Value::F64(n) => *n,
        _ => return None,
    })
}

/// The state from the player's properties (`Metadata` keys of the MPRIS / xesam spec).
fn state_of(bus: &str, player: String, props: &HashMap<String, OwnedValue>) -> MediaState {
    let get = |k: &str| props.get(k).map(|v| &**v);
    let meta: HashMap<String, OwnedValue> = props
        .get("Metadata")
        .and_then(|m| m.try_clone().ok())
        .and_then(|m| HashMap::try_from(m).ok())
        .unwrap_or_default();
    let field = |k: &str| meta.get(k).and_then(|v| text(v)).unwrap_or_default();
    let micros = |v: Option<&Value>| v.and_then(number).unwrap_or(0.0) / 1e6;
    MediaState {
        bus: bus.to_owned(),
        player,
        status: get("PlaybackStatus").and_then(text).unwrap_or_else(|| "Stopped".into()),
        title: field("xesam:title"),
        artist: field("xesam:artist"),
        album: field("xesam:album"),
        art: field("mpris:artUrl"),
        length: micros(meta.get("mpris:length").map(|v| &**v)),
        position: micros(get("Position")),
        volume: get("Volume").and_then(number),
    }
}

/// `media` request: the current state, or an action on the followed player (`play_pause`,
/// `play`, `pause`, `stop`, `next`, `previous`, `seek` by `value` seconds, `volume` by `value`).
pub fn request(shared: &Shared, req: &Json) -> Result<Option<Json>, String> {
    let state = shared.media.lock().unwrap().clone();
    let Some(action) = req.get("action").and_then(Json::as_str) else {
        let mut v = state.map_or_else(|| json!({"player": null}), |s| s.to_json());
        v["type"] = json!("media");
        return Ok(Some(v));
    };
    let s = state.ok_or("no media player")?;
    let conn = connection().map_err(|e| e.to_string())?;
    let value = req.get("value").and_then(Json::as_f64).unwrap_or(0.0);
    let bus = s.bus.as_str();
    let r = match action {
        "play_pause" => call(conn, bus, "PlayPause", &()),
        "play" => call(conn, bus, "Play", &()),
        "pause" => call(conn, bus, "Pause", &()),
        "stop" => call(conn, bus, "Stop", &()),
        "next" => call(conn, bus, "Next", &()),
        "previous" => call(conn, bus, "Previous", &()),
        "seek" => call(conn, bus, "Seek", &((value * 1e6) as i64,)),
        "volume" => {
            let v = (s.volume.unwrap_or(1.0) + value).clamp(0.0, 1.0);
            conn.call_method(
                Some(s.bus.as_str()),
                PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Set",
                &(PLAYER, "Volume", Value::from(v)),
            )
            .map(|_| ())
        }
        other => return Err(format!("unknown media action \"{other}\"")),
    };
    r.map(|_| None).map_err(|e| format!("{}: {e}", s.player))
}

/// Method `method` of the player `bus`.
fn call<B>(conn: &Connection, bus: &str, method: &str, body: &B) -> zbus::Result<()>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(Some(bus), PATH, Some(PLAYER), method, body)
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Array;

    #[test]
    fn state_from_mpris_properties() {
        let ov = |v: Value| OwnedValue::try_from(v).unwrap();
        let mut meta: HashMap<String, Value> = HashMap::new();
        meta.insert("xesam:title".into(), Value::from("Song"));
        let artists = Array::from(vec!["A", "B"]);
        meta.insert("xesam:artist".into(), Value::from(artists));
        meta.insert("mpris:length".into(), Value::from(180_000_000i64));
        meta.insert("mpris:artUrl".into(), Value::from("file:///tmp/cover.jpg"));
        let mut props: HashMap<String, OwnedValue> = HashMap::new();
        props.insert("Metadata".into(), ov(Value::from(meta)));
        props.insert("PlaybackStatus".into(), ov(Value::from("Playing")));
        props.insert("Position".into(), ov(Value::from(42_500_000i64)));
        props.insert("Volume".into(), ov(Value::from(0.5f64)));
        let s = state_of("org.mpris.MediaPlayer2.x", "X".into(), &props);
        assert_eq!((s.title.as_str(), s.artist.as_str()), ("Song", "A, B"));
        assert_eq!((s.length, s.position, s.volume), (180.0, 42.5, Some(0.5)));
        assert_eq!(s.art, "file:///tmp/cover.jpg");
        assert_eq!(s.status, "Playing");
    }

    /// The players of this session (prints them): `cargo test -- --ignored live_players
    /// --nocapture`.
    #[test]
    #[ignore]
    fn live_players() {
        let conn = connection().unwrap();
        for bus in players(conn).unwrap() {
            let s = read(conn, &bus, identity(conn, &bus)).unwrap();
            println!("{} [{}] {} – {} ({:.0}/{:.0} s)", s.player, s.status, s.artist, s.title, s.position, s.length);
        }
    }
}
