//! Plugins Lua 5.4.
//!
//! La bibliothèque Lua est chargée à l'exécution (liblua5.4.so / lua54.dll) :
//! le serveur démarre même sans elle, les plugins sont alors désactivés.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use crate::plugin::manifest::{self, CommandDecl, Manifest, PermDefault};

pub(crate) type L = *mut c_void;
pub(crate) type CFn = unsafe extern "C" fn(L) -> c_int;
/// Fonction appelée par Lua toutes les N instructions (peut lever une erreur Lua).
type Hook = unsafe extern "C-unwind" fn(L, *mut c_void);
type Alloc = unsafe extern "C" fn(*mut c_void, *mut c_void, usize, usize) -> *mut c_void;

/// Code Lua du bac à sable, exécuté avant chaque plugin protégé.
const SANDBOX: &str = include_str!("sandbox.lua");
/// Temps accordé au chargement d'un plugin, puis à chacun de ses appels (millisecondes).
const LOAD_BUDGET_MS: u64 = 3000;
const CALL_BUDGET_MS: u64 = 250;
/// Marge de mémoire pour les petites allocations : le dépassement est alors signalé
/// proprement par le contrôle périodique plutôt qu'au milieu d'un appel du serveur.
const MEM_MARGIN: usize = 8 * 1024 * 1024;

/// Mémoire d'un plugin protégé.
pub(crate) struct MemState {
    pub(crate) used: usize,
    pub(crate) limit: usize,
}

extern "C" {
    /// Allocateur C, utilisé par limited_alloc.
    fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

/// Allocateur donné à Lua : compte la mémoire du plugin et refuse au-delà de sa limite.
pub(crate) unsafe extern "C" fn limited_alloc(ud: *mut c_void, ptr: *mut c_void, osize: usize, nsize: usize) -> *mut c_void {
    let mem = &mut *(ud as *mut MemState);
    let old = if ptr.is_null() { 0 } else { osize };
    if nsize == 0 {
        free(ptr);
        mem.used = mem.used.saturating_sub(old);
        return std::ptr::null_mut();
    }
    if nsize > old {
        let cap = if nsize <= 65_536 { mem.limit + MEM_MARGIN } else { mem.limit };
        if mem.used - old.min(mem.used) + nsize > cap {
            return std::ptr::null_mut();
        }
    }
    let p = realloc(ptr, nsize);
    if !p.is_null() {
        mem.used = mem.used.saturating_sub(old) + nsize;
    }
    p
}

static CLOCK: OnceLock<Instant> = OnceLock::new();
// Échéance de l'appel en cours (millisecondes depuis le démarrage) et mémoire du plugin appelé.
// Propres à chaque fil d'exécution : un plugin ne tourne que sur le fil qui l'a chargé.
thread_local! {
    pub(crate) static DEADLINE_MS: std::cell::Cell<u64> = const { std::cell::Cell::new(u64::MAX) };
    pub(crate) static CURRENT_MEM: std::cell::Cell<*mut MemState> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Plugin en cours d'exécution (vide si aucun), pour nommer le responsable d'un retard.
static CURRENT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Nom du plugin en cours d'exécution (pour les messages d'erreur et les mesures).
pub fn current_plugin() -> String {
    CURRENT.lock().map(|c| c.clone()).unwrap_or_default()
}

/// Note le plugin qui s'exécute, sans allouer à chaque appel.
fn set_current(name: &str) {
    if let Ok(mut c) = CURRENT.lock() {
        c.clear();
        c.push_str(name);
    }
}

/// Dépassements de temps tolérés avant de désactiver un plugin.
const MAX_STRIKES: u32 = 3;

/// Millisecondes depuis le démarrage, pour les limites de temps.
pub(crate) fn now_ms() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Contrôle périodique : interrompt un plugin qui dépasse son temps ou sa mémoire.
pub(crate) unsafe extern "C-unwind" fn watchdog(l: L, _: *mut c_void) {
    let api = &*API.load(Ordering::Relaxed);
    let mem = CURRENT_MEM.with(|m| m.get());
    let message: &[u8] = if now_ms() > DEADLINE_MS.with(|d| d.get()) {
        b"basalt: limite de temps d\xc3\xa9pass\xc3\xa9e (boucle sans fin ?)"
    } else if !mem.is_null() && (*mem).used > (*mem).limit {
        b"basalt: limite de m\xc3\xa9moire d\xc3\xa9pass\xc3\xa9e"
    } else {
        return;
    };
    (api.pushlstring)(l, message.as_ptr() as *const c_char, message.len());
    (api.error)(l);
}

pub(crate) const REGISTRY: c_int = -1_001_000;
const T_BOOLEAN: c_int = 1;
pub(crate) const T_NUMBER: c_int = 3;
pub(crate) const T_STRING: c_int = 4;
pub(crate) const T_TABLE: c_int = 5;
pub(crate) const T_FUNCTION: c_int = 6;

#[cfg(unix)]
mod dl {
    use std::os::raw::{c_char, c_int, c_void};
    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    extern "C" {
        /// Fonctions du système pour charger la bibliothèque Lua à l'exécution.
        fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    pub const NAMES: &[&str] = &[
        "liblua5.4.so.0\0",
        "liblua5.4.so\0",
        "liblua.so.5.4\0",
        "liblua5.4.dylib\0",
        "liblua.5.4.dylib\0",
        "liblua.so\0",
        "liblua.dylib\0",
    ];
    /// Ouvre une bibliothèque (nom terminé par \0) ; nul si elle est absente.
    pub unsafe fn open(name: &str) -> *mut c_void {
        dlopen(name.as_ptr() as *const c_char, 2)
    }
    /// Adresse d'une fonction de la bibliothèque (nom terminé par \0).
    pub unsafe fn sym(h: *mut c_void, name: &str) -> *mut c_void {
        dlsym(h, name.as_ptr() as *const c_char)
    }
}

#[cfg(windows)]
mod dl {
    use std::os::raw::{c_char, c_void};
    #[link(name = "kernel32")]
    extern "system" {
        /// Équivalents Windows.
        fn LoadLibraryA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    pub const NAMES: &[&str] = &["lua54.dll\0", "lua5.4.dll\0", "lua.dll\0"];
    /// Ouvre une bibliothèque (nom terminé par \0) ; nul si elle est absente.
    pub unsafe fn open(name: &str) -> *mut c_void {
        LoadLibraryA(name.as_ptr() as *const c_char)
    }
    /// Adresse d'une fonction de la bibliothèque (nom terminé par \0).
    pub unsafe fn sym(h: *mut c_void, name: &str) -> *mut c_void {
        GetProcAddress(h, name.as_ptr() as *const c_char)
    }
}

/// Fonctions de l'API C de Lua 5.4 dont le serveur a besoin.
pub(crate) struct Api {
    pub(crate) newstate: unsafe extern "C" fn() -> L,
    pub(crate) newstate_alloc: unsafe extern "C" fn(Alloc, *mut c_void) -> L,
    pub(crate) sethook: unsafe extern "C" fn(L, Option<Hook>, c_int, c_int),
    pub(crate) error: unsafe extern "C-unwind" fn(L) -> c_int,
    pub(crate) loadbufferx: unsafe extern "C" fn(L, *const c_char, usize, *const c_char, *const c_char) -> c_int,
    pub(crate) openlibs: unsafe extern "C" fn(L),
    pub(crate) close: unsafe extern "C" fn(L),
    pub(crate) loadfilex: unsafe extern "C" fn(L, *const c_char, *const c_char) -> c_int,
    pub(crate) pcallk: unsafe extern "C" fn(L, c_int, c_int, c_int, isize, *const c_void) -> c_int,
    pub(crate) tolstring: unsafe extern "C" fn(L, c_int, *mut usize) -> *const c_char,
    pub(crate) pushlstring: unsafe extern "C" fn(L, *const c_char, usize) -> *const c_char,
    pub(crate) pushinteger: unsafe extern "C" fn(L, i64),
    pub(crate) pushnumber: unsafe extern "C" fn(L, f64),
    pub(crate) pushboolean: unsafe extern "C" fn(L, c_int),
    pub(crate) pushnil: unsafe extern "C" fn(L),
    pub(crate) pushvalue: unsafe extern "C" fn(L, c_int),
    pub(crate) pushlightuserdata: unsafe extern "C" fn(L, *mut c_void),
    pub(crate) pushcclosure: unsafe extern "C" fn(L, CFn, c_int),
    pub(crate) createtable: unsafe extern "C" fn(L, c_int, c_int),
    pub(crate) setfield: unsafe extern "C" fn(L, c_int, *const c_char),
    pub(crate) setglobal: unsafe extern "C" fn(L, *const c_char),
    pub(crate) rawgeti: unsafe extern "C" fn(L, c_int, i64) -> c_int,
    pub(crate) rawseti: unsafe extern "C" fn(L, c_int, i64),
    pub(crate) settop: unsafe extern "C" fn(L, c_int),
    pub(crate) gettop: unsafe extern "C" fn(L) -> c_int,
    pub(crate) type_: unsafe extern "C" fn(L, c_int) -> c_int,
    pub(crate) toboolean: unsafe extern "C" fn(L, c_int) -> c_int,
    pub(crate) tointegerx: unsafe extern "C" fn(L, c_int, *mut c_int) -> i64,
    pub(crate) tonumberx: unsafe extern "C" fn(L, c_int, *mut c_int) -> f64,
    pub(crate) touserdata: unsafe extern "C" fn(L, c_int) -> *mut c_void,
    pub(crate) ref_: unsafe extern "C" fn(L, c_int) -> c_int,
    pub(crate) unref: unsafe extern "C" fn(L, c_int, c_int),
    pub(crate) getfield: unsafe extern "C" fn(L, c_int, *const c_char) -> c_int,
}

macro_rules! sym {
    ($h:expr, $name:literal) => {{
        let p = dl::sym($h, concat!($name, "\0"));
        if p.is_null() {
            return Err(format!("symbole {} absent de la bibliothèque Lua", $name));
        }
        std::mem::transmute(p)
    }};
}

impl Api {
    /// La bibliothèque Lua, chargée une fois pour tout le serveur (`None` si absente).
    pub(crate) fn shared() -> Option<&'static Api> {
        static SHARED: OnceLock<Option<usize>> = OnceLock::new();
        let p = *SHARED.get_or_init(|| {
            let known = API.load(Ordering::Relaxed);
            if !known.is_null() {
                return Some(known as usize);
            }
            Api::load().ok().map(|a| a as *const Api as usize)
        });
        p.map(|p| unsafe { &*(p as *const Api) })
    }

    /// Cherche Lua 5.4 (à côté du serveur, puis dans le système) et lit toutes ses fonctions.
    fn load() -> Result<&'static Api, String> {
        unsafe {
            let mut h = std::ptr::null_mut();
            // D'abord la bibliothèque livrée à côté du serveur (rien à installer sur la
            // machine), puis celle du système
            if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
                for name in dl::NAMES {
                    let path = dir.join(name.trim_end_matches('\0'));
                    if !path.is_file() {
                        continue;
                    }
                    if let Some(text) = path.to_str() {
                        h = dl::open(&format!("{}\0", text));
                        if !h.is_null() {
                            break;
                        }
                    }
                }
            }
            for name in dl::NAMES {
                if !h.is_null() {
                    break;
                }
                h = dl::open(name);
            }
            if h.is_null() {
                return Err("bibliothèque Lua 5.4 introuvable".into());
            }
            let api = Api {
                newstate: sym!(h, "luaL_newstate"),
                newstate_alloc: sym!(h, "lua_newstate"),
                sethook: sym!(h, "lua_sethook"),
                error: sym!(h, "lua_error"),
                loadbufferx: sym!(h, "luaL_loadbufferx"),
                openlibs: sym!(h, "luaL_openlibs"),
                close: sym!(h, "lua_close"),
                loadfilex: sym!(h, "luaL_loadfilex"),
                pcallk: sym!(h, "lua_pcallk"),
                tolstring: sym!(h, "lua_tolstring"),
                pushlstring: sym!(h, "lua_pushlstring"),
                pushinteger: sym!(h, "lua_pushinteger"),
                pushnumber: sym!(h, "lua_pushnumber"),
                pushboolean: sym!(h, "lua_pushboolean"),
                pushnil: sym!(h, "lua_pushnil"),
                pushvalue: sym!(h, "lua_pushvalue"),
                pushlightuserdata: sym!(h, "lua_pushlightuserdata"),
                pushcclosure: sym!(h, "lua_pushcclosure"),
                createtable: sym!(h, "lua_createtable"),
                setfield: sym!(h, "lua_setfield"),
                setglobal: sym!(h, "lua_setglobal"),
                rawgeti: sym!(h, "lua_rawgeti"),
                rawseti: sym!(h, "lua_rawseti"),
                settop: sym!(h, "lua_settop"),
                gettop: sym!(h, "lua_gettop"),
                type_: sym!(h, "lua_type"),
                toboolean: sym!(h, "lua_toboolean"),
                tointegerx: sym!(h, "lua_tointegerx"),
                tonumberx: sym!(h, "lua_tonumberx"),
                touserdata: sym!(h, "lua_touserdata"),
                ref_: sym!(h, "luaL_ref"),
                unref: sym!(h, "luaL_unref"),
                getfield: sym!(h, "lua_getfield"),
            };
            let api: &'static Api = Box::leak(Box::new(api));
            API.store(api as *const Api as *mut Api, Ordering::Relaxed);
            Ok(api)
        }
    }
}

