//! Speedo tick-side Unity work: resolve, discover, drain, restore.
//!
//! Runs on the framework tick thread (~60 Hz) with an attached IL2CPP
//! domain. Every handle is alive-checked before any invoke; stale
//! addresses are never dereferenced (scene churn frees wrappers —
//! touching one is the delayed SIGSEGV this design was built to avoid).

use crate::mods::speedo::model::{
    CompKind, ObjSnap, OrigRec, OP_ACTIVE, OP_CGALPHA, OP_COLOR, OP_FONT, OP_POS, OP_ROT, OP_SCALE,
};
use super::MinimapMod;
use crate::mods::common::lock;
use crate::unity::UnityCache;
use std::sync::atomic::Ordering;

/// Retry cadence (ticks, >= 5 s at 60 Hz) for a failed cache init
/// (game loading). Forced on arming / Refresh / Restore / disable.
const RESOLVE_EVERY_TICKS: u64 = 300;

/// Minimap root every map object hangs under (verified via wyr-menu
/// v0.2 binary strings): `UIHUDMinimap` → `Map/...` (`ImgFrame`,
/// `Mask/Base/...`, `PlayerIconContainer`, `NorthIcon`…).
const DASH_ROOT: &str = "UIHUDMinimap";
/// Fallback roots when the dashboard itself is missing (other scenes
/// may mount sections directly). Same names wyr's minhud searches —
/// confirmed present in its binary strings alongside ours.
const FALLBACK_ROOTS: &[&str] = &["Map"];
/// Subtree walk caps (explicit Refresh only — never periodic).
const WALK_MAX_DEPTH: usize = 8;
const WALK_MAX_CHILDREN: usize = 64;
const WALK_MAX_NODES: usize = 512;

/// One walked map node: slash path under [`DASH_ROOT`]
/// (e.g. `Map/Mask/Base/RoadImage`) + graphic flavour, if any.
/// Paths come from the live walk — never guessed, so exact spellings
/// (`ImgBgGlow (1)`, `Text (TMP) Gear`…) are the game's own.
struct Want {
    path: String,
    comp: Option<CompKind>,
}

