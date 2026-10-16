# AGENTS.md — CXSFM (CarX Street Framework Mod)

> Framework de mods Rust (Edition 2024) pour CarX Street : couche Vulkan
> implicite, overlay egui 0.36, pont IL2CPP. Mods cosmétiques/interface/
> caméra uniquement — jamais de gameplay, d'économie ni de physique.

## Commandes

```bash
nix develop --command cargo build --release  # build (rustc 1.98 via flake)
nix develop --command bash -c 'cargo test --lib'
./tools/cxsfm-setup.sh                       # stage le layer après chaque rebuild
```

## Architecture (10 lignes)

- `src/layer.rs` — couche Vulkan implicite : le loader nous `dlopen`, on
  route `vkQueuePresentKHR` (négociation, link nodes, `advance_link`).
- `src/render/overlay.rs` — rendu egui dans les images swapchain au moment
  du present (render pass `loadOp=LOAD`, 1 frame in-flight, replay par image).
- `src/render/vk.rs` — LEGACY : GOT/inline patching (le layer est le chemin
  retenu ; ne pas étendre sans accord).
- `src/il2cpp.rs` — pont IL2CPP (`resolve`, `invoke`/`invoke_void`,
  `domain_checked`, classes/méthodes cachées, objets re-résolus à l'usage).
- `src/unity/` — helpers (`object_alive` via `m_CachedPtr`, camera, scene,
  text/TMP, transforms) + `UnityCache` des méthodes résolues.
- `src/mods/{api,loader,tool,…}` — trait `Mod`, registre avec quarantaine
  au panic, chrome fenêtres (pin/opacité), mods livrés
  (`inspector`, `gamelog`, `speedo`, `fpv_camera` parqué).
- `src/mainthread.rs` — infra d'échange `MethodInfo.methodPointer`
  (phase 1 : sondes, aucun mod migré).
- `src/hotkey.rs` — F8 = visibilité UI + capture (SDL + X11 ; chemin Unity
  coupé : il segfaultait au teardown).
- `src/lib.rs` — `#[ctor]` → thread init différé → tick ~60 Hz
  (`on_update`), jamais d'IL2CPP pendant le teardown.
- `src/menu.rs` + `src/ui/` — menu tuiles + thème CarX (panneaux noirs,
  accent jaune `#F5C518`, coins 3px, icônes Material Symbols).

## Règles prouvées en jeu (docs/field-notes.md — LIRE avant de modifier)

- **bind-then-match** sur les verrous : jamais de `match` direct sur un
  garde si un bras peut re-locker (self-deadlock du tick, incident 2026-10-06).
- **`invoke_void` pour les setters** : le runtime rend NULL au succès —
  `invoke` dessus = échec silencieux alors que l'écriture a eu lieu.
- **`object_alive` avant TOUT accès Unity** (lecture comme écriture) ;
  pointeurs stockés interdits — re-résoudre par chemin/nom à chaque usage.
- **Aucun appel Unity depuis le thread de present** ; le tick saute tout
  IL2CPP pendant le teardown (domaine tué avant Vulkan).
- **Pas d'arithmétique non bornée sur le chemin de rendu** (`checked`/
  saturant — un panic dans `extern "C"` = abort).
- **`#[repr(C)]` sur les structs FFI** (ex. `VkPresentInfoKHR` local).
- Textures : un free est obsolète si son id est (re-)set ; `set_texture` /
  `free_texture` par item ; jamais de `vkQueueWaitIdle`, timeouts bornés.
- Handles Vulkan suivis jusqu'à destruction (instances/devices/swapchains).

## Contraintes

- Binaire final : symboles **max `GLIBC_2.35`** (Steam Runtime sniper =
  2.36) — vérifier après chaque bump toolchain :
  `objdump -T target/release/libcxsfm.so | grep -oP "GLIBC_[0-9.]+"`.
- **Jamais** de modification gameplay/économie/physique/réseau.
- Mods **désarmés au boot**, restore à la désactivation — sauf
  speedo/map : leurs modifs restent à la désactivation (demande
  utilisateur), retour aux originaux via le bouton Restaurer.
- Un changement = un commit, message clair, un seul sujet.
- Pas de nouvelle dépendance sans accord (pure Rust uniquement).
- `cargo test` ne prouve rien en jeu : indiquer ce qui reste à valider.

## Test en jeu

1. Rebuild + `./tools/cxsfm-setup.sh`, vider `~/.cache/cxsfm/cxsfm.log`.
2. Lancer (options Steam : `VK_ADD_LAYER_PATH="$HOME/.cache/cxsfm"
   VK_INSTANCE_LAYERS=VK_LAYER_CXSFM_overlay %command%`).
3. Attendu : `negotiated` → `render hook installed: Vulkan` →
   `first present intercepted` → ligne `input: ...` sans appui touche.
4. F8 → `hotkey F8 (...)` ; scénarios : course, garage↔ville, quit propre.
5. Après quit : `grep -a -c SIGSEGV Player.log` doit rester muet sur cxsfm.