/// Valeur échangée avec Lua.
#[derive(Debug, Clone, PartialEq)]
pub enum LVal {
    Nil,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    /// Un joueur : devient un objet `Player` côté Lua
    Player(String),
    List(Vec<LVal>),
    Map(Vec<(&'static str, LVal)>),
    /// Table aux clés quelconques (état d'un événement)
    Obj(Vec<(String, LVal)>),
}

impl LVal {
    /// Valeur Lua d'une valeur JSON.
    pub fn from_json(v: &crate::json::Value) -> LVal {
        use crate::json::Value;
        match v {
            Value::Null => LVal::Nil,
            Value::Bool(b) => LVal::Bool(*b),
            Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => LVal::Int(*n as i64),
            Value::Num(n) => LVal::Num(*n),
            Value::Str(s) => LVal::Str(s.clone()),
            Value::Arr(items) => LVal::List(items.iter().map(LVal::from_json).collect()),
            Value::Obj(items) => LVal::Obj(items.iter().map(|(k, v)| (k.clone(), LVal::from_json(v))).collect()),
        }
    }
}

/// Priorités des écouteurs, dans l'ordre d'appel (comme PocketMine).
pub const PRIORITIES: [&str; 6] = ["LOWEST", "LOW", "NORMAL", "HIGH", "HIGHEST", "MONITOR"];

/// Action demandée par un plugin, appliquée par le serveur après l'appel.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SetMotd(String),
    Kick(String, String),
    Stop,
    /// Modifie une donnée d'un joueur : (pseudo, donnée, valeur)
    PlayerSet(String, String, LVal),
    PlayerMessage(String, String),
    PlayerTeleport(String, f32, f32, f32),
    PlayerSave(String),
    /// Donne des objets à un joueur : (pseudo, nom de l'objet, quantité)
    PlayerGive(String, String, u16),
    /// Pose des blocs : (x, y, z, état)
    SetBlocks(Vec<(i32, i32, i32, u32)>),
    /// Personnage non joueur (PNJ) : (plugin, ordre)
    Npc(String, NpcOrder),
}

/// Ordres donnés aux PNJ d'un plugin (voir `server.spawnNpc`).
#[derive(Debug, Clone, PartialEq)]
pub enum NpcOrder {
    /// Fait apparaître (ou replace) un PNJ, pieds en (x, y, z)
    Spawn(String, f32, f32, f32),
    /// Marche en continu : vitesse en blocs par seconde sur x et z (0, 0 : s'arrête)
    Walk(String, f32, f32),
    Teleport(String, f32, f32, f32),
    Remove(String),
    /// Retire tous les PNJ du plugin
    RemoveAll,
}

/// État d'un joueur tel que les plugins le voient pendant un appel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerSnap {
    pub name: String,
    pub display_name: String,
    pub name_tag: String,
    pub health: f32,
    pub max_health: f32,
    pub food: f32,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub pos: (f32, f32, f32),
    pub first_played: u64,
    /// Direction du regard, en degrés
    pub yaw: f32,
    pub pitch: f32,
    /// Nom de l'objet tenu en main (vide : main vide)
    pub held: String,
    /// Opérateur du serveur
    pub op: bool,
    /// Mode de jeu : 0 survie, 1 créatif, 2 aventure, 3 spectateur
    pub gamemode: i32,
    /// Permissions fixées pour ce joueur (elles priment sur la règle par défaut)
    pub perms: Vec<(String, bool)>,
}

