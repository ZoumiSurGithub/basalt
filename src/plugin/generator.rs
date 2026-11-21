//! Générateurs de terrain écrits en Lua par les plugins.
//!
//! Un plugin fournit un générateur en déposant un fichier `generators/<nom>.lua` dans son
//! dossier ; un monde l'emploie avec `level-type=plugin:<nom>`. Le fichier renvoie une table :
//!
//! - `terrain(chunk)` (obligatoire) : remplit le chunk ;
//! - `decorate(chunk)` : arbres, minerais, herbes… après le terrain ;
//! - `structures` : liste de structures placées sur une grille, qui peuvent déborder sur les
//!   chunks voisins ;
//! - `height(x, z)` : hauteur du sol, pour placer les structures ;
//! - `spawn()` : point d'apparition.
//!
//! La génération se fait sur les fils de génération du terrain, en parallèle : chaque fil
//! charge sa propre copie du générateur. Un générateur ne voit que le chunk qu'il remplit,
//! la graine et ses outils (bruit, hasard déterministe) : ni joueurs, ni fichiers, ni
//! réseau. Il est limité en temps et en mémoire, comme un plugin protégé. Rien de ce qu'il
//! produit n'est enregistré : le terrain se régénère à l'identique à partir de la graine.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::lua::{self, Api, CFn, MemState, L, REGISTRY, T_FUNCTION, T_NUMBER, T_STRING, T_TABLE};
use crate::data::GameData;
use crate::world::generator::{mix, Biome, Column, Palette, Perlin, Rng};
use crate::world::vanilla::{Vanilla, B};
use crate::world::{MAX_Y, MIN_Y};

/// Temps accordé pour remplir un chunk (millisecondes).
const CHUNK_BUDGET_MS: u64 = 2000;
const HEIGHT: usize = (MAX_Y - MIN_Y + 1) as usize;
const SECTIONS: usize = HEIGHT / 16;

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// Une structure déclarée par un générateur.
#[derive(Clone)]
struct StructureDef {
    spacing: i32,
    separation: i32,
    salt: u64,
    reach: i32,
    build: c_int,
    check: c_int,
}

/// Générateur chargé depuis un plugin.
pub struct LuaGenerator {
    id: usize,
    pub name: String,
    /// Plugin qui le fournit
    pub plugin: String,
    source: String,
    chunk_name: String,
    seed: u64,
    data: Arc<GameData>,
    vanilla: Vanilla,
    perlin: Perlin,
    memory: usize,
    /// Erreurs déjà signalées (une seule fois chacune)
    reported: std::sync::Mutex<std::collections::HashSet<String>>,
}

/// Copie du générateur sur un fil.
struct State {
    l: L,
    #[allow(dead_code)]
    mem: *mut MemState,
    ctx: *mut Ctx,
    chunk: c_int,
    structure: c_int,
    terrain: c_int,
    decorate: c_int,
    spawn: c_int,
    structures: Vec<StructureDef>,
}

/// Ce que les fonctions C d'un état voient.
struct Ctx {
    api: &'static Api,
    generator: *const LuaGenerator,
    palette: *const Palette,
    /// Noms de blocs déjà résolus
    names: HashMap<String, u32>,
    buf: Option<Buf>,
    errors: Vec<String>,
    /// Fonction height(x, z) du générateur (0 : absente)
    height: c_int,
}

/// Chunk en cours de remplissage.
struct Buf {
    cx: i32,
    cz: i32,
    blocks: Vec<u32>,
    biomes: [u8; 256],
    biome_set: bool,
    biome_ids: Option<Vec<u8>>,
    rng: Rng,
    /// Structure en cours : départ et hasard
    sx: i32,
    sz: i32,
    srng: Rng,
}

impl Buf {
    /// Index d'un bloc dans le tampon du chunk : colonne (x, z), puis hauteur.
    fn at(x: usize, z: usize, y: i32) -> usize {
        (x * 16 + z) * HEIGHT + (y - MIN_Y) as usize
    }
}

thread_local! {
    static STATES: RefCell<HashMap<usize, Option<State>>> = RefCell::new(HashMap::new());
}

impl LuaGenerator {
    /// Cherche `generators/<nom>.lua` dans les dossiers des plugins et le charge (un essai
    /// sur ce fil, pour signaler tout de suite une erreur).
    pub fn load(plugins_dir: &Path, name: &str, seed: u64, data: Arc<GameData>, memory_mb: usize) -> Result<Arc<LuaGenerator>, String> {
        if Api::shared().is_none() {
            return Err("bibliothèque Lua 5.4 introuvable".into());
        }
        if !super::manifest::valid_name(name) {
            return Err(format!("nom de générateur invalide : {}", name));
        }
        let (plugin, path) = find(plugins_dir, name).ok_or_else(|| format!("aucun plugin ne fournit generators/{}.lua", name))?;
        let source = std::fs::read_to_string(&path).map_err(|e| format!("{} : {}", path.display(), e))?;
        let g = Arc::new(LuaGenerator {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: name.to_string(),
            plugin,
            source,
            chunk_name: format!("@{}", path.display()),
            seed,
            data,
            vanilla: Vanilla::new(seed),
            perlin: Perlin::new(mix(seed, 0, 0, 0x6c75_6167)),
            memory: memory_mb.clamp(4, 4096) * 1024 * 1024,
            reported: Default::default(),
        });
        // Chargement d'essai : erreurs de syntaxe, fonction `terrain` absente…
        let first = unsafe { g.open() }?;
        STATES.with(|s| s.borrow_mut().insert(g.id, Some(first)));
        Ok(g)
    }

