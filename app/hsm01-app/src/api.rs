//! Open API (docs/app/api.md): local programs, in any language, drive the Stream 100 through
//! the app. Unix socket `$XDG_RUNTIME_DIR/hsm01/api.sock`, one JSON object per line both ways:
//! requests (`type`) from the program, replies and events (`event`) from the app. A program can
//! show alerts and own pages (shown after the configuration's pages): their widgets are the same
//! as in the configuration, and the knobs / buttons on them are sent to it as events.
//! The app keeps the device, the drawing and the sending (only what changed, scrolling as COPY).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::i18n::tr;
use crate::config::{Action, ApiSettings, Color, Page, Widget};
use crate::engine::Shared;
use crate::notify::{self, ICON, Notification};

/// Version of the protocol, in `welcome`; grows only by additions.
pub const API_VERSION: u32 = 1;
/// Longest request line accepted (images go by path, not inline).
const MAX_LINE: usize = 1 << 20;

/// A page of a client, shown after the configuration's pages.
#[derive(Clone)]
pub struct ClientPage {
    pub owner: u64,
    pub id: String,
    /// Widgets by id, in drawing order.
    pub widgets: Vec<(String, Widget)>,
    /// What the engine draws (rebuilt from `widgets`).
    pub page: Page,
    /// The power button goes to the client (otherwise: next page, so one can always leave).
    pub grab_power: bool,
    pub leds: [u8; 4],
    /// A widget of the module (`widget: true`), not a page: its content size (w, h), drawn in
    /// the `module` widgets of the pages that show it, never alone (not in the modules menu).
    pub size: Option<(i32, i32)>,
}

/// A page or a widget of a running module (the window lists them).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolPage {
    pub tool: String,
    pub id: String,
    pub name: String,
    pub size: Option<(i32, i32)>,
}

impl ClientPage {
    fn rebuild(&mut self) {
        self.page.widgets = self.widgets.iter().map(|(_, w)| w.clone()).collect();
    }
}

#[derive(Default)]
pub struct Hub {
    next_client: AtomicU64,
    next_alert: AtomicU64,
    /// Connected clients: id → (name, lines to send).
    clients: Mutex<HashMap<u64, (String, mpsc::Sender<String>)>>,
    pages: Mutex<Vec<ClientPage>>,
    /// Incremented at each change of the pages (the engine copies them then).
    pub version: AtomicU64,
    /// Page a client asked to show: (client, page id).
    pub show: Mutex<Option<(u64, String)>>,
    /// Local WebSocket, while it runs.
    pub websocket: Mutex<Option<WebSocketState>>,
    /// Inputs taken by a client on every page: input name (`knob1`, `button2`, `power`…) →
    /// client.
    pub grabs: Mutex<HashMap<String, u64>>,
    /// Actions asked by local programs (`action`), run by the engine at its next frame.
    pub actions: Mutex<Vec<crate::config::Action>>,
    /// Clients connected through the WebSocket (no `action` for them: it can run commands).
    websocket_clients: Mutex<std::collections::HashSet<u64>>,
    /// Commands offered by each client (`commands`), and their states (`command_state`).
    commands: Mutex<HashMap<u64, Vec<Command>>>,
    command_states: Mutex<HashMap<(u64, String), CommandState>>,
    /// Lists of choices each client gives its settings form (`choices`): name → values.
    choices: Mutex<HashMap<u64, HashMap<String, Vec<String>>>>,
    /// Modules frozen by « Load only when shown » (by name, `tools.rs`): broadcasts and the inputs
    /// of others skip them; a message sent to one of them wakes it up (`woken`).
    paused: Mutex<std::collections::HashSet<String>>,
    woken: Mutex<std::collections::HashSet<String>>,
    /// Last line of each broadcast event a paused module missed (name → event → line), sent to
    /// it when it runs again.
    missed: Mutex<HashMap<String, HashMap<String, String>>>,
}

/// A command a running tool offers (`commands`): buttons run it, a knob turns it. Chosen in the
/// window while the tool runs; config: `{ type = "module_command" }` actions, `{ type = "module" }`
/// knob targets and LEDs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Command {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: CommandKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Run by a click (button, knob click, power, deck tile).
    #[default]
    Button,
    /// Turned by a knob (`delta` in the `command` event).
    Knob,
}

/// What a tool says of one of its commands (`command_state`): LEDs, widgets of a knob on it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandState {
    pub active: Option<bool>,
    /// 0..1 (gauge, `{volume}`).
    pub value: Option<f32>,
    /// Shown as the knob's `{name}`.
    pub text: Option<String>,
}

impl Hub {
    pub fn pages(&self) -> Vec<ClientPage> {
        self.pages.lock().unwrap().clone()
    }

    /// Send `msg` to client `id` (dropped if it is gone); a paused module wakes up for it.
    pub fn send(&self, id: u64, msg: &Value) {
        if let Some((name, tx)) = self.clients.lock().unwrap().get(&id) {
            self.wake(name);
            let _ = tx.send(msg.to_string());
        }
    }

    /// An input event to every client, `"mine": true` for `owner` only (the one that takes it).
    /// A paused module gets it only if it is the owner (it wakes up then).
    pub fn emit(&self, owner: Option<u64>, mut msg: Value) {
        let paused = self.paused.lock().unwrap().clone();
        for (id, (name, tx)) in self.clients.lock().unwrap().iter() {
            let mine = Some(*id) == owner;
            if mine {
                self.wake(name);
            } else if paused.contains(name) {
                continue;
            }
            msg["mine"] = json!(mine);
            let _ = tx.send(msg.to_string());
        }
    }

    /// An event to every client; a paused module gets the last one of each kind when it runs
    /// again (`set_paused`).
    pub fn broadcast(&self, msg: &Value) {
        let line = msg.to_string();
        let paused = self.paused.lock().unwrap().clone();
        for (name, tx) in self.clients.lock().unwrap().values() {
            if !paused.contains(name) {
                let _ = tx.send(line.clone());
            }
        }
        if let (Some(event), false) = (msg.get("event").and_then(Value::as_str), paused.is_empty()) {
            let mut missed = self.missed.lock().unwrap();
            for name in paused {
                missed.entry(name).or_default().insert(event.to_owned(), line.clone());
            }
        }
    }

    /// Module `name` frozen (`on`) or running again (`tools.rs`); running again, it gets the
    /// last broadcast of each event it missed (standby, media player…).
    pub fn set_paused(&self, name: &str, on: bool) {
        if on {
            self.paused.lock().unwrap().insert(name.to_owned());
            return;
        }
        if !self.paused.lock().unwrap().remove(name) {
            return;
        }
        self.woken.lock().unwrap().remove(name);
        let missed = self.missed.lock().unwrap().remove(name).unwrap_or_default();
        for (n, tx) in self.clients.lock().unwrap().values() {
            if n == name {
                for line in missed.values() {
                    let _ = tx.send(line.clone());
                }
            }
        }
    }

    /// A message for module `name`: woken up if it is paused (`take_woken`).
    fn wake(&self, name: &str) {
        if self.paused.lock().unwrap().contains(name) {
            self.woken.lock().unwrap().insert(name.to_owned());
        }
    }

    /// Whether paused module `name` got a message since the last call.
    pub fn take_woken(&self, name: &str) -> bool {
        self.woken.lock().unwrap().remove(name)
    }

    /// Whether a client named `name` is connected (a module started by the app said hello).
    pub fn connected(&self, name: &str) -> bool {
        self.clients.lock().unwrap().values().any(|(n, _)| n == name)
    }

    /// Name of client `id` (`hello`: a module started by the app says its name).
    pub fn client_name(&self, id: u64) -> Option<String> {
        self.clients.lock().unwrap().get(&id).map(|c| c.0.clone())
    }

    /// The commands of the running tools: (tool name, command), by tool then in their order.
    pub fn tool_commands(&self) -> Vec<(String, Command)> {
        let clients = self.clients.lock().unwrap();
        let commands = self.commands.lock().unwrap();
        let mut out: Vec<(String, Command)> = commands
            .iter()
            .filter_map(|(id, list)| Some((clients.get(id)?.0.clone(), list)))
            .flat_map(|(tool, list)| list.iter().map(move |c| (tool.clone(), c.clone())))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// The lists of choices of the running module named `tool` (the most recent if several),
    /// for the `choice` fields of its settings form.
    pub fn tool_choices(&self, tool: &str) -> HashMap<String, Vec<String>> {
        let clients = self.clients.lock().unwrap();
        let choices = self.choices.lock().unwrap();
        let mut ids: Vec<u64> = clients.iter().filter(|(_, (n, _))| n == tool).map(|(id, _)| *id).collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids.iter().find_map(|id| choices.get(id).cloned()).unwrap_or_default()
    }

    /// The client named `name` that offers the command `command`, the most recent if several.
    fn command_client(&self, tool: &str, command: &str) -> Option<u64> {
        let clients = self.clients.lock().unwrap();
        let commands = self.commands.lock().unwrap();
        commands
            .iter()
            .filter(|(id, list)| {
                clients.get(id).is_some_and(|c| c.0 == tool) && list.iter().any(|c| c.id == command)
            })
            .map(|(id, _)| *id)
            .max()
    }

    /// Run a tool's command: a `command` event to it (`delta`: knob steps). False if no running
    /// tool of that name offers it.
    pub fn run_command(&self, tool: &str, command: &str, delta: Option<i32>) -> bool {
        let Some(id) = self.command_client(tool, command) else {
            return false;
        };
        let mut ev = json!({"event": "command", "command": command});
        if let Some(d) = delta {
            ev["delta"] = json!(d);
        }
        self.send(id, &ev);
        true
    }

    /// A tool's command and its state, while the tool runs.
    pub fn command_state(&self, tool: &str, command: &str) -> Option<(Command, CommandState)> {
        let id = self.command_client(tool, command)?;
        let c = self.commands.lock().unwrap().get(&id)?.iter().find(|c| c.id == command)?.clone();
        let key = (id, command.to_owned());
        let st = self.command_states.lock().unwrap().get(&key).cloned().unwrap_or_default();
        Some((c, st))
    }

    /// Page `page` (empty: the first one) of the client named `name` (a tool started by the app
    /// says its name in `hello`): (client, page id); the most recent client if several.
    pub fn page_of_tool(&self, name: &str, page: &str) -> Option<(u64, String)> {
        let clients = self.clients.lock().unwrap();
        let pages = self.pages.lock().unwrap();
        let mut ids: Vec<u64> = clients
            .iter()
            .filter(|(_, (n, _))| n == name)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids.into_iter().find_map(|id| {
            pages
                .iter()
                .find(|p| p.owner == id && (p.id == page || page.is_empty() && p.size.is_none()))
                .map(|p| (id, p.id.clone()))
        })
    }

    /// The pages and widgets of the running modules (named clients), for the window.
    pub fn tool_pages(&self) -> Vec<ToolPage> {
        let clients = self.clients.lock().unwrap();
        let pages = self.pages.lock().unwrap();
        pages
            .iter()
            .filter_map(|p| {
                Some(ToolPage {
                    tool: clients.get(&p.owner)?.0.clone(),
                    id: p.id.clone(),
                    name: p.page.name.clone(),
                    size: p.size,
                })
            })
            .collect()
    }

    fn changed(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Client `id`'s page `page`, changed by `f`.
    fn with_page<T>(
        &self,
        id: u64,
        page: &str,
        f: impl FnOnce(&mut ClientPage) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut pages = self.pages.lock().unwrap();
        let p = pages
            .iter_mut()
            .find(|p| p.owner == id && p.id == page)
            .ok_or_else(|| format!("no page \"{page}\" (send a \"page\" request first)"))?;
        let r = f(p)?;
        p.rebuild();
        drop(pages);
        self.changed();
        Ok(r)
    }
}

/// `$HSM01_API_SOCKET`, else `$XDG_RUNTIME_DIR/hsm01/api.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("HSM01_API_SOCKET") {
        return p.into();
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("hsm01").join("api.sock")
}

/// Listen for clients in a thread of its own.
pub fn start(shared: Arc<Shared>) {
    let path = socket_path();
    let listener = match bind(&path) {
        Ok(l) => l,
        Err(e) => return eprintln!("hsm01-app: API {}: {e}", path.display()),
    };
    let accept = move || {
        for stream in listener.incoming().flatten() {
            let shared = shared.clone();
            let _ = std::thread::Builder::new()
                .name("hsm01-api".into())
                .spawn(move || serve(&shared, stream));
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("hsm01-api-listen".into())
        .spawn(accept)
    {
        eprintln!("hsm01-app: API: {e}");
    }
}

/// Socket readable by this user only; a stale file (app gone) is replaced, a live one kept.
fn bind(path: &PathBuf) -> std::io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    if UnixStream::connect(path).is_ok() {
        return Err(std::io::Error::other("already served by another instance"));
    }
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(l)
}

fn serve(shared: &Shared, stream: UnixStream) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let (id, tx, rx) = register(shared);
    std::thread::spawn(move || {
        for line in rx {
            if writer.write_all(line.as_bytes()).is_err() || writer.write_all(b"\n").is_err() {
                break;
            }
        }
    });
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match std::io::Read::take(&mut reader, MAX_LINE as u64).read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if !line.ends_with('\n') && line.len() >= MAX_LINE => {
                let _ = tx.send(error(None, "line too long").to_string());
                break;
            }
            Ok(_) => {}
        }
        if let Some(reply) = process(shared, id, &line) {
            let _ = tx.send(reply);
        }
    }
    unregister(shared, id);
}

