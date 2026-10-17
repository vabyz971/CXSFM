//! HUD layouts (TOML) + live values + pure helpers. No Unity, no IO:
//! everything here is unit-tested.
//!
//! A config holds named `[sources]` (game values, bound by field path
//! as found with the Explorer tab) and any number of `[[hud]]` blocks;
//! each HUD is a box anchored on the screen and drawn from
//! `[[hud.widget]]` entries (`bar` bound to a source, with bounds that
//! are numbers or sources; `text` with `{source}` placeholders).
//! Parsing is tolerant like the save files: a bad widget/key is
//! skipped with a warning, only a TOML syntax error rejects the whole
//! file (the last good layout then stays on screen).

use egui::Color32;

/// Schema version written in the example file.
pub(crate) const SCHEMA_VERSION: i64 = 2;

/// Default repaint rate while a HUD is visible (frames per second).
pub(crate) const DEFAULT_FPS: u32 = 30;

/// Screen anchor of a HUD box (the box corner/edge matching the
/// anchor sits on that screen point, then `offset` shifts it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "top_left" => Anchor::TopLeft,
            "top" => Anchor::Top,
            "top_right" => Anchor::TopRight,
            "left" => Anchor::Left,
            "center" => Anchor::Center,
            "right" => Anchor::Right,
            "bottom_left" => Anchor::BottomLeft,
            "bottom" => Anchor::Bottom,
            "bottom_right" => Anchor::BottomRight,
            _ => return None,
        })
    }

    /// Horizontal / vertical factors (0 = left/top, 1 = right/bottom).
    pub(crate) fn factors(self) -> (f32, f32) {
        match self {
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::Top => (0.5, 0.0),
            Anchor::TopRight => (1.0, 0.0),
            Anchor::Left => (0.0, 0.5),
            Anchor::Center => (0.5, 0.5),
            Anchor::Right => (1.0, 0.5),
            Anchor::BottomLeft => (0.0, 1.0),
            Anchor::Bottom => (0.5, 1.0),
            Anchor::BottomRight => (1.0, 1.0),
        }
    }
}

/// A bar bound: a constant or a named source (per-car ranges).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Bound {
    Num(f32),
    Src(String),
}

impl Bound {
    pub(crate) fn eval(&self, v: &Values) -> Option<f32> {
        match self {
            Bound::Num(x) => Some(*x),
            Bound::Src(name) => v.get(name),
        }
    }
}

/// Text alignment relative to the widget `pos`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Widget {
    /// Horizontal fill bar. `segments = 0` is a continuous fill; the
    /// fill turns `redline_color` past `redline`. Not drawn while its
    /// source or a bound is unavailable. `from_right`: the fill grows
    /// from the right edge back to the value's position (the value
    /// lands at the same spot either way).
    Bar {
        source: String,
        from_right: bool,
        min: Bound,
        max: Bound,
        redline: Option<Bound>,
        pos: [f32; 2],
        size: [f32; 2],
        segments: u32,
        color: Color32,
        redline_color: Color32,
        track_color: Color32,
    },
    /// Text with `{source}` / `{source:N}` placeholders.
    Text {
        text: String,
        pos: [f32; 2],
        size: f32,
        align: Align,
        color: Color32,
        mono: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hud {
    pub(crate) name: String,
    pub(crate) enabled: bool,
    pub(crate) anchor: Anchor,
    pub(crate) offset: [f32; 2],
    /// Box size before `scale` (widget coordinates live inside it).
    pub(crate) size: [f32; 2],
    pub(crate) scale: f32,
    pub(crate) opacity: f32,
    pub(crate) background: Color32,
    /// Hide while no game value is available (menus, loading).
    pub(crate) hide_when_missing: bool,
    pub(crate) widgets: Vec<Widget>,
}

/// Where values come from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mode {
    /// Read the game through `[sources]` (phase 3).
    Live,
    /// Synthetic loop (layout work without driving).
    Demo,
}

/// One named game value: `Root/ref/…/field`, then `raw * scale + offset`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SourceDef {
    pub(crate) name: String,
    /// Root class short name (found with `FindObjectsOfType`).
    pub(crate) root: String,
    /// Reference fields walked from the root, then the value field.
    pub(crate) refs: Vec<String>,
    pub(crate) field: String,
    pub(crate) scale: f32,
    pub(crate) offset: f32,
}

