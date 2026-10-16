//! HUD layouts (TOML) + live values + pure helpers. No Unity, no IO:
//! everything here is unit-tested.
//!
//! A config holds any number of `[[hud]]` blocks; each HUD is a box
//! anchored on the screen and drawn from `[[hud.widget]]` entries
//! (`bar` bound to a value, `text` with `{placeholders}`). Parsing is
//! tolerant like the save files: a bad widget/key is skipped with a
//! warning, only a TOML syntax error rejects the whole file (the last
//! good layout then stays on screen).

use egui::Color32;

/// Schema version written in the example file.
pub(crate) const SCHEMA_VERSION: i64 = 1;

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

/// A game value a widget can bind to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Source {
    Speed,
    SpeedMph,
    Rpm,
    Gear,
}

impl Source {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "speed" => Source::Speed,
            "speed_mph" => Source::SpeedMph,
            "rpm" => Source::Rpm,
            "gear" => Source::Gear,
            _ => return None,
        })
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
    /// Horizontal fill bar (RPM sweep, speed…). `segments = 0` is a
    /// continuous fill; the fill turns `redline_color` past `redline`.
    Bar {
        source: Source,
        min: f32,
        max: f32,
        redline: Option<f32>,
        pos: [f32; 2],
        size: [f32; 2],
        segments: u32,
        color: Color32,
        redline_color: Color32,
        track_color: Color32,
    },
    /// Text with `{speed}` `{speed_mph}` `{rpm}` `{gear}` placeholders.
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
    pub(crate) widgets: Vec<Widget>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HudConfig {
    pub(crate) fps: u32,
    pub(crate) huds: Vec<Hud>,
}

/// One frame of game values (km/h, rev/min, gear: -1 = R, 0 = N).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Values {
    pub(crate) speed_kmh: f32,
    pub(crate) rpm: f32,
    pub(crate) gear: i32,
}