/// A new client: its id, and the channel of the lines to send it.
pub(crate) fn register(shared: &Shared) -> (u64, mpsc::Sender<String>, mpsc::Receiver<String>) {
    let hub = &shared.api;
    let id = hub.next_client.fetch_add(1, Ordering::Relaxed) + 1;
    let (tx, rx) = mpsc::channel::<String>();
    hub.clients
        .lock()
        .unwrap()
        .insert(id, ("?".into(), tx.clone()));
    (id, tx, rx)
}

/// The client went away: its pages go too.
pub(crate) fn unregister(shared: &Shared, id: u64) {
    let hub = &shared.api;
    hub.clients.lock().unwrap().remove(&id);
    hub.websocket_clients.lock().unwrap().remove(&id);
    hub.commands.lock().unwrap().remove(&id);
    hub.command_states.lock().unwrap().retain(|(c, _), _| *c != id);
    hub.choices.lock().unwrap().remove(&id);
    hub.grabs.lock().unwrap().retain(|_, c| *c != id);
    hub.pages.lock().unwrap().retain(|p| p.owner != id);
    hub.changed();
}

/// One request (a JSON object) → the line to answer, if any.
pub(crate) fn process(shared: &Shared, id: u64, text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    let reply = match serde_json::from_str::<Value>(text) {
        Ok(req) => {
            let tag = req.get("req").cloned();
            match handle(shared, id, &req) {
                Ok(Some(mut v)) => {
                    if let (Some(t), Some(o)) = (tag, v.as_object_mut()) {
                        o.insert("req".into(), t);
                    }
                    Some(v)
                }
                Ok(None) => tag.map(|t| json!({"type": "ok", "req": t})),
                Err(e) => Some(error(tag, &e)),
            }
        }
        Err(e) => Some(error(None, &format!("not JSON: {e}"))),
    };
    reply.map(|v| v.to_string())
}

/// The local WebSocket while it runs: its port, its token, the flag that stops it.
pub struct WebSocketState {
    pub port: u16,
    pub token: String,
    stop: Arc<AtomicBool>,
}

/// Start, stop or move the local WebSocket to match the settings (at start and when they
/// change): 127.0.0.1:`port`, same messages as the Unix socket, one JSON object per text
/// message. A web page of any site could reach a local port: the address must carry the secret
/// token (`?token=…`), kept for this user only in `token_path()`, the same across launches.
pub fn set_websocket(shared: &Arc<Shared>, s: &ApiSettings) {
    let want = s.websocket.then_some(s.port);
    let mut running = shared.api.websocket.lock().unwrap();
    if running.as_ref().map(|w| w.port) == want {
        return;
    }
    if let Some(w) = running.take() {
        w.stop.store(true, Ordering::Relaxed);
        // wake the accept loop up so that it sees the flag and frees the port
        let _ = std::net::TcpStream::connect(("127.0.0.1", w.port));
    }
    drop(running);
    if let Some(port) = want {
        let started = token(&token_path()).and_then(|t| listen_websocket(shared.clone(), port, t));
        match started {
            Ok(_) => shared.set_message(tf!("WebSocket API on 127.0.0.1:{port}", port = port)),
            Err(e) => shared.set_message(tf!("WebSocket API 127.0.0.1:{port}: {e}", port = port, e = e)),
        }
    } else {
        shared.set_message(tr("WebSocket API stopped"));
    }
}

/// `$XDG_STATE_HOME/hsm01/api-token` (by default `~/.local/state/hsm01/api-token`).
pub fn token_path() -> PathBuf {
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/state"))
        .join("hsm01")
        .join("api-token")
}

/// A new token, replacing the old one (the running WebSocket asks for it from now on; open
/// connections stay).
pub fn new_token(shared: &Shared) -> std::io::Result<()> {
    let t = write_token(&token_path())?;
    if let Some(w) = shared.api.websocket.lock().unwrap().as_mut() {
        w.token = t;
    }
    Ok(())
}

/// The token kept in `path`, created if missing or malformed.
fn token(path: &std::path::Path) -> std::io::Result<String> {
    if let Ok(t) = std::fs::read_to_string(path) {
        let t = t.trim();
        if t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(t.to_owned());
        }
    }
    write_token(path)
}

/// Listen on 127.0.0.1:`port` (0: any free port); returns the port.
fn listen_websocket(shared: Arc<Shared>, port: u16, token: String) -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    let stop = Arc::new(AtomicBool::new(false));
    *shared.api.websocket.lock().unwrap() = Some(WebSocketState {
        port,
        token,
        stop: stop.clone(),
    });
    let accept = move || {
        for stream in listener.incoming().flatten() {
            if stop.load(Ordering::Relaxed) {
                break; // the listener goes, the port is free
            }
            let (shared, stop) = (shared.clone(), stop.clone());
            let _ = std::thread::Builder::new()
                .name("hsm01-ws".into())
                .spawn(move || serve_websocket(&shared, stream, &stop));
        }
    };
    std::thread::Builder::new()
        .name("hsm01-ws-listen".into())
        .spawn(accept)?;
    Ok(port)
}

/// New random token in `path` (readable by this user only).
fn write_token(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    let token: String = b.iter().map(|x| format!("{x:02x}")).collect();
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(token.as_bytes())?;
    Ok(token)
}

#[allow(clippy::result_large_err)] // the handshake callback's type is tungstenite's
fn serve_websocket(shared: &Shared, stream: std::net::TcpStream, stop: &AtomicBool) {
    use std::time::Duration;
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};
    use tungstenite::{Message, http};
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5))); // the handshake
    let token = shared.api.websocket.lock().unwrap().as_ref().map(|w| w.token.clone());
    let check = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        let query = req.uri().query().unwrap_or_default();
        let given = query.split('&').find_map(|kv| kv.strip_prefix("token="));
        if given.is_some() && given == token.as_deref() {
            return Ok(resp);
        }
        let mut no = http::Response::new(Some("token missing or wrong".to_owned()));
        *no.status_mut() = http::StatusCode::FORBIDDEN;
        Err(no)
    };
    let Ok(mut ws) = tungstenite::accept_hdr(stream, check) else {
        return;
    };
    // short reads: the events waiting for this client go out between them
    let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(10)));
    let (id, _tx, rx) = register(shared);
    shared.api.websocket_clients.lock().unwrap().insert(id);
    'session: while !stop.load(Ordering::Relaxed) {
        let mut wrote = false;
        while let Ok(line) = rx.try_recv() {
            if ws.write(Message::text(line)).is_err() {
                break 'session;
            }
            wrote = true;
        }
        if wrote && ws.flush().is_err() {
            break;
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                if let Some(reply) = process(shared, id, t.as_str())
                    && ws.send(Message::text(reply)).is_err()
                {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {} // pings are answered by tungstenite, binary ignored
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    if stop.load(Ordering::Relaxed) {
        let _ = ws.close(None); // WebSocket switched off
        let _ = ws.flush();
    }
    unregister(shared, id);
}

fn error(req: Option<Value>, message: &str) -> Value {
    let mut v = json!({"type": "error", "message": message});
    if let Some(r) = req {
        v["req"] = r;
    }
    v
}

fn str_field<'a>(req: &'a Value, key: &str) -> Result<&'a str, String> {
    req.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("\"{key}\" (string) is missing"))
}