    /// Erreur du générateur, affichée une seule fois (et 50 messages différents au plus).
    fn report(&self, text: &str) {
        if let Ok(mut seen) = self.reported.lock() {
            if seen.len() < 50 && seen.insert(text.to_string()) {
                error!("[{}] générateur {} : {}", self.plugin, self.name, text);
            }
        }
    }

    /// Crée la copie Lua du générateur pour le fil courant.
    unsafe fn open(&self) -> Result<State, String> {
        let api = Api::shared().ok_or("bibliothèque Lua 5.4 introuvable")?;
        let mem = Box::into_raw(Box::new(MemState { used: 0, limit: self.memory }));
        let l = (api.newstate_alloc)(lua::limited_alloc, mem as *mut c_void);
        if l.is_null() {
            drop(Box::from_raw(mem));
            return Err("mémoire insuffisante".into());
        }
        (api.openlibs)(l);
        let ctx = Box::into_raw(Box::new(Ctx { api, generator: self, palette: std::ptr::null(), names: HashMap::new(), buf: None, errors: Vec::new(), height: 0 }));
        let reg = |name: &str, f: CFn| {
            (api.pushlightuserdata)(l, ctx as *mut c_void);
            (api.pushcclosure)(l, f, 1);
            let n = CString::new(name).unwrap_or_default();
            (api.setglobal)(l, n.as_ptr());
        };
        reg("block", g_block);
        reg("noise", g_noise);
        reg("vanillaHeight", g_vanilla_height);
        reg("vanillaBiome", g_vanilla_biome);
        reg("__random", g_random);
        (api.pushinteger)(l, (self.seed >> 1) as i64);
        (api.setglobal)(l, "seed\0".as_ptr() as *const c_char);
        // Objets `chunk` et `structure`, passés aux fonctions du générateur
        let table = |fields: &[(&str, CFn)]| -> c_int {
            (api.createtable)(l, 0, fields.len() as c_int + 4);
            for (name, f) in fields {
                (api.pushlightuserdata)(l, ctx as *mut c_void);
                (api.pushcclosure)(l, *f, 1);
                let n = CString::new(*name).unwrap_or_default();
                (api.setfield)(l, -2, n.as_ptr());
            }
            (api.pushinteger)(l, MIN_Y as i64);
            (api.setfield)(l, -2, "minY\0".as_ptr() as *const c_char);
            (api.pushinteger)(l, MAX_Y as i64);
            (api.setfield)(l, -2, "maxY\0".as_ptr() as *const c_char);
            (api.ref_)(l, REGISTRY)
        };
        let chunk = table(&[
            ("set", c_set),
            ("get", c_get),
            ("fill", c_fill),
            ("height", c_height),
            ("biome", c_biome),
            ("vanilla", c_vanilla),
            ("random", c_random),
            ("setAt", s_set),
        ]);
        let structure = table(&[("set", s_set), ("fill", s_fill), ("get", s_get), ("random", s_random), ("height", s_height)]);
        // Bac à sable : le langage, sans accès au système ; un hasard déterministe
        let prelude = "io, os, package, require, dofile, loadfile, load, debug, collectgarbage = nil, nil, nil, nil, nil, nil, nil, nil, nil\n\
                       math.random = __random\nmath.randomseed = function() end\n__random = nil\n";
        if (api.loadbufferx)(l, prelude.as_ptr() as *const c_char, prelude.len(), "=preambule\0".as_ptr() as *const c_char, "t\0".as_ptr() as *const c_char) != 0
            || (api.pcallk)(l, 0, 0, 0, 0, std::ptr::null()) != 0
        {
            return Err(error_text(api, l));
        }
        (api.sethook)(l, Some(lua::watchdog), 8, 10_000);
        let chunk_name = CString::new(self.chunk_name.as_str()).unwrap_or_default();
        let arm = |ms: u64| {
            lua::DEADLINE_MS.with(|d| d.set(lua::now_ms() + ms));
            lua::CURRENT_MEM.with(|m| m.set(mem));
        };
        let disarm = || {
            lua::DEADLINE_MS.with(|d| d.set(u64::MAX));
            lua::CURRENT_MEM.with(|m| m.set(std::ptr::null_mut()));
        };
        arm(3000);
        let failed = (api.loadbufferx)(l, self.source.as_ptr() as *const c_char, self.source.len(), chunk_name.as_ptr(), "t\0".as_ptr() as *const c_char) != 0
            || (api.pcallk)(l, 0, 1, 0, 0, std::ptr::null()) != 0;
        disarm();
        if failed {
            let e = error_text(api, l);
            (api.close)(l);
            return Err(e);
        }
        if (api.type_)(l, -1) != T_TABLE {
            (api.close)(l);
            return Err("le fichier doit renvoyer une table (return { terrain = function(chunk) … end })".into());
        }
        let func = |name: &str| -> c_int {
            let n = CString::new(name).unwrap_or_default();
            if (api.getfield)(l, -1, n.as_ptr()) == T_FUNCTION {
                (api.ref_)(l, REGISTRY)
            } else {
                (api.settop)(l, -2);
                0
            }
        };
        let terrain = func("terrain");
        if terrain == 0 {
            (api.close)(l);
            return Err("fonction terrain(chunk) absente".into());
        }
        let (decorate, height, spawn) = (func("decorate"), func("height"), func("spawn"));
        (*ctx).height = height;
        // structures = { { spacing = 32, separation = 8, salt = 1, reach = 1, build = f, check = f }, … }
        let mut structures = Vec::new();
        if (api.getfield)(l, -1, "structures\0".as_ptr() as *const c_char) == T_TABLE {
            let mut i = 1;
            while (api.rawgeti)(l, -1, i) == T_TABLE {
                let num = |name: &str, default: i64| -> i64 {
                    let n = CString::new(name).unwrap_or_default();
                    let v = if (api.getfield)(l, -1, n.as_ptr()) == T_NUMBER { (api.tointegerx)(l, -1, std::ptr::null_mut()) } else { default };
                    (api.settop)(l, -2);
                    v
                };
                let (spacing, separation, salt, reach) = (num("spacing", 32).clamp(1, 4096) as i32, num("separation", 8).max(0) as i32, num("salt", i as i64) as u64, num("reach", 1).clamp(0, 8) as i32);
                let (build, check) = (func("build"), func("check"));
                if build == 0 {
                    (api.close)(l);
                    return Err(format!("structures[{}] : fonction build(structure) absente", i));
                }
                structures.push(StructureDef { spacing, separation: separation.min(spacing - 1).max(0), salt, reach, build, check });
                (api.settop)(l, -2);
                i += 1;
            }
            (api.settop)(l, -2);
        }
        (api.settop)(l, -2);
        (api.settop)(l, 0);
        Ok(State { l, mem, ctx, chunk, structure, terrain, decorate, spawn, structures })
    }

