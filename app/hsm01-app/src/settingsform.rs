//! Settings form of a module (`editor = "formulaire"` in its `module.toml`): its `[[settings.field]]`
//! shown as controls (numbers, texts, switches, choices, lists of items) over the values of its
//! settings file, written back keeping the file's comments. A `choice` field's values: its
//! `choices`, or the lists the running module gives (`source`, API `choices`), `{key}` in the
//! source being the value of a sibling field (the channels of the server chosen…).

use std::collections::HashMap;

use eframe::egui;
use egui::{Button, DragValue, RichText, TextEdit, Ui};

use crate::i18n::tr;
use crate::tools::Field;

/// Width of the labels column.
const LABEL_W: f32 = 190.0;

/// The value at dotted `key` ("alertes.gpu.seuil") of `t`.
pub fn get<'a>(t: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let mut parts = key.split('.');
    let mut v = t.get(parts.next()?)?;
    for p in parts {
        v = v.as_table()?.get(p)?;
    }
    Some(v)
}

/// Set (or with None remove) the value at dotted `key` of `t`, making the tables on the way.
pub fn set(t: &mut toml::Table, key: &str, v: Option<toml::Value>) {
    let (path, last) = match key.rsplit_once('.') {
        Some((path, last)) => (Some(path), last),
        None => (None, key),
    };
    let mut table = t;
    for p in path.into_iter().flat_map(|p| p.split('.')) {
        let entry = table.entry(p).or_insert_with(|| toml::Value::Table(Default::default()));
        if !entry.is_table() {
            *entry = toml::Value::Table(Default::default());
        }
        table = entry.as_table_mut().expect("a table");
    }
    match v {
        Some(v) => {
            table.insert(last.to_owned(), v);
        }
        None => {
            table.remove(last);
        }
    }
}

/// `values` as the text of the settings file, keeping the comments and order of `old`.
pub fn to_toml(values: &toml::Table, old: &str) -> Result<String, String> {
    let text = toml::to_string_pretty(values).map_err(|e| e.to_string())?;
    let fresh: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let merged = old.parse::<toml_edit::DocumentMut>().ok().map(|mut doc| {
        crate::config::keep::table(doc.as_table_mut(), fresh.as_table());
        doc.to_string()
    });
    let reads_back = |t: &String| toml::from_str::<toml::Table>(t).ok().as_ref() == Some(values);
    Ok(merged.filter(reads_back).unwrap_or(text))
}

/// The guide of the form: one paragraph per line, its addresses as links.
pub fn guide(ui: &mut Ui, text: &str) {
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for word in line.split(' ') {
                let url = word.trim_end_matches([',', '.', ')', ';']);
                if url.starts_with("https://") || url.starts_with("http://") {
                    ui.hyperlink(url);
                } else {
                    ui.label(word);
                }
            }
        });
    }
    ui.add_space(6.0);
}

/// The fields over `values` (the file's table, or an item of a list); `choices`: the lists the
/// running module gave. Returns whether a value changed.
pub fn form(
    ui: &mut Ui,
    salt: &str,
    fields: &[Field],
    values: &mut toml::Table,
    choices: &HashMap<String, Vec<String>>,
) -> bool {
    let mut changed = false;
    for (i, f) in fields.iter().enumerate() {
        ui.push_id((salt, i, &f.key), |ui| {
            changed |= match f.kind.as_str() {
                "section" => {
                    ui.add_space(8.0);
                    ui.label(RichText::new(&f.label).strong().size(15.0));
                    if !f.help.is_empty() {
                        ui.weak(&f.help);
                    }
                    false
                }
                "list" => list(ui, f, values, choices),
                _ => ui
                    .horizontal(|ui| {
                        let label = if f.label.is_empty() { f.key.as_str() } else { &f.label };
                        let r = ui.add_sized([LABEL_W, 20.0], egui::Label::new(label).truncate());
                        if !f.help.is_empty() {
                            r.on_hover_text(&f.help);
                        }
                        control(ui, f, values, choices)
                    })
                    .inner,
            };
        });
    }
    changed
}

