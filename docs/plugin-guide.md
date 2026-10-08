# Plugin guide — écrire un mod CXSFM

Un mod = **un dossier** sous `src/mods/`, compilé dans le framework
`.so` (aucun fichier externe injecté). Exemples à copier :

| Dossier | Opération démontrée |
|---|---|
| `hud_scout/` | inspecter (read-only, zéro écriture) |
| `hud_hide/` | **masquer** un élément (`Behaviour.enabled = false` + restore) |
| `hud_text/` | **remplacer** un texte (drift-repair + restore) ; contenu vide = **masquer** le texte |
| `inspector/` | outil : arbre de scène (roots + hiérarchie + position, snapshot throttlé) |
| `gamelog/` | outil : `Player.log` du jeu en direct (filtre + erreurs, lecture seule) |
| `video/` | PARKÉ : setters vidéo = crash jeu (lecture OK, écriture à isoler) |

## 1. Copier un dossier

```bash
cp -r src/mods/hud_hide src/mods/mon_mod
```

## 2. Implémenter le trait `Mod` (`mod.rs`)

```rust
use crate::mods::api::{Mod, TileIcon};

pub struct MonMod { /* atomics + Mutex, jamais de &mut partagé */ }

impl Mod for MonMod {
    fn name(&self) -> &'static str { "Mon Mod" }       // unique !
    fn menu_label_key(&self) -> &'static str { "mod_mon" } // clé stable
    fn menu_icon(&self) -> TileIcon { TileIcon::Tag }
    fn describe_in(&self, lang: &str) -> &'static str {
        self::i18n::describe(lang)                     // page About
    }
    fn on_update(&mut self, dt: f32) { /* ~60 Hz, rapide, jamais bloquant */ }
    fn on_enable(&mut self) { /* armer : drop handles, pending = true */ }
    fn on_disable(&mut self) { /* RESTAURER l'état exact du jeu */ }
}
```

Règles Unity (prouvées en jeu, ne pas improviser) :

- thread appelant déjà attaché (tick + render) ; `domain_checked` avant usage ;
- classes/méthodes cachées (`UnityCache`), **objets jamais cachés** —
  handles en `usize`, re-résolus à chaque usage, validés par lecture ;
- cadences : recherche ~0.2 Hz sans handle, vérif ~1 Hz avec handle,
  `pending` pour agir dès le tick suivant l'armement ;
- **jamais d'écriture aveugle** : capturer l'original → écrire →
  relire (read-back) → restaurer sur `on_disable` ;
- verrous : binder le snapshot (`let cached = ...`) avant le `match`
  (self-deadlock sinon) ; voir `hud_hide::maintain`.

## 3. Textes du mod (`i18n.rs`, 7 langues, fallback EN)

```rust
pub fn tile_label(lang_code: &str) -> &'static str { match lang_code {
    "fr" => "Mon Mod", /* de es it pt ru */ _ => "My Mod",
}}
pub fn describe(lang_code: &str) -> &'static str { /* idem */ }
```

Puis **une ligne** dans `src/i18n/mod.rs::mod_tile` :

```rust
"mod_mon" => Some(crate::mods::mon_mod::i18n::tile_label(code)),
```

Le menu ne connaît que la clé stable ; chaque lettre displayed vient
du dossier du mod. Les fichiers framework (`en.rs`, `fr.rs`, …) ne
sont jamais touchés pour un mod.

## 4. Enregistrer (2 lignes)

`src/mods.rs` : `pub mod mon_mod;`
`src/lib.rs` (registrations) : `mods::mon_mod::register();`

```rust
pub fn register() {
    crate::mod_api::register_mod(Box::new(MonMod::new())); // boot désarmé
}
```

## 5. Isolation — ce qui est garanti

Chaque hook tourne sous `catch_unwind` : panique → mod mis en
quarantaine (auto-désactivé, loggé), menu + autres mods vivants.
Le verrou registre n'est jamais tenu pendant le code mod ; les mutex
empoisonnés sont récupérés. Un mod ne peut pas spoof le menu
(contexte egui non partagé avec la fenêtre).
