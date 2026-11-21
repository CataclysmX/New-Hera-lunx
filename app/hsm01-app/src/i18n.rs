//! Translations. The texts in the code are in English, the default language; a translation file
//! `<code>.toml` gives them in another language:
//!
//! ```toml
//! name = "Français"                 # the language in its own words (« Language » list)
//!
//! [text]
//! "Save" = "Enregistrer"
//! "{n} modules reinstalled" = "{n} modules réinstallés"   # same {placeholders}
//! ```
//!
//! Built in: `i18n/fr.toml` (next to `Cargo.toml`). A file `~/.config/hsm01/i18n/<code>.toml`
//! adds a language, or replaces entries of a built-in one. A text without a translation stays
//! in English. Modules have their own files, same format (`i18n/<code>.toml` in their folder):
//! the app translates their `module.toml` with it and gives them the language (`HSM01_LANG`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use serde::Deserialize;

/// The language of the code, used when no other is chosen.
pub const DEFAULT: &str = "en";

/// Translation files built into the app: (code, TOML).
const BUILT_IN: &[(&str, &str)] = &[("fr", include_str!("../i18n/fr.toml"))];

/// A language: its name and its texts (English → translation).
#[derive(Debug, Default)]
pub struct Catalog {
    pub name: String,
    text: HashMap<String, String>,
}

impl Catalog {
    /// `text` translated, or as it is.
    pub fn get<'a>(&'a self, text: &'a str) -> &'a str {
        self.text.get(text).map_or(text, String::as_str)
    }
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    name: String,
    #[serde(default)]
    text: HashMap<String, String>,
}

/// A translation file's text read.
pub fn parse(toml_text: &str) -> Result<Catalog, String> {
    let f: File = toml::from_str(toml_text).map_err(|e| e.to_string())?;
    Ok(Catalog { name: f.name, text: f.text })
}

/// A language code usable as a file name (`fr`, `pt_BR`, `zh-Hant`).
pub fn valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 16
        && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Folder of the user's translation files.
pub fn user_dir() -> PathBuf {
    let config = crate::config::Config::path();
    config.parent().map_or_else(|| PathBuf::from("i18n"), |d| d.join("i18n"))
}

/// Translation file `<code>.toml` of folder `dir`, if it is there and reads.
pub fn read_file(dir: &Path, code: &str) -> Option<Catalog> {
    if !valid_code(code) {
        return None;
    }
    let path = dir.join(format!("{code}.toml"));
    let text = std::fs::read_to_string(&path).ok()?;
    parse(&text)
        .map_err(|e| eprintln!("hsm01-app: {}: {e}", path.display()))
        .ok()
}

/// Language `code` of the app: built in, completed or replaced by the user's file.
fn load(code: &str) -> Option<Catalog> {
    let built = BUILT_IN
        .iter()
        .find(|(c, _)| *c == code)
        .and_then(|(_, t)| parse(t).map_err(|e| eprintln!("hsm01-app: i18n/{code}.toml: {e}")).ok());
    let user = read_file(&user_dir(), code);
    match (built, user) {
        (None, None) => None,
        (Some(c), None) | (None, Some(c)) => Some(c),
        (Some(mut b), Some(u)) => {
            b.text.extend(u.text);
            if !u.name.is_empty() {
                b.name = u.name;
            }
            Some(b)
        }
    }
}

struct Current {
    code: String,
    catalog: Option<&'static Catalog>,
}