    /// Génère un chunk. En cas d'erreur du script, le chunk reste vide (de l'air) et
    /// l'erreur est signalée une fois.
    pub fn generate(&self, palette: &Palette, cx: i32, cz: i32) -> Column {
        STATES.with(|states| {
            let mut states = states.borrow_mut();
            let entry = states.entry(self.id).or_insert_with(|| match unsafe { self.open() } {
                Ok(s) => Some(s),
                Err(e) => {
                    self.report(&e);
                    None
                }
            });
            match entry {
                Some(state) => unsafe { self.run(state, palette, cx, cz) },
                None => Column::from_blocks(&vec![palette.air; 256 * HEIGHT], palette.air, Biome::Plains),
            }
        })
    }

    /// Exécute le générateur Lua sur un chunk : remplissage, structures proches, décoration.
    unsafe fn run(&self, st: &State, palette: &Palette, cx: i32, cz: i32) -> Column {
        let ctx = &mut *st.ctx;
        let api = ctx.api;
        let l = st.l;
        ctx.palette = palette;
        ctx.errors.clear();
        ctx.buf = Some(Buf {
            cx,
            cz,
            blocks: vec![palette.air; 256 * HEIGHT],
            biomes: [B::Plains.id(); 256],
            biome_set: false,
            biome_ids: None,
            rng: Rng::new(mix(self.seed, cx as i64, cz as i64, 0x6765_6e)),
            sx: 0,
            sz: 0,
            srng: Rng::new(0),
        });
        // chunk.x, chunk.z
        (api.rawgeti)(l, REGISTRY, st.chunk as i64);
        (api.pushinteger)(l, cx as i64);
        (api.setfield)(l, -2, "x\0".as_ptr() as *const c_char);
        (api.pushinteger)(l, cz as i64);
        (api.setfield)(l, -2, "z\0".as_ptr() as *const c_char);
        (api.settop)(l, 0);
        lua::DEADLINE_MS.with(|d| d.set(lua::now_ms() + CHUNK_BUDGET_MS));
        lua::CURRENT_MEM.with(|m| m.set(st.mem));
        let call = |func: c_int, arg: c_int| -> Result<bool, String> {
            (api.rawgeti)(l, REGISTRY, func as i64);
            (api.rawgeti)(l, REGISTRY, arg as i64);
            if (api.pcallk)(l, 1, 1, 0, 0, std::ptr::null()) != 0 {
                let e = error_text(api, l);
                (api.settop)(l, 0);
                return Err(e);
            }
            let answer = (api.type_)(l, -1) == 0 || (api.toboolean)(l, -1) != 0;
            (api.settop)(l, 0);
            Ok(answer)
        };
        let mut failure = None;
        if let Err(e) = call(st.terrain, st.chunk) {
            failure = Some(e);
        }
        if failure.is_none() && st.decorate != 0 {
            if let Err(e) = call(st.decorate, st.chunk) {
                failure = Some(e);
            }
        }
        // Structures dont un morceau tombe dans ce chunk
        if failure.is_none() {
            'defs: for def in &st.structures {
                let (rx0, rz0) = (cx.div_euclid(def.spacing), cz.div_euclid(def.spacing));
                let span = def.reach / def.spacing + 1;
                for rx in rx0 - span..=rx0 + span {
                    for rz in rz0 - span..=rz0 + span {
                        let mut r = Rng::new(mix(self.seed, rx as i64, rz as i64, def.salt.wrapping_add(0x7374_7275)));
                        let range = (def.spacing - def.separation).max(1) as u32;
                        let (sx, sz) = (rx * def.spacing + r.below(range) as i32, rz * def.spacing + r.below(range) as i32);
                        if (sx - cx).abs() > def.reach || (sz - cz).abs() > def.reach {
                            continue;
                        }
                        if let Some(b) = ctx.buf.as_mut() {
                            b.sx = sx * 16 + 8;
                            b.sz = sz * 16 + 8;
                        }
                        (api.rawgeti)(l, REGISTRY, st.structure as i64);
                        (api.pushinteger)(l, (sx * 16 + 8) as i64);
                        (api.setfield)(l, -2, "x\0".as_ptr() as *const c_char);
                        (api.pushinteger)(l, (sz * 16 + 8) as i64);
                        (api.setfield)(l, -2, "z\0".as_ptr() as *const c_char);
                        (api.settop)(l, 0);
                        // Même hasard pour chaque chunk que la structure touche
                        let seed = mix(self.seed, sx as i64, sz as i64, def.salt);
                        if let Some(b) = ctx.buf.as_mut() {
                            b.srng = Rng::new(seed);
                        }
                        if def.check != 0 {
                            match call(def.check, st.structure) {
                                Ok(true) => {}
                                Ok(false) => continue,
                                Err(e) => {
                                    failure = Some(e);
                                    break 'defs;
                                }
                            }
                            if let Some(b) = ctx.buf.as_mut() {
                                b.srng = Rng::new(seed);
                            }
                        }
                        if let Err(e) = call(def.build, st.structure) {
                            failure = Some(e);
                            break 'defs;
                        }
                    }
                }
            }
        }
        lua::DEADLINE_MS.with(|d| d.set(u64::MAX));
        lua::CURRENT_MEM.with(|m| m.set(std::ptr::null_mut()));
        if let Some(e) = failure {
            self.report(&e);
        }
        for e in ctx.errors.drain(..).collect::<Vec<_>>() {
            self.report(&e);
        }
        let buf = ctx.buf.take().unwrap_or_else(|| unreachable!());
        let mut column = Column::from_blocks(&buf.blocks, palette.air, Biome::Plains);
        column.biome_ids = Some(match (buf.biome_ids, buf.biome_set) {
            (Some(ids), false) => ids,
            _ => {
                let mut ids = vec![0u8; SECTIONS * 64];
                for qx in 0..4usize {
                    for qz in 0..4usize {
                        let id = buf.biomes[(qx * 4 + 2) * 16 + qz * 4 + 2];
                        for qy in 0..SECTIONS * 4 {
                            ids[(qy / 4) * 64 + qx * 16 + qz * 4 + (qy % 4)] = id;
                        }
                    }
                }
                ids
            }
        });
        column
    }

    /// Point d'apparition donné par `spawn()`, s'il y en a un.
    pub fn spawn(&self) -> Option<(i32, i32, i32)> {
        STATES.with(|states| {
            let mut states = states.borrow_mut();
            let st = states.entry(self.id).or_insert_with(|| unsafe { self.open() }.ok()).as_ref()?;
            if st.spawn == 0 {
                return None;
            }
            unsafe {
                let api = (*st.ctx).api;
                (api.rawgeti)(st.l, REGISTRY, st.spawn as i64);
                lua::DEADLINE_MS.with(|d| d.set(lua::now_ms() + 1000));
                let ok = (api.pcallk)(st.l, 0, 3, 0, 0, std::ptr::null()) == 0;
                lua::DEADLINE_MS.with(|d| d.set(u64::MAX));
                let v = ok.then(|| ((api.tointegerx)(st.l, -3, std::ptr::null_mut()), (api.tointegerx)(st.l, -2, std::ptr::null_mut()), (api.tointegerx)(st.l, -1, std::ptr::null_mut())));
                (api.settop)(st.l, 0);
                v.map(|(x, y, z)| (x as i32, y as i32, z as i32))
            }
        })
    }
}

