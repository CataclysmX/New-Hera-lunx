//! Modules (« tools » in the code): programs the app runs while it runs (window tab « Modules »,
//! docs/app/modules.md). Every module is a sub-folder of one external folder,
//! `~/.local/share/hsm01/modules/<id>/`, with its `module.toml` (name, description, program) and
//! its files; the folder is read when asked (« Rafraîchir les modules »), nothing is compiled in.
//! The modules shipped with the app (`app/modules` next to the build, or
//! `/usr/share/hsm01/modules`) are copied there once each and can be changed like the others
//! (« Réinstaller les modules livrés » puts them back). Formerly `tools/` and `tool.toml`: an old
//! folder is moved at start, an old `tool.toml` still read.
//!
//! The config (`[[module]]`) says which ones run (`enabled`) and with which `args`. A running
//! module gets `HSM01_API_SOCKET`, `HSM01_MODULE` (its name; also `HSM01_TOOL`, the former name)
//! and, for Python, `PYTHONPATH` to the API's
//! library; it is restarted when it stops (1 s, 2 s, 4 s… 30 s at most; back to 1 s after a
//! minute of running) and when one of its code files changes, stopped when it is switched off,
//! removed or when the app quits. Its last output lines are kept for the window.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::i18n::tr;
use crate::config::Tool;
use crate::engine::Shared;

/// Output lines kept per tool.
const LOG_LINES: usize = 80;
/// A changed tool is restarted once its settings stay the same this long (typing).
const SETTLE: Duration = Duration::from_millis(1200);
/// Files whose change restarts a running tool (data files a tool writes do not).
const CODE: [&str; 11] = ["py", "js", "mjs", "cjs", "ts", "sh", "lua", "rb", "pl", "php", "toml"];

/// What the window shows of a tool.
#[derive(Debug, Clone, Default)]
pub struct ToolStatus {
    pub running: bool,
    pub pid: Option<u32>,
    /// Starts after the first one.
    pub restarts: u32,
    /// Why it stopped last (exit status, or why it could not start).
    pub last_exit: Option<String>,
    pub log: VecDeque<String>,
    /// Asked by the window (its secrets changed): restarted at the next check.
    pub restart: bool,
    /// Frozen because hidden (« Load only when shown »).
    pub paused: bool,
    /// Not started, or stopped: « Load only when shown », it starts when the screen needs it.
    pub waiting: bool,
}

/// A tool of the tools folder (`<id>/module.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct Module {
    /// Name of its folder.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Python file, relative to `dir`…
    #[serde(default)]
    pub file: String,
    /// …or any program and its arguments (relative paths: from `dir`).
    #[serde(default)]
    pub command: Vec<String>,
    /// Default arguments, editable in the window.
    #[serde(default)]
    pub args: Vec<String>,
    /// Its settings file: while the tool is enabled, the window gives it a tab.
    #[serde(default)]
    pub settings: Option<Settings>,
    /// Its sensitive data (`[[secret]]`): typed in its tab, kept in its `.env`.
    #[serde(default, rename = "secret")]
    pub secrets: Vec<SecretField>,
    /// Its folder (also its working directory).
    #[serde(skip)]
    pub dir: PathBuf,
}

/// `[settings]` of a `module.toml`: a file of the tool's folder, edited in a tab of the window.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Settings {
    pub file: String,
    /// `deck`: the stream deck editor (tracks and tiles); `formulaire`: a form of its `field`s;
    /// anything else (`texte`): the TOML text, checked before it is saved.
    #[serde(default)]
    pub editor: String,
    /// Shown at the top of the tab (one paragraph per line; addresses become links).
    #[serde(default)]
    pub guide: String,
    /// The form (`[[settings.field]]`).
    #[serde(default, rename = "field")]
    pub fields: Vec<Field>,
}

/// A field of a settings form: `key` in the file (dotted for nested tables:
/// "alertes.gpu.seuil"), shown as `label` with `help` on hover. `kind`: `number` (`min`, `max`,
/// `step`, `unit`; `optional`: may be absent), `text`, `bool`, `choice` (`choices`, or `source`:
/// lists the running module gives, `{key}` = a sibling field's value; `choice_labels`: what is
/// shown for them), `list` (array of tables:
/// its item `fields`, button `add`), `section` (a heading).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Field {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub help: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub choices: Vec<String>,
    /// What the window shows for each of `choices` (same order; the value saved stays the
    /// choice), translated with the module's file.
    #[serde(default)]
    pub choice_labels: Vec<String>,
    #[serde(default)]
    pub source: String,
    pub default: Option<toml::Value>,
    #[serde(default, rename = "field")]
    pub fields: Vec<Field>,
    #[serde(default)]
    pub add: String,
}

/// `[[secret]]` of a `module.toml`: a value the module needs and that must not be shared (token,
/// password, key); typed in the module's tab, kept in its `.env`, given to its program as the
/// environment variable `name`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SecretField {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    /// Shown as dots (a secret; an ID can stay visible).
    #[serde(default)]
    pub hidden: bool,
}

/// A module's sensitive data, in its folder: `KEY=VALUE` lines, owner-only rights (0600), given
/// to its program as environment variables; never in config.toml, never shipped.
pub const ENV_FILE: &str = ".env";

/// The `.env` of `dir`: `KEY=VALUE` (or `export KEY=VALUE`), values in double quotes (with
/// `\"`, `\\`, `\n`), single quotes or bare; comments and malformed lines skipped.
pub fn read_env(dir: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(dir.join(ENV_FILE)) else {
        return Vec::new();
    };
    text.lines().filter_map(env_line).collect()
}

fn env_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (k, v) = line.strip_prefix("export ").unwrap_or(line).split_once('=')?;
    let k = k.trim();
    if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let v = v.trim();
    let v = if let Some(inner) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match (c, c == '\\') {
                (_, true) => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some(other) => out.push(other),
                    None => {}
                },
                (c, false) => out.push(c),
            }
        }
        out
    } else if let Some(inner) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        inner.to_owned()
    } else {
        v.to_owned()
    };
    Some((k.to_owned(), v))
}