static CURRENT: RwLock<Current> = RwLock::new(Current { code: String::new(), catalog: None });
/// Languages loaded once (a switch back and forth does not read the files again).
static LOADED: Mutex<Vec<(String, &'static Catalog)>> = Mutex::new(Vec::new());

/// Use language `code` from now on (`en`, or one with a translation file; otherwise English).
pub fn set_language(code: &str) {
    let code = if code.is_empty() { DEFAULT } else { code };
    if CURRENT.read().unwrap().code == code {
        return;
    }
    let catalog = if code == DEFAULT {
        None
    } else {
        let mut loaded = LOADED.lock().unwrap();
        match loaded.iter().find(|(c, _)| c == code) {
            Some((_, cat)) => Some(*cat),
            None => match load(code) {
                Some(cat) => {
                    let cat: &'static Catalog = Box::leak(Box::new(cat));
                    loaded.push((code.to_owned(), cat));
                    Some(cat)
                }
                None => {
                    eprintln!("hsm01-app: language « {code} »: no translation file, English used");
                    None
                }
            },
        }
    };
    *CURRENT.write().unwrap() = Current { code: code.to_owned(), catalog };
}

/// The language in use (`en` by default).
pub fn language() -> String {
    let code = CURRENT.read().unwrap().code.clone();
    if code.is_empty() { DEFAULT.to_owned() } else { code }
}

/// `text` (English, from the code) in the language in use.
pub fn tr(text: &'static str) -> &'static str {
    match CURRENT.read().unwrap().catalog {
        Some(c) => c.get(text),
        None => text,
    }
}

/// Marks a text of a `const` to translate where it is shown (with `tr`).
pub const fn tx(text: &'static str) -> &'static str {
    text
}

/// `template` with each `{name}` replaced by its value.
pub fn fill(template: &str, args: &[(&str, String)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// `tf!("{n} pages", n = count)`: the text translated, then its `{placeholders}` filled (values
/// formatted with `Display`; format them first for a precision, `x = format!("{v:.1}")`).
#[macro_export]
macro_rules! tf {
    ($text:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($text), &[$((stringify!($name), ($value).to_string())),*])
    };
}

/// Languages offered: English, those built in, those of the user's folder: (code, name).
pub fn available() -> Vec<(String, String)> {
    let mut out = vec![(DEFAULT.to_owned(), "English".to_owned())];
    let mut add = |code: &str, name: String| {
        if !out.iter().any(|(c, _)| c == code) {
            let name = if name.is_empty() { code.to_owned() } else { name };
            out.push((code.to_owned(), name));
        }
    };
    for (code, text) in BUILT_IN {
        add(code, parse(text).map(|c| c.name).unwrap_or_default());
    }
    if let Ok(dir) = std::fs::read_dir(user_dir()) {
        let mut files: Vec<_> = dir.flatten().map(|e| e.path()).collect();
        files.sort();
        for p in files {
            if p.extension().is_some_and(|e| e == "toml")
                && let Some(code) = p.file_stem().and_then(|s| s.to_str())
                && let Some(cat) = read_file(&user_dir(), code)
            {
                add(code, cat.name);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `tr("…")`, `tx("…")` and `tf!("…"` texts of the sources (Rust string literals, `\` line
    /// continuations included).
    fn texts_in_sources() -> Vec<(String, String)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![];
        for d in [dir.clone(), dir.join("bin")] {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                if e.path().extension().is_some_and(|x| x == "rs") {
                    files.push(e.path());
                }
            }
        }
        let mut out = vec![];
        for f in files {
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            if name == "i18n.rs" {
                continue; // its examples and this scanner
            }
            let src = std::fs::read_to_string(&f).unwrap();
            for marker in ["tr(", "tx(", "tf!("] {
                let mut from = 0;
                while let Some(k) = src[from..].find(marker) {
                    let at = from + k;
                    from = at + marker.len();
                    let prev = src[..at].chars().next_back().unwrap_or(' ');
                    if prev.is_alphanumeric() || prev == '_' {
                        continue;
                    }
                    let rest = src[from..].trim_start();
                    if !rest.starts_with('"') {
                        continue;
                    }
                    out.push((name.clone(), literal(&rest[1..])));
                }
            }
        }
        out
    }

    /// The value of a string literal whose opening quote is just before `s`.
    fn literal(s: &str) -> String {
        let (mut out, mut it) = (String::new(), s.chars().peekable());
        while let Some(c) = it.next() {
            match c {
                '"' => break,
                '\\' => match it.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('\n') => {
                        while it.peek().is_some_and(|c| c.is_whitespace()) {
                            it.next();
                        }
                    }
                    Some('u') => {
                        let hex: String = it.by_ref().skip(1).take_while(|c| *c != '}').collect();
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                    }
                    Some(c) => out.push(c),
                    None => break,
                },
                c => out.push(c),
            }
        }
        out
    }

    fn placeholders(s: &str) -> Vec<String> {
        let mut v: Vec<String> = s
            .split('{')
            .skip(1)
            .filter_map(|p| p.split_once('}').map(|(n, _)| n.to_owned()))
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_'))
            .collect();
        v.sort();
        v
    }

    /// Every text of the code has its French translation, with the same placeholders, and the
    /// French file has no text the code no longer uses.
    #[test]
    fn french_is_complete() {
        let fr = parse(BUILT_IN[0].1).unwrap();
        assert_eq!(fr.name, "Français");
        let used = texts_in_sources();
        let mut missing = vec![];
        for (file, t) in &used {
            match fr.text.get(t) {
                None => missing.push(format!("{file}: {t:?}")),
                Some(f) => assert_eq!(placeholders(t), placeholders(f), "{t:?} → {f:?}"),
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "sans traduction :\n{}", missing.join("\n"));
        let mut unused: Vec<_> = fr.text.keys().filter(|k| !used.iter().any(|(_, t)| t == *k)).collect();
        unused.sort();
        assert!(unused.is_empty(), "plus utilisés : {unused:?}");
    }

    #[test]
    fn translate_and_fill() {
        let cat = parse("name = \"Test\"\n[text]\n\"{n} pages\" = \"{n} pages (fr)\"\n").unwrap();
        assert_eq!(cat.get("{n} pages"), "{n} pages (fr)");
        assert_eq!(cat.get("other"), "other");
        assert_eq!(fill("{n} pages, {n}", &[("n", "3".into())]), "3 pages, 3");
        assert!(valid_code("pt_BR") && !valid_code("../x") && !valid_code(""));
    }
}