/// Informations du serveur visibles depuis les plugins.
#[derive(Clone, Default)]
pub struct Info {
    pub motd: String,
    pub players: Vec<String>,
    /// Position de chaque joueur, dans le même ordre que `players`.
    pub positions: Vec<(f32, f32, f32)>,
    /// Données des joueurs en jeu
    pub pdata: Vec<PlayerSnap>,
    /// Permissions déclarées par les plugins, avec leur règle par défaut
    pub perms: Vec<(String, PermDefault)>,
    /// Vue du monde, le temps d'un appel (lecture des blocs)
    pub world: Option<crate::world::WorldView>,
    /// Noms des blocs et de leurs états
    pub blocks: Option<std::sync::Arc<crate::data::BlockNames>>,
}

/// Un joueur a-t-il cette permission ? Réglage propre au joueur d'abord, puis règle par
/// défaut de la permission ; une permission inconnue est réservée aux opérateurs.
pub fn has_permission(p: &PlayerSnap, declared: &[(String, PermDefault)], node: &str) -> bool {
    crate::permission::check(p.op, &p.perms, declared, node)
}

struct Timer {
    at: u64,
    every: u64,
    func: c_int,
}

struct Ctx {
    api: &'static Api,
    name: String,
    handlers: HashMap<String, Vec<c_int>>,
    commands: Vec<(String, String, c_int)>,
    timers: Vec<Timer>,
    actions: Vec<Action>,
    tick: u64,
    info: Info,
    /// Fonction Lua qui fabrique un objet Player à partir d'un pseudo (0 : absente)
    wrap: c_int,
    /// Commandes annoncées dans plugin.yml
    declared: Vec<CommandDecl>,
}

static API: AtomicPtr<Api> = AtomicPtr::new(std::ptr::null_mut());

/// Retrouve le contexte du plugin, passé en première « upvalue » de chaque fonction.
unsafe fn ctx<'a>(l: L) -> &'a mut Ctx {
    let api = &*API.load(Ordering::Relaxed);
    &mut *((api.touserdata)(l, REGISTRY - 1) as *mut Ctx)
}

/// Arguments des fonctions de l'API : texte (un nombre est converti), fonction gardée dans
/// le registre.
unsafe fn arg_str(c: &Ctx, l: L, i: c_int) -> Option<String> {
    let t = (c.api.type_)(l, i);
    if t != T_STRING && t != T_NUMBER {
        return None;
    }
    let mut len = 0usize;
    let p = (c.api.tolstring)(l, i, &mut len);
    if p.is_null() {
        return None;
    }
    Some(String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len)).into_owned())
}

unsafe fn arg_fn(c: &Ctx, l: L, i: c_int) -> Option<c_int> {
    if (c.api.type_)(l, i) != T_FUNCTION {
        return None;
    }
    (c.api.pushvalue)(l, i);
    Some((c.api.ref_)(l, REGISTRY))
}

/// Signale un appel mal formé dans la console, sans erreur Lua.
fn bad_call(c: &Ctx, what: &str) -> c_int {
    warn!("[{}] appel invalide : {}", c.name, what);
    0
}

/// server.log / server.warn : ligne de console préfixée du nom du plugin.
unsafe extern "C" fn l_log(l: L) -> c_int {
    let c = ctx(l);
    let msg = arg_str(c, l, 1).unwrap_or_default();
    info!("[{}] {}", c.name, msg);
    0
}

unsafe extern "C" fn l_warn(l: L) -> c_int {
    let c = ctx(l);
    let msg = arg_str(c, l, 1).unwrap_or_default();
    warn!("[{}] {}", c.name, msg);
    0
}

/// server.on(événement, fonction)
unsafe extern "C" fn l_on(l: L) -> c_int {
    let c = ctx(l);
    match (arg_str(c, l, 1), arg_fn(c, l, 2)) {
        (Some(ev), Some(f)) => {
            c.handlers.entry(ev).or_insert_with(Vec::new).push(f);
            0
        }
        _ => bad_call(c, "server.on(événement, fonction)"),
    }
}

/// server.command(nom, [description,] fonction)
unsafe extern "C" fn l_command(l: L) -> c_int {
    let c = ctx(l);
    // Forme courte : server.command(nom, fonction), la description vient de plugin.yml
    if (c.api.type_)(l, 2) == T_FUNCTION {
        if let (Some(name), Some(f)) = (arg_str(c, l, 1), arg_fn(c, l, 2)) {
            let name = name.trim_start_matches('/').to_lowercase();
            let desc = c.declared.iter().find(|d| d.name == name).map(|d| d.description.clone()).unwrap_or_default();
            c.commands.push((name, desc, f));
            return 0;
        }
    }
    match (arg_str(c, l, 1), arg_str(c, l, 2), arg_fn(c, l, 3)) {
        (Some(name), Some(desc), Some(f)) => {
            c.commands.push((name.trim_start_matches('/').to_lowercase(), desc, f));
            0
        }
        _ => bad_call(c, "server.command(nom, description, fonction)"),
    }
}

/// Minuteries en ticks, utilisées par afterSeconds / everySeconds.
unsafe fn add_timer(l: L, repeat: bool) -> c_int {
    let c = ctx(l);
    let mut ok = 0;
    let ticks = (c.api.tointegerx)(l, 1, &mut ok);
    match arg_fn(c, l, 2) {
        Some(f) if ok != 0 && ticks >= 1 => {
            let ticks = ticks as u64;
            c.timers.push(Timer { at: c.tick + ticks, every: if repeat { ticks } else { 0 }, func: f });
            0
        }
        _ => bad_call(c, "server.afterSeconds/everySeconds(secondes, fonction)"),
    }
}

unsafe extern "C" fn l_after(l: L) -> c_int {
    add_timer(l, false)
}

unsafe extern "C" fn l_every(l: L) -> c_int {
    add_timer(l, true)
}

/// server.setMotd(texte) / server.getMotd()
unsafe extern "C" fn l_set_motd(l: L) -> c_int {
    let c = ctx(l);
    match arg_str(c, l, 1) {
        Some(m) => {
            c.info.motd = m.clone();
            c.actions.push(Action::SetMotd(m));
            0
        }
        None => bad_call(c, "server.setMotd(texte)"),
    }
}

unsafe extern "C" fn l_motd(l: L) -> c_int {
    let c = ctx(l);
    (c.api.pushlstring)(l, c.info.motd.as_ptr() as *const c_char, c.info.motd.len());
    1
}