/// Fichier `generators/<nom>.lua` d'un plugin (dossiers parcourus dans l'ordre
/// alphabétique). Seul un vrai plugin compte : un dossier avec un `plugin.yml` valide et
/// son fichier principal, comme pour le chargement des plugins.
fn find(plugins_dir: &Path, name: &str) -> Option<(String, PathBuf)> {
    let file = format!("{}.lua", name);
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(plugins_dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for d in dirs {
        let candidate = d.join("generators").join(&file);
        if !candidate.is_file() {
            continue;
        }
        let manifest = std::fs::read_to_string(d.join("plugin.yml")).ok().and_then(|s| crate::plugin::manifest::Manifest::parse(&s).ok());
        match manifest {
            Some(m) if d.join(&m.main).is_file() => return Some((m.name, candidate)),
            _ => warn!("Générateur {} ignoré : {} n'est pas un plugin complet (plugin.yml et fichier principal)", name, d.display()),
        }
    }
    None
}

/// Message d'erreur en haut de la pile Lua.
unsafe fn error_text(api: &Api, l: L) -> String {
    let mut len = 0usize;
    let p = (api.tolstring)(l, -1, &mut len);
    if p.is_null() {
        return "erreur inconnue".into();
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len)).into_owned()
}

// --- Fonctions offertes aux scripts ---------------------------------------------------

/// Contexte du générateur rangé dans le registre de l'état Lua.
unsafe fn ctx<'a>(l: L) -> &'a mut Ctx {
    let api = Api::shared().unwrap_or_else(|| unreachable!());
    &mut *((api.touserdata)(l, REGISTRY - 1) as *mut Ctx)
}

/// Arguments Lua : entier, nombre ou texte à cette position, sinon None.
unsafe fn int(c: &Ctx, l: L, i: c_int) -> Option<i64> {
    if (c.api.type_)(l, i) != T_NUMBER {
        return None;
    }
    Some((c.api.tonumberx)(l, i, std::ptr::null_mut()).floor() as i64)
}