/// The control of one field (number, text, switch, choice).
fn control(
    ui: &mut Ui,
    f: &Field,
    values: &mut toml::Table,
    choices: &HashMap<String, Vec<String>>,
) -> bool {
    let current = get(values, &f.key).cloned();
    let shown = current.clone().or_else(|| f.default.clone());
    let new = match f.kind.as_str() {
        "number" => number(ui, f, current.as_ref(), shown.as_ref()),
        "bool" => {
            let mut on = shown.as_ref().and_then(toml::Value::as_bool).unwrap_or(false);
            ui.checkbox(&mut on, "").changed().then_some(Some(toml::Value::Boolean(on)))
        }
        "choice" => choice(ui, f, values, shown.as_ref(), choices),
        _ => {
            let mut text = shown.as_ref().and_then(toml::Value::as_str).unwrap_or("").to_owned();
            let hint = f.default.as_ref().and_then(toml::Value::as_str).unwrap_or("");
            ui.add(TextEdit::singleline(&mut text).hint_text(hint).desired_width(260.0))
                .changed()
                .then(|| (!text.trim().is_empty()).then(|| toml::Value::String(text.trim().to_owned())))
        }
    };
    if !f.unit.is_empty() && f.kind != "number" {
        ui.weak(&f.unit);
    }
    match new {
        Some(v) if v != current => {
            set(values, &f.key, v);
            true
        }
        _ => false,
    }
}

/// A number: whole when its default, its value or its bounds are; `optional`: a switch makes it
/// absent (the module then does without).
fn number(
    ui: &mut Ui,
    f: &Field,
    current: Option<&toml::Value>,
    shown: Option<&toml::Value>,
) -> Option<Option<toml::Value>> {
    let whole = |v: Option<f64>| v.is_none_or(|v| v.fract() == 0.0);
    let integer = match shown {
        Some(toml::Value::Integer(_)) => true,
        Some(toml::Value::Float(_)) => false,
        _ => whole(f.min) && whole(f.max) && whole(f.step),
    };
    // optional: a switch first (off: the key is absent)
    let mut on = !f.optional || current.is_some();
    let toggled = f.optional && ui.checkbox(&mut on, "").changed();
    let mut v = match shown {
        Some(toml::Value::Integer(i)) => *i as f64,
        Some(toml::Value::Float(x)) => *x,
        _ => f.min.unwrap_or(0.0),
    };
    let (lo, hi) = (f.min.unwrap_or(f64::MIN), f.max.unwrap_or(f64::MAX));
    let step = f.step.unwrap_or(if integer { 1.0 } else { 0.1 });
    let decimals = if integer { 0 } else { 2 };
    let mut drag = DragValue::new(&mut v).range(lo..=hi).speed(step.max(0.1)).max_decimals(decimals);
    if !f.unit.is_empty() {
        drag = drag.suffix(format!(" {}", f.unit));
    }
    let edited = ui.add_enabled(on, drag).changed();
    if toggled && !on {
        return Some(None);
    }
    // switched on (the value shown becomes the value) or edited (on its step)
    (toggled || edited).then(|| {
        let v = (v / step).round() * step;
        Some(if integer { toml::Value::Integer(v.round() as i64) } else { toml::Value::Float(v) })
    })
}