/// Pseudos des joueurs en jeu (utilisé par getOnlinePlayers).
unsafe extern "C" fn l_players(l: L) -> c_int {
    let c = ctx(l);
    (c.api.createtable)(l, c.info.players.len() as c_int, 0);
    for (i, p) in c.info.players.iter().enumerate() {
        (c.api.pushlstring)(l, p.as_ptr() as *const c_char, p.len());
        (c.api.rawseti)(l, -2, i as i64 + 1);
    }
    1
}

/// Hauteur des pieds d'après celle des yeux, arrondie au dix-millième (les positions
/// voyagent en nombres à virgule simple : sans arrondi, 64 deviendrait 64,0000027…).
fn feet(eye: f32) -> f64 {
    ((eye as f64 - 1.62) * 10_000.0).round() / 10_000.0
}

/// Nombre fini à cette position.
unsafe fn arg_num(c: &Ctx, l: L, i: c_int) -> Option<f64> {
    let mut ok = 0;
    let v = (c.api.tonumberx)(l, i, &mut ok);
    if ok != 0 && v.is_finite() {
        Some(v)
    } else {
        None
    }
}

/// `server._wrap(fonction)` : enregistre la fabrique d'objets Player (appelé par le préambule).
unsafe extern "C" fn l_set_wrap(l: L) -> c_int {
    let c = ctx(l);
    if let Some(f) = arg_fn(c, l, 1) {
        c.wrap = f;
    }
    0
}

/// `server._pget(pseudo, donnée)` : lit une donnée d'un joueur en jeu.
unsafe extern "C" fn l_pget(l: L) -> c_int {
    let c = ctx(l);
    let (name, key) = match (arg_str(c, l, 1), arg_str(c, l, 2)) {
        (Some(n), Some(k)) => (n, k),
        _ => return 0,
    };
    let p = match c.info.pdata.iter().find(|p| p.name == name) {
        Some(p) => p,
        None => return 0,
    };
    let text = |s: &str| {
        (c.api.pushlstring)(l, s.as_ptr() as *const c_char, s.len());
        1
    };
    let num = |v: f64| {
        (c.api.pushnumber)(l, v);
        1
    };
    match key.as_str() {
        "online" => {
            (c.api.pushboolean)(l, 1);
            1
        }
        "op" => {
            (c.api.pushboolean)(l, p.op as c_int);
            1
        }
        "perm" => {
            let node = arg_str(c, l, 3).unwrap_or_default().to_lowercase();
            (c.api.pushboolean)(l, has_permission(p, &c.info.perms, &node) as c_int);
            1
        }
        "display" => text(&p.display_name),
        "tag" => text(&p.name_tag),
        "health" => num(p.health as f64),
        "max_health" => num(p.max_health as f64),
        "food" => num(p.food as f64),
        "xp_level" => {
            (c.api.pushinteger)(l, p.xp_level as i64);
            1
        }
        "xp_progress" => num(p.xp_progress as f64),
        "first_played" => {
            (c.api.pushinteger)(l, p.first_played as i64);
            1
        }
        "yaw" => num(p.yaw as f64),
        "pitch" => num(p.pitch as f64),
        "held" => text(&p.held),
        "gamemode" => {
            (c.api.pushinteger)(l, p.gamemode as i64);
            1
        }
        "pos" => {
            // Les plugins voient la position des pieds (le serveur garde celle des yeux)
            (c.api.pushnumber)(l, p.pos.0 as f64);
            (c.api.pushnumber)(l, feet(p.pos.1));
            (c.api.pushnumber)(l, p.pos.2 as f64);
            3
        }
        _ => 0,
    }
}

/// `server._pset(pseudo, donnée, valeur)` : demande au serveur de changer une donnée.
/// La copie visible par le plugin est mise à jour tout de suite ; le serveur applique
/// (et borne) le changement à la fin de l'appel.
unsafe extern "C" fn l_pset(l: L) -> c_int {
    let c = ctx(l);
    let (name, key) = match (arg_str(c, l, 1), arg_str(c, l, 2)) {
        (Some(n), Some(k)) => (n, k),
        _ => return bad_call(c, "joueur:setXxx(valeur)"),
    };
    if !c.info.pdata.iter().any(|p| p.name == name) {
        return 0; // joueur parti entre-temps
    }
    let flag = match (c.api.type_)(l, 3) {
        T_BOOLEAN => LVal::Bool((c.api.toboolean)(l, 3) != 0),
        _ => LVal::Nil,
    };
    let value = match key.as_str() {
        "op" => match flag {
            LVal::Bool(_) => flag,
            _ => return bad_call(c, "joueur:setOp(true ou false)"),
        },
        k if k.starts_with("perm:") && k.len() > 5 && k.len() <= 69 => flag,
        "display" | "tag" => match arg_str(c, l, 3) {
            Some(s) => LVal::Str(s),
            None => return bad_call(c, "un texte est attendu"),
        },
        "gamemode" => match arg_num(c, l, 3) {
            Some(v) if [0.0, 1.0, 2.0, 3.0].contains(&v) => LVal::Num(v),
            _ => return bad_call(c, "joueur:setGamemode(0 à 3) : 0 survie, 1 créatif, 2 aventure, 3 spectateur"),
        },
        "health" | "max_health" | "food" | "xp_level" | "xp_progress" => match arg_num(c, l, 3) {
            Some(v) => LVal::Num(v),
            None => return bad_call(c, "un nombre est attendu"),
        },
        _ => return 0,
    };
    if let Some(p) = c.info.pdata.iter_mut().find(|p| p.name == name) {
        match (key.as_str(), &value) {
            ("display", LVal::Str(s)) => p.display_name = s.clone(),
            ("tag", LVal::Str(s)) => p.name_tag = s.clone(),
            ("health", LVal::Num(v)) => p.health = *v as f32,
            ("max_health", LVal::Num(v)) => p.max_health = *v as f32,
            ("food", LVal::Num(v)) => p.food = *v as f32,
            ("xp_level", LVal::Num(v)) => p.xp_level = *v as i32,
            ("xp_progress", LVal::Num(v)) => p.xp_progress = *v as f32,
            ("gamemode", LVal::Num(v)) => p.gamemode = *v as i32,
            ("op", LVal::Bool(b)) => p.op = *b,
            (k, v) if k.starts_with("perm:") => {
                let node = &k[5..];
                p.perms.retain(|(n, _)| n != node);
                if let LVal::Bool(b) = v {
                    p.perms.push((node.to_string(), *b));
                }
            }
            _ => {}
        }
    }
    if c.actions.len() < 4096 {
        c.actions.push(Action::PlayerSet(name, key, value));
    }
    0
}

/// Plus grand volume lu d'un coup par `server._blocks`.
const MAX_READ: i64 = 1_000_000;
/// Plus grand nombre de blocs posés d'un coup (`server._setblocks`).
const MAX_WRITE: usize = 1_000_000;

/// Coordonnée entière, dans les limites du monde.
unsafe fn arg_int(c: &Ctx, l: L, i: c_int) -> Option<i32> {
    arg_num(c, l, i).filter(|v| v.abs() < 3.0e7).map(|v| v.floor() as i32)
}

/// `server._block(x, y, z)` : état du bloc (entier), ou nil hors d'un appel.
unsafe extern "C" fn l_block(l: L) -> c_int {
    let c = ctx(l);
    let (x, y, z) = match (arg_int(c, l, 1), arg_int(c, l, 2), arg_int(c, l, 3)) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => return bad_call(c, "world:getBlock(x, y, z)"),
    };
    match &c.info.world {
        Some(w) => {
            (c.api.pushinteger)(l, w.get(x, y, z) as i64);
            1
        }
        None => 0,
    }
}

