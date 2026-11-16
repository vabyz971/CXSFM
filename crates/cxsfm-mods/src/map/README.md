# Minimap mod (map)

Same Inspector-style interface as [speedo](../speedo/README.md),
applied to the game's minimap HUD — cosmetic only. Roots and
vocabulary come from wyr-menu v0.2 (`minhud.so` strings):
`UIHUDMinimap` → `Map/...` (`ImgFrame`, `Mask/Base/RoadImage`,
`PlayerIconContainer`, `NorthIcon`…), hide via `SetActive`, opacity
via `CanvasGroup`.

![Minimap window in game: 179 bound objects, ImgFrame selected](Image.png)

## What it does

Walks `UIHUDMinimap` and lists **everything** under it — 179 objects
in the screenshot, including sections the dashboard walk would miss:
`Fuel (25)`, `TireWear (9)`, `GlobalMapBtn`, `RadialMenuBtn`,
`Map (124)` with icons, markers, texts (inactive ones flagged `OFF`).
Move, resize, recolor, fade, hide or deactivate any of them; the
minimap bottom-left updates live.

## How to use

Identical to speedo: enable the **Map** tile, Refresh after scene
changes, edit cards in realtime, Restore (or disable) to write all
originals back. **Save** writes modified params to
`~/.config/cxsfm/saves.toml`, **Load** applies them back in one pass.
See [speedo's README](../speedo/README.md) for the card layout, the
realtime drain and the safety design — this mod shares its data model
(`speedometer::model`) and behavior, only the roots (`UIHUDMinimap`,
fallback `Map`) and the tile (globe, `mod_map`) differ.

Values persist per object path in `~/.config/cxsfm/saves.toml`
(Save button), re-applied with Load — or automatically a few
seconds after arriving in game when the Settings toggle is on.

## Map specifics: roads, opacity, color

- **Roads are `RawImage`** (dynamic textures — a sibling of `Image`
  that `GetComponent(Image)` never matches, confirmed in wyr's
  binary too). The mod probes `Image`, `RawImage`, TMP in order.
- **Opacity** (wyr's `map_*_opacity`): every entry shows the slider —
  own `CanvasGroup` first, else the nearest ancestor group (climb
  bounded so it never leaves the walked subtree), else a clear skip
  in the log. Whole-subtree fade like wyr, no `AddComponent` (we use
  existing groups only).
- **Color**: honest limitation — road traces use a custom material
  that ignores vertex color (wyr itself only ships opacity keys for
  map layers, no color). A write that the game swallows is logged
  once per object (`couleur ignorée sur …`), proven by immediate
  read-back rather than assumed.

See `docs/field-notes.md` (WYR-MENU + MINIMAP sections) for roots,
hide/opacity mechanisms and camera names reserved for the FPV future.
