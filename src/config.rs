//! Persisted mod values (TOML save files).
//!
//! Save files live OUTSIDE the game install dir
//! (`$XDG_CONFIG_HOME/cxsfm/saves.toml`, else `~/.config/cxsfm/`),
//! are hand-editable (TOML comments survive — we never rewrite user
//! text, only whole-file replace on Save), and store values BY OBJECT
//! PATH (`[speedo."Speedometer/Background/ImgBg"]`). Never addresses,
//! never originals (those are re-read from the game every session).
//!
//! All file IO happens on the tick thread (Save/Load buttons only
//! stage data + set flags on the render thread). Writes are atomic
//! (tmp + rename). Parsing is tolerant: unknown mods/objects/keys and
//! mistyped values are skipped + counted, never fatal.

use crate::mods::speedo::model::SavedObj;
use std::path::PathBuf;

/// Schema version written by Save and expected by Load. A mismatch
/// logs a warning but still parses (tolerant reader).
pub const SCHEMA_VERSION: i64 = 1;

/// Save-file location: `$XDG_CONFIG_HOME/cxsfm/saves.toml`, falling
/// back to `~/.config/cxsfm/saves.toml` (blocked when neither `HOME`
/// nor `XDG_CONFIG_HOME` exists — tread carefully there).
pub fn saves_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".config");
                p
            })
        })?;
    let mut p = base;
    p.push("cxsfm");
    p.push("saves.toml");
    Some(p)
}

/// Atomic whole-file write: parent dirs created, tmp + rename (a torn
/// write must never leave half a save behind).
pub fn save_toml(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read the whole save file.
pub fn load_toml(path: &std::path::Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}

fn f3(v: &toml::Value) -> Option<[f32; 3]> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    let mut out = [0.0f32; 3];
    for (i, x) in a.iter().enumerate() {
        let f = x.as_float()?;
        if !f.is_finite() {
            return None;
        }
        out[i] = f as f32;
    }
    Some(out)
}

fn f4(v: &toml::Value) -> Option<[f32; 4]> {
    let a = v.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut out = [0.0f32; 4];
    for (i, x) in a.iter().enumerate() {
        let f = x.as_float()?;
        if !f.is_finite() {
            return None;
        }
        out[i] = f as f32;
    }
    Some(out)
}

fn f1(v: &toml::Value) -> Option<f32> {
    let f = v.as_float()?;
    if !f.is_finite() {
        return None;
    }
    Some(f as f32)
}

fn arr3(v: [f32; 3]) -> toml::Value {
    toml::Value::Array(v.iter().map(|x| toml::Value::Float(*x as f64)).collect())
}

fn arr4(v: [f32; 4]) -> toml::Value {
    toml::Value::Array(v.iter().map(|x| toml::Value::Float(*x as f64)).collect())
}

/// Build save text: `version` + one `[mod."object/path"]` table per
/// entry, only modified params present (see `diff_object`).
pub fn build_saves(entries: &[(String, String, SavedObj)]) -> String {
    let mut root = toml::Table::new();
    root.insert(
        "version".to_string(),
        toml::Value::Integer(SCHEMA_VERSION),
    );
    // Deterministic output: sorted by (mod, path).
    let mut sorted: Vec<&(String, String, SavedObj)> = entries.iter().collect();
    sorted.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    for (mod_id, path, s) in sorted {
        let mod_table = root
            .entry(mod_id.clone())
            .or_insert(toml::Value::Table(toml::Table::new()));
        let obj = toml::Table::new();
        let mut obj = obj;
        if let Some(v) = s.pos {
            obj.insert("pos".to_string(), arr3(v));
        }
        if let Some(v) = s.euler {
            obj.insert("euler".to_string(), arr3(v));
        }
        if let Some(v) = s.scale {
            obj.insert("scale".to_string(), arr3(v));
        }
        if let Some(v) = s.color {
            obj.insert("color".to_string(), arr4(v));
        }
        if let Some(v) = s.opacity {
            obj.insert("opacity".to_string(), toml::Value::Float(v as f64));
        }
        if let Some(v) = s.active {
            obj.insert("active".to_string(), toml::Value::Boolean(v));
        }
        if let Some(v) = s.visible {
            obj.insert("visible".to_string(), toml::Value::Boolean(v));
        }
        if let Some(v) = s.size {
            obj.insert("size".to_string(), toml::Value::Float(v as f64));
        }
        if let toml::Value::Table(t) = mod_table {
            t.insert(path.clone(), toml::Value::Table(obj));
        }
    }
    let mut text = String::from("# CXSFM saved mod values. Hand-editable; unknown keys ignored.\n");
    text.push_str(&toml::to_string(&root).unwrap_or_default());
    text
}