unsafe fn num(c: &Ctx, l: L, i: c_int) -> Option<f64> {
    ((c.api.type_)(l, i) == T_NUMBER).then(|| (c.api.tonumberx)(l, i, std::ptr::null_mut()))
}

unsafe fn text(c: &Ctx, l: L, i: c_int) -> Option<String> {
    if (c.api.type_)(l, i) != T_STRING {
        return None;
    }
    let mut len = 0usize;
    let p = (c.api.tolstring)(l, i, &mut len);
    (!p.is_null()).then(|| String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len)).into_owned())
}

/// Appel invalide : noté pour être affiché, sans interrompre le générateur.
fn fail(c: &mut Ctx, what: &str) -> c_int {
    if c.errors.len() < 8 {
        c.errors.push(format!("appel invalide : {}", what));
    }
    0
}

/// Bloc donné par son identifiant (`block("stone")`) ou par son nom.
unsafe fn block_arg(c: &mut Ctx, l: L, i: c_int) -> Option<u32> {
    match (c.api.type_)(l, i) {
        T_NUMBER => Some((c.api.tointegerx)(l, i, std::ptr::null_mut()) as u32),
        T_STRING => {
            let name = text(c, l, i)?;
            resolve(c, &name)
        }
        _ => None,
    }
}

/// Nom de bloc -> état, avec un cache (les plugins demandent souvent les mêmes blocs).
fn resolve(c: &mut Ctx, name: &str) -> Option<u32> {
    if let Some(id) = c.names.get(name) {
        return Some(*id);
    }
    let g = unsafe { &*c.generator };
    let full = if name.contains(':') { name.to_string() } else { format!("minecraft:{}", name) };
    let id = g.data.state_from_spec(&full);
    match id {
        Some(id) => {
            if c.names.len() < 4096 {
                c.names.insert(name.to_string(), id);
            }
            Some(id)
        }
        None => {
            if c.errors.len() < 8 {
                c.errors.push(format!("bloc inconnu : {}", name));
            }
            None
        }
    }
}

/// `block(nom)` : identifiant d'un bloc (à garder dans une variable : plus rapide).
unsafe extern "C" fn g_block(l: L) -> c_int {
    let c = ctx(l);
    match text(c, l, 1).and_then(|n| resolve(c, &n)) {
        Some(id) => {
            (c.api.pushinteger)(l, id as i64);
            1
        }
        None => fail(c, "block(nom) : bloc inconnu"),
    }
}

/// `noise(x, z)` ou `noise(x, y, z)` : bruit de Perlin de -1 à 1, propre à la graine.
unsafe extern "C" fn g_noise(l: L) -> c_int {
    let c = ctx(l);
    let g = &*c.generator;
    let v = match (num(c, l, 1), num(c, l, 2), num(c, l, 3)) {
        (Some(x), Some(y), Some(z)) => g.perlin.noise3(x, y, z),
        (Some(x), Some(z), None) => g.perlin.noise2(x, z),
        _ => return fail(c, "noise(x, z) ou noise(x, y, z)"),
    };
    (c.api.pushnumber)(l, v);
    1
}

/// vanillaHeight(x, z) : hauteur du sol du terrain vanilla à cette colonne.
unsafe extern "C" fn g_vanilla_height(l: L) -> c_int {
    let c = ctx(l);
    match (int(c, l, 1), int(c, l, 2)) {
        (Some(x), Some(z)) => {
            (c.api.pushinteger)(l, (*c.generator).vanilla.surface(x as i32, z as i32).0 as i64);
            1
        }
        _ => fail(c, "vanillaHeight(x, z)"),
    }
}

/// vanillaBiome(x, z) : biome vanilla de surface, en texte.
unsafe extern "C" fn g_vanilla_biome(l: L) -> c_int {
    let c = ctx(l);
    match (int(c, l, 1), int(c, l, 2)) {
        (Some(x), Some(z)) => {
            let name = (*c.generator).vanilla.surface_biome_at(x as i32, z as i32).name();
            (c.api.pushlstring)(l, name.as_ptr() as *const c_char, name.len());
            1
        }
        _ => fail(c, "vanillaBiome(x, z)"),
    }
}

/// Hasard déterministe : `random()` de 0 à 1, `random(n)` de 1 à n, `random(a, b)`.
unsafe fn push_random(c: &Ctx, l: L, rng: &mut Rng, first: c_int) -> c_int {
    let (a, b) = (int(c, l, first), int(c, l, first + 1));
    match (a, b) {
        (None, _) => (c.api.pushnumber)(l, rng.unit()),
        (Some(n), None) => (c.api.pushinteger)(l, 1 + (rng.unit() * n.max(1) as f64) as i64),
        (Some(lo), Some(hi)) => {
            let (lo, hi) = (lo.min(hi), lo.max(hi));
            (c.api.pushinteger)(l, lo + (rng.unit() * (hi - lo + 1) as f64) as i64)
        }
    }
    1
}

/// math.random des générateurs : tiré de la graine du chunk, donc le même à chaque génération.
unsafe extern "C" fn g_random(l: L) -> c_int {
    let c = ctx(l);
    let mut rng = match c.buf.as_mut() {
        Some(b) => std::mem::replace(&mut b.rng, Rng::new(0)),
        None => Rng::new(0),
    };
    let r = push_random(c, l, &mut rng, 1);
    if let Some(b) = c.buf.as_mut() {
        b.rng = rng;
    }
    r
}

