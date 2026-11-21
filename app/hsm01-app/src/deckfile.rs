//! `deck.toml` of the « Stream deck » tool (`app/modules/streamdeck/`): layout and up to 4 tracks
//! of tiles, edited in the tool's own tab of the window. Written back into the existing text
//! (comments and hand layout kept, actions as inline tables like the shipped file).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{Color, Tile};

/// Tracks of the tool: one per knob, in order.
pub const MAX_TRACKS: usize = 4;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DeckFile {
    /// `colonnes` (track k above knob k) or `lignes` (one row per knob).
    #[serde(default = "columns")]
    pub layout: String,
    #[serde(default, rename = "track")]
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Track {
    #[serde(default)]
    pub name: String,
    #[serde(default = "teal")]
    pub color: Color,
    #[serde(default, rename = "tile")]
    pub tiles: Vec<Tile>,
}

impl Track {
    /// A new track, in the next colour of the shipped ones.
    pub fn new(index: usize) -> Track {
        const COLORS: [Color; 4] = [
            Color(0x00, 0xf0, 0xff),
            Color(0xff, 0xaa, 0x00),
            Color(0x00, 0xff, 0x88),
            Color(0xb0, 0x26, 0xff),
        ];
        Track {
            name: tf!("Track {n}", n = index + 1),
            color: COLORS[index % COLORS.len()],
            tiles: Vec::new(),
        }
    }
}

fn columns() -> String {
    "colonnes".into()
}

fn teal() -> Color {
    Color(0x00, 0xb4, 0xa8)
}

/// The file, or an empty deck if it does not exist yet.
pub fn load(path: &Path) -> Result<(DeckFile, String), String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let deck = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((deck, text))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((DeckFile::default(), String::new())),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Text of `deck`, merged into `old` (its comments and layout kept) when that reads back as the
/// same deck, otherwise written afresh.
pub fn to_toml(deck: &DeckFile, old: &str) -> Result<String, String> {
    let text = toml::to_string_pretty(deck).map_err(|e| e.to_string())?;
    let mut fresh: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    inline_actions(&mut fresh);
    let merged = old.parse::<toml_edit::DocumentMut>().ok().map(|mut doc| {
        crate::config::keep::table(doc.as_table_mut(), fresh.as_table());
        doc.to_string()
    });
    let reads_back = |t: &String| toml::from_str::<DeckFile>(t).ok().as_ref() == Some(deck);
    Ok(merged.filter(reads_back).unwrap_or_else(|| fresh.to_string()))
}

/// `action = { type = "…", … }` on one line, as in the shipped file (instead of an
/// `[track.tile.action]` table under each tile).
fn inline_actions(doc: &mut toml_edit::DocumentMut) {
    let Some(tracks) = doc.get_mut("track").and_then(|t| t.as_array_of_tables_mut()) else {
        return;
    };
    for track in tracks.iter_mut() {
        let Some(tiles) = track.get_mut("tile").and_then(|t| t.as_array_of_tables_mut()) else {
            continue;
        };
        for tile in tiles.iter_mut() {
            if let Some(toml_edit::Item::Table(t)) = tile.get("action") {
                let inline = t.clone().into_inline_table();
                tile.insert("action", toml_edit::value(inline));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Action;

    fn shipped() -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../modules/streamdeck/deck.toml");
        std::fs::read_to_string(path).expect("shipped deck.toml")
    }

    /// The shipped file reads; edited (layout, a tile changed, a tile and a track added), it is
    /// written back with its comments, the actions on one line, and reads back the same.
    #[test]
    fn shipped_deck_edited_and_written_back() {
        let old = shipped();
        let mut deck: DeckFile = toml::from_str(&old).unwrap();
        assert_eq!((deck.layout.as_str(), deck.tracks.len()), ("colonnes", 4));
        assert_eq!(deck.tracks[1].tiles[1].action, Action::Media { action: "next".into() });
        deck.layout = "lignes".into();
        deck.tracks[0].tiles[0].title = "Couper".into();
        deck.tracks[2].tiles.push(Tile::new("Kate", "éditeur", "kate", Action::Command {
            cmd: "kate".into(),
        }));
        deck.tracks.truncate(3);
        deck.tracks.push(Track::new(3));
        let text = to_toml(&deck, &old).unwrap();
        assert!(text.contains("# Cases du « Stream deck »"), "comments kept:\n{text}");
        assert!(text.contains("action = { type = \"command\", cmd = \"kate\" }"), "{text}");
        assert!(!text.contains("[track.tile.action]"), "{text}");
        assert!(text.contains("layout = \"lignes\""));
        let back: DeckFile = toml::from_str(&text).unwrap();
        assert_eq!(back, deck);
        // unchanged: the very same text
        let same: DeckFile = toml::from_str(&old).unwrap();
        assert_eq!(to_toml(&same, &old).unwrap(), old);
    }

    /// A missing file is an empty deck (the tab creates it); a broken one is an error.
    #[test]
    fn missing_and_broken_files() {
        let dir = std::env::temp_dir().join(format!("hsm01-deckfile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("deck.toml");
        let (deck, text) = load(&path).unwrap();
        assert_eq!((deck.tracks.len(), text.as_str()), (0, ""));
        let mut deck = deck;
        deck.tracks.push(Track::new(0));
        deck.tracks[0].tiles = Tile::media();
        let written = to_toml(&deck, "").unwrap();
        std::fs::write(&path, &written).unwrap();
        assert_eq!(load(&path).unwrap(), (deck, written));
        std::fs::write(&path, "[[track]\n").unwrap();
        assert!(load(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