/// `server._blocks(x1, y1, z1, x2, y2, z2)` : états de tous les blocs du volume, dans une
/// table plate rangée par y, puis x, puis z (comme on lit les étages d'un bâtiment).
unsafe extern "C" fn l_blocks(l: L) -> c_int {
    let c = ctx(l);
    let mut v = [0i32; 6];
    for (i, slot) in v.iter_mut().enumerate() {
        match arg_int(c, l, i as c_int + 1) {
            Some(n) => *slot = n,
            None => return bad_call(c, "world:getBlocks(x1, y1, z1, x2, y2, z2)"),
        }
    }
    let (x1, x2) = (v[0].min(v[3]), v[0].max(v[3]));
    let (y1, y2) = (v[1].min(v[4]), v[1].max(v[4]));
    let (z1, z2) = (v[2].min(v[5]), v[2].max(v[5]));
    let volume = (x2 as i64 - x1 as i64 + 1) * (y2 as i64 - y1 as i64 + 1) * (z2 as i64 - z1 as i64 + 1);
    if volume > MAX_READ {
        return bad_call(c, "world:getBlocks : volume trop grand (1 000 000 blocs au plus)");
    }
    let w = match &c.info.world {
        Some(w) => w.clone(),
        None => return 0,
    };
    (c.api.createtable)(l, volume as c_int, 0);
    let mut n = 0i64;
    for y in y1..=y2 {
        for x in x1..=x2 {
            for z in z1..=z2 {
                n += 1;
                (c.api.pushinteger)(l, w.get(x, y, z) as i64);
                (c.api.rawseti)(l, -2, n);
            }
        }
    }
    1
}

/// `server._setblocks(t)` : pose des blocs, `t` étant une table plate { x, y, z, état, x, y, z,
/// état, … }. Appliqué par le serveur à la fin de l'appel.
unsafe extern "C" fn l_setblocks(l: L) -> c_int {
    let c = ctx(l);
    if (c.api.type_)(l, 1) != T_TABLE {
        return bad_call(c, "world:setBlocks({ x, y, z, état, … })");
    }
    let mut list: Vec<(i32, i32, i32, u32)> = Vec::new();
    let mut i = 1i64;
    loop {
        let mut q = [0i64; 4];
        for (k, slot) in q.iter_mut().enumerate() {
            (c.api.rawgeti)(l, 1, i + k as i64);
            let mut ok = 0;
            *slot = (c.api.tointegerx)(l, -1, &mut ok);
            (c.api.settop)(l, -2);
            if ok == 0 {
                if k == 0 {
                    // fin de la table
                    if !list.is_empty() && c.actions.len() < 4096 {
                        c.actions.push(Action::SetBlocks(list));
                    }
                    return 0;
                }
                return bad_call(c, "world:setBlocks : entrée incomplète (x, y, z, état)");
            }
        }
        if list.len() >= MAX_WRITE {
            return bad_call(c, "world:setBlocks : trop de blocs d'un coup (1 000 000 au plus)");
        }
        let fits = |v: i64| v.abs() < 30_000_000;
        if fits(q[0]) && fits(q[1]) && fits(q[2]) && (0..=u32::MAX as i64).contains(&q[3]) {
            list.push((q[0] as i32, q[1] as i32, q[2] as i32, q[3] as u32));
        }
        i += 4;
    }
}

/// `server.blockState(nom)` : état par défaut d'un bloc (« stone » ou « minecraft:stone »).
unsafe extern "C" fn l_block_state(l: L) -> c_int {
    let c = ctx(l);
    let name = match arg_str(c, l, 1) {
        Some(n) => n.trim().to_lowercase(),
        None => return 0,
    };
    let full = if name.contains(':') { name } else { format!("minecraft:{}", name) };
    match c.info.blocks.as_ref().and_then(|b| b.defaults.get(&full)) {
        Some(s) => {
            (c.api.pushinteger)(l, *s as i64);
            1
        }
        None => 0,
    }
}

/// `server.blockName(état)` : nom du bloc d'un état (« minecraft:stone »).
unsafe extern "C" fn l_block_name(l: L) -> c_int {
    let c = ctx(l);
    let mut ok = 0;
    let state = (c.api.tointegerx)(l, 1, &mut ok);
    if ok == 0 {
        return 0;
    }
    match c.info.blocks.as_ref().and_then(|b| b.names.get(&(state as u32))) {
        Some(n) => {
            (c.api.pushlstring)(l, n.as_ptr() as *const c_char, n.len());
            1
        }
        None => 0,
    }
}


/// `server._pdo(pseudo, action, …)` : message, téléportation, sauvegarde.
unsafe extern "C" fn l_pdo(l: L) -> c_int {
    let c = ctx(l);
    let (name, verb) = match (arg_str(c, l, 1), arg_str(c, l, 2)) {
        (Some(n), Some(v)) => (n, v),
        _ => return 0,
    };
    if c.actions.len() >= 4096 {
        return 0;
    }
    match verb.as_str() {
        "message" => match arg_str(c, l, 3) {
            Some(text) => c.actions.push(Action::PlayerMessage(name, text)),
            None => return bad_call(c, "joueur:sendMessage(texte)"),
        },
        "teleport" => match (arg_num(c, l, 3), arg_num(c, l, 4), arg_num(c, l, 5)) {
            (Some(x), Some(y), Some(z)) => {
                // Le plugin donne les pieds ; le serveur place les yeux au-dessus
                let eye = y as f32 + crate::player::EYE_HEIGHT;
                if let Some(p) = c.info.pdata.iter_mut().find(|p| p.name == name) {
                    p.pos = (x as f32, eye, z as f32);
                }
                c.actions.push(Action::PlayerTeleport(name, x as f32, eye, z as f32))
            }
            _ => return bad_call(c, "joueur:teleport(x, y, z)"),
        },
        "save" => c.actions.push(Action::PlayerSave(name)),
        "give" => match arg_str(c, l, 3) {
            Some(item) => {
                let count = arg_num(c, l, 4).unwrap_or(1.0).clamp(1.0, 2304.0) as u16;
                c.actions.push(Action::PlayerGive(name, item, count))
            }
            None => return bad_call(c, "joueur:give(objet, quantité)"),
        },
        _ => {}
    }
    0
}

/// Expulse un joueur (joueur:kick).
unsafe extern "C" fn l_kick(l: L) -> c_int {
    let c = ctx(l);
    match arg_str(c, l, 1) {
        Some(name) => {
            let reason = arg_str(c, l, 2).unwrap_or_else(|| "Expulsé du serveur.".to_string());
            c.actions.push(Action::Kick(name, reason));
            0
        }
        None => bad_call(c, "joueur:kick(raison)"),
    }
}

/// `server.spawnNpc(nom, x, y, z)`, `server.walkNpc(nom, vx, vz)`, `server.teleportNpc(nom, x, y, z)`,
/// `server.removeNpc(nom)`, `server.removeNpcs()`.
unsafe fn npc_order(l: L, verb: &str) -> c_int {
    let c = ctx(l);
    if c.actions.len() >= 4096 {
        return 0;
    }
    let name = arg_str(c, l, 1).map(|n| n.chars().filter(|c| !c.is_control()).take(32).collect::<String>());
    let n = |i| arg_num(c, l, i).map(|v| v as f32);
    let order = match (verb, name) {
        ("spawn", Some(name)) => match (n(2), n(3), n(4)) {
            (Some(x), Some(y), Some(z)) if !name.is_empty() => NpcOrder::Spawn(name, x, y, z),
            _ => return bad_call(c, "server.spawnNpc(nom, x, y, z)"),
        },
        ("walk", Some(name)) => match (n(2), n(3)) {
            (Some(vx), Some(vz)) => NpcOrder::Walk(name, vx.clamp(-20.0, 20.0), vz.clamp(-20.0, 20.0)),
            _ => return bad_call(c, "server.walkNpc(nom, vx, vz)"),
        },
        ("teleport", Some(name)) => match (n(2), n(3), n(4)) {
            (Some(x), Some(y), Some(z)) => NpcOrder::Teleport(name, x, y, z),
            _ => return bad_call(c, "server.teleportNpc(nom, x, y, z)"),
        },
        ("remove", Some(name)) => NpcOrder::Remove(name),
        ("remove_all", _) => NpcOrder::RemoveAll,
        _ => return bad_call(c, "server.removeNpc(nom)"),
    };
    c.actions.push(Action::Npc(c.name.clone(), order));
    0
}