/// Coordonnées locales (0 à 15) valides ?
fn local(x: i64, y: i64, z: i64) -> Option<(usize, i32, usize)> {
    ((0..16).contains(&x) && (0..16).contains(&z) && (MIN_Y as i64..=MAX_Y as i64).contains(&y)).then_some((x as usize, y as i32, z as usize))
}

/// `chunk:set(x, y, z, bloc)` (x et z de 0 à 15)
unsafe extern "C" fn c_set(l: L) -> c_int {
    let c = ctx(l);
    let (x, y, z) = match (int(c, l, 2), int(c, l, 3), int(c, l, 4)) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return fail(c, "chunk:set(x, y, z, bloc)"),
    };
    let Some(b) = block_arg(c, l, 5) else { return 0 };
    if let (Some((x, y, z)), Some(buf)) = (local(x, y, z), c.buf.as_mut()) {
        buf.blocks[Buf::at(x, z, y)] = b;
    }
    0
}

/// `chunk:get(x, y, z)` : identifiant du bloc
unsafe extern "C" fn c_get(l: L) -> c_int {
    let c = ctx(l);
    let (x, y, z) = match (int(c, l, 2), int(c, l, 3), int(c, l, 4)) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return fail(c, "chunk:get(x, y, z)"),
    };
    let air = (*c.palette).air;
    let v = match (local(x, y, z), c.buf.as_ref()) {
        (Some((x, y, z)), Some(buf)) => buf.blocks[Buf::at(x, z, y)],
        _ => air,
    };
    (c.api.pushinteger)(l, v as i64);
    1
}

/// Remplit un pavé, en coordonnées locales (`absolute` : coordonnées du monde).
unsafe fn fill(c: &mut Ctx, l: L, absolute: bool) -> c_int {
    let mut v = [0i64; 6];
    for (i, slot) in v.iter_mut().enumerate() {
        match int(c, l, i as c_int + 2) {
            Some(n) => *slot = n,
            None => return fail(c, "fill(x1, y1, z1, x2, y2, z2, bloc)"),
        }
    }
    let Some(b) = block_arg(c, l, 8) else { return 0 };
    let Some(buf) = c.buf.as_mut() else { return 0 };
    let (ox, oz) = if absolute { (buf.cx as i64 * 16, buf.cz as i64 * 16) } else { (0, 0) };
    let (x1, x2) = ((v[0].min(v[3]) - ox).max(0), (v[0].max(v[3]) - ox).min(15));
    let (y1, y2) = (v[1].min(v[4]).max(MIN_Y as i64), v[1].max(v[4]).min(MAX_Y as i64));
    let (z1, z2) = ((v[2].min(v[5]) - oz).max(0), (v[2].max(v[5]) - oz).min(15));
    for x in x1..=x2 {
        for z in z1..=z2 {
            let base = Buf::at(x as usize, z as usize, y1 as i32);
            for i in 0..=(y2 - y1).max(-1) {
                buf.blocks[base + i as usize] = b;
            }
        }
    }
    0
}

/// chunk:fill(x1, y1, z1, x2, y2, z2, bloc)
unsafe extern "C" fn c_fill(l: L) -> c_int {
    fill(ctx(l), l, false)
}

/// `chunk:height(x, z)` : y du plus haut bloc qui n'est pas de l'air (minY - 1 si aucun)
unsafe extern "C" fn c_height(l: L) -> c_int {
    let c = ctx(l);
    let (x, z) = match (int(c, l, 2), int(c, l, 3)) {
        (Some(x), Some(z)) if (0..16).contains(&x) && (0..16).contains(&z) => (x as usize, z as usize),
        _ => return fail(c, "chunk:height(x, z) avec x et z de 0 à 15"),
    };
    let air = (*c.palette).air;
    let mut h = MIN_Y - 1;
    if let Some(buf) = c.buf.as_ref() {
        for y in (MIN_Y..=MAX_Y).rev() {
            if buf.blocks[Buf::at(x, z, y)] != air {
                h = y;
                break;
            }
        }
    }
    (c.api.pushinteger)(l, h as i64);
    1
}

/// `chunk:biome(nom)` pour tout le chunk, ou `chunk:biome(x, z, nom)` pour une colonne
unsafe extern "C" fn c_biome(l: L) -> c_int {
    let c = ctx(l);
    let (cols, name) = match (int(c, l, 2), int(c, l, 3), text(c, l, 4), text(c, l, 2)) {
        (Some(x), Some(z), Some(n), _) if (0..16).contains(&x) && (0..16).contains(&z) => (Some((x as usize, z as usize)), n),
        (_, _, _, Some(n)) => (None, n),
        _ => return fail(c, "chunk:biome(nom) ou chunk:biome(x, z, nom)"),
    };
    let short = name.trim_start_matches("minecraft:").to_ascii_lowercase();
    let Some(b) = B::ALL.iter().find(|b| b.name() == short) else { return fail(c, &format!("biome inconnu : {}", name)) };
    if let Some(buf) = c.buf.as_mut() {
        if !buf.biome_set {
            // Premier changement : on part des biomes du terrain vanilla, s'il a été posé
            if let Some(ids) = &buf.biome_ids {
                for x in 0..16 {
                    for z in 0..16 {
                        // Biome au niveau de la mer (section 8 : y de 64 à 79)
                        buf.biomes[x * 16 + z] = ids[8 * 64 + (x / 4) * 16 + (z / 4) * 4];
                    }
                }
            }
        }
        buf.biome_set = true;
        match cols {
            Some((x, z)) => buf.biomes[x * 16 + z] = b.id(),
            None => buf.biomes = [b.id(); 256],
        }
    }
    0
}