/// Set `values` in the `.env` of `dir` (the other keys kept; an empty value removes its key),
/// written atomically with owner-only rights.
pub fn write_env(dir: &Path, values: &[(String, String)]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut all = read_env(dir);
    for (k, v) in values {
        match all.iter_mut().find(|(a, _)| a == k) {
            Some(e) => e.1.clone_from(v),
            None => all.push((k.clone(), v.clone())),
        }
    }
    all.retain(|(_, v)| !v.is_empty());
    let mut text = String::from(tr("# Sensitive data of the module (new hera-nux): do not share.\n"));
    for (k, v) in &all {
        let v = v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        text.push_str(&format!("{k}=\"{v}\"\n"));
    }
    let tmp = dir.join(".env.tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(text.as_bytes())?;
    drop(f);
    std::fs::rename(tmp, dir.join(ENV_FILE))
}

impl Module {
    /// Path of its settings file, if it has one.
    pub fn settings_path(&self) -> Option<PathBuf> {
        self.settings.as_ref().map(|s| self.dir.join(&s.file))
    }
}

/// The app's data folder: `$XDG_DATA_HOME/hsm01` (by default `~/.local/share/hsm01`).
fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("hsm01")
}

/// The modules folder: `$XDG_DATA_HOME/hsm01/modules` (by default `~/.local/share/hsm01/modules`).
pub fn user_dir() -> PathBuf {
    data_dir().join("modules")
}

/// Description file of a module, and its former name (still read).
const MODULE_TOML: &str = "module.toml";
const OLD_TOML: &str = "tool.toml";

/// Former layout (`hsm01/tools/<id>/tool.toml`) moved to `hsm01/modules/<id>/module.toml`.
fn migrate(data: &Path) -> std::io::Result<()> {
    let (old, new) = (data.join("tools"), data.join("modules"));
    if old.is_dir() && !new.exists() {
        std::fs::rename(&old, &new)?;
        eprintln!("hsm01-app: {} → {}", old.display(), new.display());
    }
    for dir in std::fs::read_dir(&new).into_iter().flatten().flatten().map(|e| e.path()) {
        if dir.join(OLD_TOML).is_file() && !dir.join(MODULE_TOML).exists() {
            std::fs::rename(dir.join(OLD_TOML), dir.join(MODULE_TOML))?;
        }
    }
    Ok(())
}

/// Candidates next to the build (`app/target/release/hsm01-app` → `app/<rel>`).
fn near_build(rel: &str) -> Vec<PathBuf> {
    std::env::current_exe()
        .map(|exe| exe.ancestors().skip(1).take(4).map(|d| d.join(rel)).collect())
        .unwrap_or_default()
}

/// `app/<rel>` of the source tree the app was built from: found wherever the build went
/// (`CARGO_TARGET_DIR`, CI); a folder that is not there is skipped.
fn near_source(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
}

/// The modules shipped with the app: `$HSM01_SHIPPED_MODULES`, `app/modules` next to the build
/// or in the source tree it was built from, or `/usr/share/hsm01/modules`.
pub fn shipped_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::env::var_os("HSM01_SHIPPED_MODULES")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    candidates.extend(near_build("modules"));
    candidates.push(near_source("modules"));
    candidates.push("/usr/share/hsm01/modules".into());
    candidates.into_iter().find(|d| !scan(d).is_empty())
}

/// The Python library of the API (folder holding the `hsm01` package): `$HSM01_PYLIB`,
/// `app/clients/python` next to the build or in its source tree, or `/usr/share/hsm01/python`.
pub fn python_lib() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::env::var_os("HSM01_PYLIB")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    candidates.extend(near_build("clients/python"));
    candidates.push(near_source("clients/python"));
    candidates.push("/usr/share/hsm01/python".into());
    candidates
        .into_iter()
        .find(|d| d.join("hsm01/__init__.py").is_file())
}

/// The tools of the tools folder, by name.
pub fn modules() -> Vec<Module> {
    scan(&user_dir())
}

/// A fingerprint of the modules' descriptions (each folder's `module.toml` or `tool.toml` and its
/// modification time): it changes when one is edited, added or removed, and the window then
/// reads them again (a module updated in place shows its new settings form without a refresh).
pub fn descriptions_stamp() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(user_dir())
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    dirs.sort();
    // the language and the modules' translation files change their texts too
    crate::i18n::language().hash(&mut h);
    for d in dirs {
        let lang_file = d.join("i18n").join(format!("{}.toml", crate::i18n::language()));
        for file in ["module.toml", "tool.toml"].map(|f| d.join(f)).into_iter().chain([lang_file]) {
            if let Ok(t) = std::fs::metadata(&file).and_then(|m| m.modified()) {
                (&file, t).hash(&mut h);
            }
        }
    }
    h.finish()
}

/// The tools of another folder (tests of the window).
#[cfg(test)]
pub fn scan_dir(dir: &Path) -> Vec<Module> {
    scan(dir)
}

/// The sub-folders of `dir` that have a `module.toml` (or a former `tool.toml`).
fn scan(dir: &Path) -> Vec<Module> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for dir in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        let Some(id) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        let file = [MODULE_TOML, OLD_TOML].map(|f| dir.join(f)).into_iter().find(|f| f.is_file());
        let Some(text) = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()) else {
            continue;
        };
        match toml::from_str::<Module>(&text) {
            Ok(mut m) => {
                m.id = id;
                m.dir = dir;
                translate(&mut m, &crate::i18n::language());
                out.push(m);
            }
            Err(e) => eprintln!("hsm01-app: {} : {e}", file.unwrap_or_default().display()),
        }
    }
    out.sort_by_key(|m| m.name.to_lowercase());
    out
}