/// A choice among the field's `choices`, or the module's list `source` (`{key}`: the value of a
/// sibling field); typed by hand while the module gives none (not running). `optional`: first
/// « (par défaut) », which removes the key (the module then picks).
fn choice(
    ui: &mut Ui,
    f: &Field,
    values: &toml::Table,
    shown: Option<&toml::Value>,
    choices: &HashMap<String, Vec<String>>,
) -> Option<Option<toml::Value>> {
    let current = shown.and_then(toml::Value::as_str).unwrap_or("").to_owned();
    let options: Vec<String> = if f.source.is_empty() {
        f.choices.clone()
    } else {
        let mut source = f.source.clone();
        for (k, v) in values {
            if let Some(v) = v.as_str() {
                source = source.replace(&format!("{{{k}}}"), v);
            }
        }
        choices.get(&source).cloned().unwrap_or_default()
    };
    if options.is_empty() {
        let mut text = current.clone();
        let hint = if f.source.is_empty() { "" } else { tr("(module running: list here)") };
        let r = ui.add(TextEdit::singleline(&mut text).hint_text(hint).desired_width(260.0));
        return r.changed().then(|| (!text.trim().is_empty()).then(|| toml::Value::String(text.trim().to_owned())));
    }
    let mut picked = current.clone();
    // the label of a choice of the field (`choice_labels`), or the value itself
    let label = |v: &str| -> String {
        let at = f.choices.iter().position(|c| c == v).filter(|_| f.source.is_empty());
        at.and_then(|i| f.choice_labels.get(i)).map_or(v, |l| l.as_str()).to_owned()
    };
    let shown_text = match current.as_str() {
        "" if f.optional => tr("(default)").to_owned(),
        "" => tr("(choose)").to_owned(),
        c => label(c),
    };
    egui::ComboBox::from_id_salt("choice")
        .selected_text(shown_text)
        .width(260.0)
        .show_ui(ui, |ui| {
            if f.optional {
                ui.selectable_value(&mut picked, String::new(), tr("(default)"));
            }
            if !current.is_empty() && !options.contains(&current) {
                ui.selectable_value(&mut picked, current.clone(), tf!("{current} (not found)", current = current));
            }
            for o in &options {
                ui.selectable_value(&mut picked, o.clone(), label(o));
            }
        });
    (picked != current).then(|| (!picked.is_empty()).then_some(toml::Value::String(picked)))
}