/// Fonctions des PNJ, toutes traitées par npc_order.
unsafe extern "C" fn l_npc_spawn(l: L) -> c_int {
    npc_order(l, "spawn")
}
unsafe extern "C" fn l_npc_walk(l: L) -> c_int {
    npc_order(l, "walk")
}
unsafe extern "C" fn l_npc_teleport(l: L) -> c_int {
    npc_order(l, "teleport")
}
unsafe extern "C" fn l_npc_remove(l: L) -> c_int {
    npc_order(l, "remove")
}
unsafe extern "C" fn l_npc_remove_all(l: L) -> c_int {
    npc_order(l, "remove_all")
}

/// server.stop()
unsafe extern "C" fn l_stop(l: L) -> c_int {
    ctx(l).actions.push(Action::Stop);
    0
}

pub struct Plugin {
    pub name: String,
    /// Fiche du plugin (plugin.yml, ou fiche par défaut pour un simple script)
    pub manifest: Manifest,
    api: &'static Api,
    state: L,
    ctx: *mut Ctx,
    /// Compteur de mémoire d'un plugin protégé (nul pour un plugin de confiance)
    mem: *mut MemState,
    /// Dépassements de temps constatés ; au-delà de la limite, le plugin n'est plus appelé
    strikes: std::cell::Cell<u32>,
}

impl Plugin {
    /// Vrai si le plugin tourne dans le bac à sable.
    pub fn sandboxed(&self) -> bool {
        !self.mem.is_null()
    }

    /// Arme les limites pour l'appel qui suit.
    fn arm(&self, budget_ms: u64) {
        let deadline = if self.mem.is_null() { u64::MAX } else { now_ms() + budget_ms };
        DEADLINE_MS.with(|d| d.set(deadline));
        CURRENT_MEM.with(|m| m.set(self.mem));
        set_current(&self.name);
    }

    /// Fin d'un appel : plus de limite de temps ni de compte de mémoire.
    fn disarm(&self) {
        DEADLINE_MS.with(|d| d.set(u64::MAX));
        CURRENT_MEM.with(|m| m.set(std::ptr::null_mut()));
        set_current("");
    }

    /// Pose une valeur Rust sur la pile Lua.
    unsafe fn push(&self, v: &LVal) {
        let a = self.api;
        match v {
            LVal::Nil => (a.pushnil)(self.state),
            LVal::Bool(b) => (a.pushboolean)(self.state, *b as c_int),
            LVal::Int(i) => (a.pushinteger)(self.state, *i),
            LVal::Num(n) => (a.pushnumber)(self.state, *n),
            LVal::Player(name) => {
                // L'objet est fabriqué par le préambule Lua ; à défaut, on donne le pseudo
                let wrap = (*self.ctx).wrap;
                if wrap != 0 {
                    (a.rawgeti)(self.state, REGISTRY, wrap as i64);
                    (a.pushlstring)(self.state, name.as_ptr() as *const c_char, name.len());
                    if (a.pcallk)(self.state, 1, 1, 0, 0, std::ptr::null()) != 0 {
                        (a.settop)(self.state, -2);
                        (a.pushlstring)(self.state, name.as_ptr() as *const c_char, name.len());
                    }
                } else {
                    (a.pushlstring)(self.state, name.as_ptr() as *const c_char, name.len());
                }
            }
            LVal::Str(s) => {
                (a.pushlstring)(self.state, s.as_ptr() as *const c_char, s.len());
            }
            LVal::List(items) => {
                (a.createtable)(self.state, items.len() as c_int, 0);
                for (i, it) in items.iter().enumerate() {
                    self.push(it);
                    (a.rawseti)(self.state, -2, i as i64 + 1);
                }
            }
            LVal::Map(items) => {
                (a.createtable)(self.state, 0, items.len() as c_int);
                for (k, it) in items {
                    self.push(it);
                    let key = CString::new(*k).unwrap_or_default();
                    (a.setfield)(self.state, -2, key.as_ptr());
                }
            }
            LVal::Obj(items) => {
                (a.createtable)(self.state, 0, items.len() as c_int);
                for (k, it) in items {
                    self.push(it);
                    let key = CString::new(k.as_str()).unwrap_or_default();
                    (a.setfield)(self.state, -2, key.as_ptr());
                }
            }
        }
    }

    /// Message d'erreur en haut de la pile.
    unsafe fn error_text(&self) -> String {
        let mut len = 0usize;
        let p = (self.api.tolstring)(self.state, -1, &mut len);
        if p.is_null() {
            "erreur inconnue".to_string()
        } else {
            String::from_utf8_lossy(std::slice::from_raw_parts(p as *const u8, len)).into_owned()
        }
    }

    /// Appelle une fonction Lua référencée ; les erreurs sont journalisées.
    fn call(&self, func: c_int, args: &[LVal]) -> LVal {
        if self.strikes.get() >= MAX_STRIKES {
            return LVal::Nil; // plugin désactivé
        }
        unsafe {
            let a = self.api;
            let base = (a.gettop)(self.state);
            (a.rawgeti)(self.state, REGISTRY, func as i64);
            for v in args {
                self.push(v);
            }
            self.arm(CALL_BUDGET_MS);
            let status = (a.pcallk)(self.state, args.len() as c_int, 1, 0, 0, std::ptr::null());
            self.disarm();
            let res = if status != 0 {
                let text = self.error_text();
                error!("[{}] {}", self.name, text);
                if text.starts_with("basalt: limite de temps") {
                    self.strikes.set(self.strikes.get() + 1);
                    if self.strikes.get() >= MAX_STRIKES {
                        error!("[{}] plugin désactivé : il a dépassé son temps {} fois. Corrigez-le puis tapez « reload ».", self.name, MAX_STRIKES);
                    }
                }
                LVal::Nil
            } else {
                match (a.type_)(self.state, -1) {
                    T_BOOLEAN => LVal::Bool((a.toboolean)(self.state, -1) != 0),
                    T_STRING => LVal::Str(self.error_text()),
                    _ => LVal::Nil,
                }
            };
            (a.settop)(self.state, base);
            res
        }
    }

    /// Fonctions abonnées à cet événement.
    fn handlers(&self, event: &str) -> Vec<c_int> {
        unsafe { (*self.ctx).handlers.get(event).cloned().unwrap_or_default() }
    }
}

impl Drop for Plugin {
    /// Ferme l'état Lua (les finaliseurs du plugin tournent avec une limite de temps).
    fn drop(&mut self) {
        unsafe {
            self.arm(CALL_BUDGET_MS); // les finaliseurs du plugin sont limités eux aussi
            (self.api.close)(self.state);
            self.disarm();
            drop(Box::from_raw(self.ctx));
            if !self.mem.is_null() {
                drop(Box::from_raw(self.mem));
            }
        }
    }
}

/// Plugin repéré sur le disque, avant chargement.
struct Found {
    manifest: Manifest,
    /// Dossier du plugin : son code et ses données
    dir: PathBuf,
    /// Fichier principal
    script: PathBuf,
}

pub struct PluginManager {
    api: Option<&'static Api>,
    dir: PathBuf,
    pub plugins: Vec<Plugin>,
    /// Bac à sable actif pour tous les plugins sauf ceux de `trusted`
    sandbox: bool,
    trusted: Vec<String>,
    memory_limit: usize,
}

impl PluginManager {
    /// Cherche la bibliothèque Lua ; les plugins eux-mêmes sont chargés par load_all.
    pub fn new(dir: &Path) -> PluginManager {
        let api = match Api::load() {
            Ok(a) => Some(a),
            Err(e) => {
                warn!("Plugins désactivés : {}", e);
                None
            }
        };
        PluginManager { api, dir: dir.to_path_buf(), plugins: Vec::new(), sandbox: true, trusted: Vec::new(), memory_limit: 64 * 1024 * 1024 }
    }