/// The texts of a module's `module.toml` (name, description, guide line by line, form labels,
/// helps, units, buttons, sensitive data) in the language in use, from the module's own
/// translation file: `i18n/<code>.toml` in its folder, same format as the app's (i18n.rs);
/// English too (`en.toml`: the wording shown, the module.toml's text when it has none).
fn translate(m: &mut Module, lang: &str) {
    let Some(cat) = crate::i18n::read_file(&m.dir.join("i18n"), lang) else {
        return;
    };
    let t = |s: &mut String| {
        if !s.is_empty() {
            *s = cat.get(s).to_owned();
        }
    };
    fn fields(fs: &mut [Field], t: &dyn Fn(&mut String)) {
        for f in fs {
            for s in [&mut f.label, &mut f.help, &mut f.unit, &mut f.add] {
                t(s);
            }
            f.choice_labels.iter_mut().for_each(t);
            fields(&mut f.fields, t);
        }
    }
    t(&mut m.name);
    t(&mut m.description);
    if let Some(s) = &mut m.settings {
        let guide: Vec<String> = s.guide.lines().map(|l| cat.get(l).to_owned()).collect();
        s.guide = guide.join("\n");
        fields(&mut s.fields, &t);
    }
    for s in &mut m.secrets {
        t(&mut s.label);
        t(&mut s.description);
    }
}

/// Copy the shipped tools into the tools folder: the missing ones, or all of them (replacing
/// those of the same id) with `overwrite`. Returns how many were copied.
pub fn install_shipped(overwrite: bool) -> std::io::Result<usize> {
    let from = shipped_dir().ok_or_else(|| std::io::Error::other(tr("shipped modules not found")))?;
    install_in(&from, &user_dir(), overwrite)
}

fn install_in(from: &Path, to: &Path, overwrite: bool) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut n = 0;
    for m in scan(from) {
        let target = to.join(&m.id);
        if target.exists() && !overwrite {
            continue;
        }
        copy_dir(&m.dir, &target)?;
        n += 1;
    }
    Ok(n)
}

/// List (in the tools folder) of the shipped tools already copied once.
const OFFERED: &str = ".livres";

/// Copy each shipped tool once: the new ones arrive with an update of the app, one removed from
/// the folder does not come back. A folder from before the list counts its tools as offered.
fn offer_shipped(from: &Path, to: &Path) -> std::io::Result<usize> {
    let list = to.join(OFFERED);
    let mut offered: Vec<String> = match std::fs::read_to_string(&list) {
        Ok(text) => text.lines().map(String::from).collect(),
        Err(_) => scan(to).into_iter().map(|m| m.id).collect(),
    };
    std::fs::create_dir_all(to)?;
    let mut n = 0;
    for m in scan(from) {
        if offered.contains(&m.id) {
            continue;
        }
        let target = to.join(&m.id);
        if !target.exists() {
            copy_dir(&m.dir, &target)?;
            n += 1;
        }
        offered.push(m.id);
    }
    std::fs::write(list, offered.join("\n") + "\n")?;
    Ok(n)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let name = e.file_name();
        // Sensitive data stays with whoever typed it (never shipped nor reinstalled over).
        if name == "__pycache__" || name == ENV_FILE {
            continue;
        }
        let (src, dst) = (e.path(), to.join(&name));
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// How to start a tool.
#[derive(Debug, PartialEq)]
struct Launch {
    prog: String,
    args: Vec<String>,
    /// Working directory: the tool's folder.
    cwd: Option<PathBuf>,
    /// Folder of the `hsm01` Python package, for `PYTHONPATH`.
    pylib: Option<PathBuf>,
}

/// The command line of a tool (its folder looked up in `tools`).
fn command_of(t: &Tool, tools: &Path) -> Result<Launch, String> {
    if !t.command.trim().is_empty() {
        return Ok(Launch {
            prog: "sh".into(),
            args: vec!["-c".into(), t.command.clone()],
            cwd: None,
            pylib: python_lib(),
        });
    }
    if t.module.is_empty() {
        return Err(tr("neither module nor command").into());
    }
    let m = scan(tools)
        .into_iter()
        .find(|m| m.id == t.module)
        .ok_or_else(|| tf!("folder \"{folder}\" not found in {dir}", folder = t.module, dir = tools.display()))?;
    launch_of(&m, &t.args, python_lib())
}

fn launch_of(m: &Module, args: &[String], pylib: Option<PathBuf>) -> Result<Launch, String> {
    // a relative path given in module.toml is from the tool's folder; a bare name is a program
    // of the PATH (node, python3…) unless such a file is in the folder
    let abs = |p: &str| {
        let path = PathBuf::from(p);
        let local = path.components().count() > 1 || m.dir.join(&path).exists();
        if path.is_relative() && local {
            m.dir.join(path).display().to_string()
        } else {
            p.to_owned()
        }
    };
    let (prog, mut a) = if let Some((first, rest)) = m.command.split_first() {
        (abs(first), rest.iter().map(|r| abs(r)).collect::<Vec<_>>())
    } else if !m.file.is_empty() {
        // PYTHONPATH gives the library: a plain `from hsm01 import Client`
        let file = m.dir.join(&m.file).display().to_string();
        ("python3".to_owned(), vec![file])
    } else {
        return Err(tf!("{module}: neither `file` nor `command` in module.toml", module = m.id));
    };
    a.extend(args.iter().cloned());
    Ok(Launch {
        prog,
        args: a,
        cwd: Some(m.dir.clone()),
        pylib,
    })
}

/// Last change of the code files of a tool's folder (sub-folders too, not `__pycache__`,
/// `node_modules` nor hidden ones).
fn code_stamp(dir: &Path, depth: u8) -> Option<SystemTime> {
    let mut last = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "__pycache__" || name == "node_modules" {
            continue;
        }
        let t = if path.is_dir() {
            if depth == 0 {
                continue;
            }
            code_stamp(&path, depth - 1)
        } else if path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| CODE.contains(&x))
        {
            e.metadata().and_then(|m| m.modified()).ok()
        } else {
            None
        };
        last = last.max(t);
    }
    last
}

/// A new tool from the template (a page, a knob, an alert), in a new folder of the tools folder
/// named after `name`; returns its id (the folder's name).
pub fn create_tool(name: &str) -> std::io::Result<String> {
    create_tool_in(&user_dir(), name)
}