/// A list of items (array of tables): each with its fields and « retirer », then « + … ».
fn list(
    ui: &mut Ui,
    f: &Field,
    values: &mut toml::Table,
    choices: &HashMap<String, Vec<String>>,
) -> bool {
    ui.add_space(8.0);
    let r = ui.label(RichText::new(&f.label).strong().size(15.0));
    if !f.help.is_empty() {
        r.on_hover_text(&f.help);
        ui.weak(&f.help);
    }
    let mut items: Vec<toml::Table> = get(values, &f.key)
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_table().cloned()).collect())
        .unwrap_or_default();
    let mut changed = false;
    let mut remove = None;
    for (i, item) in items.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                changed |= form(ui, &format!("{}-{i}", f.key), &f.fields, item, choices);
                if ui.add(Button::new("retirer").small()).clicked() {
                    remove = Some(i);
                }
            });
        });
    }
    if let Some(i) = remove {
        items.remove(i);
        changed = true;
    }
    let add = if f.add.is_empty() { tr("+ Add") } else { f.add.as_str() };
    if ui.button(add).clicked() {
        items.push(toml::Table::new());
        changed = true;
    }
    if changed {
        let array = items.into_iter().map(toml::Value::Table).collect::<Vec<_>>();
        set(values, &f.key, (!array.is_empty()).then_some(toml::Value::Array(array)));
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Discord module's form reads: sections, a number, lists whose items have choices fed
    /// by the module (the channels of the chosen server), an optional level.
    #[test]
    fn shipped_forms_read() {
        let text = include_str!("../../modules/discord/module.toml");
        let m: crate::tools::Module = toml::from_str(text).unwrap();
        let s = m.settings.unwrap();
        assert_eq!(s.editor, "formulaire");
        assert!(s.guide.contains("https://discord.com/developers/applications"));
        let kinds: Vec<_> = s.fields.iter().map(|f| f.kind.as_str()).collect();
        assert_eq!(kinds, ["section", "number", "list", "list", "section", "bool", "bool", "section", "text"]);
        let salon = &s.fields[2];
        assert_eq!(salon.fields[1].source, "salons:{serveur}");
        let niveau = &s.fields[3].fields[1];
        assert!(niveau.optional && niveau.max == Some(200.0));
        // the Spotify module: its playlists from the module, its two secrets
        let m: crate::tools::Module = toml::from_str(include_str!("../../modules/spotify/module.toml")).unwrap();
        let s = m.settings.unwrap();
        assert_eq!((s.editor.as_str(), s.fields[2].fields[0].source.as_str()), ("formulaire", "playlists"));
        assert_eq!(m.secrets.len(), 2);
        // the OBS module: knobs' sources from the module, « (par défaut) » allowed; favourites
        // whose second field depends on the first; a decimal step; one secret
        let m: crate::tools::Module = toml::from_str(include_str!("../../modules/obs/module.toml")).unwrap();
        let s = m.settings.unwrap();
        let knob = &s.fields[1];
        assert_eq!((knob.key.as_str(), knob.source.as_str(), knob.optional), ("molettes.k2", "entrees", true));
        let pas = s.fields.iter().find(|f| f.key == "pas").unwrap();
        assert_eq!((pas.step, pas.default.clone()), (Some(0.5), Some(toml::Value::Float(2.0))));
        let src = s.fields.iter().find(|f| f.key == "source").unwrap();
        assert_eq!(src.fields[1].source, "elements:{scene}");
        assert_eq!((m.secrets.len(), m.secrets[0].name.as_str()), (1, "OBS_PASSWORD"));
        let values: toml::Table = toml::from_str(include_str!("../../modules/obs/obs.toml")).unwrap();
        assert!(values.is_empty(), "obs.toml: comments only, the defaults of the form");
        // the telemetry modules (same dashboard): a list of buttons and their keys
        for text in [include_str!("../../modules/motortown/module.toml"),
                     include_str!("../../modules/simhub/module.toml")] {
            let m: crate::tools::Module = toml::from_str(text).unwrap();
            let s = m.settings.unwrap();
            let keys = s.fields.iter().find(|f| f.key == "touche").expect("touche");
            assert_eq!((s.editor.as_str(), keys.fields.len()), ("formulaire", 2), "{}", m.name);
        }
        // the Système module: dotted keys into systeme.toml, all of them read there
        let text = include_str!("../../modules/system/module.toml");
        let m: crate::tools::Module = toml::from_str(text).unwrap();
        let fields = m.settings.unwrap().fields;
        let values: toml::Table = toml::from_str(include_str!("../../modules/system/systeme.toml")).unwrap();
        for f in fields.iter().filter(|f| f.kind != "section") {
            assert!(get(&values, &f.key).is_some(), "{} absent from systeme.toml", f.key);
            assert_eq!(get(&values, &f.key), f.default.as_ref(), "default of {}", f.key);
        }
    }

    /// Dotted keys read and written through nested tables; the file keeps its comments.
    #[test]
    fn dotted_keys_and_comments_kept() {
        let old = "# guide\npas = 2 # % par cran\n\n[alertes.gpu]\n# la sonde\nsonde = \"auto\"\n";
        let mut t: toml::Table = toml::from_str(old).unwrap();
        assert_eq!(get(&t, "alertes.gpu.sonde").and_then(|v| v.as_str()), Some("auto"));
        set(&mut t, "alertes.gpu.seuil", Some(toml::Value::Integer(85)));
        set(&mut t, "pas", Some(toml::Value::Integer(5)));
        set(&mut t, "page.afficher", Some(toml::Value::Boolean(false)));
        let text = to_toml(&t, old).unwrap();
        assert!(text.contains("# guide") && text.contains("# la sonde"), "{text}");
        let back: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(back, t);
        set(&mut t, "alertes.gpu.seuil", None);
        assert!(get(&t, "alertes.gpu.seuil").is_none());
    }
}