/// Parse save text → `((mod, path, values), skipped)`. Tolerant: a bad
/// object/table is skipped + counted, parsing continues with the rest.
/// Unknown top-level keys (including future mods) are ignored here and
/// filtered by the caller.
pub fn parse_saves(text: &str) -> (Vec<(String, String, SavedObj)>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0usize;
    let root: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(_) => return (out, 1),
    };
    if root.get("version").and_then(|v| v.as_integer()) != Some(SCHEMA_VERSION) {
        // Tolerant: warn by counting (caller logs), still parse.
        skipped += 1;
    }
    for (mod_id, mod_val) in &root {
        if mod_id == "version" {
            continue;
        }
        let mod_table = match mod_val.as_table() {
            Some(t) => t,
            None => {
                skipped += 1;
                continue;
            }
        };
        for (path, obj_val) in mod_table {
            let obj = match obj_val.as_table() {
                Some(t) => t,
                None => {
                    skipped += 1;
                    continue;
                }
            };
            let mut s = SavedObj::default();
            let mut any = false;
            if let Some(v) = obj.get("pos").and_then(f3) {
                s.pos = Some(v);
                any = true;
            }
            if let Some(v) = obj.get("euler").and_then(f3) {
                s.euler = Some(v);
                any = true;
            }
            if let Some(v) = obj.get("scale").and_then(f3) {
                s.scale = Some(v);
                any = true;
            }
            if let Some(v) = obj.get("color").and_then(f4) {
                s.color = Some(v);
                any = true;
            }
            if let Some(v) = obj.get("opacity").and_then(f1) {
                s.opacity = Some(v);
                any = true;
            }
            if let Some(v) = obj.get("active").and_then(|v| v.as_bool()) {
                s.active = Some(v);
                any = true;
            }
            if let Some(v) = obj.get("visible").and_then(|v| v.as_bool()) {
                s.visible = Some(v);
                any = true;
            }
            if let Some(v) = obj.get("size").and_then(f1) {
                s.size = Some(v);
                any = true;
            }
            if any {
                out.push((mod_id.clone(), path.clone(), s));
            } else {
                skipped += 1;
            }
        }
    }
    (out, skipped)
}

#[cfg(test)]
mod tests {
    use super::{build_saves, parse_saves};
    use crate::mods::speedo::model::SavedObj;

    fn sample() -> Vec<(String, String, SavedObj)> {
        vec![
            (
                "map".to_string(),
                "Map/Mask/Base/RoadImage".to_string(),
                SavedObj {
                    opacity: Some(0.1),
                    active: Some(false),
                    ..SavedObj::default()
                },
            ),
            (
                "speedo".to_string(),
                "Speed/Text (TMP) Gear \"quoted\"".to_string(),
                SavedObj {
                    scale: Some([1.2, 1.2, 1.0]),
                    ..SavedObj::default()
                },
            ),
        ]
    }

    #[test]
    fn roundtrip_tricky_paths() {
        let text = build_saves(&sample());
        let (entries, skipped) = parse_saves(&text);
        assert_eq!(skipped, 0);
        assert_eq!(entries.len(), 2);
        let road = entries
            .iter()
            .find(|(m, p, _)| m == "map" && p == "Map/Mask/Base/RoadImage")
            .expect("road entry");
        assert_eq!(road.2.opacity, Some(0.1));
        assert_eq!(road.2.active, Some(false));
        let gear = entries
            .iter()
            .find(|(m, _, _)| m == "speedo")
            .expect("gear entry");
        assert_eq!(gear.2.scale, Some([1.2, 1.2, 1.0]));
    }

    #[test]
    fn garbage_tolerated() {
        let text = "version = 1\n[map]\nbroken = 42\n[map.\"X\"]\nopacity = \"loud\"\n[map.\"Y\"]\nscale = [1.0, 2.0]\n[map.\"Z\"]\nactive = true\n";
        let (entries, skipped) = parse_saves(text);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "Z");
        assert!(skipped >= 3);
    }

    #[test]
    fn version_mismatch_still_parses() {
        let text = "version = 99\n[map.\"X\"]\nactive = true\n";
        let (entries, skipped) = parse_saves(text);
        assert_eq!(entries.len(), 1);
        assert!(skipped >= 1);
    }
}