/// `chunk:vanilla()` : remplit le chunk avec le terrain du jeu (relief, grottes, arbres,
/// structures…), pour le retoucher ensuite.
unsafe extern "C" fn c_vanilla(l: L) -> c_int {
    let c = ctx(l);
    let g = &*c.generator;
    let Some(buf) = c.buf.as_mut() else { return 0 };
    let column = g.vanilla.generate(&*c.palette, buf.cx, buf.cz);
    for x in 0..16usize {
        for z in 0..16usize {
            for y in MIN_Y..=MAX_Y {
                buf.blocks[Buf::at(x, z, y)] = column.get(x, y, z);
            }
        }
    }
    buf.biome_ids = column.biome_ids;
    buf.biome_set = false;
    0
}

/// chunk:random(…) : même tirage que math.random, propre au chunk.
unsafe extern "C" fn c_random(l: L) -> c_int {
    let c = ctx(l);
    let Some(mut rng) = c.buf.as_mut().map(|b| std::mem::replace(&mut b.rng, Rng::new(0))) else { return 0 };
    let r = push_random(c, l, &mut rng, 2);
    if let Some(b) = c.buf.as_mut() {
        b.rng = rng;
    }
    r
}

/// `structure:set(x, y, z, bloc)` (et `chunk:setAt`) : coordonnées du monde ; seuls les
/// blocs qui tombent dans le chunk en cours sont posés.
unsafe extern "C" fn s_set(l: L) -> c_int {
    let c = ctx(l);
    let (x, y, z) = match (int(c, l, 2), int(c, l, 3), int(c, l, 4)) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return fail(c, "structure:set(x, y, z, bloc)"),
    };
    let Some(b) = block_arg(c, l, 5) else { return 0 };
    if let Some(buf) = c.buf.as_mut() {
        if let Some((lx, y, lz)) = local(x - buf.cx as i64 * 16, y, z - buf.cz as i64 * 16) {
            buf.blocks[Buf::at(lx, lz, y)] = b;
        }
    }
    0
}

/// structure:fill(…), en coordonnées du monde.
unsafe extern "C" fn s_fill(l: L) -> c_int {
    fill(ctx(l), l, true)
}

/// `structure:get(x, y, z)` : bloc du chunk en cours (coordonnées du monde), nil ailleurs
unsafe extern "C" fn s_get(l: L) -> c_int {
    let c = ctx(l);
    let (x, y, z) = match (int(c, l, 2), int(c, l, 3), int(c, l, 4)) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return fail(c, "structure:get(x, y, z)"),
    };
    let Some(buf) = c.buf.as_ref() else { return 0 };
    match local(x - buf.cx as i64 * 16, y, z - buf.cz as i64 * 16) {
        Some((lx, y, lz)) => {
            (c.api.pushinteger)(l, buf.blocks[Buf::at(lx, lz, y)] as i64);
            1
        }
        None => 0,
    }
}

/// structure:random(…) : tirage propre à la structure, identique quel que soit le chunk
/// qui la construit.
unsafe extern "C" fn s_random(l: L) -> c_int {
    let c = ctx(l);
    let Some(mut rng) = c.buf.as_mut().map(|b| std::mem::replace(&mut b.srng, Rng::new(0))) else { return 0 };
    let r = push_random(c, l, &mut rng, 2);
    if let Some(b) = c.buf.as_mut() {
        b.srng = rng;
    }
    r
}