fn opt_str(req: &Value, key: &str) -> Option<String> {
    req.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Answer one request: `Ok(Some(reply))`, `Ok(None)` (just "ok" if a `req` tag was given) or
/// an error message.
fn handle(shared: &Shared, id: u64, req: &Value) -> Result<Option<Value>, String> {
    let hub = &shared.api;
    match str_field(req, "type")? {
        "hello" => {
            let name = opt_str(req, "name").unwrap_or_else(|| "?".into());
            if let Some(c) = hub.clients.lock().unwrap().get_mut(&id) {
                c.0 = name;
            }
            let fw = shared.status.lock().unwrap().firmware;
            Ok(Some(json!({
                "type": "welcome",
                "api": API_VERSION,
                "app": crate::APP_NAME,
                "screen": {"width": crate::config::SCREEN_W, "height": crate::config::SCREEN_H},
                "firmware": format!("{}.{:02}", fw.0, fw.1),
                "client": id,
            })))
        }
        "alert" => {
            let n = alert(id, hub.next_alert.fetch_add(1, Ordering::Relaxed) + 1, req)?;
            let alert_id = n.origin.map_or(0, |o| o.1);
            let urgent = req.get("priority").and_then(Value::as_str) == Some("high");
            notify::push(shared, n, urgent);
            Ok(Some(json!({"type": "ok", "id": alert_id})))
        }
        "page" => {
            let page_id = str_field(req, "page")?.to_owned();
            let widgets = match req.get("widgets") {
                Some(Value::Array(a)) => a.iter().map(widget).collect::<Result<Vec<_>, _>>()?,
                None => Vec::new(),
                Some(_) => return Err("\"widgets\" must be an array".into()),
            };
            let background: Color = match req.get("background") {
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| format!("background: {e}"))?,
                None => Color(0, 0, 0),
            };
            let mut p = ClientPage {
                owner: id,
                id: page_id.clone(),
                widgets,
                page: Page {
                    name: opt_str(req, "name").unwrap_or_else(|| page_id.clone()),
                    background,
                    knobs: Vec::new(),
                    buttons: Vec::new(),
                    power: Action::Back, // back where a jump came from, else next page
                    power_hold: Action::None,
                    wallpaper: None,
                    widgets: Vec::new(),
                },
                grab_power: req.get("grab_power").and_then(Value::as_bool).unwrap_or(false),
                leds: leds(req.get("leds"))?.unwrap_or([0; 4]),
                size: widget_size(req)?,
            };
            let shown_alone = p.size.is_none();
            p.rebuild();
            {
                let mut pages = hub.pages.lock().unwrap();
                match pages.iter_mut().find(|q| q.owner == id && q.id == page_id) {
                    Some(q) => *q = p,
                    None => pages.push(p),
                }
            }
            hub.changed();
            if shown_alone && req.get("show").and_then(Value::as_bool) == Some(true) {
                *hub.show.lock().unwrap() = Some((id, page_id));
            }
            Ok(None)
        }
        "set" => {
            let new = match req.get("widgets") {
                Some(Value::Array(a)) => a.iter().map(widget).collect::<Result<Vec<_>, _>>()?,
                _ => return Err("\"widgets\" (array) is missing".into()),
            };
            hub.with_page(id, str_field(req, "page")?, |p| {
                for (wid, w) in new {
                    match p.widgets.iter_mut().find(|(i, _)| *i == wid) {
                        Some(slot) => slot.1 = w,
                        None => p.widgets.push((wid, w)),
                    }
                }
                Ok(None)
            })
        }
        "update" => {
            let Some(Value::Object(changes)) = req.get("widgets") else {
                return Err("\"widgets\" must be an object {id: {field: value}}".into());
            };
            hub.with_page(id, str_field(req, "page")?, |p| {
                for (wid, fields) in changes {
                    let Some(slot) = p.widgets.iter_mut().find(|(i, _)| i == wid) else {
                        return Err(format!("no widget \"{wid}\""));
                    };
                    slot.1 = merged(&slot.1, fields).map_err(|e| format!("{wid}: {e}"))?;
                }
                Ok(None)
            })
        }
        "remove" => {
            let ids: Vec<String> = match req.get("widgets") {
                Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
                _ => return Err("\"widgets\" (array of ids) is missing".into()),
            };
            hub.with_page(id, str_field(req, "page")?, |p| {
                p.widgets.retain(|(i, _)| !ids.contains(i));
                Ok(None)
            })
        }
        "leds" => {
            let values = leds(req.get("values"))?.ok_or("\"values\" (4 numbers) is missing")?;
            hub.with_page(id, str_field(req, "page")?, |p| {
                p.leds = values;
                Ok(None)
            })
        }
        "show" => {
            let page = str_field(req, "page")?.to_owned();
            let size = hub
                .pages
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.owner == id && p.id == page)
                .map(|p| p.size);
            match size {
                None => return Err(format!("no page \"{page}\"")),
                Some(Some(_)) => return Err(format!("\"{page}\" is a widget, not shown alone")),
                Some(None) => {}
            }
            *hub.show.lock().unwrap() = Some((id, page));
            Ok(None)
        }
        "media" => crate::media::request(shared, req),
        "action" => {
            // the app's own actions (keys, command, media, page, mute…), as in config.toml
            if hub.websocket_clients.lock().unwrap().contains(&id) {
                return Err("\"action\" is for local programs only (Unix socket)".into());
            }
            let a = req.get("action").ok_or("\"action\" (object) is missing")?;
            let a: crate::config::Action =
                serde_json::from_value(a.clone()).map_err(|e| format!("\"action\": {e}"))?;
            hub.actions.lock().unwrap().push(a);
            Ok(None)
        }
        "commands" => {
            // the full list each time: replaces the previous one
            let list = req.get("commands").ok_or("\"commands\" (array) is missing")?;
            let list: Vec<Command> = serde_json::from_value(list.clone())
                .map_err(|e| format!("\"commands\": {e} (each: id, name, kind button|knob)"))?;
            if list.iter().any(|c| c.id.is_empty()) {
                return Err("a command without \"id\"".into());
            }
            let ids: Vec<String> = list.iter().map(|c| c.id.clone()).collect();
            hub.command_states
                .lock()
                .unwrap()
                .retain(|(c, cmd), _| *c != id || ids.contains(cmd));
            hub.commands.lock().unwrap().insert(id, list);
            Ok(None)
        }
        "choices" => {
            // lists for the module's settings form, merged with those given before
            let Some(Value::Object(lists)) = req.get("choices") else {
                return Err("\"choices\" must be an object {name: [values]}".into());
            };
            let mut all = hub.choices.lock().unwrap();
            let mine = all.entry(id).or_default();
            for (name, values) in lists {
                let Value::Array(values) = values else {
                    return Err(format!("choices.{name}: an array of texts"));
                };
                let texts = values.iter().filter_map(|v| v.as_str().map(str::to_owned));
                mine.insert(name.clone(), texts.take(1000).collect());
            }
            Ok(None)
        }
        "command_state" => {
            let command = str_field(req, "command")?;
            let known = hub.commands.lock().unwrap().get(&id).is_some_and(|l| {
                l.iter().any(|c| c.id == command)
            });
            if !known {
                return Err(format!("no command \"{command}\" (send \"commands\" first)"));
            }
            let mut states = hub.command_states.lock().unwrap();
            let st = states.entry((id, command.to_owned())).or_default();
            if let Some(v) = req.get("active") {
                st.active = v.as_bool();
            }
            if let Some(v) = req.get("value") {
                st.value = v.as_f64().map(|v| (v as f32).clamp(0.0, 1.5));
            }
            if let Some(v) = req.get("text") {
                st.text = v.as_str().map(str::to_owned);
            }
            Ok(None)
        }
        "grab" => {
            let names = input_names(req)?.ok_or("\"inputs\" (array) is missing")?;
            let mut grabs = hub.grabs.lock().unwrap();
            if let Some(n) = names.iter().find(|n| grabs.get(*n).is_some_and(|c| *c != id)) {
                return Err(format!("\"{n}\" is already taken by another program"));
            }
            for n in names {
                grabs.insert(n, id);
            }
            Ok(None)
        }
        "ungrab" => {
            let names = input_names(req)?; // none: all of them
            hub.grabs
                .lock()
                .unwrap()
                .retain(|n, c| *c != id || names.as_ref().is_some_and(|l| !l.contains(n)));
            Ok(None)
        }
        "close_page" => {
            let page = str_field(req, "page")?;
            hub.pages
                .lock()
                .unwrap()
                .retain(|p| !(p.owner == id && p.id == page));
            hub.changed();
            Ok(None)
        }
        other => Err(format!("unknown request type \"{other}\"")),
    }
}

/// `"inputs": ["knob1", "button4", "power", …]` (None if absent).
fn input_names(req: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(v) = req.get("inputs") else {
        return Ok(None);
    };
    let names: Vec<String> =
        serde_json::from_value(v.clone()).map_err(|_| "\"inputs\" is an array of names")?;
    let known = |n: &str| n == "power" || (0..8).any(|b| button_name(b) == n);
    match names.iter().find(|n| !known(n)) {
        Some(n) => Err(format!(
            "unknown input \"{n}\" (knob1-4, button1-4, power)"
        )),
        None => Ok(Some(names)),
    }
}

/// `{"id": "…", "x": …, "kind": "…", …}` → (id, widget).
fn widget(v: &Value) -> Result<(String, Widget), String> {
    let Value::Object(o) = v else {
        return Err("a widget is an object".into());
    };
    let mut o = o.clone();
    let id = match o.remove("id") {
        Some(Value::String(s)) => s,
        _ => return Err("a widget needs an \"id\" (string)".into()),
    };
    let w = serde_json::from_value(Value::Object(o)).map_err(|e| format!("widget \"{id}\": {e}"))?;
    Ok((id, w))
}