fn create_tool_in(dir: &Path, name: &str) -> std::io::Result<String> {
    let words: Vec<String> = name
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let base = if words.is_empty() { "module".to_owned() } else { words.join("-") };
    let id = (1..)
        .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
        .find(|id| !dir.join(id).exists())
        .unwrap_or_default();
    let folder = dir.join(&id);
    std::fs::create_dir_all(&folder)?;
    let toml = TOOL_TOML.replace("{name}", &toml::Value::String(name.to_owned()).to_string());
    std::fs::write(folder.join(MODULE_TOML), toml)?;
    std::fs::write(folder.join("main.py"), TEMPLATE)?;
    std::fs::create_dir_all(folder.join("i18n"))?;
    std::fs::write(folder.join("i18n").join("en.toml"), TEMPLATE_EN)?;
    std::fs::write(folder.join("i18n").join("fr.toml"), TEMPLATE_FR)?;
    Ok(id)
}

/// `module.toml` of a new module (`{name}`: its name, quoted).
const TOOL_TOML: &str = r#"# Module of new hera-nux (docs/app/modules.md). A change in this folder restarts the module
# if it runs. Its texts in other languages: i18n/<code>.toml.
name = {name}
description = "My module: a page, knob 1 counts, button 1 sends an alert."
file = "main.py"
args = []
"#;

/// `main.py` of a new tool, and its English and French texts (`i18n/en.toml`, `fr.toml`).
const TEMPLATE: &str = include_str!("../../clients/python/examples/tool_template.py");
const TEMPLATE_EN: &str = include_str!("../../clients/python/examples/tool_template_en.toml");
const TEMPLATE_FR: &str = include_str!("../../clients/python/examples/tool_template_fr.toml");

struct Running {
    tool: Tool,
    child: Child,
    since: Instant,
    /// Folder and last change of its code when started (`None`: a command).
    code: Option<(PathBuf, Option<SystemTime>)>,
    /// Language given to it (`HSM01_LANG`): restarted in the new one when it changes.
    lang: String,
    /// Frozen because hidden (`pause_hidden`), and the last time it was on screen, woken up,
    /// not connected yet or without the option.
    paused: bool,
    seen: Instant,
}

/// « Load only when shown »: a module is frozen once no longer wanted for this long (a page
/// turned past it is not worth it; a message that woke it up gives it as long), and stopped
/// (unloaded) after this long.
#[cfg(not(test))]
const HIDDEN_GRACE: Duration = Duration::from_secs(3);
#[cfg(test)]
const HIDDEN_GRACE: Duration = Duration::from_millis(400);
#[cfg(not(test))]
const IDLE_UNLOAD: Duration = Duration::from_secs(30);
#[cfg(test)]
const IDLE_UNLOAD: Duration = Duration::from_millis(1500);

/// « Load only when shown » (`pause_hidden`): a module with the option is started only when
/// wanted (`Shared::wanted`: the screen needs it, an action or the window asks for it, supervise);
/// no longer wanted for `HIDDEN_GRACE`, it is frozen (SIGSTOP: no processor time) and runs again
/// (SIGCONT) as soon as it is wanted or a message is sent to it (a command, an input it grabbed);
/// for `IDLE_UNLOAD`, it is stopped. Only once connected: its pages exist, they can be shown.
/// Returns the modules stopped.
fn pause_hidden(shared: &Shared, running: &mut HashMap<String, Running>) -> Vec<String> {
    let options: Vec<String> = shared
        .config
        .read()
        .unwrap()
        .tools
        .iter()
        .filter(|t| t.enabled && t.pause_hidden)
        .map(|t| t.name.clone())
        .collect();
    let now = Instant::now();
    let mut idle = Vec::new();
    for (name, r) in running.iter_mut() {
        let option = options.contains(name);
        let woken = shared.api.take_woken(name);
        if !option || woken || shared.is_wanted(name) || !shared.api.connected(name) {
            r.seen = now;
        }
        if option && now.duration_since(r.seen) >= IDLE_UNLOAD {
            idle.push(name.clone());
            continue;
        }
        let pause = option && now.duration_since(r.seen) >= HIDDEN_GRACE;
        if pause != r.paused {
            // marked paused before it stops, running after it runs: a message sent meanwhile
            // wakes it up, a broadcast is kept for it
            if pause {
                shared.api.set_paused(name, true);
            }
            match freeze(&r.child, pause) {
                Ok(()) => {
                    r.paused = pause;
                    if !pause {
                        shared.api.set_paused(name, false);
                    }
                    status(shared, name, |s| s.paused = pause);
                }
                Err(e) => {
                    if pause {
                        shared.api.set_paused(name, false);
                    }
                    eprintln!("hsm01-app: {name}: {e}");
                }
            }
        }
    }
    for name in &idle {
        if let Some(mut r) = running.remove(name) {
            let _ = r.child.kill(); // frozen too
            let _ = r.child.wait();
            unpause(shared, name, &r);
            status(shared, name, |s| {
                s.running = false;
                s.pid = None;
                s.waiting = true;
            });
        }
    }
    idle
}