impl SourceDef {
    /// `"Root/a/b/field"` → root + refs + field (at least root/field).
    pub(crate) fn from_path(name: &str, path: &str, scale: f32, offset: f32) -> Option<Self> {
        let parts: Vec<&str> = path.split('/').map(str::trim).collect();
        if parts.len() < 2 || parts.iter().any(|p| p.is_empty()) {
            return None;
        }
        Some(SourceDef {
            name: name.to_string(),
            root: parts[0].to_string(),
            refs: parts[1..parts.len() - 1].iter().map(|s| s.to_string()).collect(),
            field: parts[parts.len() - 1].to_string(),
            scale,
            offset,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HudConfig {
    pub(crate) fps: u32,
    pub(crate) mode: Mode,
    pub(crate) sources: Vec<SourceDef>,
    pub(crate) huds: Vec<Hud>,
}

/// One frame of named values. Small (a handful of entries): a linear
/// scan beats a map here. `speed_mph` derives from `speed` (km/h).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Values(pub(crate) Vec<(String, f32)>);

impl Values {
    pub(crate) fn get(&self, name: &str) -> Option<f32> {
        if let Some((_, v)) = self.0.iter().find(|(n, _)| n == name) {
            return Some(*v);
        }
        if name == "speed_mph" {
            return self.get("speed").map(|kmh| kmh * 0.621_371);
        }
        None
    }

    pub(crate) fn set(&mut self, name: &str, v: f32) {
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = v,
            None => self.0.push((name.to_string(), v)),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `R` / `N` / `1`…
pub(crate) fn gear_label(gear: i32) -> String {
    match gear {
        g if g < 0 => "R".to_string(),
        0 => "N".to_string(),
        g => g.to_string(),
    }
}

/// Fill `{name}` (0 decimals; `gear` as R/N/1…) and `{name:N}`
/// (N decimals, max 6). Unknown or unavailable names stay literal
/// (a typo stays visible on screen).
pub(crate) fn format_template(t: &str, v: &Values) -> String {
    if !t.contains('{') {
        return t.to_string();
    }
    let mut out = String::with_capacity(t.len());
    let mut rest = t;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let inner = &after[..close];
        let (name, decimals) = match inner.split_once(':') {
            Some((n, d)) => (n, d.parse::<usize>().ok().map(|d| d.min(6))),
            None => (inner, None),
        };
        match v.get(name) {
            Some(x) if name == "gear" && decimals.is_none() => out.push_str(&gear_label(x.round() as i32)),
            Some(x) => out.push_str(&format!("{:.*}", decimals.unwrap_or(0), x)),
            None => {
                out.push('{');
                out.push_str(inner);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Fill fraction 0..=1 (degenerate ranges and NaN read as empty).
pub(crate) fn bar_fraction(v: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if !(span > 0.0) || !v.is_finite() {
        return 0.0;
    }
    ((v - min) / span).clamp(0.0, 1.0)
}

/// Demo source: a looping launch (6 upshifts, then a short idle at N),
/// in the same units as the live `UITachometer` values (km/h, RPM as
/// the dial fill amount with its own min/max). Pure function of time.
pub(crate) fn demo_values(t: f64) -> Values {
    const GEAR_S: f64 = 1.5;
    const GEARS: i32 = 6;
    const IDLE_S: f64 = 1.5;
    const TOPS: [f32; 6] = [60.0, 95.0, 130.0, 165.0, 200.0, 240.0];
    const MIN_FILL: f32 = 0.1;
    const MAX_FILL: f32 = 0.9;
    let period = GEAR_S * GEARS as f64 + IDLE_S;
    let p = if t.is_finite() { t.rem_euclid(period) } else { 0.0 };
    let (speed, k, gear) = if p >= GEAR_S * GEARS as f64 {
        (0.0, 0.05, 0)
    } else {
        let idx = (p / GEAR_S) as usize;
        let k = 0.35 + 0.6 * ((p - idx as f64 * GEAR_S) / GEAR_S) as f32;
        (TOPS[idx.min(5)] * k, k, idx as i32 + 1)
    };
    let mut v = Values::default();
    v.set("speed", speed);
    v.set("gear", gear as f32);
    v.set("max_gear", GEARS as f32);
    v.set("rpm", MIN_FILL + (MAX_FILL - MIN_FILL) * k);
    v.set("rpm_min", MIN_FILL);
    v.set("rpm_max", MAX_FILL);
    v.set("redline", MIN_FILL + (MAX_FILL - MIN_FILL) * 0.85);
    v
}

fn num(v: &toml::Value) -> Option<f32> {
    v.as_float()
        .map(|f| f as f32)
        .or_else(|| v.as_integer().map(|i| i as f32))
        .filter(|f| f.is_finite())
}

fn pair(v: &toml::Value) -> Option<[f32; 2]> {
    let a = v.as_array()?;
    if a.len() != 2 {
        return None;
    }
    Some([num(&a[0])?, num(&a[1])?])
}

fn bound(v: &toml::Value) -> Option<Bound> {
    num(v).map(Bound::Num).or_else(|| v.as_str().map(|s| Bound::Src(s.to_string())))
}

/// `#RRGGBB` or `#RRGGBBAA` (alpha unmultiplied, like CSS).
pub(crate) fn parse_color(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    if !h.is_ascii() || (h.len() != 6 && h.len() != 8) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    let a = if h.len() == 8 { byte(6)? } else { 255 };
    Some(Color32::from_rgba_unmultiplied(byte(0)?, byte(2)?, byte(4)?, a))
}

/// Typed field reader: absent → default, mistyped → default + warning.
struct Fields<'a> {
    t: &'a toml::Table,
    ctx: String,
    warnings: &'a mut Vec<String>,
}

impl Fields<'_> {
    fn warn(&mut self, key: &str) {
        self.warnings.push(format!("{}: `{key}` invalide, ignoré", self.ctx));
    }
    fn get<T>(&mut self, key: &str, def: T, f: impl Fn(&toml::Value) -> Option<T>) -> T {
        match self.t.get(key) {
            None => def,
            Some(v) => match f(v) {
                Some(x) => x,
                None => {
                    self.warn(key);
                    def
                }
            },
        }
    }
    fn f32(&mut self, key: &str, def: f32) -> f32 {
        self.get(key, def, num)
    }
    fn pair(&mut self, key: &str, def: [f32; 2]) -> [f32; 2] {
        self.get(key, def, pair)
    }
    fn color(&mut self, key: &str, def: Color32) -> Color32 {
        self.get(key, def, |v| v.as_str().and_then(parse_color))
    }
    fn bool(&mut self, key: &str, def: bool) -> bool {
        self.get(key, def, |v| v.as_bool())
    }
    fn str(&mut self, key: &str, def: &str) -> String {
        self.get(key, def.to_string(), |v| v.as_str().map(str::to_string))
    }
    fn bound(&mut self, key: &str, def: Bound) -> Bound {
        self.get(key, def, bound)
    }
}

fn parse_widget(t: &toml::Table, ctx: String, warnings: &mut Vec<String>) -> Option<Widget> {
    let mut f = Fields { t, ctx, warnings };
    let kind = f.str("type", "");
    match kind.as_str() {
        "bar" => {
            let source = f.str("source", "");
            if source.is_empty() {
                f.warnings.push(format!("{}: `source` manquante, widget ignoré", f.ctx));
                return None;
            }
            let redline = match f.t.get("redline") {
                None => None,
                Some(v) => match bound(v) {
                    Some(b) => Some(b),
                    None => {
                        f.warn("redline");
                        None
                    }
                },
            };
            let from_right = match f.str("fill", "left").as_str() {
                "left" => false,
                "right" => true,
                _ => {
                    f.warn("fill");
                    false
                }
            };
            Some(Widget::Bar {
                source,
                from_right,
                min: f.bound("min", Bound::Num(0.0)),
                max: f.bound("max", Bound::Num(1.0)),
                redline,
                pos: f.pair("pos", [0.0, 0.0]),
                size: f.pair("size", [200.0, 8.0]),
                segments: f.f32("segments", 0.0).clamp(0.0, 200.0) as u32,
                color: f.color("color", Color32::WHITE),
                redline_color: f.color("redline_color", Color32::from_rgb(0xE0, 0x30, 0x30)),
                track_color: f.color("track_color", Color32::from_white_alpha(40)),
            })
        }
        "text" => {
            let align = match f.str("align", "left").as_str() {
                "left" => Align::Left,
                "center" => Align::Center,
                "right" => Align::Right,
                _ => {
                    f.warn("align");
                    Align::Left
                }
            };
            Some(Widget::Text {
                text: f.str("text", ""),
                pos: f.pair("pos", [0.0, 0.0]),
                size: f.f32("size", 16.0).clamp(4.0, 300.0),
                align,
                color: f.color("color", Color32::WHITE),
                mono: f.bool("mono", false),
            })
        }
        other => {
            f.warnings.push(format!("{}: type `{other}` inconnu, widget ignoré", f.ctx));
            None
        }
    }
}

/// `[sources]`: `name = "Root/…/field"` or
/// `name = { path = "…", scale = 1.0, offset = 0.0 }`.
fn parse_sources(root: &toml::Table, warnings: &mut Vec<String>) -> Vec<SourceDef> {
    let mut out = Vec::new();
    let Some(table) = root.get("sources") else {
        return out;
    };
    let Some(table) = table.as_table() else {
        warnings.push("`sources` doit être une table, ignoré".into());
        return out;
    };
    for (name, v) in table {
        let def = match v {
            toml::Value::String(path) => SourceDef::from_path(name, path, 1.0, 0.0),
            toml::Value::Table(t) => {
                let mut f = Fields { t, ctx: format!("source `{name}`"), warnings };
                let path = f.str("path", "");
                let scale = f.f32("scale", 1.0);
                let offset = f.f32("offset", 0.0);
                SourceDef::from_path(name, &path, scale, offset)
            }
            _ => None,
        };
        match def {
            Some(d) => out.push(d),
            None => warnings.push(format!("source `{name}` : chemin `Classe/…/champ` attendu, ignorée")),
        }
    }
    out
}

/// Parse a whole `hud.toml`. `Err` = syntax error (keep the previous
/// layout); `Ok` carries the per-key warnings.
pub(crate) fn parse_config(text: &str) -> Result<(HudConfig, Vec<String>), String> {
    let root: toml::Table = text.parse().map_err(|e: toml::de::Error| e.message().to_string())?;
    let mut warnings = Vec::new();
    // Tolerant reader (same policy as saves.toml): a mismatch warns.
    if let Some(v) = root.get("version").and_then(|v| v.as_integer()) {
        if v != SCHEMA_VERSION {
            warnings.push(format!("version {v} (attendue {SCHEMA_VERSION}), lecture quand même"));
        }
    }
    let fps = match root.get("fps").and_then(num) {
        Some(f) => (f as u32).clamp(5, 60),
        None => DEFAULT_FPS,
    };
    let mode = match root.get("mode").and_then(|v| v.as_str()) {
        None | Some("live") => Mode::Live,
        Some("demo") => Mode::Demo,
        Some(other) => {
            warnings.push(format!("mode `{other}` inconnu (live | demo), live utilisé"));
            Mode::Live
        }
    };
    let sources = parse_sources(&root, &mut warnings);
    let mut huds = Vec::new();
    let list = root.get("hud").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for (i, h) in list.iter().enumerate() {
        let Some(t) = h.as_table() else {
            warnings.push(format!("hud #{}: pas une table, ignoré", i + 1));
            continue;
        };
        let mut f = Fields { t, ctx: format!("hud #{}", i + 1), warnings: &mut warnings };
        let name = f.str("name", &format!("hud {}", i + 1));
        f.ctx = format!("hud `{name}`");
        let anchor_s = f.str("anchor", "bottom_right");
        let anchor = Anchor::parse(&anchor_s).unwrap_or_else(|| {
            f.warn("anchor");
            Anchor::BottomRight
        });
        let mut hud = Hud {
            enabled: f.bool("enabled", true),
            anchor,
            offset: f.pair("offset", [0.0, 0.0]),
            size: f.pair("size", [300.0, 100.0]),
            scale: f.f32("scale", 1.0).clamp(0.1, 10.0),
            opacity: f.f32("opacity", 1.0).clamp(0.0, 1.0),
            background: f.color("background", Color32::TRANSPARENT),
            hide_when_missing: f.bool("hide_when_missing", true),
            widgets: Vec::new(),
            name,
        };
        let ctx = f.ctx.clone();
        let widgets = t.get("widget").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for (j, w) in widgets.iter().enumerate() {
            let wctx = format!("{ctx} widget #{}", j + 1);
            match w.as_table() {
                Some(wt) => {
                    if let Some(w) = parse_widget(wt, wctx, &mut warnings) {
                        hud.widgets.push(w);
                    }
                }
                None => warnings.push(format!("{wctx}: pas une table, ignoré")),
            }
        }
        huds.push(hud);
    }
    if mode == Mode::Live && sources.is_empty() && !huds.is_empty() {
        warnings.push(
            "mode live sans [sources] : aucune valeur, HUD masqués (ajoute [sources] ou supprime hud.toml pour régénérer l'exemple)"
                .into(),
        );
    }
    Ok((HudConfig { fps, mode, sources, huds }, warnings))
}

/// Example written on first enable when no `hud.toml` exists.
pub(crate) const DEFAULT_CONFIG: &str = r##"# CXSFM — HUD personnalisés (rechargé à chaud : enregistre, regarde en jeu).
#
# mode = "live" lit le jeu via [sources] ; "demo" anime des valeurs
# factices (pour travailler la mise en page sans rouler).
#
# [sources] : nom = "Classe/réf/…/champ" (chemins trouvés avec l'onglet
# Explorateur ; survole un champ pour copier son chemin), ou
# nom = { path = "…", scale = 1.0, offset = 0.0 } → valeur*scale+offset.
# `speed_mph` est dérivée de `speed` (km/h) automatiquement.
#
# Plusieurs [[hud]] possibles. Chaque HUD est une boîte `size` (avant
# `scale`) posée sur un coin/bord de l'écran (`anchor` : top_left, top,
# top_right, left, center, right, bottom_left, bottom, bottom_right),
# décalée de `offset` (px). Les widgets se placent DANS la boîte (`pos`).
# `hide_when_missing` (défaut true) cache le HUD hors course (menus).
#
# Widgets :
#   type = "bar"  : source = nom, min / max / redline = nombre OU nom de
#                   source, segments (0 = continu), fill = "left" (défaut)
#                   | "right" (remplit depuis le bord droit jusqu'à la
#                   valeur), color, redline_color, track_color
#   type = "text" : text avec {nom} (0 décimale ; {gear} → R/N/1…) ou
#                   {nom:2} (2 décimales), size, align = "left" | "center"
#                   | "right", mono, color
# Couleurs : "#RRGGBB" ou "#RRGGBBAA".

version = 2
fps = 30
mode = "live"

[sources]
speed = "UITachometer/m_currentSpeed"
gear = "UITachometer/m_gear"
max_gear = "UITachometer/m_maxGear"
rpm = "UITachometer/m_rpmFillImage/m_FillAmount"
rpm_min = "UITachometer/m_minRpmFillAmount"
rpm_max = "UITachometer/m_maxRpmFillAmount"
redline = "UITachometer/m_readZoneRpm"

[[hud]]
name = "GT minimal"
enabled = true
anchor = "bottom_right"
offset = [-48.0, -48.0]
size = [320.0, 120.0]
scale = 1.0
opacity = 1.0
background = "#00000000"

  [[hud.widget]]
  type = "bar"
  source = "rpm"
  min = "rpm_min"
  max = "rpm_max"
  redline = "redline"
  pos = [0.0, 0.0]
  size = [320.0, 6.0]
  color = "#FFFFFF"
  redline_color = "#E03030"
  track_color = "#FFFFFF28"

  [[hud.widget]]
  type = "text"
  text = "{gear}"
  pos = [0.0, 18.0]
  size = 56
  mono = true
  color = "#F5C518"

  [[hud.widget]]
  type = "text"
  text = "{speed}"
  pos = [320.0, 12.0]
  size = 64
  align = "right"
  mono = true

  [[hud.widget]]
  type = "text"
  text = "km/h"
  pos = [320.0, 84.0]
  size = 16
  align = "right"
  color = "#FFFFFFA0"

# Deuxième HUD (désactivé) : passe `enabled` à true pour l'essayer.
[[hud]]
name = "Bandeau"
enabled = false
anchor = "top"
offset = [0.0, 24.0]
size = [240.0, 40.0]
background = "#000000A0"

  [[hud.widget]]
  type = "bar"
  source = "rpm"
  min = "rpm_min"
  max = "rpm_max"
  redline = "redline"
  segments = 20
  pos = [8.0, 6.0]
  size = [224.0, 8.0]

  [[hud.widget]]
  type = "text"
  text = "{gear}/{max_gear}  ·  {speed} km/h"
  pos = [120.0, 18.0]
  size = 16
  align = "center"
"##;

// --- Field explorer (phase 2): pure decoding + view data ---------------------

/// `Il2CppTypeEnum` values the explorer understands.
pub(crate) mod te {
    pub(crate) const BOOLEAN: i32 = 0x02;
    pub(crate) const CHAR: i32 = 0x03;
    pub(crate) const I1: i32 = 0x04;
    pub(crate) const U1: i32 = 0x05;
    pub(crate) const I2: i32 = 0x06;
    pub(crate) const U2: i32 = 0x07;
    pub(crate) const I4: i32 = 0x08;
    pub(crate) const U4: i32 = 0x09;
    pub(crate) const I8: i32 = 0x0a;
    pub(crate) const U8: i32 = 0x0b;
    pub(crate) const R4: i32 = 0x0c;
    pub(crate) const R8: i32 = 0x0d;
    pub(crate) const STRING: i32 = 0x0e;
    pub(crate) const PTR: i32 = 0x0f;
    pub(crate) const VALUETYPE: i32 = 0x11;
    pub(crate) const CLASS: i32 = 0x12;
    pub(crate) const ARRAY: i32 = 0x14;
    pub(crate) const GENERICINST: i32 = 0x15;
    pub(crate) const I: i32 = 0x18;
    pub(crate) const U: i32 = 0x19;
    pub(crate) const OBJECT: i32 = 0x1c;
    pub(crate) const SZARRAY: i32 = 0x1d;
}

/// Payload size of a primitive / pointer-sized type (`None` = not a
/// fixed-size primitive: value types ask the runtime instead).
pub(crate) fn primitive_size(t: i32) -> Option<usize> {
    Some(match t {
        te::BOOLEAN | te::I1 | te::U1 => 1,
        te::CHAR | te::I2 | te::U2 => 2,
        te::I4 | te::U4 | te::R4 => 4,
        te::I8 | te::U8 | te::R8 | te::I | te::U | te::PTR => 8,
        te::STRING | te::CLASS | te::OBJECT | te::SZARRAY | te::ARRAY => 8,
        _ => return None,
    })
}

/// C#-style name of a primitive (`None` = use the class name).
pub(crate) fn primitive_name(t: i32) -> Option<&'static str> {
    Some(match t {
        te::BOOLEAN => "bool",
        te::CHAR => "char",
        te::I1 => "sbyte",
        te::U1 => "byte",
        te::I2 => "short",
        te::U2 => "ushort",
        te::I4 => "int",
        te::U4 => "uint",
        te::I8 => "long",
        te::U8 => "ulong",
        te::R4 => "float",
        te::R8 => "double",
        te::STRING => "string",
        te::I => "nint",
        te::U => "nuint",
        te::PTR => "ptr",
        _ => return None,
    })
}

/// Reference-typed payload (a pointer to another managed object).
pub(crate) fn is_reference(t: i32) -> bool {
    matches!(t, te::STRING | te::CLASS | te::OBJECT | te::SZARRAY | te::ARRAY)
}

fn le<const N: usize>(b: &[u8]) -> Option<[u8; N]> {
    b.get(..N)?.try_into().ok()
}

/// Display a numeric primitive payload (`None` = not numeric).
pub(crate) fn decode_primitive(t: i32, b: &[u8]) -> Option<String> {
    Some(match t {
        te::BOOLEAN => (b.first()? != &0).to_string(),
        te::CHAR => char::from_u32(u16::from_le_bytes(le(b)?) as u32)
            .map(|c| format!("'{c}'"))
            .unwrap_or_else(|| "?".into()),
        te::I1 => (b.first()?.cast_signed()).to_string(),
        te::U1 => b.first()?.to_string(),
        te::I2 => i16::from_le_bytes(le(b)?).to_string(),
        te::U2 => u16::from_le_bytes(le(b)?).to_string(),
        te::I4 => i32::from_le_bytes(le(b)?).to_string(),
        te::U4 => u32::from_le_bytes(le(b)?).to_string(),
        te::I8 | te::I => i64::from_le_bytes(le(b)?).to_string(),
        te::U8 | te::U => u64::from_le_bytes(le(b)?).to_string(),
        te::R4 => format!("{:.3}", f32::from_le_bytes(le(b)?)),
        te::R8 => format!("{:.3}", f64::from_le_bytes(le(b)?)),
        te::PTR => format!("{:#x}", u64::from_le_bytes(le(b)?)),
        _ => return None,
    })
}

/// Numeric reading of a payload for live sources: primitives (bool,
/// ints, floats) and small value types (enums) as integers. `None` for
/// references and wider structs.
pub(crate) fn decode_number(t: i32, b: &[u8]) -> Option<f32> {
    Some(match t {
        te::BOOLEAN => f32::from(*b.first()? != 0),
        te::I1 => f32::from(b.first()?.cast_signed()),
        te::U1 => f32::from(*b.first()?),
        te::I2 | te::CHAR => f32::from(i16::from_le_bytes(le(b)?)),
        te::U2 => f32::from(u16::from_le_bytes(le(b)?)),
        te::I4 => i32::from_le_bytes(le(b)?) as f32,
        te::U4 => u32::from_le_bytes(le(b)?) as f32,
        te::I8 | te::I => i64::from_le_bytes(le(b)?) as f32,
        te::U8 | te::U => u64::from_le_bytes(le(b)?) as f32,
        te::R4 => f32::from_le_bytes(le(b)?),
        te::R8 => f64::from_le_bytes(le(b)?) as f32,
        te::VALUETYPE => match b.len() {
            1 => f32::from(b[0]),
            2 => f32::from(i16::from_le_bytes([b[0], b[1]])),
            4 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
            8 => i64::from_le_bytes(le(b)?) as f32,
            _ => return None,
        },
        _ => return None,
    })
    .filter(|x| x.is_finite())
}

/// Display a value-type payload: known Unity structs as floats, small
/// payloads (enums, packed ints) as an integer, else leading hex.
pub(crate) fn decode_struct(class_name: &str, b: &[u8]) -> String {
    let floats = |n: usize| -> Option<String> {
        let v: Option<Vec<String>> = (0..n)
            .map(|i| le::<4>(b.get(i * 4..)?).map(|x| format!("{:.3}", f32::from_le_bytes(x))))
            .collect();
        v.map(|v| format!("({})", v.join(", ")))
    };
    let known = match class_name {
        "Vector2" => floats(2),
        "Vector3" => floats(3),
        "Vector4" | "Quaternion" | "Color" => floats(4),
        _ => None,
    };
    if let Some(s) = known {
        return s;
    }
    match b.len() {
        1 => b[0].to_string(),
        2 => i16::from_le_bytes([b[0], b[1]]).to_string(),
        4 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string(),
        8 => le::<8>(b).map(|x| i64::from_le_bytes(x).to_string()).unwrap_or_default(),
        _ => {
            let hex: Vec<String> = b.iter().take(16).map(|x| format!("{x:02x}")).collect();
            format!("[{}{}]", hex.join(" "), if b.len() > 16 { " …" } else { "" })
        }
    }
}

/// Binding path of a field, the syntax phase 3 will accept in
/// `hud.toml`: `Root/ref/ref/field`.
pub(crate) fn binding_path(root: &str, path: &[String], field: &str) -> String {
    let mut s = root.to_string();
    for seg in path.iter().map(String::as_str).chain(std::iter::once(field)) {
        s.push('/');
        s.push_str(seg);
    }
    s
}

/// One class-search hit (class pointer is process-stable metadata).
#[derive(Clone, Debug)]
pub(crate) struct ClassHit {
    pub(crate) klass: usize,
    pub(crate) full: String,
    pub(crate) image: String,
}

/// One displayed field of the opened object.
#[derive(Clone, Debug)]
pub(crate) struct FieldRow {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) value: String,
    /// Non-null object reference: the UI offers to step into it.
    pub(crate) follow: bool,
}

/// Opened object: root class instance, then a chain of reference
/// fields re-walked from that root on every (re)read.
#[derive(Clone, Debug, Default)]
pub(crate) struct OpenView {
    pub(crate) klass: usize,
    pub(crate) root: String,
    pub(crate) path: Vec<String>,
    pub(crate) instance: usize,
    pub(crate) instances: usize,
    /// Runtime class of the object reached at the end of `path`.
    pub(crate) at_class: String,
    pub(crate) fields: Vec<FieldRow>,
    pub(crate) error: Option<String>,
}

/// Render → tick explorer request (one pending at a time).
#[derive(Clone, Debug)]
pub(crate) enum ExploreReq {
    Search(String),
    Open { klass: usize, root: String, path: Vec<String>, instance: usize },
}

/// Tick → render explorer state.
#[derive(Clone, Debug, Default)]
pub(crate) struct ExploreView {
    pub(crate) hits: Vec<ClassHit>,
    pub(crate) truncated: bool,
    pub(crate) searched: Option<String>,
    pub(crate) open: Option<OpenView>,
    pub(crate) error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses_cleanly() {
        let (cfg, warnings) = parse_config(DEFAULT_CONFIG).expect("example must parse");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(cfg.fps, 30);
        assert_eq!(cfg.mode, Mode::Live);
        assert_eq!(cfg.sources.len(), 7);
        assert_eq!(cfg.huds.len(), 2);
        assert_eq!(cfg.huds[0].widgets.len(), 4);
        assert!(cfg.huds[0].enabled && !cfg.huds[1].enabled);
        let rpm = cfg.sources.iter().find(|s| s.name == "rpm").unwrap();
        assert_eq!(rpm.root, "UITachometer");
        assert_eq!(rpm.refs, vec!["m_rpmFillImage".to_string()]);
        assert_eq!(rpm.field, "m_FillAmount");
    }

    /// Every source name the example HUDs use exists in demo values,
    /// so `mode = "demo"` renders the full layout.
    #[test]
    fn demo_covers_example_sources() {
        let (cfg, _) = parse_config(DEFAULT_CONFIG).unwrap();
        let v = demo_values(1.0);
        for s in &cfg.sources {
            assert!(v.get(&s.name).is_some(), "demo lacks {}", s.name);
        }
    }

    #[test]
    fn version_mismatch_warns_but_parses() {
        let (cfg, w) = parse_config("version = 99\nmode = \"demo\"\n[[hud]]").unwrap();
        assert_eq!(cfg.huds.len(), 1);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn live_without_sources_warns() {
        let (_, w) = parse_config("[[hud]]").unwrap();
        assert_eq!(w.len(), 1, "{w:?}");
    }

    #[test]
    fn syntax_error_is_rejected() {
        assert!(parse_config("[[hud]\nname = ").is_err());
    }

    #[test]
    fn bad_widget_is_skipped_with_warning() {
        let text = r##"
[[hud]]
name = "x"
anchor = "nowhere"
  [[hud.widget]]
  type = "bar"
  [[hud.widget]]
  type = "text"
  text = "{speed}"
  color = "red"
"##;
        let (cfg, warnings) = parse_config(&format!("mode = \"demo\"\n{text}")).unwrap();
        assert_eq!(cfg.huds[0].anchor, Anchor::BottomRight);
        assert_eq!(cfg.huds[0].widgets.len(), 1);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn bar_fill_side() {
        let text = "mode = \"demo\"\n[[hud]]\n[[hud.widget]]\ntype = \"bar\"\nsource = \"rpm\"\nfill = \"right\"\n[[hud.widget]]\ntype = \"bar\"\nsource = \"rpm\"\nfill = \"up\"";
        let (cfg, w) = parse_config(text).unwrap();
        let sides: Vec<bool> = cfg.huds[0]
            .widgets
            .iter()
            .map(|w| matches!(w, Widget::Bar { from_right: true, .. }))
            .collect();
        assert_eq!(sides, vec![true, false]);
        assert_eq!(w.len(), 1, "{w:?}");
    }

    #[test]
    fn sources_forms_and_errors() {
        let text = r##"
mode = "warp"
[sources]
a = "Root/field"
b = { path = "Root/ref/field", scale = 3.6, offset = -1 }
c = "nofield"
d = 4
"##;
        let (cfg, w) = parse_config(text).unwrap();
        assert_eq!(cfg.mode, Mode::Live);
        assert_eq!(cfg.sources.len(), 2);
        let b = cfg.sources.iter().find(|s| s.name == "b").unwrap();
        assert_eq!((b.scale, b.offset), (3.6, -1.0));
        assert_eq!(b.refs, vec!["ref".to_string()]);
        assert_eq!(w.len(), 3, "{w:?}");
    }

    #[test]
    fn integers_accepted_where_floats_expected() {
        let (cfg, w) = parse_config("mode = \"demo\"\n[[hud]]\noffset = [-10, 5]\nscale = 2").unwrap();
        assert!(w.is_empty());
        assert_eq!(cfg.huds[0].offset, [-10.0, 5.0]);
        assert_eq!(cfg.huds[0].scale, 2.0);
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#FF0000"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(
            parse_color("#FFFFFF80"),
            Some(Color32::from_rgba_unmultiplied(255, 255, 255, 128))
        );
        assert_eq!(parse_color("FF0000"), None);
        assert_eq!(parse_color("#FFF"), None);
        assert_eq!(parse_color("#ééé"), None);
    }

    #[test]
    fn template_and_gear() {
        let mut v = Values::default();
        v.set("speed", 123.6);
        v.set("rpm", 0.4567);
        v.set("gear", 3.0);
        assert_eq!(format_template("{gear} {speed} {rpm:2}", &v), "3 124 0.46");
        assert_eq!(format_template("{speed_mph}", &v), "77");
        assert_eq!(format_template("{gear:1}", &v), "3.0");
        assert_eq!(format_template("{boost} x", &v), "{boost} x");
        assert_eq!(format_template("open {speed", &v), "open {speed");
        assert_eq!(format_template("km/h", &v), "km/h");
        assert_eq!(gear_label(-1), "R");
        assert_eq!(gear_label(0), "N");
    }

    #[test]
    fn bounds_eval() {
        let mut v = Values::default();
        v.set("rpm_max", 0.9);
        assert_eq!(Bound::Num(2.0).eval(&v), Some(2.0));
        assert_eq!(Bound::Src("rpm_max".into()).eval(&v), Some(0.9));
        assert_eq!(Bound::Src("nope".into()).eval(&v), None);
    }

    #[test]
    fn fraction_is_bounded() {
        assert_eq!(bar_fraction(4000.0, 0.0, 8000.0), 0.5);
        assert_eq!(bar_fraction(9000.0, 0.0, 8000.0), 1.0);
        assert_eq!(bar_fraction(-5.0, 0.0, 8000.0), 0.0);
        assert_eq!(bar_fraction(5.0, 10.0, 10.0), 0.0);
        assert_eq!(bar_fraction(f32::NAN, 0.0, 1.0), 0.0);
    }

    #[test]
    fn primitives_decode() {
        assert_eq!(decode_primitive(te::R4, &1.5f32.to_le_bytes()).unwrap(), "1.500");
        assert_eq!(decode_primitive(te::I4, &(-3i32).to_le_bytes()).unwrap(), "-3");
        assert_eq!(decode_primitive(te::BOOLEAN, &[1]).unwrap(), "true");
        assert_eq!(decode_primitive(te::I1, &[0xff]).unwrap(), "-1");
        assert_eq!(decode_primitive(te::R8, &[0; 2]), None);
        assert_eq!(decode_primitive(te::CLASS, &[0; 8]), None);
        assert_eq!(primitive_size(te::R4), Some(4));
        assert_eq!(primitive_size(te::VALUETYPE), None);
        assert!(is_reference(te::STRING) && !is_reference(te::R4));
    }

    #[test]
    fn numbers_decode() {
        assert_eq!(decode_number(te::R4, &2.5f32.to_le_bytes()), Some(2.5));
        assert_eq!(decode_number(te::I4, &(-1i32).to_le_bytes()), Some(-1.0));
        assert_eq!(decode_number(te::BOOLEAN, &[1]), Some(1.0));
        assert_eq!(decode_number(te::R8, &3.0f64.to_le_bytes()), Some(3.0));
        assert_eq!(decode_number(te::VALUETYPE, &4i32.to_le_bytes()), Some(4.0));
        assert_eq!(decode_number(te::VALUETYPE, &[1, 0]), Some(1.0));
        assert_eq!(decode_number(te::VALUETYPE, &[0; 12]), None);
        assert_eq!(decode_number(te::STRING, &[0; 8]), None);
    }

    #[test]
    fn structs_decode() {
        let mut b = Vec::new();
        for f in [1.0f32, 2.0, 3.0] {
            b.extend_from_slice(&f.to_le_bytes());
        }
        assert_eq!(decode_struct("Vector3", &b), "(1.000, 2.000, 3.000)");
        assert_eq!(decode_struct("Gear", &4i32.to_le_bytes()), "4");
        let big = decode_struct("Big", &[0xab; 20]);
        assert!(big.starts_with("[ab ab") && big.ends_with(" …]"), "{big}");
        assert_eq!(big.matches("ab").count(), 16);
        assert_eq!(decode_struct("Vector3", &[0; 4]), "0");
    }

    #[test]
    fn binding_paths() {
        assert_eq!(binding_path("UITach", &[], "rpm"), "UITach/rpm");
        assert_eq!(
            binding_path("UITach", &["car".into(), "engine".into()], "rpm"),
            "UITach/car/engine/rpm"
        );
        let s = SourceDef::from_path("x", &binding_path("UITach", &["car".into()], "rpm"), 1.0, 0.0).unwrap();
        assert_eq!((s.root.as_str(), s.refs.len(), s.field.as_str()), ("UITach", 1, "rpm"));
        assert!(SourceDef::from_path("x", "A//b", 1.0, 0.0).is_none());
    }

    #[test]
    fn demo_loops_through_gears() {
        assert_eq!(demo_values(0.0).get("gear"), Some(1.0));
        assert_eq!(demo_values(1.6).get("gear"), Some(2.0));
        assert_eq!(demo_values(9.5).get("gear"), Some(0.0));
        assert_eq!(demo_values(10.5).get("gear"), Some(1.0));
        for i in 0..200 {
            let v = demo_values(i as f64 * 0.07);
            let (rpm, lo, hi) = (v.get("rpm").unwrap(), v.get("rpm_min").unwrap(), v.get("rpm_max").unwrap());
            assert!((lo..=hi).contains(&rpm));
            assert!((0.0..=250.0).contains(&v.get("speed").unwrap()));
        }
        assert_eq!(demo_values(f64::NAN).get("gear"), Some(1.0));
    }
}