/// `w` with `fields` replaced (any field, even `kind`).
fn merged(w: &Widget, fields: &Value) -> Result<Widget, String> {
    let Value::Object(fields) = fields else {
        return Err("changes are an object {field: value}".into());
    };
    let mut cur: Map<String, Value> = match serde_json::to_value(w) {
        Ok(Value::Object(m)) => m,
        _ => return Err("internal: widget is not an object".into()),
    };
    for (k, v) in fields {
        if k != "id" {
            cur.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(Value::Object(cur)).map_err(|e| e.to_string())
}

fn leds(v: Option<&Value>) -> Result<Option<[u8; 4]>, String> {
    let Some(v) = v else { return Ok(None) };
    let a: Vec<u8> = serde_json::from_value(v.clone()).map_err(|e| format!("leds: {e}"))?;
    let mut out = [0u8; 4];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.min(100);
    }
    Ok(Some(out))
}

/// The popup of an `alert` request.
fn alert(client: u64, alert_id: u64, req: &Value) -> Result<Notification, String> {
    let title = notify::plain(&opt_str(req, "title").unwrap_or_default());
    let text = notify::plain(&opt_str(req, "text").unwrap_or_default());
    if title.is_empty() && text.is_empty() {
        return Err("an alert needs a \"title\" or a \"text\"".into());
    }
    let style = alert_style(req)?;
    let pill = style != notify::NoticeStyle::Card;
    let round = req.get("round").and_then(Value::as_bool).unwrap_or(false);
    let icon = match (opt_str(req, "image"), opt_str(req, "icon")) {
        _ if pill => None,
        (Some(path), _) => notify::picture_file(&path, round),
        (None, Some(name)) => crate::render::load_icon(&name, ICON, ICON, None),
        (None, None) => None,
    };
    // a pill shows one line: the text (or the title)
    let (summary, body) = if !pill {
        (title, text)
    } else if text.is_empty() {
        (title, String::new())
    } else {
        (text, String::new())
    };
    let duration = req.get("duration").and_then(Value::as_f64).map(|d| d as f32);
    Ok(Notification {
        app: opt_str(req, "app").unwrap_or_default(),
        summary,
        body,
        icon,
        duration: duration.or(pill.then_some(3.0)).map(|d| d.clamp(1.0, 600.0)),
        wake: req.get("wake").and_then(Value::as_bool),
        led: req
            .get("led")
            .and_then(Value::as_u64)
            .filter(|l| (1..=4).contains(l))
            .map(|l| l as u8),
        desktop: false,
        origin: Some((client, alert_id)),
        style,
    })
}

/// `widget: true` of a page: a widget of the module, of `size` {w, h} (16-2048 px; default the
/// screen's), drawn in `module` widgets; else a page (None).
fn widget_size(req: &Value) -> Result<Option<(i32, i32)>, String> {
    if req.get("widget").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let dim = |key: &str, default: i32| match req.get("size").and_then(|s| s.get(key)) {
        None => Ok(default),
        Some(v) => v
            .as_i64()
            .filter(|n| (16..=2048).contains(n))
            .map(|n| n as i32)
            .ok_or(format!("size.{key}: 16 to 2048 px")),
    };
    Ok(Some((dim("w", crate::config::SCREEN_W)?, dim("h", crate::config::SCREEN_H)?)))
}

/// `style` of an alert: `card` (default) or `pill` (`border`, `dot`: the border's colour by
/// default, `color` of the text: white).
fn alert_style(req: &Value) -> Result<notify::NoticeStyle, String> {
    let color = |key: &str, default: Color| match req.get(key) {
        Some(v) if !v.is_null() => {
            serde_json::from_value::<Color>(v.clone()).map_err(|e| format!("{key}: {e}"))
        }
        _ => Ok(default),
    };
    match opt_str(req, "style").as_deref() {
        None | Some("card") => Ok(notify::NoticeStyle::Card),
        Some("pill") => {
            let border = color("border", Color(0x3d, 0xdc, 0x84))?;
            Ok(notify::NoticeStyle::Pill {
                border,
                dot: color("dot", border)?,
                color: color("color", Color(255, 255, 255))?,
            })
        }
        Some(s) => Err(format!("unknown alert style \"{s}\" (card, pill)")),
    }
}

/// Name of report bit `bit` in events: knob clicks, buttons, power.
pub fn button_name(bit: usize) -> String {
    match bit {
        0..=3 => format!("knob{}", bit + 1),
        4..=7 => format!("button{}", bit - 3),
        _ => "power".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widgets_from_json_and_partial_updates() {
        let (id, w) = widget(&json!({
            "id": "title", "x": 10, "y": 20, "w": 200, "h": 30,
            "kind": "text", "text": "Hello", "color": "#ffffff"
        }))
        .unwrap();
        assert_eq!(id, "title");
        let w2 = merged(&w, &json!({"y": 40, "text": "Bye"})).unwrap();
        assert_eq!((w2.x, w2.y), (10, 40));
        let crate::config::WidgetKind::Text { text, .. } = &w2.kind else {
            panic!("kind changed")
        };
        assert_eq!(text, "Bye");
        let icon = merged(&w2, &json!({"kind": "theme_icon", "name": "discord"})).unwrap();
        assert_eq!(icon.kind.name(), "theme_icon");
        assert!(widget(&json!({"x": 1})).is_err(), "no id");
        assert!(merged(&w, &json!({"x": "a"})).is_err());
        // a deck, as the Python helpers write it
        let (_, d) = widget(&json!({
            "id": "deck0", "x": 4, "y": 28, "w": 112, "h": 244, "kind": "deck", "knob": 1,
            "vertical": true, "color": "#ffaa00",
            "tile": [{"title": "Suivant", "subtitle": "", "icon": "media-skip-forward",
                      "action": {"type": "media", "action": "next"}}]
        }))
        .unwrap();
        let crate::config::WidgetKind::Deck { tiles, .. } = &d.kind else {
            panic!("not a deck")
        };
        assert_eq!(tiles[0].action, crate::config::Action::Media { action: "next".into() });
    }

    /// `action`: the app's actions for local programs, queued for the engine; refused to the
    /// WebSocket's clients (it can run commands) and when malformed.
    #[test]
    fn actions_for_local_programs_only() {
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let (id, _tx, _rx) = register(&shared);
        let ask = |id, req: &str| process(&shared, id, req).unwrap_or_default();
        let ok = ask(id, r#"{"type": "action", "req": 1, "action": {"type": "page", "index": 2}}"#);
        assert!(ok.contains("\"ok\""), "{ok}");
        let bad = ask(id, r#"{"type": "action", "req": 2, "action": {"type": "nope"}}"#);
        assert!(bad.contains("error"), "{bad}");
        let (ws, _tx2, _rx2) = register(&shared);
        shared.api.websocket_clients.lock().unwrap().insert(ws);
        let refused = ask(ws, r#"{"type": "action", "req": 3, "action": {"type": "back"}}"#);
        assert!(refused.contains("local programs only"), "{refused}");
        let queued = shared.api.actions.lock().unwrap().clone();
        assert_eq!(queued, [crate::config::Action::Page { index: 2 }]);
    }

    /// The Python client and its menu against this server, no device: turning a knob scrolls
    /// a row, a click triggers the tile (an alert). `cargo test -- --ignored python_menu`.
    #[test]
    #[ignore]
    fn python_menu_end_to_end() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("hsm01-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("api.sock");
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let listener = bind(&sock).unwrap();
        let s = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || serve(&s, stream));
            }
        });
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../clients/python/tests/e2e_menu.py");
        let mut child = std::process::Command::new("python3")
            .args(["-I", script, "6"])
            .env("HSM01_API_SOCKET", &sock)
            .spawn()
            .expect("python3");
        let wait_for = |what: &str, ok: &dyn Fn() -> bool| {
            let t = Instant::now();
            while !ok() {
                assert!(t.elapsed() < Duration::from_secs(4), "{what}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let label = |id: &str| {
            let pages = shared.api.pages();
            let p = pages.iter().find(|p| p.id == "deck")?;
            let (_, w) = p.widgets.iter().find(|(i, _)| i == id)?;
            match &w.kind {
                crate::config::WidgetKind::Text { text, .. } => Some(text.clone()),
                _ => None,
            }
        };
        wait_for("menu page", &|| label("lb0_0").as_deref() == Some("T0"));
        let owner = shared.api.pages()[0].owner;
        assert_eq!(shared.api.pages()[0].leds, [100; 4], "LEDs of the active row");
        shared.api.send(owner, &json!({"event": "turn", "page": "deck", "knob": 1, "delta": 3}));
        wait_for("scrolled one row", &|| label("lb0_0").as_deref() == Some("T4"));
        shared.api.send(owner, &json!({"event": "click", "page": "deck", "button": "button2"}));
        let alert = |s: &Shared| s.notifications.lock().unwrap().front().map(|n| n.summary.clone());
        wait_for("alert of tile 5", &|| alert(&shared).as_deref() == Some("tile 5"));
        // endless: three rows of 4, turning back from the first row shows the last one
        shared.api.send(owner, &json!({"event": "turn", "page": "deck", "knob": 2, "delta": -6}));
        wait_for("wrapped to the last row", &|| label("lb0_0").as_deref() == Some("T8"));
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shipped modules start against this server (no device, no media player): their pages
    /// appear, filled. `cargo test -- --ignored shipped_modules`.
    #[test]
    #[ignore]
    fn shipped_modules_run() {
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("hsm01-mod-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("api.sock");
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let listener = bind(&sock).unwrap();
        let s = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || serve(&s, stream));
            }
        });
        // as the app starts the tools: their main.py, PYTHONPATH to the library, HSM01_MODULE
        let app = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let lib = format!("{app}/clients/python");
        // the stream deck in rows: its deck.toml with `layout = "lignes"`
        let rows = dir.join("deck-lignes.toml");
        let shipped = std::fs::read_to_string(format!("{app}/modules/streamdeck/deck.toml")).unwrap();
        let in_rows = shipped.replace("layout = \"colonnes\"", "layout = \"lignes\"");
        std::fs::write(&rows, in_rows).unwrap();
        let rows = rows.to_string_lossy().into_owned();
        let mut children: Vec<_> = [
            ("modules/media/main.py", "Lecteur média", vec![]),
            ("modules/system/main.py", "Système", vec![]),
            ("clients/python/examples/tool_template.py", "Mon module", vec![]),
            ("modules/deck/main.py", "Menu de raccourcis", vec![]),
            ("modules/streamdeck/main.py", "Stream deck", vec![]),
            ("modules/streamdeck/main.py", "Stream deck lignes", vec!["--file", &rows]),
        ]
        .iter()
        .map(|(file, name, args)| {
            std::process::Command::new("python3")
                .arg(format!("{app}/{file}"))
                .args(args)
                .env("PYTHONPATH", &lib)
                .env("HSM01_API_SOCKET", &sock)
                .env("HSM01_MODULE", name)
                .spawn()
                .expect("python3")
        })
        .collect();
        let text = |page: &str, id: &str| {
            let pages = shared.api.pages();
            let p = pages.iter().find(|p| p.id == page)?;
            match &p.widgets.iter().find(|(i, _)| i == id)?.1.kind {
                crate::config::WidgetKind::Text { text, .. } => Some(text.clone()),
                _ => None,
            }
        };
        let t = Instant::now();
        let ok = || {
            text("media", "player").as_deref() == Some("Media · no player")
                && text("system", "l0").as_deref() == Some("Processor")
                && text("system", "v1").is_some_and(|v| v.contains("GiB"))
                && text("main", "count").as_deref() == Some("0")
                && shared.api.page_of_tool("Menu de raccourcis", "deck").is_some()
                && shared.api.page_of_tool("Stream deck", "deck").is_some()
                && shared.api.page_of_tool("Stream deck lignes", "deck").is_some()
        };
        while !ok() {
            assert!(t.elapsed() < Duration::from_secs(8), "pages: {:?} / {:?}", text("media", "player"), text("system", "v1"));
            std::thread::sleep(Duration::from_millis(50));
        }
        // the template's knob 1 counts; the tool is known by its name (action « afficher un module »)
        let (owner, page) = shared.api.page_of_tool("Mon module", "").expect("template's page");
        assert_eq!(page, "main");
        shared.api.send(owner, &json!({"event": "turn", "page": "main", "mine": true, "knob": 1, "delta": 2}));
        let t = Instant::now();
        while text("main", "count").as_deref() != Some("2") {
            assert!(t.elapsed() < Duration::from_secs(5), "template count: {:?}", text("main", "count"));
            std::thread::sleep(Duration::from_millis(50));
        }
        // its commands, offered while it runs, used from any page: a knob, a button; their state
        let cmds = shared.api.tool_commands();
        let has = |id: &str, kind| cmds.iter().any(|(t, c)| t == "Mon module" && c.id == id && c.kind == kind);
        assert!(has("zero", CommandKind::Button) && has("counter", CommandKind::Knob), "{cmds:?}");
        let state = || shared.api.command_state("Mon module", "counter").and_then(|(_, s)| s.text);
        for (run, want) in [(Some(3), "5"), (None, "0")] {
            let id = if run.is_some() { "counter" } else { "zero" };
            assert!(shared.api.run_command("Mon module", id, run));
            let t = Instant::now();
            while text("main", "count").as_deref() != Some(want)
                || state().as_deref() != Some(&format!("Counter {want}"))
            {
                assert!(t.elapsed() < Duration::from_secs(5), "{id}: {:?} {:?}", text("main", "count"), state());
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        assert!(!shared.api.run_command("Mon module", "nope", None), "not offered");
        // a player starts: the media page follows
        shared.api.broadcast(&json!({
            "event": "media", "player": "Test", "status": "Playing", "title": "Morceau",
            "artist": "Artiste", "album": "", "art": "", "length": 100.0, "position": 10.0,
            "volume": 0.5
        }));
        let t = Instant::now();
        while text("media", "title").as_deref() != Some("Morceau")
            || text("media", "player").as_deref() != Some("Test · playing")
        {
            assert!(t.elapsed() < Duration::from_secs(5), "media page: {:?}", text("media", "title"));
            std::thread::sleep(Duration::from_millis(50));
        }
        let leds = shared.api.pages().iter().find(|p| p.id == "media").map(|p| p.leds);
        assert_eq!(leds, Some([40, 100, 40, 0]), "button 2 lit while playing");
        // what the device shows: $TMPDIR/hsm01-media.png
        std::thread::sleep(Duration::from_millis(600));
        let pages = shared.api.pages();
        let media = pages.iter().find(|p| p.id == "media").unwrap();
        let mut r = crate::render::Renderer::new().unwrap();
        let mut pm = crate::render::Renderer::new_pixmap();
        r.render(&media.page, &crate::state::View::default(), &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-media.png")).unwrap();
        // the menu, started by the app, does not take the screen; in a `tool` widget under a
        // header (as on a page 2): $TMPDIR/hsm01-deck-widget.png
        assert_eq!(*shared.api.show.lock().unwrap(), None, "no module jumped to its page");
        let deck = pages.iter().find(|p| p.id == "deck").unwrap();
        let mut page = crate::config::Config::default().pages[0].clone();
        page.wallpaper = None;
        page.widgets = vec![crate::config::Widget {
            x: 0,
            y: 32,
            w: 480,
            h: 240,
            frame: None,
            kind: crate::config::WidgetKind::Tool {
                tool: "Menu de raccourcis".into(),
                page: String::new(),
                inputs: crate::config::default_tool_inputs(),
                overflow: Default::default(),
            },
        }];
        let mut view = crate::state::View::default();
        view.embedded.insert(0, (deck.page.clone(), (crate::config::SCREEN_W, crate::config::SCREEN_H)));
        r.render(&page, &view, &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-deck-widget.png")).unwrap();
        // the stream deck tool, both layouts: 4 `deck` widgets each, the app's own drawing
        for (name, file) in [("Stream deck", "colonnes"), ("Stream deck lignes", "lignes")] {
            let (owner, _) = shared.api.page_of_tool(name, "deck").unwrap();
            let p = pages.iter().find(|p| p.owner == owner && p.id == "deck").unwrap();
            let decks: Vec<_> = p.page.widgets.iter().filter(|w| w.kind.name() == "deck").collect();
            assert_eq!(decks.len(), 4, "{name}");
            let rows = decks.iter().all(|w| w.w > w.h);
            assert_eq!(rows, file == "lignes", "{name}: layout of its deck.toml");
            r.render(&p.page, &crate::state::View::default(), &mut pm);
            pm.save_png(std::env::temp_dir().join(format!("hsm01-streamdeck-{file}.png")))
                .unwrap();
        }
        std::thread::sleep(Duration::from_secs(2));
        for c in &mut children {
            assert!(c.try_wait().unwrap().is_none(), "a module stopped");
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn websocket_needs_the_token() {
        use tungstenite::Message;
        let dir = std::env::temp_dir().join(format!("hsm01-ws-{}", std::process::id()));
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let token = token(&dir.join("token")).unwrap();
        assert_eq!(super::token(&dir.join("token")).unwrap(), token, "kept across launches");
        let port = listen_websocket(shared.clone(), 0, token.clone()).unwrap();
        let url = |t: &str| format!("ws://127.0.0.1:{port}/?token={t}");
        assert!(tungstenite::connect(url("nope")).is_err(), "wrong token accepted");
        assert!(tungstenite::connect(format!("ws://127.0.0.1:{port}/")).is_err());
        let (mut ws, _) = tungstenite::connect(url(&token)).unwrap();
        ws.send(Message::text(json!({"type": "hello", "name": "web"}).to_string()))
            .unwrap();
        let welcome: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(welcome["type"], "welcome");
        ws.send(Message::text(json!({"type": "alert", "title": "web", "req": 7}).to_string()))
            .unwrap();
        let ok: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!((ok["type"].as_str(), ok["req"].as_i64()), (Some("ok"), Some(7)));
        // events reach a WebSocket client too: inputs (without any page), with "mine"
        shared
            .api
            .emit(None, json!({"event": "turn", "knob": 2, "delta": 1, "page": "Audio"}));
        let ev: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!((ev["event"].as_str(), ev["mine"].as_bool()), (Some("turn"), Some(false)));
        // grabbing an input
        ws.send(Message::text(json!({"type": "grab", "inputs": ["knob4", "button1"], "req": 8}).to_string()))
            .unwrap();
        let ok: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(ok["type"], "ok");
        assert_eq!(shared.api.grabs.lock().unwrap().len(), 2);
        ws.send(Message::text(json!({"type": "grab", "inputs": ["knob9"], "req": 9}).to_string()))
            .unwrap();
        let err: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(err["type"], "error");
        // switched off: the session closes, no more connections, the grabs go
        let off = shared.api.websocket.lock().unwrap().take().unwrap();
        off.stop.store(true, Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(("127.0.0.1", port));
        loop {
            match ws.read() {
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
        for _ in 0..100 {
            if shared.api.grabs.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(shared.api.grabs.lock().unwrap().is_empty(), "grabs kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn requests_over_a_real_socket() {
        use std::io::{BufRead, BufReader, Write};
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let (a, b) = UnixStream::pair().unwrap();
        let s = shared.clone();
        std::thread::spawn(move || serve(&s, b));
        let mut lines = BufReader::new(a.try_clone().unwrap()).lines();
        let send = |v: Value| writeln!(&a, "{v}").unwrap();
        send(json!({"type": "hello", "name": "test"}));
        let welcome: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(welcome["type"], "welcome");
        assert_eq!(welcome["api"], API_VERSION);
        send(json!({"type": "page", "page": "p", "widgets": [
            {"id": "r", "x": 0, "y": 0, "w": 10, "h": 10, "kind": "rect", "color": "#ff0000"}
        ], "leds": [100, 0, 0, 0], "req": 1}));
        let ok: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!((ok["type"].as_str(), ok["req"].as_i64()), (Some("ok"), Some(1)));
        send(json!({"type": "update", "page": "p", "widgets": {"r": {"x": 5}}, "req": 2}));
        lines.next().unwrap().unwrap();
        let pages = shared.api.pages();
        assert_eq!(pages.len(), 1);
        assert_eq!((pages[0].page.widgets[0].x, pages[0].leds[0]), (5, 100));
        send(json!({"type": "update", "page": "nope", "widgets": {}}));
        let err: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(err["type"], "error");
        send(json!({"type": "alert", "title": "Fini", "led": 2, "req": 3}));
        let ok: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(ok["id"].as_u64().unwrap() > 0);
        assert_eq!(shared.notifications.lock().unwrap().len(), 1);
        drop(lines); // its clone of the socket too
        drop(a);
        // disconnection removes the client's pages
        for _ in 0..100 {
            if shared.api.pages().is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("pages kept after the client left");
    }

    /// A module's widget: a page with `widget: true` and its content `size`; never shown alone,
    /// not its page for « afficher un module » (empty page id), listed for the window.
    #[test]
    fn module_widgets() {
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let (me, _tx, _rx) = register(&shared);
        let call = |req: Value| handle(&shared, me, &req);
        call(json!({"type": "hello", "name": "Discord"})).unwrap();
        call(json!({"type": "page", "page": "membres", "name": "Membres", "widget": true,
                    "size": {"w": 200, "h": 600}, "widgets": []}))
        .unwrap();
        assert_eq!(shared.api.page_of_tool("Discord", ""), None, "a widget is not its page");
        call(json!({"type": "page", "page": "discord", "widgets": [], "show": true})).unwrap();
        assert_eq!(shared.api.page_of_tool("Discord", ""), Some((me, "discord".into())));
        assert_eq!(shared.api.page_of_tool("Discord", "membres"), Some((me, "membres".into())));
        let err = call(json!({"type": "show", "page": "membres"})).unwrap_err();
        assert!(err.contains("widget"), "{err}");
        let listed = shared.api.tool_pages();
        let w = listed.iter().find(|p| p.id == "membres").unwrap();
        assert_eq!((w.tool.as_str(), w.name.as_str(), w.size), ("Discord", "Membres", Some((200, 600))));
        assert!(call(json!({"type": "page", "page": "x", "widget": true, "size": {"w": 5}})).is_err());
    }

    /// The Spotify module against `tools/fake_spotify.py`: authorized through the « browser »
    /// (curl following the consent page back to the module's redirect address), tokens in its
    /// .env (owner only); its page (track, artist, cover), its commands (next, like, volume,
    /// shuffle, repeat, play / pause, a favourite playlist, the radio), the page's inputs, the
    /// playlists offered to its settings form. `TMPDIR=… cargo test -- --ignored spotify_module`
    /// → `$TMPDIR/hsm01-spotify.png`.
    #[test]
    #[ignore]
    fn spotify_module_against_a_fake_spotify() {
        use crate::config::WidgetKind;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("hsm01-spotify-{}", std::process::id()));
        let module = dir.join("module");
        std::fs::create_dir_all(&module).unwrap();
        let sock = dir.join("api.sock");
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let listener = bind(&sock).unwrap();
        let s = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || serve(&s, stream));
            }
        });
        let app = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let mut fake = Command::new("python3")
            .args(["-I", &format!("{app}/../tools/fake_spotify.py")])
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3");
        let mut out = BufReader::new(fake.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let base = format!("http://127.0.0.1:{}", line.trim().strip_prefix("port ").expect("port"));
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let l = log.clone();
        std::thread::spawn(move || {
            for line in out.lines().map_while(Result::ok) {
                l.lock().unwrap().push(line);
            }
        });
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let pairs = [("SPOTIFY_CLIENT_ID", "abc"), ("SPOTIFY_CLIENT_SECRET", "s3cret")];
        let pairs: Vec<_> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        crate::tools::write_env(&module, &pairs).unwrap();
        let settings = format!(
            "pas = 5\nredirection = \"http://127.0.0.1:{port}/callback\"\n\n[[playlist]]\nnom = \"Sport\"\n"
        );
        std::fs::write(module.join("spotify.toml"), settings).unwrap();
        let mut spotify = Command::new("python3")
            .arg(format!("{app}/modules/spotify/main.py"))
            .env("PYTHONPATH", format!("{app}/clients/python"))
            .env("HSM01_API_SOCKET", &sock)
            .env("HSM01_MODULE", "Spotify")
            .env("HSM01_MODULE_DIR", &module)
            .env("HSM01_SPOTIFY_ACCOUNTS", &base)
            .env("HSM01_SPOTIFY_API", format!("{base}/v1"))
            .env("HSM01_SPOTIFY_BROWSER", "curl -sL")
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .spawn()
            .expect("python3");
        let text = |id: &str| {
            let pages = shared.api.pages();
            let p = pages.iter().find(|p| p.id == "spotify");
            match p.and_then(|p| p.widgets.iter().find(|(i, _)| i == id)).map(|(_, w)| &w.kind) {
                Some(WidgetKind::Text { text, .. }) => text.clone(),
                _ => String::new(),
            }
        };
        let wait = |what: &str, ok: &dyn Fn() -> bool| {
            let t = Instant::now();
            while !ok() {
                assert!(t.elapsed() < Duration::from_secs(10), "{what}: {:?} / {:?} / {:?}",
                    text("player"), text("title"), text("msg1"));
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        // authorized once, then the track playing
        wait("track", &|| text("title") == "Morceau 1" && text("artist") == "Artiste A");
        assert!(text("player").starts_with("Spotify · playing · Ordinateur test"));
        let env = std::fs::read_to_string(module.join(".env")).unwrap();
        assert!(env.contains("SPOTIFY_REFRESH_TOKEN="), ".env with the tokens");
        let mode = std::fs::metadata(module.join(".env")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let lists = || shared.api.tool_choices("Spotify").get("playlists").cloned().unwrap_or_default();
        wait("playlists", &|| lists() == ["Ma liste", "Sport"]);
        // commands, from any page
        let run = |id: &str, delta| assert!(shared.api.run_command("Spotify", id, delta), "{id}");
        let state = |id: &str| shared.api.command_state("Spotify", id).map(|(_, s)| s);
        run("next", None);
        wait("next", &|| text("title") == "Morceau 2");
        run("like", None);
        wait("liked", &|| state("like").and_then(|s| s.active) == Some(true) && text("bt4") == "liked");
        run("volume", Some(2));
        wait("volume 60", &|| text("k4") == "volume 60 %");
        run("shuffle", None);
        wait("shuffle", &|| text("k2") == "shuffle: yes");
        run("repeat", None);
        wait("repeat", &|| text("k3") == "repeat: all");
        run("play_pause", None);
        wait("paused", &|| text("player").starts_with("Spotify · paused"));
        run("ajouter:Sport", None);
        let has = |l: &str| log.lock().unwrap().iter().any(|x| x.contains(l));
        wait("added to Sport", &|| has("Sport : 1 morceau(x)"));
        // the page: button 3 next, knob 4 volume (5 % a step)
        let (owner, _) = shared.api.page_of_tool("Spotify", "spotify").unwrap();
        let input = |ev: Value| {
            let mut ev = ev;
            (ev["page"], ev["mine"]) = (json!("spotify"), json!(true));
            shared.api.send(owner, &ev);
        };
        input(json!({"event": "click", "button": "button3"}));
        wait("button 3: next", &|| text("title") == "Morceau 3");
        input(json!({"event": "turn", "knob": 4, "delta": -1}));
        wait("knob 4: volume 55", &|| text("k4") == "volume 55 %");
        // what the device shows: $TMPDIR/hsm01-spotify.png
        wait("cover", &|| {
            let pages = shared.api.pages();
            let p = pages.iter().find(|p| p.id == "spotify").unwrap();
            matches!(&p.widgets.iter().find(|(i, _)| i == "cover").unwrap().1.kind,
                WidgetKind::ThemeIcon { name, .. } if name.ends_with(".img"))
        });
        let pages = shared.api.pages();
        let page = pages.iter().find(|p| p.id == "spotify").unwrap();
        let mut r = crate::render::Renderer::new().unwrap();
        let mut pm = crate::render::Renderer::new_pixmap();
        r.render(&page.page, &crate::state::View::default(), &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-spotify.png")).unwrap();
        // the radio: no recommendations for a new application: the artist, shuffled
        run("radio", None);
        wait("radio", &|| text("title") == "Morceau 1" && has("GET /v1/recommendations -> 404"));
        for c in [&mut spotify, &mut fake] {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The OBS module against a fake OBS (`tools/fake_obs.py`, obs-websocket v5): port and
    /// password read in OBS's own settings; its page (scenes in the OBS window's order, the
    /// program's, three audio sources of the mixer with their dB and meters); knob 1 choosing and
    /// showing a scene, knob 3 the volume, knob 2's click muting; the stream started by a command
    /// and stopped only by holding button 1; the recording paused by button 3; the replay buffer
    /// started then saved; favourites (source, filter, media), a profile, a volume command; the
    /// studio mode (button 4): a scene into the preview, the transition by a second click; the
    /// « État d'OBS » widget; the lists of the form; a wrong password said on the page.
    /// `TMPDIR=… cargo test -- --ignored obs_module` → `$TMPDIR/hsm01-obs.png`.
    #[test]
    #[ignore]
    fn obs_module_against_a_fake_obs() {
        use crate::config::WidgetKind;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("hsm01-obs-{}", std::process::id()));
        let module = dir.join("module");
        std::fs::create_dir_all(&module).unwrap();
        let sock = dir.join("api.sock");
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let listener = bind(&sock).unwrap();
        let s = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || serve(&s, stream));
            }
        });
        let app = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let mut fake = Command::new("python3")
            .args(["-I", &format!("{app}/../tools/fake_obs.py")])
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3");
        let mut out = BufReader::new(fake.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let port: u16 = line.trim().strip_prefix("port ").expect("port").parse().unwrap();
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let l = log.clone();
        std::thread::spawn(move || {
            for line in out.lines().map_while(Result::ok) {
                l.lock().unwrap().push(line);
            }
        });
        // OBS's own settings, as OBS writes them: the module finds the port and password there
        let conf = |password: &str| {
            let d = dir.join("obs-config/plugin_config/obs-websocket");
            std::fs::create_dir_all(&d).unwrap();
            let c = json!({"alerts_enabled": false, "auth_required": true, "first_load": false,
                           "server_enabled": true, "server_password": password, "server_port": port});
            std::fs::write(d.join("config.json"), c.to_string()).unwrap();
        };
        conf("motdepasse");
        let settings = "[boutons]\nb4 = \"mode studio\"\n\n[[source]]\nscene = \"Jeu\"\nsource = \"Titre\"\n\n\
                        [[filtre]]\nsource = \"Caméra\"\nfiltre = \"Flou\"\n\n[[media]]\nsource = \"Musique\"\n";
        std::fs::write(module.join("obs.toml"), settings).unwrap();
        let start = || {
            Command::new("python3")
                .arg(format!("{app}/modules/obs/main.py"))
                .env("PYTHONPATH", format!("{app}/clients/python"))
                .env("HSM01_API_SOCKET", &sock)
                .env("HSM01_MODULE", "OBS")
                .env("HSM01_MODULE_DIR", &module)
                .env("HSM01_OBS_CONFIG", dir.join("obs-config"))
                .env("XDG_CACHE_HOME", dir.join("cache"))
                .spawn()
                .expect("python3")
        };
        let mut obs = start();
        let widget = |page: &str, id: &str| {
            let pages = shared.api.pages();
            let p = pages.iter().find(|p| p.id == page)?;
            p.widgets.iter().find(|(i, _)| i == id).map(|(_, w)| w.clone())
        };
        let text = |page: &str, id: &str| match widget(page, id).map(|w| w.kind) {
            Some(WidgetKind::Text { text, .. }) => text,
            _ => String::new(),
        };
        let t = |id: &str| text("obs", id);
        let wait = |what: &str, ok: &dyn Fn() -> bool| {
            let t0 = Instant::now();
            while !ok() {
                assert!(t0.elapsed() < Duration::from_secs(10), "{what}: {:?} / {:?} / {:?}",
                    text("obs", "title"), text("obs", "msg1"), text("obs", "msg2"));
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let has = |l: &str| log.lock().unwrap().iter().any(|x| x.contains(l));
        // connected with OBS's own port and password: the program scene, the scenes in the
        // window's order (OBS lists them the other way round), the mixer, its meters
        wait("connected", &|| t("title") == "Démarrage");
        assert_eq!([t("n0"), t("n1"), t("n2"), t("n3")], ["Démarrage", "Jeu", "Pause", ""]);
        assert_eq!([t("an2"), t("an3"), t("an4")], ["Mic/Aux", "Audio du bureau", "Musique"]);
        assert_eq!([t("k2"), t("k3"), t("k4")], ["0.0 dB", "-6.0 dB", "-12.0 dB"]);
        wait("meters", &|| widget("obs", "m2").is_some_and(|w| w.h > 1));
        let lists = || shared.api.tool_choices("OBS");
        wait("lists", &|| lists().contains_key("filtres:Caméra"));
        let c = lists();
        assert_eq!(c["scenes"], ["Démarrage", "Jeu", "Pause"]);
        assert_eq!(c["entrees"], ["Mic/Aux", "Audio du bureau", "Musique"]);
        assert_eq!(c["elements:Jeu"], ["Musique", "Titre", "Caméra"]);
        assert_eq!((c["avec_filtres"].clone(), c["filtres:Caméra"].clone(), c["medias"].clone()),
                   (vec!["Caméra".to_string()], vec!["Flou".to_string(), "Correction".to_string()],
                    vec!["Musique".to_string()]));
        // the page: knob 1 chooses, its click shows; knob 3 the volume; knob 2's click mutes
        let (owner, _) = shared.api.page_of_tool("OBS", "obs").unwrap();
        let input = |ev: Value| {
            let mut ev = ev;
            (ev["page"], ev["mine"]) = (json!("obs"), json!(true));
            shared.api.send(owner, &ev);
        };
        input(json!({"event": "turn", "knob": 1, "delta": 1}));
        wait("knob 1 chooses", &|| t("k1") == "scene 2/3");
        input(json!({"event": "click", "button": "knob1"}));
        wait("scene shown", &|| t("title") == "Jeu" && has("requête SetCurrentProgramScene"));
        input(json!({"event": "turn", "knob": 3, "delta": -5}));
        wait("volume", &|| has("requête SetInputVolume") && t("k3") == "-10.0 dB");
        input(json!({"event": "click", "button": "knob2"}));
        wait("mic muted", &|| t("k2") == "muted");
        // commands, from any page; the stream stopped from the page only by holding button 1
        let run = |id: &str, delta| assert!(shared.api.run_command("OBS", id, delta), "{id}");
        let state = |id: &str| shared.api.command_state("OBS", id).and_then(|(_, s)| s.active);
        run("direct", None);
        wait("live", &|| t("lvt").starts_with("LIVE ") && state("direct") == Some(true));
        input(json!({"event": "click", "button": "button1"}));
        std::thread::sleep(Duration::from_millis(400));
        assert!(!has("requête StopStream"), "a short press does not stop the stream");
        input(json!({"event": "hold", "button": "button1"}));
        wait("stream stopped", &|| has("requête StopStream") && t("lvt") == "LIVE");
        run("enregistrement", None);
        wait("recording", &|| t("rct").starts_with("REC "));
        input(json!({"event": "click", "button": "button3"}));
        wait("paused", &|| t("rct").starts_with("PAUSE "));
        run("replay", None);
        wait("replay buffer on", &|| state("replay") == Some(true));
        run("replay", None);
        wait("replay saved", &|| has("évènement ReplayBufferSaved"));
        run("source:Jeu/Titre", None);
        wait("source shown", &|| state("source:Jeu/Titre") == Some(true));
        run("filtre:Caméra/Flou", None);
        wait("filter on", &|| state("filtre:Caméra/Flou") == Some(true));
        run("media:Musique", None);
        wait("media", &|| has("requête TriggerMediaInputAction") && state("media:Musique") == Some(true));
        run("profil:Test", None);
        wait("profile", &|| state("profil:Test") == Some(true) && state("profil:Principal") == Some(false));
        run("volume:Musique", Some(3));
        let vol = || shared.api.command_state("OBS", "volume:Musique").and_then(|(_, s)| s.text);
        wait("volume command", &|| vol().is_some_and(|v| v.starts_with("Musique -") && v != "Musique -12.0 dB"));
        // studio mode (button 4): the chosen scene into the preview, a second click: transition
        input(json!({"event": "click", "button": "button4"}));
        wait("studio", &|| state("studio") == Some(true));
        input(json!({"event": "turn", "knob": 1, "delta": 1}));
        wait("Pause chosen", &|| t("k1") == "scene 3/3");
        input(json!({"event": "click", "button": "knob1"}));
        wait("preview", &|| has("requête SetCurrentPreviewScene {\"sceneName\": \"Pause\"}"));
        input(json!({"event": "click", "button": "knob1"}));
        wait("transition", &|| t("title") == "Pause");
        // the widget « État d'OBS »
        assert_eq!(text("etat", "es"), "Pause");
        assert!(text("etat", "ert").starts_with("PAUSE ") && text("etat", "elt") == "not live");
        // what the device shows: $TMPDIR/hsm01-obs.png
        let pages = shared.api.pages();
        let page = pages.iter().find(|p| p.id == "obs").unwrap();
        let mut r = crate::render::Renderer::new().unwrap();
        let mut pm = crate::render::Renderer::new_pixmap();
        r.render(&page.page, &crate::state::View::default(), &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-obs.png")).unwrap();
        // a wrong password in OBS's settings: said on the page
        let _ = obs.kill();
        let _ = obs.wait();
        conf("faux");
        let mut obs = start();
        wait("refused", &|| t("msg1").contains("refused"));
        for c in [&mut obs, &mut fake] {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The telemetry modules on their shared dashboard (`hsm01.dashboard`): Motor Town against
    /// `tools/fake_motortown.py` (native v1.2 datagrams: waiting message first, then the driving
    /// screen, the race screen when the race starts, the vehicle screen by a click of knob 1, a
    /// button sending its keys), SimHub against the same fake game's « SimHub-compatible »
    /// stream (Forza Data Out, UDP) then, source « programme SimHub », against
    /// `tools/fake_simhub.py` (its web API: driving, race, a property it does not know).
    /// `TMPDIR=… cargo test -- --ignored telemetry_modules`
    /// → `$TMPDIR/hsm01-motortown.png`, `hsm01-motortown-course.png`, `hsm01-simhub.png`.
    #[test]
    #[ignore]
    fn telemetry_modules_against_fake_games() {
        use crate::config::WidgetKind;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("hsm01-telemetry-{}", std::process::id()));
        let sock = dir.join("api.sock");
        std::fs::create_dir_all(&dir).unwrap();
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let listener = bind(&sock).unwrap();
        let s = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || serve(&s, stream));
            }
        });
        let app = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let module = |id: &str, name: &str, settings: &str, env: &[(&str, String)]| {
            let d = dir.join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("{id}.toml")), settings).unwrap();
            let mut c = Command::new("python3");
            c.arg(format!("{app}/modules/{id}/main.py"))
                .env("PYTHONPATH", format!("{app}/clients/python"))
                .env("HSM01_API_SOCKET", &sock)
                .env("HSM01_MODULE", name)
                .env("HSM01_MODULE_DIR", &d)
                .env("XDG_CACHE_HOME", dir.join("cache"));
            for (k, v) in env {
                c.env(k, v);
            }
            c.spawn().expect("python3")
        };
        // the page of module `name` (both draw a page « tableau »)
        let page = |name: &str| {
            let (owner, _) = shared.api.page_of_tool(name, "tableau")?;
            shared.api.pages().into_iter().find(|p| p.owner == owner && p.id == "tableau")
        };
        let text = |name: &str, id: &str| match page(name)
            .and_then(|p| p.widgets.iter().find(|(i, _)| i == id).map(|(_, w)| w.kind.clone()))
        {
            Some(WidgetKind::Text { text, .. }) => text,
            _ => String::new(),
        };
        let wait = |what: &str, ok: &dyn Fn() -> bool| {
            let t0 = Instant::now();
            while !ok() {
                assert!(t0.elapsed() < Duration::from_secs(10), "{what}: {:?} / {:?}", text("Motor Town", "msg1"), text("SimHub", "msg1"));
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let png = |name: &str, file: &str| {
            let p = page(name).unwrap();
            let mut r = crate::render::Renderer::new().unwrap();
            let mut pm = crate::render::Renderer::new_pixmap();
            r.render(&p.page, &crate::state::View::default(), &mut pm);
            pm.save_png(std::env::temp_dir().join(file)).unwrap();
        };
        let input = |name: &str, ev: Value| {
            let (owner, _) = shared.api.page_of_tool(name, "tableau").unwrap();
            let mut ev = ev;
            (ev["page"], ev["mine"]) = (json!("tableau"), json!(true));
            shared.api.send(owner, &ev);
        };
        // Motor Town: waiting, then the fake game's datagrams
        let port = std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let relayed = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        relayed.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let settings = format!(
            "ecoute = \"127.0.0.1\"\nport = {port}\n\n[[touche]]\nnom = \"Phares\"\ntouches = \"h\"\n\n\
             [[relais]]\nip = \"127.0.0.1\"\nport = {}\n",
            relayed.local_addr().unwrap().port()
        );
        let mut mt = module("motortown", "Motor Town", &settings, &[]);
        let t = |id: &str| text("Motor Town", id);
        wait("waiting", &|| t("msg1") == "Waiting for Motor Town");
        assert_eq!(t("key1"), "Phares");
        let mut game = Command::new("python3")
            .args(["-I", &format!("{app}/../tools/fake_motortown.py"), "--port", &port.to_string(),
                   "--race", "2.5"])
            .stdout(Stdio::null())
            .spawn()
            .expect("python3");
        wait("driving", &|| t("spd") == "48" && t("gear") == "3");
        // each datagram relayed as it is
        let mut buf = [0u8; 2048];
        let (n, _) = relayed.recv_from(&mut buf).expect("relayed datagram");
        assert_eq!((n, &buf[..4]), (858, &b"MTTM"[..]));
        assert_eq!((t("veh"), t("msg1"), t("fu"), t("co")), ("Camion test".into(), String::new(),
                   "321.5 L".into(), "92 °C".into()));
        assert_eq!((t("lmt"), t("rpmt"), t("dest")), ("50".into(), "3450 rpm".into(),
                   "destination 2.4 km".into()));
        png("Motor Town", "hsm01-motortown.png");
        // the race starts: its screen comes by itself
        wait("race screen", &|| t("rpos") == "P2" && t("rlap") == "lap 2/3");
        assert_eq!((t("rlast"), t("rbest"), t("rsec")), ("last 1:23.456".into(),
                   "best 1:22.500".into(), "checkpoint 5/12".into()));
        png("Motor Town", "hsm01-motortown-course.png");
        // knob 1's click: the vehicle screen; button 1: its keys to the game
        input("Motor Town", json!({"event": "click", "button": "knob1"}));
        wait("vehicle screen", &|| t("wt0") == "64 °C" && t("e0") == "296 kW · 820 N·m");
        input("Motor Town", json!({"event": "click", "button": "button1"}));
        wait("keys", &|| shared.api.actions.lock().unwrap().iter().any(|a| format!("{a:?}").contains("\"h\"")));
        for c in [&mut mt, &mut game] {
            let _ = c.kill();
            let _ = c.wait();
        }
        // SimHub, by default: the game's « SimHub-compatible » stream (Forza Data Out, UDP)
        let port = std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let settings = format!("ecoute = \"127.0.0.1\"\nport = {port}\n\n[[relais]]\nip = \"nimporte\"\nport = 9000\n");
        let mut sh = module("simhub", "SimHub", &settings, &[]);
        let t = |id: &str| text("SimHub", id);
        wait("simhub waiting", &|| t("msg1") == "Waiting for the game");
        assert!(t("msg3").starts_with("Setting: relay: \"nimporte\""), "{}", t("msg3"));
        let mut game = Command::new("python3")
            .args(["-I", &format!("{app}/../tools/fake_motortown.py"), "--port", &port.to_string(), "--forza"])
            .stdout(Stdio::null())
            .spawn()
            .expect("python3");
        wait("simhub stream", &|| t("spd") == "48" && t("gear") == "3" && t("rpmt") == "3450 rpm");
        assert_eq!((t("veh"), t("fu"), t("bo")), ("SimHub".into(), "80 %".into(), "turbo 0.40 bar".into()));
        for c in [&mut sh, &mut game] {
            let _ = c.kill();
            let _ = c.wait();
        }
        // SimHub, the program: its web API
        let mut fake = Command::new("python3")
            .args(["-I", &format!("{app}/../tools/fake_simhub.py")])
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3");
        let mut out = BufReader::new(fake.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let url = format!("http://127.0.0.1:{}", line.trim().strip_prefix("port ").expect("port"));
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let a = asked.clone();
        std::thread::spawn(move || {
            for line in out.lines().map_while(Result::ok) {
                a.lock().unwrap().push(line);
            }
        });
        let settings = "source = \"programme SimHub\"\necran_course = false\n";
        let mut sh = module("simhub", "SimHub", settings, &[("HSM01_SIMHUB_URL", url)]);
        wait("simhub driving", &|| t("spd") == "123" && t("gear") == "4");
        assert_eq!((t("veh"), t("fu"), t("co"), t("rpmt")), ("Voiture test".into(), "31.2 L".into(),
                   "88 °C".into(), "6800 rpm".into()));
        assert!(asked.lock().unwrap().iter().any(|l| l.contains("TyreTemperatureFrontLeft")), "asked, unknown: skipped");
        png("SimHub", "hsm01-simhub.png");
        input("SimHub", json!({"event": "turn", "knob": 1, "delta": 1}));
        wait("simhub race", &|| t("rpos") == "P3" && t("rlap") == "lap 2/5" && t("rtime") == "0:41.250");
        assert_eq!((t("rposu"), t("rbest")), ("of 11".into(), "best 1:31.987".into()));
        for c in [&mut sh, &mut fake] {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Alert styles: the card by default; a pill (one line, its border, dots and text colours,
    /// the dots as the border unless given, 3 s unless given); wrong colours or styles refused.
    #[test]
    fn pill_alerts() {
        use notify::NoticeStyle::{Card, Pill};
        let card = alert(1, 1, &json!({"title": "T", "text": "x"})).unwrap();
        assert_eq!((card.style, card.summary.as_str(), card.duration), (Card, "T", None));
        let p = alert(1, 2, &json!({"title": "", "text": "EN DIRECT", "style": "pill",
                                    "border": "#ff0000"}))
            .unwrap();
        let red = Color(255, 0, 0);
        assert_eq!(p.style, Pill { border: red, dot: red, color: Color(255, 255, 255) });
        assert_eq!((p.summary.as_str(), p.body.as_str(), p.duration), ("EN DIRECT", "", Some(3.0)));
        let p = alert(1, 3, &json!({"title": "LIVE", "style": "pill", "dot": "#0000ff",
                                    "color": "#00ff00", "duration": 5}))
            .unwrap();
        let Pill { dot, color, .. } = p.style else { panic!() };
        assert_eq!((dot, color, p.summary.as_str(), p.duration), (Color(0, 0, 255), Color(0, 255, 0), "LIVE", Some(5.0)));
        assert!(alert(1, 4, &json!({"text": "x", "style": "pill", "dot": "vert"})).is_err());
        assert!(alert(1, 5, &json!({"text": "x", "style": "bulle"})).is_err());
    }

    /// The Discord module against `tools/fake_discord.py` (its RPC, token endpoint, avatars):
    /// authorized once (tokens kept in its .env, owner only; a restart reuses them), the voice
    /// channel on its page (members, avatars, who speaks), its commands (micro, a followed
    /// member's volume and level, a favourite channel), the page's inputs (choose a member, mute
    /// them for oneself, leave, join from the favourites). Without ID and secret: what to do.
    /// `TMPDIR=… cargo test -- --ignored discord_module` → `$TMPDIR/hsm01-discord.png`.
    #[test]
    #[ignore]
    fn discord_module_against_a_fake_discord() {
        use crate::config::{Color, Paint, WidgetKind};
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("hsm01-discord-{}", std::process::id()));
        let (module, empty) = (dir.join("module"), dir.join("sans-id"));
        std::fs::create_dir_all(&module).unwrap();
        std::fs::create_dir_all(&empty).unwrap();
        let sock = dir.join("api.sock");
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let listener = bind(&sock).unwrap();
        let s = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || serve(&s, stream));
            }
        });
        let app = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let ipc = dir.join("discord-ipc-0");
        let mut fake = Command::new("python3")
            .args(["-I", &format!("{app}/../tools/fake_discord.py"), "--socket"])
            .arg(&ipc)
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3");
        let mut out = BufReader::new(fake.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let base = format!("http://127.0.0.1:{}", line.trim().strip_prefix("port ").expect("port"));
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let l = log.clone();
        std::thread::spawn(move || {
            for line in out.lines().map_while(Result::ok) {
                l.lock().unwrap().push(line);
            }
        });
        let pairs = [("DISCORD_CLIENT_ID", "123"), ("DISCORD_CLIENT_SECRET", "s3cret")];
        let pairs: Vec<_> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        crate::tools::write_env(&module, &pairs).unwrap();
        let settings = "[[salon]]\nserveur = \"Serveur test\"\nsalon = \"Jeux\"\n\n\
                        [[membre]]\nnom = \"Alice\"\nniveau = 50\n";
        std::fs::write(module.join("discord.toml"), settings).unwrap();
        let lib = format!("{app}/clients/python");
        let start = |name: &str, folder: &std::path::Path| {
            Command::new("python3")
                .arg(format!("{app}/modules/discord/main.py"))
                .env("PYTHONPATH", &lib)
                .env("HSM01_API_SOCKET", &sock)
                .env("HSM01_MODULE", name)
                .env("HSM01_MODULE_DIR", folder)
                .env("HSM01_DISCORD_IPC", &ipc)
                .env("HSM01_DISCORD_API", &base)
                .env("HSM01_DISCORD_CDN", &base)
                .env("XDG_CACHE_HOME", dir.join("cache"))
                .spawn()
                .expect("python3")
        };
        let mut discord = start("Discord", &module);
        let mut unset = start("Discord sans ID", &empty);
        let widget = |name: &str, id: &str| -> Option<WidgetKind> {
            let (owner, page) = shared.api.page_of_tool(name, "discord")?;
            let pages = shared.api.pages();
            let p = pages.iter().find(|p| p.owner == owner && p.id == page)?;
            Some(p.widgets.iter().find(|(i, _)| i == id)?.1.kind.clone())
        };
        let text = |id: &str| match widget("Discord", id) {
            Some(WidgetKind::Text { text, .. }) => text,
            _ => String::new(),
        };
        let wait = |what: &str, ok: &dyn Fn() -> bool| {
            let t = Instant::now();
            while !ok() {
                assert!(t.elapsed() < Duration::from_secs(10), "{what}: {:?} / {:?} / {:?}",
                    text("title"), text("msg1"), text("n0"));
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        // authorized (AUTHORIZE, code exchanged for a token), then the channel I am in
        wait("channel", &|| text("title") == "Général" && text("where") == "Serveur test");
        assert_eq!([text("n0"), text("n1"), text("n2")], ["Moi", "Alice", "bob"]);
        assert_eq!([text("u0"), text("u1")], ["you", "100 %"]);
        assert!(matches!(widget("Discord", "m2"), Some(WidgetKind::ThemeIcon { name, .. })
            if name == "microphone-sensitivity-muted"), "bob muted himself");
        let env = std::fs::read_to_string(module.join(".env")).unwrap();
        assert!(env.contains("DISCORD_REFRESH_TOKEN=") && env.contains("DISCORD_CLIENT_ID=\"123\""));
        let mode = std::fs::metadata(module.join(".env")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, ".env owner only");
        wait("no ID", &|| matches!(widget("Discord sans ID", "msg1"),
            Some(WidgetKind::Text { text, .. }) if text.starts_with("ID and secret")));
        // avatars downloaded (their own and the default one), drawn round
        let path = |id: &str| match widget("Discord", id) {
            Some(WidgetKind::Image { path, radius }) if radius > 0.0 => path,
            _ => String::new(),
        };
        wait("avatars", &|| (0..3).all(|i| path(&format!("a{i}")).ends_with(".png")));
        // commands: micro, a followed member's volume (knob) and level (button)
        let state = |id: &str| shared.api.command_state("Discord", id).map(|(_, s)| s);
        assert_eq!(state("mute").and_then(|s| s.active), Some(false));
        assert!(shared.api.run_command("Discord", "mute", None));
        wait("muted", &|| state("mute").and_then(|s| s.active) == Some(true)
            && text("bt1") == "mic muted");
        let leds = || shared.api.pages().iter().find(|p| p.id == "discord" && p.leds[0] > 0).map(|p| p.leds);
        assert_eq!(leds().map(|l| l[0]), Some(100), "LED 1: micro muted");
        let alice = || state("volume:Alice").and_then(|s| s.text);
        assert_eq!(alice().as_deref(), Some("Alice 100 %"));
        assert!(shared.api.run_command("Discord", "volume:Alice", Some(5)));
        wait("Alice 110 %", &|| alice().as_deref() == Some("Alice 110 %") && text("u1") == "110 %");
        assert!(shared.api.run_command("Discord", "niveau:Alice", None));
        wait("Alice 50 %", &|| state("niveau:Alice").and_then(|s| s.active) == Some(true));
        // the page's inputs: knob 1 chooses bob, his click mutes him for me
        let (owner, _) = shared.api.page_of_tool("Discord", "discord").unwrap();
        let input = |ev: Value| {
            let mut ev = ev;
            (ev["page"], ev["mine"]) = (json!("discord"), json!(true));
            shared.api.send(owner, &ev);
        };
        // knob 2: the volume of the member chosen (Alice first: not me), 2 % a step
        assert_eq!(text("k2"), "volume 50 %", "Alice chosen at first (her level: 50 %)");
        input(json!({"event": "turn", "knob": 2, "delta": 3}));
        wait("Alice 56 %", &|| text("k2") == "volume 56 %" && text("u1") == "56 %");
        input(json!({"event": "turn", "knob": 1, "delta": 2}));
        wait("bob chosen", &|| text("k2") == "volume 100 %");
        input(json!({"event": "click", "button": "knob1"}));
        wait("bob muted for me", &|| text("u2") == "muted for you");
        // Alice speaks: green ring; what the device shows
        let green = Paint::Solid(Color(0x23, 0xa5, 0x5a));
        wait("Alice speaks", &|| matches!(widget("Discord", "r1"),
            Some(WidgetKind::Rect { color, .. }) if color == green));
        let pages = shared.api.pages();
        let page = pages.iter().find(|p| p.owner == owner && p.id == "discord").unwrap();
        let mut r = crate::render::Renderer::new().unwrap();
        let mut pm = crate::render::Renderer::new_pixmap();
        r.render(&page.page, &crate::state::View::default(), &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-discord.png")).unwrap();
        let (no_id, _) = shared.api.page_of_tool("Discord sans ID", "discord").unwrap();
        let page = pages.iter().find(|p| p.owner == no_id && p.id == "discord").unwrap();
        r.render(&page.page, &crate::state::View::default(), &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-discord-sans-id.png")).unwrap();
        // its widgets « Membres du salon vocal »: in a column 220 px wide, 40 px a member, taller
        // when one comes; in a row 150 px a member, 40 px high, wider; in a smaller box of a
        // page, scrolled: $TMPDIR/hsm01-discord-widget.png, hsm01-discord-widget-ligne.png
        let size = |id: &str| {
            let pages = shared.api.tool_pages();
            pages.into_iter().find(|p| p.tool == "Discord" && p.id == id).and_then(|p| p.size)
        };
        wait("widgets", &|| {
            size("membres") == Some((220, 120)) && size("membres_ligne") == Some((450, 40))
        });
        let fake_get = |path: &str| {
            use std::io::{Read, Write};
            let mut s = std::net::TcpStream::connect(base.trim_start_matches("http://")).unwrap();
            write!(s, "GET {path} HTTP/1.0\r\n\r\n").unwrap();
            let _ = s.read_to_string(&mut String::new());
        };
        fake_get("/control/join?user=chloe");
        wait("Chloé joined", &|| {
            size("membres") == Some((220, 160)) && size("membres_ligne") == Some((600, 40))
        });
        // … said by a green pill (one per application: the newest of Discord's alerts)
        let pill = || {
            let q = shared.notifications.lock().unwrap();
            q.iter().any(|n| n.summary == "CHLOÉ JOINED THE CHANNEL"
                && matches!(n.style, notify::NoticeStyle::Pill { .. }))
        };
        wait("join pill", &pill);
        // the lists of its settings form: servers, their voice channels, members met (not me)
        let list = |name: &str| shared.api.tool_choices("Discord").get(name).cloned().unwrap_or_default();
        wait("choices", &|| list("salons:Serveur test") == ["Général", "Jeux"]);
        assert_eq!(list("serveurs"), ["Serveur test"]);
        wait("members met", &|| list("membres") == ["Alice", "bob", "Chloé"]);
        let pages = shared.api.pages();
        let list = pages.iter().find(|p| p.owner == owner && p.id == "membres").unwrap();
        let mut host = crate::config::Config::default().pages[0].clone();
        host.wallpaper = None;
        host.widgets = vec![crate::config::Widget {
            x: 20,
            y: 60,
            w: 220,
            h: 100,
            frame: None,
            kind: WidgetKind::Tool {
                tool: "Discord".into(),
                page: "membres".into(),
                inputs: Vec::new(),
                overflow: crate::config::Overflow::Scroll,
            },
        }];
        let mut view = crate::state::View { time: 4.0, ..Default::default() };
        view.embedded.insert(0, (list.page.clone(), list.size.unwrap()));
        r.render(&host, &view, &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-discord-widget.png")).unwrap();
        let row = pages.iter().find(|p| p.owner == owner && p.id == "membres_ligne").unwrap();
        let w = &mut host.widgets[0];
        (w.x, w.y, w.w, w.h) = (100, 0, 289, 30);
        if let WidgetKind::Tool { page, overflow, .. } = &mut w.kind {
            (*page, *overflow) = ("membres_ligne".into(), crate::config::Overflow::ScrollSideways);
        }
        view.embedded.insert(0, (row.page.clone(), row.size.unwrap()));
        r.render(&host, &view, &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-discord-widget-ligne.png")).unwrap();
        // the favourite channel (command), leave (button 4), join it back from the page
        assert!(shared.api.run_command("Discord", "salon:Serveur test/Jeux", None));
        wait("in Jeux", &|| text("title") == "Jeux" && text("n0") == "Moi");
        input(json!({"event": "click", "button": "button4"}));
        wait("left", &|| text("title") == "Not in a voice channel" && text("n0") == "Jeux");
        input(json!({"event": "click", "button": "knob1"}));
        wait("in Jeux again", &|| text("title") == "Jeux");
        // restarted: the token kept in .env, no new authorization
        let _ = discord.kill();
        let _ = discord.wait();
        wait("gone", &|| shared.api.page_of_tool("Discord", "discord").is_none());
        let mut discord = start("Discord", &module);
        wait("back", &|| text("title") == "Jeux");
        std::thread::sleep(Duration::from_millis(300));
        let authorize = log.lock().unwrap().iter().filter(|l| l.starts_with("AUTHORIZE")).count();
        assert_eq!(authorize, 1, "authorized once: {:?}", log.lock().unwrap());
        // Discord quits: « pas lancé »; back (not knowing the tokens kept): authorized again
        let _ = fake.kill();
        let _ = fake.wait();
        wait("Discord gone", &|| text("msg1") == "Discord is not running" && text("n0").is_empty());
        let port = base.rsplit(':').next().unwrap().to_owned();
        let mut fake = Command::new("python3")
            .args(["-I", &format!("{app}/../tools/fake_discord.py"), "--port", &port, "--socket"])
            .arg(&ipc)
            .stdout(Stdio::null())
            .spawn()
            .expect("python3");
        wait("Discord back", &|| text("title") == "Général" && text("n1") == "Alice");
        for c in [&mut discord, &mut unset, &mut fake] {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The live scores module against `tools/fake_lequipe.py` (a feed shaped like L'Équipe's):
    /// only the matches under way, by sport and competition, on its page and in its widget; the
    /// lists of its form (everything met in the feed); a pill when a score changes; the feed
    /// asked often while a match of the filter is under way (every second here instead of a
    /// minute), rarely otherwise (3 s instead of 5 min); a team filter.
    /// `TMPDIR=… cargo test -- --ignored scores_module` → `$TMPDIR/hsm01-scores.png`.
    #[test]
    #[ignore]
    fn scores_module_against_a_fake_feed() {
        use crate::config::WidgetKind;
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("hsm01-scores-{}", std::process::id()));
        let sock = dir.join("api.sock");
        std::fs::create_dir_all(&dir).unwrap();
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        let listener = bind(&sock).unwrap();
        let s = shared.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let s = s.clone();
                std::thread::spawn(move || serve(&s, stream));
            }
        });
        let app = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        // the fake feed: its port, then a line per request
        let mut fake = Command::new("python3")
            .args(["-I", &format!("{app}/../tools/fake_lequipe.py")])
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3");
        let mut out = BufReader::new(fake.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let base = format!("http://127.0.0.1:{}", line.trim().strip_prefix("port ").expect("port"));
        let asked = Arc::new(Mutex::new(Vec::<Instant>::new()));
        let a = asked.clone();
        std::thread::spawn(move || {
            for line in out.lines().map_while(Result::ok) {
                if line == "GET lives" {
                    a.lock().unwrap().push(Instant::now());
                }
            }
        });
        let start = |settings: &str| {
            let d = dir.join("scores");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("scores.toml"), settings).unwrap();
            Command::new("python3")
                .arg(format!("{app}/modules/scores/main.py"))
                .env("PYTHONPATH", format!("{app}/clients/python"))
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .env("HSM01_API_SOCKET", &sock)
                .env("HSM01_MODULE", "Live scores")
                .env("HSM01_MODULE_DIR", &d)
                .env("XDG_CACHE_HOME", dir.join("cache"))
                .env("HSM01_SCORES_URL", format!("{base}/api/live/lives"))
                .env("HSM01_SCORES_PERIODS", "1,3")
                .spawn()
                .expect("python3")
        };
        let text = |page: &str, id: &str| {
            let (owner, _) = shared.api.page_of_tool("Live scores", "scores")?;
            let pages = shared.api.pages();
            let p = pages.iter().find(|p| p.owner == owner && p.id == page)?;
            match &p.widgets.iter().find(|(i, _)| i == id)?.1.kind {
                WidgetKind::Text { text, .. } => Some(text.clone()),
                _ => None,
            }
        };
        let t = |id: &str| text("scores", id).unwrap_or_default();
        let wait = |what: &str, ok: &dyn Fn() -> bool| {
            let t0 = Instant::now();
            while !ok() {
                assert!(t0.elapsed() < Duration::from_secs(10), "{what}: {:?} / {:?} / {:?}",
                        t("count"), t("msg1"), t("msg2"));
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let stop = |c: &mut std::process::Child| {
            let _ = c.kill();
            let _ = c.wait();
        };
        let mut m = start("pilule_score = true\n");
        // everything under way, by sport in the feed's order; the match to come left out
        wait("live", &|| t("count") == "2 live");
        assert_eq!([t("h0"), t("a1"), t("s1"), t("p1"), t("b1")],
                   ["FOOTBALL · Ligue 1", "Nice", "1 - 0", "45'", "Paris-SG"]);
        assert_eq!([t("h2"), t("a3"), t("s3"), t("p3"), t("b3")],
                   ["BASKET · NBA", "Atlanta Hawks", "56 - 57", "mi-temps", "Charlotte Hornets"]);
        assert!(t("a4").is_empty() && t("msg1").is_empty());
        // its widget: a line of 24 px for each, headers included
        let size = || {
            let pages = shared.api.tool_pages();
            pages.into_iter().find(|p| p.tool == "Live scores" && p.id == "scores_liste").and_then(|p| p.size)
        };
        wait("widget", &|| size() == Some((360, 96)));
        assert_eq!(text("scores_liste", "ws1").as_deref(), Some("1 - 0"));
        // the lists of its form: what the feed holds, the match to come included
        let list = |name: &str| shared.api.tool_choices("Live scores").get(name).cloned().unwrap_or_default();
        wait("choices", &|| list("sports") == ["Basket", "Football"]);
        assert_eq!(list("competitions"), ["Ligue 1", "NBA"]);
        assert!(list("equipes").iter().any(|e| e == "Lens"), "{:?}", list("equipes"));
        // a match of the filter under way: asked every second (a minute for real)
        let n = || asked.lock().unwrap().len();
        let before = n();
        std::thread::sleep(Duration::from_millis(2600));
        assert!(n() >= before + 2, "fast: {} asks in 2.6 s", n() - before);
        // a goal: its pill, the new score
        {
            use std::io::{Read, Write};
            let mut s = std::net::TcpStream::connect(base.trim_start_matches("http://")).unwrap();
            write!(s, "GET /control/goal HTTP/1.0\r\n\r\n").unwrap();
            let _ = s.read_to_string(&mut String::new());
        }
        let pill = || shared.notifications.lock().unwrap().iter().any(|n| n.summary == "NICE 2-0 PARIS-SG");
        wait("goal pill", &pill);
        wait("new score", &|| t("s1") == "2 - 0");
        let pages = shared.api.pages();
        let page = pages.iter().find(|p| p.id == "scores").unwrap();
        let mut r = crate::render::Renderer::new().unwrap();
        let mut pm = crate::render::Renderer::new_pixmap();
        r.render(&page.page, &crate::state::View::default(), &mut pm);
        pm.save_png(std::env::temp_dir().join("hsm01-scores.png")).unwrap();
        // a filter without a match under way: said so, asked rarely (5 min for real)
        stop(&mut m);
        let mut m = start("[[equipe]]\nnom = \"lens\"\n");
        wait("no match", &|| t("msg1") == "No live match" && t("msg2") == "filter: lens");
        let before = n();
        std::thread::sleep(Duration::from_millis(2600));
        assert!(n() <= before + 1, "slow: {} asks in 2.6 s", n() - before);
        // a team (part of its name, any case) and a sport: that match only
        stop(&mut m);
        let mut m = start("[[sport]]\nnom = \"football\"\n\n[[equipe]]\nnom = \"paris\"\n");
        wait("filtered", &|| t("count") == "1 live" && t("a1") == "Nice" && t("h2").is_empty());
        stop(&mut m);
        stop(&mut fake);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