/// SIGSTOP (`on`) or SIGCONT to the process group of `child` (it and the programs it started:
/// `sh -c`…). Not reaped yet, its pid cannot be another program's.
fn freeze(child: &Child, on: bool) -> std::io::Result<()> {
    let sig = if on { libc::SIGSTOP } else { libc::SIGCONT };
    let Ok(pid) = libc::pid_t::try_from(child.id()) else {
        return Err(std::io::ErrorKind::InvalidInput.into());
    };
    // SAFETY: kill() only sends a signal; the group is the child's own (`process_group(0)`)
    if unsafe { libc::kill(-pid, sig) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// A tool that stops or is stopped: no longer paused (its next start runs).
fn unpause(shared: &Shared, name: &str, r: &Running) {
    if r.paused {
        shared.api.set_paused(name, false);
    }
    status(shared, name, |s| s.paused = false);
}

/// Watch the configured tools in a thread of its own until the app quits; first, the shipped
/// tools not offered yet are copied into the tools folder.
pub fn start(shared: Arc<Shared>) {
    // before the window reads the folder
    if let Err(e) = migrate(&data_dir()) {
        eprintln!("hsm01-app: {}", tf!("modules folder: {e}", e = e));
    }
    let dir = user_dir();
    match shipped_dir().map(|from| offer_shipped(&from, &dir)) {
        Some(Ok(n)) if n > 0 => eprintln!("hsm01-app: {}", tf!("{n} shipped module(s) added", n = n)),
        Some(Err(e)) => eprintln!("hsm01-app: {}", tf!("shipped modules: {e}", e = e)),
        _ => {}
    }
    let run = move || supervise(&shared, &dir);
    if let Err(e) = std::thread::Builder::new()
        .name("hsm01-tools".into())
        .spawn(run)
    {
        eprintln!("hsm01-app: modules: {e}");
    }
}

fn supervise(shared: &Arc<Shared>, tools: &Path) {
    let mut running: HashMap<String, Running> = HashMap::new();
    // next start allowed, delay before the one after (per tool name)
    let mut backoff: HashMap<String, (Instant, Duration)> = HashMap::new();
    // settings seen last and since when (a change applies once settled)
    let mut seen: HashMap<String, (Tool, Instant)> = HashMap::new();
    let mut code_check = Instant::now();
    while !shared.quit.load(Ordering::Relaxed) {
        let lang = shared.config.read().unwrap().language.clone();
        let wanted: Vec<Tool> = shared
            .config
            .read()
            .unwrap()
            .tools
            .iter()
            .filter(|t| t.enabled && !t.name.trim().is_empty())
            .cloned()
            .collect();
        let now = Instant::now();
        for t in &wanted {
            match seen.get(&t.name) {
                Some((s, _)) if s == t => {}
                _ => {
                    seen.insert(t.name.clone(), (t.clone(), now));
                }
            }
        }
        seen.retain(|name, _| wanted.iter().any(|t| &t.name == name));
        let settled = |t: &Tool| seen.get(&t.name).is_some_and(|(_, at)| at.elapsed() >= SETTLE);

        // stop what is no longer wanted, wanted differently, or whose code changed
        let check_code = code_check.elapsed() >= Duration::from_secs(1);
        if check_code {
            code_check = Instant::now();
        }
        let mut stale: Vec<(String, &str)> = Vec::new();
        for (name, r) in &running {
            match wanted.iter().find(|t| &t.name == name) {
                None => stale.push((name.clone(), tr("stopped"))),
                Some(t) if !t.same_launch(&r.tool) && settled(t) => stale.push((name.clone(), tr("stopped"))),
                _ => {
                    let changed = check_code
                        && r.code
                            .as_ref()
                            .is_some_and(|(dir, at)| code_stamp(dir, 3) > *at);
                    let asked = shared
                        .tools
                        .lock()
                        .unwrap()
                        .get_mut(name)
                        .is_some_and(|s| std::mem::take(&mut s.restart));
                    if changed {
                        stale.push((name.clone(), tr("changed: restarted")));
                    } else if r.lang != lang {
                        stale.push((name.clone(), tr("language changed: restarted")));
                    } else if asked {
                        stale.push((name.clone(), tr("sensitive data changed: restarted")));
                    }
                }
            }
        }
        for (name, why) in stale {
            if let Some(mut r) = running.remove(&name) {
                let _ = r.child.kill(); // frozen too
                let _ = r.child.wait();
                unpause(shared, &name, &r);
                status(shared, &name, |s| {
                    s.running = false;
                    s.pid = None;
                    s.last_exit = Some(why.into());
                });
                backoff.remove(&name);
            }
        }
        // note the ones that stopped by themselves
        let mut ended = Vec::new();
        for (name, r) in &mut running {
            if let Ok(Some(code)) = r.child.try_wait() {
                ended.push((name.clone(), code.to_string(), r.since.elapsed()));
            }
        }
        for (name, why, lived) in ended {
            if let Some(r) = running.remove(&name) {
                unpause(shared, &name, &r);
            }
            let delay = match backoff.get(&name) {
                Some((_, d)) if lived < Duration::from_secs(60) => {
                    (*d * 2).min(Duration::from_secs(30))
                }
                _ => Duration::from_secs(1),
            };
            backoff.insert(name.clone(), (Instant::now() + delay, delay));
            status(shared, &name, |s| {
                s.running = false;
                s.pid = None;
                s.last_exit = Some(why);
            });
        }
        // start the missing ones
        for t in &wanted {
            let ready = backoff
                .get(&t.name)
                .is_none_or(|(at, _)| Instant::now() >= *at);
            if running.contains_key(&t.name) || !ready || !settled(t) {
                continue;
            }
            // « Load only when shown »: started once the screen (or an action…) needs it
            let lazy = t.pause_hidden && !shared.is_wanted(&t.name);
            status(shared, &t.name, |s| s.waiting = lazy);
            if lazy {
                continue;
            }
            let folder = (t.command.trim().is_empty() && !t.module.is_empty())
                .then(|| tools.join(&t.module));
            let code = folder.map(|dir| {
                let at = code_stamp(&dir, 3);
                (dir, at)
            });
            match spawn(shared, t, tools, &lang) {
                Ok(child) => {
                    let pid = child.id();
                    let restarted = backoff.contains_key(&t.name);
                    status(shared, &t.name, |s| {
                        s.running = true;
                        s.pid = Some(pid);
                        s.restarts += u32::from(restarted);
                    });
                    running.insert(
                        t.name.clone(),
                        Running {
                            tool: t.clone(),
                            child,
                            since: Instant::now(),
                            code,
                            lang: lang.clone(),
                            paused: false,
                            seen: Instant::now(),
                        },
                    );
                }
                Err(e) => {
                    // missing folder, program not found… tried again later
                    let delay = Duration::from_secs(10);
                    backoff.insert(t.name.clone(), (Instant::now() + delay, delay));
                    status(shared, &t.name, |s| s.last_exit = Some(e));
                }
            }
        }
        // statuses of removed tools go
        shared.tools.lock().unwrap().retain(|name, _| {
            running.contains_key(name)
                || shared.config.read().unwrap().tools.iter().any(|t| &t.name == name)
        });
        // a paused module runs again within 50 ms of being shown
        for _ in 0..5 {
            for name in pause_hidden(shared, &mut running) {
                backoff.remove(&name); // unloaded, not a failure: no restart delay or count
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    for (_, mut r) in running {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
}

fn status(shared: &Shared, name: &str, f: impl FnOnce(&mut ToolStatus)) {
    f(shared.tools.lock().unwrap().entry(name.to_owned()).or_default());
}

/// Start tool `t`, its output going to its log.
fn spawn(shared: &Arc<Shared>, t: &Tool, tools: &Path, lang: &str) -> Result<Child, String> {
    let launch = command_of(t, tools)?;
    let prog = launch.prog.clone();
    let mut cmd = Command::new(&launch.prog);
    // a group of its own: frozen with the programs it starts (« Load only when shown »); if the
    // app dies while it is frozen, the system ends it (SIGHUP to an orphaned stopped group)
    cmd.args(&launch.args).process_group(0);
    if let Some(dir) = &launch.cwd {
        // its sensitive data first: the app's own variables below win over it
        cmd.current_dir(dir).envs(read_env(dir)).env("HSM01_MODULE_DIR", dir);
    }
    if let Some(lib) = &launch.pylib {
        let mut path = lib.as_os_str().to_owned();
        if let Some(old) = std::env::var_os("PYTHONPATH").filter(|p| !p.is_empty()) {
            path.push(":");
            path.push(old);
        }
        cmd.env("PYTHONPATH", path).env("HSM01_PYLIB", lib);
    }
    let mut child = cmd
        .env("HSM01_API_SOCKET", crate::api::socket_path())
        .env("HSM01_MODULE", &t.name)
        .env("HSM01_TOOL", &t.name) // former name, for older modules
        .env("HSM01_LANG", lang)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{prog}: {e}"))?;
    let pipes: [Option<Box<dyn std::io::Read + Send>>; 2] = [
        child.stdout.take().map(|p| Box::new(p) as _),
        child.stderr.take().map(|p| Box::new(p) as _),
    ];
    for pipe in pipes.into_iter().flatten() {
        let (shared, name) = (shared.clone(), t.name.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                status(&shared, &name, |s| {
                    if s.log.len() >= LOG_LINES {
                        s.log.pop_front();
                    }
                    s.log.push_back(line);
                });
            }
        });
    }
    Ok(child)
}

/// Statuses of the tools, for the window: name → status.
pub type Statuses = Mutex<HashMap<String, ToolStatus>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hsm01-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// A module's `module.toml` in the language in use, from its `i18n/<code>.toml`: French,
    /// English as written or reworded by its `en.toml`, a language without a file in English.
    #[test]
    fn module_texts_translated() {
        let dir = std::env::temp_dir().join(format!("hsm01-i18n-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("i18n")).unwrap();
        std::fs::write(
            dir.join("i18n/fr.toml"),
            "name = \"Français\"\n[text]\n\"My module\" = \"Mon module\"\n\"Step\" = \"Pas\"\n\
             \"First line\" = \"Première ligne\"\n\"Token\" = \"Jeton\"\n",
        )
        .unwrap();
        let text = "name = \"My module\"\nfile = \"main.py\"\n[settings]\nfile = \"s.toml\"\n\
                    guide = \"First line\\nSecond line\"\n[[settings.field]]\nkey = \"step\"\n\
                    label = \"Step\"\nkind = \"number\"\n[[secret]]\nname = \"T\"\nlabel = \"Token\"\n";
        let read = |lang: &str| {
            let mut m: Module = toml::from_str(text).unwrap();
            m.dir = dir.clone();
            translate(&mut m, lang);
            m
        };
        let fr = read("fr");
        let s = fr.settings.as_ref().unwrap();
        assert_eq!((fr.name.as_str(), s.fields[0].label.as_str()), ("Mon module", "Pas"));
        assert_eq!(s.guide, "Première ligne\nSecond line", "line by line, untranslated kept");
        assert_eq!(fr.secrets[0].label, "Jeton");
        assert_eq!(read("en").name, "My module");
        assert_eq!(read("de").name, "My module", "no file: English");
        std::fs::write(dir.join("i18n/en.toml"), "name = \"English\"\n[text]\n\"My module\" = \"My counter\"\n")
            .unwrap();
        assert_eq!(read("en").name, "My counter", "en.toml rewords the English");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shipped tools are folders with a `module.toml` and their program; they are copied into
    /// the tools folder (missing ones, or all with overwrite).
    #[test]
    fn shipped_tools_are_installed() {
        let shipped = shipped_dir().expect("app/modules");
        let list = scan(&shipped);
        let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        for id in ["media", "system", "deck"] {
            assert!(ids.contains(&id), "{ids:?}");
        }
        for m in &list {
            assert!(m.dir.join(&m.file).is_file(), "{}: {}", m.id, m.file);
        }
        assert!(python_lib().is_some_and(|l| l.join("hsm01").is_dir()));
        let to = temp("install");
        assert_eq!(install_in(&shipped, &to, false).unwrap(), list.len());
        std::fs::write(to.join("media/main.py"), "# mine").unwrap();
        assert_eq!(install_in(&shipped, &to, false).unwrap(), 0, "kept");
        assert_eq!(std::fs::read_to_string(to.join("media/main.py")).unwrap(), "# mine");
        install_in(&shipped, &to, true).unwrap();
        assert_ne!(std::fs::read_to_string(to.join("media/main.py")).unwrap(), "# mine");
        let _ = std::fs::remove_dir_all(&to);
    }

    /// The former layout (`tools/<id>/tool.toml`) becomes `modules/<id>/module.toml`; a
    /// `modules` folder already there is left as it is.
    #[test]
    fn former_tools_folder_moved() {
        let data = temp("migrate");
        std::fs::create_dir_all(data.join("tools/minuteur")).unwrap();
        std::fs::write(data.join("tools/minuteur/tool.toml"), "name = \"Minuteur\"\n").unwrap();
        migrate(&data).unwrap();
        assert!(!data.join("tools").exists());
        assert!(data.join("modules/minuteur/module.toml").is_file());
        assert_eq!(scan(&data.join("modules"))[0].name, "Minuteur");
        std::fs::create_dir_all(data.join("tools/autre")).unwrap();
        migrate(&data).unwrap();
        assert!(data.join("tools/autre").is_dir(), "not merged into an existing folder");
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Each shipped tool is copied once: a new one comes with an update, a removed one does not
    /// come back; a folder from before the list keeps what it has.
    #[test]
    fn shipped_tools_are_offered_once() {
        let shipped = shipped_dir().expect("app/modules");
        let n = scan(&shipped).len();
        let to = temp("offer");
        assert_eq!(offer_shipped(&shipped, &to).unwrap(), n, "first start: all of them");
        std::fs::remove_dir_all(to.join("deck")).unwrap();
        assert_eq!(offer_shipped(&shipped, &to).unwrap(), 0, "removed: stays removed");
        assert!(!to.join("deck").exists());
        // a folder from before the list, without the newest tool: it arrives, the rest is kept
        std::fs::remove_file(to.join(OFFERED)).unwrap();
        std::fs::remove_dir_all(to.join("system")).unwrap();
        std::fs::write(to.join("media/main.py"), "# mine").unwrap();
        assert_eq!(offer_shipped(&shipped, &to).unwrap(), 2, "deck and system: not listed");
        assert_eq!(std::fs::read_to_string(to.join("media/main.py")).unwrap(), "# mine");
        let _ = std::fs::remove_dir_all(&to);
    }

    /// Tools are the sub-folders with a `module.toml`, started from their folder (Python with the
    /// library in PYTHONPATH, or any command); a new one from the template.
    #[test]
    fn tools_from_their_folders() {
        let dir = temp("tools");
        let id = create_tool_in(&dir, "Mon Minuteur !").unwrap();
        assert_eq!(id, "mon-minuteur");
        assert_eq!(create_tool_in(&dir, "Mon Minuteur !").unwrap(), "mon-minuteur-2");
        std::fs::create_dir_all(dir.join("js")).unwrap();
        std::fs::write(
            dir.join("js/tool.toml"), // the former name: still read
            "name = \"Node\"\ncommand = [\"node\", \"index.js\", \"--x\"]\n",
        )
        .unwrap();
        std::fs::write(dir.join("js/index.js"), "").unwrap();
        std::fs::create_dir_all(dir.join("not-a-tool")).unwrap();
        let all = scan(&dir);
        assert_eq!(all.len(), 3, "folders without module.toml are not tools");
        let mine = all.iter().find(|m| m.id == "mon-minuteur").unwrap();
        assert_eq!(mine.name, "Mon Minuteur !");
        let l = launch_of(mine, &["--y".into()], Some("/lib".into())).unwrap();
        let main = dir.join("mon-minuteur/main.py").display().to_string();
        assert_eq!((l.prog.as_str(), l.args.clone()), ("python3", vec![main, "--y".into()]));
        assert_eq!(l.cwd.as_deref(), Some(dir.join("mon-minuteur").as_path()));
        let js = all.iter().find(|m| m.id == "js").unwrap();
        let l = launch_of(js, &[], None).unwrap();
        let file = dir.join("js/index.js").display().to_string();
        assert_eq!((l.prog.as_str(), l.args), ("node", vec![file, "--x".to_owned()]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A command tool runs, its output is kept, it is restarted when it stops and stopped when
    /// switched off.
    #[test]
    fn tools_run_restart_and_stop() {
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        shared.config.write().unwrap().tools.push(Tool {
            name: "echo".into(),
            enabled: true,
            module: String::new(),
            args: Vec::new(),
            command: "echo \"bonjour $HSM01_MODULE\"; sleep 0.3".into(),
            keep_awake: false,
            pause_hidden: false,
        });
        let s = shared.clone();
        let sup = std::thread::spawn(move || supervise(&s, Path::new("/nonexistent")));
        let wait = |what: &str, ok: &dyn Fn(&ToolStatus) -> bool| {
            let t = Instant::now();
            loop {
                if shared.tools.lock().unwrap().get("echo").is_some_and(ok) {
                    return;
                }
                assert!(t.elapsed() < Duration::from_secs(10), "{what}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait("started", &|st| st.running);
        wait("output kept", &|st| st.log.iter().any(|l| l == "bonjour echo"));
        wait("restarted", &|st| st.restarts >= 1);
        shared.config.write().unwrap().tools[0].enabled = false;
        wait("stopped", &|st| !st.running);
        shared.quit.store(true, Ordering::Relaxed);
        sup.join().unwrap();
    }

    /// « Load only when shown »: not started until wanted; a connected module no longer wanted is
    /// frozen after a moment (the broadcasts and the others' inputs skip it), runs again when
    /// wanted (with the last broadcast it missed) or when a message is sent to it, and is stopped
    /// after a longer while; the option changes without a restart.
    #[test]
    fn modules_load_only_when_shown() {
        use serde_json::json;
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        shared.config.write().unwrap().tools.push(Tool {
            name: "dormeur".into(),
            enabled: true,
            module: String::new(),
            args: Vec::new(),
            command: "sleep 60".into(),
            keep_awake: false,
            pause_hidden: true,
        });
        // its connection to the API, as a module says hello
        let (id, _tx, rx) = crate::api::register(&shared);
        crate::api::process(&shared, id, r#"{"type": "hello", "name": "dormeur"}"#);
        let s = shared.clone();
        let sup = std::thread::spawn(move || supervise(&s, Path::new("/nonexistent")));
        let status = || shared.tools.lock().unwrap().get("dormeur").cloned().unwrap_or_default();
        let wait = |what: &str, ok: &dyn Fn(&ToolStatus) -> bool| {
            let t = Instant::now();
            while !ok(&status()) {
                assert!(t.elapsed() < Duration::from_secs(10), "{what}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        // `T`: stopped (/proc/<pid>/stat, the field after the name)
        let frozen = |pid: u32| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
            stat.rsplit_once(") ").unwrap().1.starts_with('T')
        };
        wait("not loaded", &|st| st.waiting);
        std::thread::sleep(Duration::from_millis(400));
        assert!(!status().running, "not started while nothing wants it");
        shared.want("dormeur", Duration::from_secs(1));
        wait("wanted: started", &|st| st.running && !st.waiting);
        let pid = status().pid.unwrap();
        wait("paused, no longer wanted", &|st| st.paused);
        assert!(frozen(pid));
        shared.api.broadcast(&json!({"event": "standby", "asleep": true}));
        shared.api.emit(None, json!({"event": "turn", "knob": 1, "delta": 1}));
        assert!(rx.try_recv().is_err(), "nothing for a paused module");

        shared.want("dormeur", Duration::from_secs(30));
        wait("wanted again: runs", &|st| !st.paused);
        assert!(!frozen(pid));
        let missed: Vec<serde_json::Value> = rx.try_iter().map(|l| serde_json::from_str(&l).unwrap()).collect();
        assert_eq!(missed, [json!({"event": "standby", "asleep": true})], "the missed broadcast, not the input");

        shared.wanted.lock().unwrap().clear();
        wait("hidden again", &|st| st.paused);
        shared.api.send(id, &json!({"event": "command", "command": "x"}));
        wait("woken up by a message", &|st| !st.paused);
        wait("paused after it", &|st| st.paused);

        shared.config.write().unwrap().tools[0].pause_hidden = false;
        wait("option off: runs", &|st| !st.paused);
        assert!(!frozen(pid));
        std::thread::sleep(SETTLE + Duration::from_millis(300));
        assert_eq!((status().pid, status().restarts), (Some(pid), 0), "not restarted");

        shared.config.write().unwrap().tools[0].pause_hidden = true;
        wait("option on: paused", &|st| st.paused);
        wait("unloaded", &|st| !st.running && st.waiting && st.pid.is_none());
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "its process ended");
        std::thread::sleep(Duration::from_millis(400));
        assert!(!status().running && status().last_exit.is_none(), "not restarted, not a failure");
        shared.quit.store(true, Ordering::Relaxed);
        sup.join().unwrap();
    }

    /// `.env`: written owner-only with quotes and escapes, read back the same (also lines written
    /// by hand); the other keys kept, an empty value removes its key.
    #[test]
    fn env_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("env");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(ENV_FILE),
            "# mine\nexport A=1\nB='deux mots'\nC=\"x \\\"y\\\" \\\\ z\"\nnot a line\n=x\nD=\n",
        )
        .unwrap();
        let env = read_env(&dir);
        let get = |env: &[(String, String)], k: &str| env.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        assert_eq!(get(&env, "A").as_deref(), Some("1"));
        assert_eq!(get(&env, "B").as_deref(), Some("deux mots"));
        assert_eq!(get(&env, "C").as_deref(), Some("x \"y\" \\ z"));
        assert_eq!(env.len(), 4, "{env:?}");
        let secret = "s3cr\"et\\\nligne 2".to_string();
        write_env(&dir, &[("TOKEN".into(), secret.clone()), ("A".into(), String::new())]).unwrap();
        let env = read_env(&dir);
        assert_eq!(get(&env, "TOKEN"), Some(secret));
        assert_eq!(get(&env, "A"), None, "emptied: removed");
        assert_eq!(get(&env, "B").as_deref(), Some("deux mots"), "kept");
        let mode = std::fs::metadata(dir.join(ENV_FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A module gets its `.env` as environment variables; writing it does not restart it, the
    /// window's request does (and it starts with the new values).
    #[test]
    fn modules_get_their_secrets() {
        let dir = temp("secrets");
        std::fs::create_dir_all(dir.join("t")).unwrap();
        std::fs::write(dir.join("t/module.toml"), "name = \"T\"\ncommand = [\"sh\", \"run.sh\"]\n").unwrap();
        std::fs::write(dir.join("t/run.sh"), "echo \"jeton=$MON_JETON\"; sleep 30\n").unwrap();
        write_env(&dir.join("t"), &[("MON_JETON".into(), "abc".into())]).unwrap();
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        shared.config.write().unwrap().tools.push(Tool {
            name: "T".into(),
            enabled: true,
            module: "t".into(),
            args: Vec::new(),
            command: String::new(),
            keep_awake: false,
            pause_hidden: false,
        });
        let (s, d) = (shared.clone(), dir.clone());
        let sup = std::thread::spawn(move || supervise(&s, &d));
        let wait = |what: &str, ok: &dyn Fn(&ToolStatus) -> bool| {
            let t = Instant::now();
            while !shared.tools.lock().unwrap().get("T").is_some_and(ok) {
                assert!(t.elapsed() < Duration::from_secs(10), "{what}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait("its secret", &|st| st.log.iter().any(|l| l == "jeton=abc"));
        std::thread::sleep(Duration::from_millis(1100)); // a later modification time
        write_env(&dir.join("t"), &[("MON_JETON".into(), "def".into())]).unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(shared.tools.lock().unwrap()["T"].restarts, 0, ".env: no restart by itself");
        shared.tools.lock().unwrap().get_mut("T").unwrap().restart = true;
        wait("the new one", &|st| st.running && st.log.iter().any(|l| l == "jeton=def"));
        shared.quit.store(true, Ordering::Relaxed);
        sup.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A tool of the folder is restarted when one of its code files changes.
    #[test]
    fn a_changed_tool_is_restarted() {
        let dir = temp("reload");
        std::fs::create_dir_all(dir.join("t")).unwrap();
        let write = |v: &str| {
            let toml = "name = \"T\"\ncommand = [\"sh\", \"run.sh\"]\n";
            std::fs::write(dir.join("t/module.toml"), toml).unwrap();
            std::fs::write(dir.join("t/run.sh"), format!("echo {v}; sleep 30\n")).unwrap();
        };
        write("v1");
        let shared = Arc::new(Shared::new(crate::config::Config::default()));
        shared.config.write().unwrap().tools.push(Tool {
            name: "T".into(),
            enabled: true,
            module: "t".into(),
            args: Vec::new(),
            command: String::new(),
            keep_awake: false,
            pause_hidden: false,
        });
        let (s, d) = (shared.clone(), dir.clone());
        let sup = std::thread::spawn(move || supervise(&s, &d));
        let wait = |what: &str, ok: &dyn Fn(&ToolStatus) -> bool| {
            let t = Instant::now();
            loop {
                if shared.tools.lock().unwrap().get("T").is_some_and(ok) {
                    return;
                }
                assert!(t.elapsed() < Duration::from_secs(10), "{what}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait("v1 runs", &|st| st.log.iter().any(|l| l == "v1"));
        std::thread::sleep(Duration::from_millis(1100)); // a later modification time
        write("v2");
        wait("v2 runs", &|st| st.running && st.log.iter().any(|l| l == "v2"));
        shared.quit.store(true, Ordering::Relaxed);
        sup.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