impl Values {
    pub(crate) fn get(self, s: Source) -> f32 {
        match s {
            Source::Speed => self.speed_kmh,
            Source::SpeedMph => self.speed_kmh * 0.621_371,
            Source::Rpm => self.rpm,
            Source::Gear => self.gear as f32,
        }
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

/// Fill placeholders; unknown `{…}` stays literal (visible typo).
pub(crate) fn format_template(t: &str, v: &Values) -> String {
    if !t.contains('{') {
        return t.to_string();
    }
    t.replace("{speed_mph}", &format!("{:.0}", v.get(Source::SpeedMph).max(0.0)))
        .replace("{speed}", &format!("{:.0}", v.speed_kmh.max(0.0)))
        .replace("{rpm}", &format!("{:.0}", v.rpm.max(0.0)))
        .replace("{gear}", &gear_label(v.gear))
}

/// Fill fraction 0..=1 (degenerate ranges and NaN read as empty).
pub(crate) fn bar_fraction(v: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if !(span > 0.0) || !v.is_finite() {
        return 0.0;
    }
    ((v - min) / span).clamp(0.0, 1.0)
}

/// Phase-1 value source: a looping launch (6 upshifts, then a short
/// idle at N). Pure function of time so the render thread computes it
/// without any shared state.
pub(crate) fn demo_values(t: f64) -> Values {
    const GEAR_S: f64 = 1.5;
    const GEARS: i32 = 6;
    const IDLE_S: f64 = 1.5;
    const TOPS: [f32; 6] = [60.0, 95.0, 130.0, 165.0, 200.0, 240.0];
    let period = GEAR_S * GEARS as f64 + IDLE_S;
    let p = if t.is_finite() { t.rem_euclid(period) } else { 0.0 };
    if p >= GEAR_S * GEARS as f64 {
        return Values { speed_kmh: 0.0, rpm: 900.0, gear: 0 };
    }
    let idx = (p / GEAR_S) as usize;
    let k = ((p - idx as f64 * GEAR_S) / GEAR_S) as f32;
    let rpm = 3000.0 + 4500.0 * k;
    Values {
        speed_kmh: TOPS[idx.min(5)] * rpm / 7500.0,
        rpm,
        gear: idx as i32 + 1,
    }
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
}

fn parse_widget(t: &toml::Table, ctx: String, warnings: &mut Vec<String>) -> Option<Widget> {
    let mut f = Fields { t, ctx, warnings };
    let kind = f.str("type", "");
    match kind.as_str() {
        "bar" => {
            let src = f.str("source", "");
            let source = match Source::parse(&src) {
                Some(s) => s,
                None => {
                    f.warnings.push(format!("{}: source `{src}` inconnue, widget ignoré", f.ctx));
                    return None;
                }
            };
            let redline = match f.t.get("redline") {
                None => None,
                Some(v) => match num(v) {
                    Some(x) => Some(x),
                    None => {
                        f.warn("redline");
                        None
                    }
                },
            };
            Some(Widget::Bar {
                source,
                min: f.f32("min", 0.0),
                max: f.f32("max", 8000.0),
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
    Ok((HudConfig { fps, huds }, warnings))
}

/// Example written on first enable when no `hud.toml` exists.
pub(crate) const DEFAULT_CONFIG: &str = r##"# CXSFM — HUD personnalisés (rechargé à chaud : enregistre, regarde en jeu).
#
# Plusieurs [[hud]] possibles. Chaque HUD est une boîte `size` (avant
# `scale`) posée sur un coin/bord de l'écran (`anchor` : top_left, top,
# top_right, left, center, right, bottom_left, bottom, bottom_right),
# décalée de `offset` (px). Les widgets se placent DANS la boîte (`pos`).
#
# Widgets :
#   type = "bar"  : source = "rpm" | "speed" | "speed_mph" | "gear",
#                   min, max, redline (optionnel), segments (0 = continu),
#                   color, redline_color, track_color
#   type = "text" : text avec {speed} {speed_mph} {rpm} {gear},
#                   size, align = "left" | "center" | "right", mono, color
# Couleurs : "#RRGGBB" ou "#RRGGBBAA".

version = 1
fps = 30

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
  min = 0
  max = 8000
  redline = 7000
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
  max = 8000
  redline = 7000
  segments = 20
  pos = [8.0, 6.0]
  size = [224.0, 8.0]

  [[hud.widget]]
  type = "text"
  text = "{gear}  ·  {speed} km/h"
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
        assert_eq!(cfg.huds.len(), 2);
        assert_eq!(cfg.huds[0].widgets.len(), 4);
        assert!(cfg.huds[0].enabled && !cfg.huds[1].enabled);
    }

    #[test]
    fn version_mismatch_warns_but_parses() {
        let (cfg, w) = parse_config("version = 99\n[[hud]]").unwrap();
        assert_eq!(cfg.huds.len(), 1);
        assert_eq!(w.len(), 1);
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
  source = "boost"
  [[hud.widget]]
  type = "text"
  text = "{speed}"
  color = "red"
"##;
        let (cfg, warnings) = parse_config(text).unwrap();
        assert_eq!(cfg.huds[0].anchor, Anchor::BottomRight);
        assert_eq!(cfg.huds[0].widgets.len(), 1);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn integers_accepted_where_floats_expected() {
        let (cfg, w) = parse_config("[[hud]]\noffset = [-10, 5]\nscale = 2").unwrap();
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
        let v = Values { speed_kmh: 123.6, rpm: 6512.2, gear: 3 };
        assert_eq!(format_template("{gear} {speed} {rpm}", &v), "3 124 6512");
        assert_eq!(format_template("{speed_mph}", &v), "77");
        assert_eq!(format_template("{boost}", &v), "{boost}");
        assert_eq!(gear_label(-1), "R");
        assert_eq!(gear_label(0), "N");
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
    }

    #[test]
    fn demo_loops_through_gears() {
        assert_eq!(demo_values(0.0).gear, 1);
        assert_eq!(demo_values(1.6).gear, 2);
        assert_eq!(demo_values(9.5).gear, 0);
        assert_eq!(demo_values(10.5).gear, 1);
        for i in 0..200 {
            let v = demo_values(i as f64 * 0.07);
            assert!((0.0..=8000.0).contains(&v.rpm));
            assert!((0.0..=250.0).contains(&v.speed_kmh));
        }
        assert_eq!(demo_values(f64::NAN).gear, 1);
    }
}