/// `structure:height(x, z)` : hauteur du sol n'importe où, d'après `height(x, z)` du
/// générateur (à défaut : le relief du jeu).
unsafe extern "C" fn s_height(l: L) -> c_int {
    let c = ctx(l);
    let (x, z) = match (int(c, l, 2), int(c, l, 3)) {
        (Some(x), Some(z)) => (x, z),
        _ => return fail(c, "structure:height(x, z)"),
    };
    let g = &*c.generator;
    match c.height {
        f if f != 0 => {
            // Appel de la fonction height(x, z) du générateur, dans le même état
            (c.api.rawgeti)(l, REGISTRY, f as i64);
            (c.api.pushinteger)(l, x);
            (c.api.pushinteger)(l, z);
            if (c.api.pcallk)(l, 2, 1, 0, 0, std::ptr::null()) != 0 {
                (c.api.settop)(l, -2);
                return fail(c, "height(x, z) a échoué");
            }
            1
        }
        _ => {
            (c.api.pushinteger)(l, g.vanilla.surface(x as i32, z as i32).0 as i64);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Option<(Arc<GameData>, Palette)> {
        Api::shared()?;
        let data = Arc::new(GameData::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("data")).ok()?);
        let d = data.clone();
        let p = Palette::new(&move |n| d.state_from_spec(n));
        Some((data, p))
    }

    fn plugins() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins")
    }

    /// Le générateur d'exemple : îles, eau, phares ; le même résultat sur tous les fils.
    #[test]
    fn example_generator_builds_islands_and_lighthouses() {
        let Some((data, p)) = setup() else { return };
        let g = LuaGenerator::load(&plugins(), "archipel", 42, data.clone(), 64).expect("chargement");
        assert_eq!(g.plugin, "Archipel");
        let name = |s: u32| data.block_name(s).unwrap_or("?").trim_start_matches("minecraft:").to_string();
        let (mut water, mut grass, mut concrete) = (0, 0, 0);
        // Autour du point d'apparition, qui est sur une île
        let (sx, _, sz) = g.spawn().expect("apparition");
        for cx in (sx >> 4) - 10..(sx >> 4) + 10 {
            for cz in (sz >> 4) - 10..(sz >> 4) + 10 {
                let col = g.generate(&p, cx, cz);
                assert_eq!(name(col.get(3, -64, 3)), "bedrock");
                for x in 0..16 {
                    for z in 0..16 {
                        match name(col.get(x, 62, z)).as_str() {
                            "water" => water += 1,
                            _ => {}
                        }
                        for y in 60..110 {
                            let n = name(col.get(x, y, z));
                            if n == "grass_block" {
                                grass += 1;
                            } else if n.ends_with("_concrete") {
                                concrete += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(water > 1000 && grass > 1000, "eau {} herbe {}", water, grass);
        let start = std::time::Instant::now();
        for cx in 100..110 {
            for cz in 0..10 {
                g.generate(&p, cx, cz);
            }
        }
        eprintln!("100 chunks « archipel » en {:?}", start.elapsed());
        assert!(concrete > 0, "aucun phare sur 20 × 20 chunks");
        // Un autre fil (sa propre copie du générateur) donne exactement le même chunk
        let here = g.generate(&p, 3, -2);
        let g2 = g.clone();
        let data2 = data.clone();
        let there = std::thread::spawn(move || {
            let d = data2.clone();
            let p = Palette::new(&move |n| d.state_from_spec(n));
            g2.generate(&p, 3, -2)
        })
        .join()
        .unwrap();
        for y in [-64, 0, 60, 62, 70, 90] {
            for x in 0..16 {
                assert_eq!(here.get(x, y, 7), there.get(x, y, 7));
            }
        }
        // Apparition sur une île : au-dessus de la mer, sur de l'herbe
        let (x, y, z) = g.spawn().expect("apparition");
        let col = g.generate(&p, x >> 4, z >> 4);
        assert!(y > 63, "apparition sous la mer : {}", y);
        assert_eq!(name(col.get((x & 15) as usize, y - 1, (z & 15) as usize)), "grass_block");
    }

    fn write_gen(dir: &Path, name: &str, code: &str) {
        let d = dir.join("Test").join("generators");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(dir.join("Test").join("plugin.yml"), "name: Test\nversion: 1.0.0\nmain: main.lua\napi: 1\n").unwrap();
        std::fs::write(dir.join("Test").join("main.lua"), "").unwrap();
        std::fs::write(d.join(format!("{}.lua", name)), code).unwrap();
    }

    /// Un script fautif ne fait jamais tomber le serveur : erreur signalée, chunk vide ;
    /// une boucle sans fin est arrêtée ; un fichier sans `terrain` est refusé au chargement.
    #[test]
    fn broken_generators_are_contained() {
        let Some((data, p)) = setup() else { return };
        let dir = std::env::temp_dir().join(format!("basalt-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_gen(&dir, "plante", "return { terrain = function(chunk) error('oups') end }");
        write_gen(&dir, "boucle", "return { terrain = function(chunk) while true do end end }");
        write_gen(&dir, "vide", "return { }");
        write_gen(&dir, "syntaxe", "return { terrain = function(chunk) chunk:set( end }");
        write_gen(&dir, "systeme", "return { terrain = function(chunk) os.execute('echo') end }");
        let air = p.air;
        let g = LuaGenerator::load(&dir, "plante", 1, data.clone(), 16).unwrap();
        assert_eq!(g.generate(&p, 0, 0).get(0, 0, 0), air);
        let g = LuaGenerator::load(&dir, "boucle", 1, data.clone(), 16).unwrap();
        let start = std::time::Instant::now();
        assert_eq!(g.generate(&p, 0, 0).get(0, 0, 0), air);
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "boucle sans fin non arrêtée");
        assert!(LuaGenerator::load(&dir, "vide", 1, data.clone(), 16).is_err());
        assert!(LuaGenerator::load(&dir, "syntaxe", 1, data.clone(), 16).is_err());
        assert!(LuaGenerator::load(&dir, "absent", 1, data.clone(), 16).is_err());
        // Pas d'accès au système : os n'existe pas, le chunk reste vide
        let g = LuaGenerator::load(&dir, "systeme", 1, data.clone(), 16).unwrap();
        assert_eq!(g.generate(&p, 0, 0).get(0, 0, 0), air);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `chunk:vanilla()` puis des retouches : un décorateur du terrain du jeu.
    #[test]
    fn a_generator_can_decorate_the_vanilla_terrain() {
        let Some((data, p)) = setup() else { return };
        let dir = std::env::temp_dir().join(format!("basalt-gen-v-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_gen(
            &dir,
            "deco",
            "local GOLD = block('gold_block')\n\
             return { terrain = function(chunk)\n\
               chunk:vanilla()\n\
               local h = chunk:height(8, 8)\n\
               chunk:set(8, h + 1, 8, GOLD)\n\
             end }",
        );
        let g = LuaGenerator::load(&dir, "deco", 12345, data.clone(), 32).unwrap();
        let col = g.generate(&p, 0, 0);
        let v = Vanilla::new(12345).generate(&p, 0, 0);
        assert_eq!(col.get(3, 0, 3), v.get(3, 0, 3), "terrain du jeu sous la décoration");
        let gold = (MIN_Y..=MAX_Y).any(|y| data.block_name(col.get(8, y, 8)) == Some("minecraft:gold_block"));
        assert!(gold, "bloc d'or posé au sommet");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