impl MinimapMod {
    /// Resolve the method/field cache once, then serve from memory.
    /// The hot path (cache present) performs zero Unity calls. A
    /// missing cache attempts resolution at most every
    /// `RESOLVE_EVERY_TICKS` unless `force`.
    fn ensure_unity(&self, tick: u64, force: bool) -> bool {
        if lock(&self.unity).is_some() {
            return true;
        }
        if !force {
            let last = self.last_init.load(Ordering::SeqCst);
            if last != 0 && tick.saturating_sub(last) < RESOLVE_EVERY_TICKS {
                return false;
            }
        }
        self.last_init.store(tick.saturating_add(1), Ordering::SeqCst);
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return false,
        };
        // SAFETY: attached framework threads, null-checked chain.
        let cache = unsafe {
            let domain = match crate::il2cpp::domain_checked(api) {
                Some(d) => d,
                None => return false,
            };
            match crate::unity::init(api, domain) {
                Some(c) => c,
                None => return false,
            }
        };
        *lock(&self.unity) = Some(cache);
        true
    }

    /// First graphic component on a GameObject (HUD leaves carry
    /// one); containers get `(None, 0)`.
    ///
    /// # Safety
    /// Attached tick thread; alive-checked before use, consumed
    /// immediately — same contract as [`walk_dashboard`].
    unsafe fn probe_components(
        api: &crate::il2cpp::Il2cppApi,
        cache: &UnityCache,
        go: *mut std::ffi::c_void,
        pairs: &[(*mut std::ffi::c_void, CompKind); 2],
    ) -> (Option<CompKind>, usize) {
        // SAFETY: see above; a dead component unbinds silently.
        unsafe {
            for (klass, kind) in pairs {
                if klass.is_null() {
                    continue;
                }
                let comp = match crate::unity::go_get_component(api, cache, go, *klass) {
                    Some(c) => c,
                    None => continue,
                };
                if comp.is_null() || crate::unity::object_alive(api, cache, comp) != Some(true)
                {
                    continue;
                }
                return (Some(*kind), comp as usize);
            }
            (None, 0)
        }
    }

    /// Walk the dashboard once, collecting EVERY descendant node —
    /// containers (`Map/Mask`, `Map/PlayerIconContainer`…) as well as graphic
    /// leaves. Containers get `comp: None` (transform + active only).
    /// Roots tried in order: [`DASH_ROOT`], then [`FALLBACK_ROOTS`]
    /// (authoritative first — no double walk when the dashboard mounts).
    /// Returns `(candidates, total)`.
    ///
    /// # Safety
    /// Attached framework thread; every handle alive-checked before any
    /// further invoke, consumed immediately, nothing stored but data.
    unsafe fn walk_dashboard(
        api: &crate::il2cpp::Il2cppApi,
        cache: &UnityCache,
    ) -> (Vec<(Want, usize, usize)>, usize) {
        // SAFETY: see above; a dead intermediate unbinds silently.
        unsafe {
            let mut cands: Vec<(Want, usize, usize)> = Vec::new();
            let pairs = [
                (cache.image_klass, CompKind::Image),
                (cache.tmp_klass, CompKind::Tmp),
            ];
            if pairs.iter().all(|(k, _)| k.is_null()) {
                return (cands, 0);
            }
            // Bind one root object by name (dashboard, else fallbacks).
            // Returns its transform, or logs once and gives up.
            let mut roots: Vec<(String, *mut std::ffi::c_void)> = Vec::new();
            // Local helper body inlined per candidate (no closures over
            // the unsafe walk — keeps the SAFETY story auditable).
            for name in std::iter::once(DASH_ROOT).chain(FALLBACK_ROOTS.iter().copied()) {
                let go = match crate::unity::go_find(api, cache, name) {
                    Some(g) => g,
                    None => continue,
                };
                if go.is_null() || crate::unity::object_alive(api, cache, go) != Some(true) {
                    continue;
                }
                let tr = match crate::unity::go_transform(api, cache, go) {
                    Some(t)
                        if !t.is_null()
                            && crate::unity::object_alive(api, cache, t) == Some(true) =>
                    {
                        t
                    }
                    _ => continue,
                };
                // The root node itself is an entry (ultimate parent,
                // selectable like any container).
                let (kind, addr) = Self::probe_components(api, cache, go, &pairs);
                cands.push((
                    Want {
                        path: name.to_string(),
                        comp: kind,
                    },
                    addr,
                    go as usize,
                ));
                roots.push((name.to_string(), tr));
                if name == DASH_ROOT {
                    // Authoritative — sections below cover the rest.
                    break;
                }
            }
            if roots.is_empty() {
                crate::log_line(
                    "map: ni UIHUDMinimap ni Map trouvés (Refresh hors course ?)",
                );
                return (cands, 0);
            }
            if roots.len() > 1 || roots[0].0 != DASH_ROOT {
                let names: Vec<&str> = roots.iter().map(|(n, _)| n.as_str()).collect();
                crate::log_line(&format!("map: racines fallback [{}]", names.join(", ")));
            }
            // Iterative DFS per root: (path-prefix, transform, depth).
            // Minimap children stay relative (`Map/...`);
            // fallback roots carry their own name (same shape, so one
            // resolver serves both).
            for (root_name, root_tr) in roots {
                let prefix = if root_name == DASH_ROOT {
                    String::new()
                } else {
                    root_name.clone()
                };
                let mut stack = vec![(prefix, root_tr, 0usize)];
            while let Some((prefix, tr, depth)) = stack.pop() {
                if cands.len() >= WALK_MAX_NODES || depth >= WALK_MAX_DEPTH {
                    continue;
                }
                let n = crate::unity::tr_child_count(api, cache, tr).min(WALK_MAX_CHILDREN);
                for i in 0..n {
                    let child = match crate::unity::tr_child(api, cache, tr, i as i32) {
                        Some(c) => c,
                        None => continue,
                    };
                    if crate::unity::object_alive(api, cache, child) != Some(true) {
                        continue;
                    }
                    let go = match crate::unity::component_gameobject(api, cache, child) {
                        Some(g) => g,
                        None => continue,
                    };
                    if crate::unity::object_alive(api, cache, go) != Some(true) {
                        continue;
                    }
                    let name = crate::unity::object_name(api, cache, go);
                    let path = if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{prefix}/{name}")
                    };
                    // First graphic component wins (HUD leaves carry one);
                    // containers keep `comp: None`.
                    let (comp_kind, comp_addr) =
                        Self::probe_components(api, cache, go, &pairs);
                    cands.push((
                        Want {
                            path: path.clone(),
                            comp: comp_kind,
                        },
                        comp_addr,
                        go as usize,
                    ));
                    stack.push((path, child, depth + 1));
                }
            }
            }
            let total = cands.len();
            (cands, total)
        }
    }

    /// One-shot discovery (tick thread, explicit trigger only):
    /// [`walk_dashboard`] binds every dashboard component, then one
    /// snapshot read per match, first-seen originals recorded, snapshot
    /// published. No periodic work — the game owns the values afterwards.
    /// A missing target stays missing until the next manual Refresh
    /// (status n/m).
    pub(crate) fn discover(&self) {
        let tick = self.tick.load(Ordering::SeqCst);
        if !self.ensure_unity(tick, true) {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = lock(&self.unity).as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        let t0 = std::time::Instant::now();
        // Bind: resolve MAP root once, then walk its subtree.
        // `Transform` children enumerate INACTIVE branches too
        // (`Numbers`, `ElectroBooster`, `CoopBooster`… are OFF in-game)
        // — something `GameObject.Find` can never see. Bounded walk
        // (depth / children / nodes), explicit Refresh only. A dead
        // intermediate unbinds silently.
        // SAFETY: attached tick thread; every handle alive-checked
        // before any further invoke, consumed immediately.
        let (found, total): (Vec<(Want, usize, usize)>, usize) =
            unsafe { Self::walk_dashboard(api, &cache) };
        // Read one snapshot per match. Originals accumulate locally
        // and merge under a brief lock AFTER the Unity calls — no
        // Mutex is ever held across an invoke (a slow call would
        // stall the present thread into skipping this window).
        // SAFETY: same contract; transform derived from the live owner.
        // NOTE: no screen projection here. The old overlay markers
        // projected each object once at discovery, but the anchor went
        // stale as soon as the camera moved — dragging a marker then
        // wrote the object to the wrong place on Apply. All edits go
        // through the panel buffers now (LOCAL space, camera-proof).
        let mut snaps = Vec::new();
        let mut fresh_origs = Vec::new();
        // Addresses already recorded (merge skips them). The guard
        // ends here (last use) — well before any Unity call below.
        // Raw addresses (0 = container without graphic component);
        // dedupe only, never dereferenced.
        let known = lock(&self.origs);
        let mut seen: Vec<(usize, usize)> =
            known.iter().map(|r| (r.tr, r.comp.unwrap_or(0))).collect();
        drop(known);
            unsafe {
                for (want, comp_addr, go_addr) in found {
                    // Owner game object comes from the bind step above;
                    // re-probed here (a churn between bind and read
                    // frees it), then its transform.
                    let go = go_addr as *mut std::ffi::c_void;
                    match crate::unity::object_alive(api, &cache, go) {
                        Some(true) => {}
                        _ => continue,
                    }
                    let tr = match crate::unity::go_transform(api, &cache, go) {
                        Some(t) => t,
                        None => continue,
                    };
                if tr.is_null() {
                    continue;
                }
                match crate::unity::object_alive(api, &cache, tr) {
                    Some(true) => {}
                    _ => continue,
                }
                let active = crate::unity::go_active(api, &cache, go).unwrap_or(true);
                let scale = crate::unity::tr_get_scale(api, &cache, tr).unwrap_or([1.0, 1.0, 1.0]);
                // LOCAL transforms: what the Canvas keeps (world-space
                // writes get discarded on every layout rebuild).
                let pos = crate::unity::tr_get_local_position(api, &cache, tr)
                    .unwrap_or([0.0, 0.0, 0.0]);
                let euler = crate::unity::tr_get_local_euler(api, &cache, tr)
                    .unwrap_or([0.0, 0.0, 0.0]);
                // Graphic reads only when the walk found a component
                // (containers skip straight to the snapshot push).
                let comp_p = comp_addr as *mut std::ffi::c_void;
                let color = if want.comp.is_some() {
                    crate::unity::graphic_get_color(api, &cache, comp_p)
                        .unwrap_or([1.0, 1.0, 1.0, 1.0])
                } else {
                    [1.0, 1.0, 1.0, 1.0]
                };
                // Opacity: CanvasGroup alpha when present (whole
                // subtree), else the color alpha for graphics, else N/A.
                // `go_get_component` already degrades on a null klass.
                let cg_raw = crate::unity::go_get_component(api, &cache, go, cache.canvasgroup_klass);
                let cg_live = match cg_raw {
                    Some(c) if !c.is_null() => {
                        match crate::unity::object_alive(api, &cache, c) {
                            Some(true) => Some(c),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let (cg, opacity) = match cg_live {
                    Some(c) => {
                        let a = crate::unity::canvasgroup_get_alpha(api, &cache, c)
                            .unwrap_or(color[3]);
                        (Some(c as usize), Some(a))
                    }
                    None if want.comp.is_some() => (None, Some(color[3])),
                    None => (None, None),
                };
                let (font_size, text) = if want.comp == Some(CompKind::Tmp) {
                    let size = crate::unity::tmp_get_font_size(api, &cache, comp_p);
                    let txt = crate::unity::tmp_get_text(api, &cache, comp_p).unwrap_or_default();
                    (size, txt)
                } else {
                    (None, String::new())
                };
                let tr_addr = tr as usize;
                let comp_opt = if comp_addr == 0 { None } else { Some(comp_addr) };
                if !seen.iter().any(|&(t, c)| t == tr_addr && c == comp_addr) {
                    seen.push((tr_addr, comp_addr));
                    fresh_origs.push(OrigRec {
                        owner: want.path.clone(),
                        kind: want.comp,
                        tr: tr_addr,
                        comp: comp_opt,
                        pos,
                        euler,
                        scale,
                        color: want.comp.map(|_| color),
                        cg,
                        alpha: match cg {
                            Some(_) => opacity,
                            None => None,
                        },
                        font_size,
                        active,
                    });
                }
                snaps.push(ObjSnap {
                    owner: want.path.clone(),
                    comp: want.comp,
                    active,
                    tr_addr,
                    comp_addr: comp_opt,
                    pos,
                    euler,
                    scale,
                    color: want.comp.map(|_| color),
                    opacity,
                    cg,
                    font_size,
                    text,
                });
            }
        }
        // Merge first-seen originals + swap the snapshot, both under
        // brief locks with zero Unity calls in between.
        lock(&self.origs).extend(fresh_origs);
        let n = snaps.len();
        // `total` counts every bound entry (containers included);
        // `n` drops only what scene churn stole mid-walk.
        *lock(&self.snaps) = snaps;
        self.snap_ver.fetch_add(1, Ordering::SeqCst);
        self.status_ok.store(n as u64, Ordering::SeqCst);
        self.status_total.store(total as u64, Ordering::SeqCst);
        let ms = t0.elapsed().as_millis();
        crate::log_line(&format!("map: objets {n}/{total} ({ms} ms)"));
    }

    /// Fresh re-resolve of a dashboard target (tick thread only).
    ///
    /// Stored `usize` handles go stale on scene change (menu → city
    /// rebuilds the HUD, GC frees old wrappers). `owner` is a slash
    /// path (`Map/Mask/Base/RoadImage`): tried under [`DASH_ROOT`]
    /// first, then with its first segment as root (fallback scenes
    /// mount sections directly — same shape the walk produces, so one
    /// format serves both). Each leg is root `GameObject.Find` +
    /// chained `Transform.Find`, which sees INACTIVE branches too.
    /// Returns the *live* `(gameobject, transform, component?,
    /// canvas_group?)`. `None` = target gone — caller skips, never
    /// touches stale memory. Legacy bare names (no `/`) still resolve
    /// via plain `Find`; `UIHUDMinimap` itself resolves directly.
    unsafe fn resolve_live(
        api: &crate::il2cpp::Il2cppApi,
        cache: &UnityCache,
        owner: &str,
        kind: Option<CompKind>,
    ) -> Option<(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        Option<*mut std::ffi::c_void>,
        Option<*mut std::ffi::c_void>,
    )> {
        // SAFETY: attached tick thread; every handle alive-checked
        // before return, consumed immediately by the caller.
        unsafe {
            // Bind one GameObject: (root, rest-segments) candidates in
            // order — dashboard first, then first-segment-as-root for
            // fallback scenes. Walk paths are relative to the root's
            // children (`Map/Mask/Base/RoadImage`), so every
            // segment resolves from the root transform down.
            let stripped = owner
                .strip_prefix(DASH_ROOT)
                .and_then(|s| s.strip_prefix('/'))
                .unwrap_or(owner);
            let mut go: *mut std::ffi::c_void = std::ptr::null_mut();
            if !stripped.contains('/') {
                // Dashboard root itself or a legacy bare name.
                go = crate::unity::go_find(api, cache, stripped)?;
            } else {
                let segs: Vec<&str> = stripped.split('/').collect();
                // (root name, segments below it)
                let mut cands: Vec<(&str, &[&str])> = vec![(DASH_ROOT, &segs)];
                if segs.len() > 1 {
                    cands.push((segs[0], &segs[1..]));
                }
                for (root_name, rest) in cands {
                    let r = match crate::unity::go_find(api, cache, root_name) {
                        Some(g) => g,
                        None => continue,
                    };
                    if r.is_null() || crate::unity::object_alive(api, cache, r) != Some(true) {
                        continue;
                    }
                    let mut tr = match crate::unity::go_transform(api, cache, r) {
                        Some(t) => t,
                        None => continue,
                    };
                    let mut ok = true;
                    for seg in rest {
                        if tr.is_null()
                            || crate::unity::object_alive(api, cache, tr) != Some(true)
                        {
                            ok = false;
                            break;
                        }
                        tr = match crate::unity::tr_find(api, cache, tr, seg) {
                            Some(t) => t,
                            None => {
                                ok = false;
                                break;
                            }
                        };
                    }
                    if !ok
                        || tr.is_null()
                        || crate::unity::object_alive(api, cache, tr) != Some(true)
                    {
                        continue;
                    }
                    match crate::unity::component_gameobject(api, cache, tr) {
                        Some(g)
                            if !g.is_null()
                                && crate::unity::object_alive(api, cache, g) == Some(true) =>
                        {
                            go = g;
                            break;
                        }
                        _ => continue,
                    }
                }
            }
            if go.is_null() || crate::unity::object_alive(api, cache, go) != Some(true) {
                return None;
            }
            let tr = crate::unity::go_transform(api, cache, go)?;
            if tr.is_null() || crate::unity::object_alive(api, cache, tr) != Some(true) {
                return None;
            }
            // Graphic component only when the entry has one (containers
            // resolve to transform + GameObject alone).
            let comp = match kind {
                Some(k) => {
                    let klass = k.klass(cache);
                    if klass.is_null() {
                        return None;
                    }
                    let c = crate::unity::go_get_component(api, cache, go, klass)?;
                    if c.is_null() || crate::unity::object_alive(api, cache, c) != Some(true)
                    {
                        return None;
                    }
                    Some(c)
                }
                None => None,
            };
            let cg = match crate::unity::go_get_component(api, cache, go, cache.canvasgroup_klass) {
                Some(c) if !c.is_null() && crate::unity::object_alive(api, cache, c) == Some(true) => Some(c),
                _ => None,
            };
            Some((go, tr, comp, cg))
        }
    }

    /// Drain the staged batch (tick thread): every staged edit lands
    /// in ONE Unity pass. Realtime edits stage continuously, so the
    /// log stays quiet on clean drains — only problems surface.
    /// Each target re-resolves by dashboard path right before its
    /// invoke; the stored `addr` is only a log hint.
    pub(crate) fn drain_batch(&self) {
        let reqs = lock(&self.pending_batch).take();
        let reqs = match reqs {
            Some(r) if !r.is_empty() => r,
            _ => return,
        };
        let tick = self.tick.load(Ordering::SeqCst);
        if !self.ensure_unity(tick, false) {
            *lock(&self.pending_batch) = Some(reqs);
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = lock(&self.unity).as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        let total = reqs.len();
        let mut ok = 0usize;
        let mut skipped_stale = 0usize;
        // SAFETY: attached tick thread. Stale addresses are NEVER
        // dereferenced: each request re-resolves its named target first
        // (menu → city rebuilds HUD objects, GC frees old wrappers —
        // touching the stored pointer is the delayed SIGSEGV). The
        // stored `addr` is only a log hint.
        unsafe {
            // Batch re-resolve per owner (one walk per object, not per op).
            // Tuple: (gameobject, transform, component?, canvasgroup?).
            let mut live: std::collections::HashMap<
                String,
                (usize, usize, Option<usize>, Option<usize>),
            > = std::collections::HashMap::new();
            for r in &reqs {
                let entry = match live.get(&r.owner) {
                    Some(e) => *e,
                    None => match Self::resolve_live(api, &cache, &r.owner, r.kind) {
                        Some((go, tr, comp, cg)) => {
                            let e = (
                                go as usize,
                                tr as usize,
                                comp.map(|p| p as usize),
                                cg.map(|p| p as usize),
                            );
                            live.insert(r.owner.clone(), e);
                            e
                        }
                        None => {
                            skipped_stale += 1;
                            continue;
                        }
                    },
                };
                let obj = match r.op {
                    OP_SCALE | OP_POS | OP_ROT => entry.1 as *mut std::ffi::c_void,
                    OP_COLOR | OP_FONT => match entry.2 {
                        Some(a) => a as *mut std::ffi::c_void,
                        None => continue,
                    },
                    OP_CGALPHA => match entry.3 {
                        Some(a) => a as *mut std::ffi::c_void,
                        None => continue,
                    },
                    OP_ACTIVE => entry.0 as *mut std::ffi::c_void,
                    _ => continue,
                };
                // Fresh handle, still probe (churn between resolve+write).
                match crate::unity::object_alive(api, &cache, obj) {
                    Some(true) => {}
                    _ => {
                        skipped_stale += 1;
                        continue;
                    }
                }
                let wrote = match r.op {
                    OP_SCALE => crate::unity::tr_set_scale(api, &cache, obj, [r.v[0], r.v[1], r.v[2]]),
                    OP_POS => crate::unity::tr_set_local_position(api, &cache, obj, [r.v[0], r.v[1], r.v[2]]),
                    OP_COLOR => crate::unity::graphic_set_color(api, &cache, obj, r.v),
                    OP_FONT => crate::unity::tmp_set_font_size(api, &cache, obj, r.v[0]),
                    OP_ROT => crate::unity::tr_set_local_euler(api, &cache, obj, [r.v[0], r.v[1], r.v[2]]),
                    OP_CGALPHA => crate::unity::canvasgroup_set_alpha(api, &cache, obj, r.v[0]),
                    OP_ACTIVE => crate::unity::go_set_active(api, &cache, obj, r.v[0] != 0.0),
                    _ => false,
                };
                if wrote {
                    ok += 1;
                }
            }
        }
        // Quiet on clean realtime drains; problems always surface.
        if skipped_stale > 0 || ok != total {
            crate::log_line(&format!(
                "map: application {ok}/{total} (stale ignorés: {skipped_stale})"
            ));
        }
    }

    /// Write back first-recorded originals (disable / Restore paths,
    /// tick thread, one shot). Dead objects are skipped; the records
    /// are consumed so the next discovery starts fresh.
    pub(crate) fn restore_all(&self) {
        let tick = self.tick.load(Ordering::SeqCst);
        if !self.ensure_unity(tick, true) {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = lock(&self.unity).as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        // Take the records (brief lock, no Unity under it).
        let recs = std::mem::take(&mut *lock(&self.origs));
        if recs.is_empty() {
            return;
        }
        let mut ok = 0usize;
        let total = recs.len();
        let mut gone = 0usize;
        // SAFETY: attached tick thread. Same stale-handle rule as
        // `drain_batch`: re-resolve each record by owner name, write to
        // the live object. Never touch `r.tr / r.comp` directly.
        unsafe {
            for r in &recs {
                let (go, tr, comp_live, cg_live) =
                    match Self::resolve_live(api, &cache, &r.owner, r.kind) {
                        Some(v) => v,
                        None => {
                            gone += 1;
                            continue;
                        }
                    };
                let mut restored = false;
                restored |= crate::unity::tr_set_scale(api, &cache, tr, r.scale);
                restored |= crate::unity::tr_set_local_position(api, &cache, tr, r.pos);
                restored |= crate::unity::tr_set_local_euler(api, &cache, tr, r.euler);
                if let (Some(comp), Some(color)) = (comp_live, r.color) {
                    restored |= crate::unity::graphic_set_color(api, &cache, comp, color);
                    if r.kind == Some(CompKind::Tmp) {
                        if let Some(size) = r.font_size {
                            restored |= crate::unity::tmp_set_font_size(api, &cache, comp, size);
                        }
                    }
                }
                // CanvasGroup original on the live CG object (if any).
                if let (Some(cg), Some(a)) = (cg_live, r.alpha) {
                    restored |= crate::unity::canvasgroup_set_alpha(api, &cache, cg, a);
                }
                // Active state back to discovery-time (last: a re-enabled
                // branch must exist before it matters).
                restored |= crate::unity::go_set_active(api, &cache, go, r.active);
                if restored {
                    ok += 1;
                }
            }
        }
        crate::log_line(&format!("map: restaurés {ok}/{total} (disparus: {gone})"));
        // Fresh UI next arming (scene may have changed meanwhile).
        *lock(&self.snaps) = Vec::new();
        self.snap_ver.fetch_add(1, Ordering::SeqCst);
        self.status_ok.store(0, Ordering::SeqCst);
        self.status_total.store(0, Ordering::SeqCst);
    }}