    /// Règle la protection : `sandbox` l'active, `trusted` nomme les plugins qui en sont
    /// dispensés, `memory_mb` plafonne la mémoire de chaque plugin protégé.
    pub fn set_sandbox(&mut self, sandbox: bool, trusted: &str, memory_mb: usize) {
        self.sandbox = sandbox;
        self.trusted = trusted.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        self.memory_limit = memory_mb.clamp(4, 4096) * 1024 * 1024;
    }

    /// Lua est-il installé ? Sans lui, le serveur tourne sans plugins.
    pub fn available(&self) -> bool {
        self.api.is_some()
    }

    /// Charge `plugins/*.lua` et `plugins/<nom>/main.lua`.
    pub fn load_all(&mut self, info: &Info, constants: &[(&'static str, LVal)]) {
        let api = match self.api {
            Some(a) => a,
            None => return,
        };
        let _ = std::fs::create_dir_all(&self.dir);
        // 1. Inventaire : dossiers avec plugin.yml, dossiers avec main.lua, simples fichiers .lua
        let mut found: Vec<Found> = Vec::new();
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&self.dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        entries.sort();
        for p in entries {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let candidate = if p.is_dir() && p.join("plugin.yml").is_file() {
                let source = match std::fs::read_to_string(p.join("plugin.yml")) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Plugin {} : plugin.yml illisible ({})", stem, e);
                        continue;
                    }
                };
                match Manifest::parse(&source) {
                    Ok(m) => {
                        let script = p.join(&m.main);
                        if !script.is_file() {
                            error!("Plugin {} : fichier principal « {} » introuvable", m.name, m.main);
                            continue;
                        }
                        Found { manifest: m, dir: p.clone(), script }
                    }
                    Err(e) => {
                        error!("Plugin {} : plugin.yml refusé — {}", stem, e);
                        continue;
                    }
                }
            } else if p.is_dir() && p.join("main.lua").is_file() {
                Found { manifest: Manifest::implicit(&stem, "main.lua"), dir: p.clone(), script: p.join("main.lua") }
            } else if p.is_file() && p.extension().map(|x| x == "lua").unwrap_or(false) {
                Found { manifest: Manifest::implicit(&stem, "main.lua"), dir: self.dir.join(&stem), script: p.clone() }
            } else {
                continue;
            };
            if !manifest::valid_name(&candidate.manifest.name) {
                error!("Plugin {} : nom invalide (lettres, chiffres, _ et - seulement)", candidate.manifest.name);
                continue;
            }
            if found.iter().any(|f| f.manifest.name.eq_ignore_ascii_case(&candidate.manifest.name)) {
                error!("Plugin {} : un autre plugin porte déjà ce nom, celui de {} est ignoré", candidate.manifest.name, p.display());
                continue;
            }
            found.push(candidate);
        }
        // 2. Ordre : chaque plugin après ceux dont il dépend
        let manifests: Vec<Manifest> = found.iter().map(|f| f.manifest.clone()).collect();
        let (order, rejected) = manifest::load_order(&manifests);
        for (i, why) in rejected {
            error!("Plugin {} non chargé : {}", manifests[i].name, why);
        }
        for i in order {
            let f = &found[i];
            let name = f.manifest.name.clone();
            // Une dépendance qui a échoué au chargement entraîne ceux qui en ont besoin
            if let Some(missing) = f.manifest.depend.iter().find(|d| !self.plugins.iter().any(|p| p.name.eq_ignore_ascii_case(d))) {
                error!("Plugin {} non chargé : sa dépendance « {} » n'a pas pu être chargée", name, missing);
                continue;
            }
            match self.load_one(api, f, info, constants) {
                Ok(p) => {
                    let label = if f.manifest.version == "0" { name.clone() } else { format!("{} v{}", name, f.manifest.version) };
                    if p.sandboxed() {
                        info!("Plugin chargé : {} (protégé)", label);
                    } else {
                        warn!("Plugin chargé : {} — SANS protection : il a les mêmes droits que le serveur", label);
                    }
                    let handled: Vec<String> = unsafe { (*p.ctx).commands.iter().map(|c| c.0.clone()).collect() };
                    for c in &f.manifest.commands {
                        if !handled.contains(&c.name) {
                            warn!("[{}] la commande « {} » est annoncée dans plugin.yml mais le plugin ne la traite pas", name, c.name);
                        }
                    }
                    self.plugins.push(p);
                }
                Err(e) => error!("Plugin {} : {}", name, e),
            }
        }
        self.dispatch("enable", &[]);
    }

    /// Charge un plugin : état Lua neuf, préambule, puis son fichier principal.
    fn load_one(
        &self,
        api: &'static Api,
        found: &Found,
        info: &Info,
        constants: &[(&'static str, LVal)],
    ) -> Result<Plugin, String> {
        let name = found.manifest.name.as_str();
        let path: &Path = &found.script;
        unsafe {
            let protected = self.sandbox && !self.trusted.iter().any(|t| t == name);
            let mem = if protected {
                Box::into_raw(Box::new(MemState { used: 0, limit: self.memory_limit }))
            } else {
                std::ptr::null_mut()
            };
            let state = if protected { (api.newstate_alloc)(limited_alloc, mem as *mut c_void) } else { (api.newstate)() };
            if state.is_null() {
                if !mem.is_null() {
                    drop(Box::from_raw(mem));
                }
                return Err("mémoire insuffisante".into());
            }
            (api.openlibs)(state);
            let ctx = Box::into_raw(Box::new(Ctx {
                api,
                name: name.to_string(),
                handlers: HashMap::new(),
                commands: Vec::new(),
                timers: Vec::new(),
                actions: Vec::new(),
                tick: 0,
                info: info.clone(),
                wrap: 0,
                declared: found.manifest.commands.clone(),
            }));
            let plugin = Plugin { name: name.to_string(), manifest: found.manifest.clone(), api, state, ctx, mem, strikes: std::cell::Cell::new(0) };

            (api.createtable)(state, 0, 24);
            let funcs: [(&str, CFn); 25] = [
                ("spawnNpc\0", l_npc_spawn),
                ("walkNpc\0", l_npc_walk),
                ("teleportNpc\0", l_npc_teleport),
                ("removeNpc\0", l_npc_remove),
                ("removeNpcs\0", l_npc_remove_all),
                ("_block\0", l_block),
                ("_blocks\0", l_blocks),
                ("_setblocks\0", l_setblocks),
                ("blockState\0", l_block_state),
                ("blockName\0", l_block_name),
                ("_wrap\0", l_set_wrap),
                ("_pget\0", l_pget),
                ("_pset\0", l_pset),
                ("_pdo\0", l_pdo),
                ("log\0", l_log),
                ("warn\0", l_warn),
                ("on\0", l_on),
                ("command\0", l_command),
                ("_after\0", l_after),
                ("_every\0", l_every),
                ("setMotd\0", l_set_motd),
                ("getMotd\0", l_motd),
                ("_players\0", l_players),
                ("_kick\0", l_kick),
                ("stop\0", l_stop),
            ];
            for (fname, f) in funcs.iter() {
                (api.pushlightuserdata)(state, ctx as *mut c_void);
                (api.pushcclosure)(state, *f, 1);
                (api.setfield)(state, -2, fname.as_ptr() as *const c_char);
            }
            for (k, v) in constants {
                plugin.push(v);
                let key = CString::new(*k).unwrap_or_default();
                (api.setfield)(state, -2, key.as_ptr());
            }
            // server.plugin : la fiche du plugin, telle que lue dans plugin.yml
            let m = &found.manifest;
            plugin.push(&LVal::Map(vec![
                ("name", LVal::Str(m.name.clone())),
                ("version", LVal::Str(m.version.clone())),
                ("description", LVal::Str(m.description.clone())),
                ("website", LVal::Str(m.website.clone())),
                ("authors", LVal::List(m.authors.iter().map(|a| LVal::Str(a.clone())).collect())),
                ("depend", LVal::List(m.depend.iter().map(|a| LVal::Str(a.clone())).collect())),
            ]));
            (api.setfield)(state, -2, "plugin\0".as_ptr() as *const c_char);
            (api.setglobal)(state, "server\0".as_ptr() as *const c_char);

            {
                // Dossier réservé au plugin : le seul où il peut lire et écrire
                let own = found.dir.clone();
                let _ = std::fs::create_dir_all(&own);
                let mut prefix = own.to_string_lossy().replace('\\', "/");
                if !prefix.ends_with('/') {
                    prefix.push('/');
                }
                (api.pushlstring)(state, prefix.as_ptr() as *const c_char, prefix.len());
                (api.setglobal)(state, "__basalt_dir\0".as_ptr() as *const c_char);
                (api.pushboolean)(state, protected as c_int);
                (api.setglobal)(state, "__basalt_protected\0".as_ptr() as *const c_char);
                plugin.arm(LOAD_BUDGET_MS);
                let failed = (api.loadbufferx)(state, SANDBOX.as_ptr() as *const c_char, SANDBOX.len(), "=preambule\0".as_ptr() as *const c_char, "t\0".as_ptr() as *const c_char) != 0
                    || (api.pcallk)(state, 0, 0, 0, 0, std::ptr::null()) != 0;
                plugin.disarm();
                if failed {
                    return Err(format!("préambule : {}", plugin.error_text()));
                }
            }
            if protected {
                // 8 = contrôle toutes les N instructions
                (api.sethook)(state, Some(watchdog), 8, 10_000);
            }
            let cpath = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| "chemin invalide")?;
            // Mode « t » : du texte Lua seulement, jamais de code compilé
            let mode = if protected { "t\0".as_ptr() as *const c_char } else { std::ptr::null() };
            if (api.loadfilex)(state, cpath.as_ptr(), mode) != 0 {
                return Err(plugin.error_text());
            }
            plugin.arm(LOAD_BUDGET_MS);
            let status = (api.pcallk)(state, 0, 0, 0, 0, std::ptr::null());
            plugin.disarm();
            if status != 0 {
                return Err(plugin.error_text());
            }
            Ok(plugin)
        }
    }

    /// Événement disable, puis fermeture de tous les plugins.
    pub fn unload_all(&mut self) {
        self.dispatch("disable", &[]);
        self.plugins.clear();
    }

    /// Oublie la vue du monde à la fin d'un appel : le serveur peut alors modifier ses blocs
    /// sans les recopier.
    pub fn clear_world(&mut self) {
        for p in &self.plugins {
            unsafe {
                (*p.ctx).info.world = None;
            }
        }
    }

    /// Donne aux plugins l'état du serveur (joueurs, monde) pour les prochains appels.
    pub fn set_info(&mut self, info: &Info, tick: u64) {
        for p in &self.plugins {
            unsafe {
                (*p.ctx).info = info.clone();
                (*p.ctx).tick = tick;
            }
        }
    }

    /// Appelle tous les gestionnaires d'un événement et renvoie leurs résultats.
    pub fn dispatch(&mut self, event: &str, args: &[LVal]) -> Vec<LVal> {
        let mut out = Vec::new();
        for p in &self.plugins {
            for f in p.handlers(event) {
                out.push(p.call(f, args));
            }
        }
        out
    }

    /// Événement à la manière de PocketMine : chaque écouteur, par ordre de priorité puis de
    /// chargement des plugins, reçoit l'état laissé par le précédent (JSON) et renvoie l'état
    /// modifié. Renvoie l'état final, ou None si personne n'écoute.
    pub fn fire(&mut self, event: &str, state: &str) -> Option<String> {
        let mut current = state.to_string();
        let mut heard = false;
        for prio in PRIORITIES {
            let key = format!("{}#{}", event, prio);
            for p in &self.plugins {
                for f in p.handlers(&key) {
                    heard = true;
                    let arg = match crate::json::parse(&current) {
                        Ok(v) => LVal::from_json(&v),
                        Err(_) => return Some(current),
                    };
                    // L'écouteur renvoie le nouvel état ; nil (MONITOR, erreur) le laisse tel quel
                    if let LVal::Str(next) = p.call(f, &[arg]) {
                        if next.len() <= 1 << 20 && crate::json::parse(&next).is_ok() {
                            current = next;
                        }
                    }
                }
            }
        }
        if heard {
            Some(current)
        } else {
            None
        }
    }

    /// Événements écoutés (sans la priorité), pour ne pas déranger les plugins pour rien.
    pub fn listening(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in &self.plugins {
            for k in unsafe { (*p.ctx).handlers.keys() } {
                if !out.contains(k) {
                    out.push(k.clone());
                }
            }
        }
        out
    }

    /// Exécute les minuteries arrivées à échéance.
    pub fn run_timers(&mut self, tick: u64) {
        for p in &self.plugins {
            let timers = unsafe { std::mem::take(&mut (*p.ctx).timers) };
            let mut keep = Vec::new();
            for t in timers {
                if t.at > tick {
                    keep.push(t);
                    continue;
                }
                p.call(t.func, &[]);
                if t.every > 0 {
                    keep.push(Timer { at: tick + t.every, every: t.every, func: t.func });
                } else {
                    unsafe { (p.api.unref)(p.state, REGISTRY, t.func) };
                }
            }
            unsafe { (*p.ctx).timers.extend(keep) };
        }
    }

    /// Commandes déclarées par les plugins : (nom, description, plugin).
    pub fn commands(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for p in &self.plugins {
            for (n, d, _) in unsafe { (*p.ctx).commands.iter() } {
                out.push((n.clone(), d.clone(), p.name.clone()));
            }
        }
        out
    }

    /// Exécute une commande de plugin ; renvoie `false` si elle n'existe pas.
    /// Lance une commande de plugin. `sender` est le joueur qui l'a tapée, ou `None` pour la
    /// console ; la fonction du plugin reçoit `(arguments, joueur ou nil)`.
    pub fn run_command(&mut self, name: &str, args: &[&str]) -> bool {
        self.run_command_as(name, args, None)
    }

    /// Lance la commande d'un plugin ; `sender` est le joueur, ou None pour la console.
    pub fn run_command_as(&mut self, name: &str, args: &[&str], sender: Option<&str>) -> bool {
        for p in &self.plugins {
            let found = unsafe { (*p.ctx).commands.iter().find(|c| c.0 == name).map(|c| c.2) };
            if let Some(f) = found {
                let list = LVal::List(args.iter().map(|a| LVal::Str(a.to_string())).collect());
                let who = sender.map(|s| LVal::Player(s.to_string())).unwrap_or(LVal::Nil);
                p.call(f, &[list, who]);
                return true;
            }
        }
        false
    }

    /// Permissions déclarées par les plugins chargés.
    pub fn permissions(&self) -> Vec<(String, PermDefault)> {
        let mut out: Vec<(String, PermDefault)> = Vec::new();
        for p in &self.plugins {
            for d in &p.manifest.permissions {
                if !out.iter().any(|(n, _)| *n == d.name) {
                    out.push((d.name.clone(), d.default));
                }
            }
        }
        out
    }

    /// Permission exigée par une commande de plugin, d'après plugin.yml.
    pub fn command_permission(&self, command: &str) -> Option<String> {
        self.plugins.iter().flat_map(|p| p.manifest.commands.iter()).find(|c| c.name == command).and_then(|c| c.permission.clone())
    }

    /// Récupère les actions demandées par les plugins depuis le dernier appel.
    pub fn take_actions(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        for p in &self.plugins {
            out.append(unsafe { &mut (*p.ctx).actions });
        }
        out
    }
}
