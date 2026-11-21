//! Boucle principale : relie le réseau, la console et les plugins.

mod mobs;

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::auth::{self, AuthError, KeyFetcher, KeySet};
use crate::bedrock;
use crate::timings;
use crate::json::Value as Json;
use crate::command::VanillaCommands;
use crate::block::{placement, Block, BlockRegistry, Interaction};
use crate::data::Prop;
use crate::tile::{self, TileContext};
use crate::item::VanillaItems;
use crate::config::Config;
use crate::permission::{self, NameList, OpList};
use crate::player::{clean_text, GameMode, PlayerData};
use crate::entity::item_physics::{self, Cell, Step};
use crate::lua::{Action, Info, LVal, NpcOrder, PlayerSnap};
use crate::plugin::host::PluginHost;
use crate::inventory::{self, PlayerWindows, Target};
use crate::data::GameData;
use crate::nethernet::NetherNet;
use crate::raknet::{RakEvent, RakServer};
use crate::signaling::{json_string, sdp_fingerprints, sdp_identity, sdp_without_identity, Request, Signaling};

const TICK: Duration = Duration::from_millis(50);
/// Retard rattrapé au plus (ticks enchaînés sans pause) ; au-delà, il est abandonné.
const MAX_CATCH_UP: Duration = Duration::from_secs(1);

/// Couleur d'un joueur dans la liste (AARRGGBB), choisie d'après son identifiant.
fn list_colour(id: i64) -> u32 {
    const COLOURS: [u32; 8] = [0xffed_eded, 0xff54_ffff, 0xfff7_0c0c, 0xff00_c800, 0xfff9_a602, 0xffb1_54ff, 0xff40_a0ff, 0xffff_82c8];
    COLOURS[(id.unsigned_abs() % COLOURS.len() as u64) as usize]
}

/// Nom courant d'un paquet du jeu, pour les rapports de mesures.
fn packet_label(id: u32) -> &'static str {
    match id {
        1 => " (login)",
        9 => " (chat)",
        30 => " (clic sur un bloc)",
        31 => " (objet en main)",
        33 => " (interaction)",
        44 => " (geste du bras)",
        47 => " (fermeture d'écran)",
        77 => " (commande)",
        123 => " (son)",
        144 => " (mouvement)",
        147 => " (inventaire)",
        156 => " (paquet refusé par le jeu)",
        193 => " (réglages réseau)",
        _ => "",
    }
}

/// Mémoire occupée par le serveur, en Mo (Linux uniquement : lue dans /proc).
fn memory_mb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// Le paquet de déplacement (MovePlayer) en mode normal (déplacement d'un autre joueur) n'est
/// connu que pour les protocoles d'avant la 1.26.60 : ces versions reçoivent MoveActorAbsolute.
/// La téléportation, elle, a été relevée (`bedrock::teleport_player`).
fn move_player_known(protocol: i32) -> bool {
    protocol < 2225
}

/// Blocs désignés par un texte de commande (« stone », « minecraft:stone », « stone,dirt »),
/// pour /setblock et /fill : leur état par défaut.
fn parse_pattern(data: Option<&GameData>, text: &str) -> Result<Vec<u32>, String> {
    let data = data.ok_or("les données du jeu ne sont pas chargées")?;
    let mut out = Vec::new();
    for part in text.split(',').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).take(16) {
        let full = if part.contains(':') { part.clone() } else { format!("minecraft:{}", part) };
        match data.default_state(&full) {
            Some(state) => out.push(state),
            None => return Err(format!("bloc inconnu : {}", part)),
        }
    }
    if out.is_empty() {
        return Err("aucun bloc indiqué".into());
    }
    Ok(out)
}

/// Blocs changés en une fois : envoyés un par un s'ils sont peu nombreux, sinon par chunks.
#[derive(Debug, Default)]
struct Outcome {
    changed: usize,
    chunks: Vec<(i32, i32)>,
    blocks: Vec<((i32, i32, i32), u32)>,
}

/// Catalogue des objets pour les données attachées aux blocs : noms, numéros, numéros de pile.
struct ItemCatalog<'a> {
    data: &'a GameData,
    next: &'a mut i32,
}

impl TileContext for ItemCatalog<'_> {
    /// Les blocs à données (fours, coffres) passent par ce catalogue pour nommer leurs objets.
    fn name_of(&self, id: i16) -> Option<String> {
        self.data.item_by_id(id).map(|i| i.name.clone())
    }
    fn id_of(&self, name: &str) -> Option<i16> {
        self.data.item(name).map(|i| i.id)
    }
    /// Identifiant réseau unique pour un objet d'inventaire.
    fn next_net_id(&mut self) -> i32 {
        *self.next = if *self.next >= i32::MAX - 1 { 1_000_000_000 } else { *self.next + 1 };
        *self.next
    }
}

// --- Limites anti-abus par joueur ---------------------------------------------
/// Lots de paquets acceptés par seconde.
const MAX_BATCHES_PER_SEC: u32 = 300;
/// Octets de paquets (après décompression) acceptés par seconde.
const MAX_BYTES_PER_SEC: usize = 6 * 1024 * 1024;
/// Octets décompressés par seconde, tous joueurs confondus ; au-delà, les lots sont ignorés.
const MAX_GLOBAL_BYTES_PER_SEC: usize = 64 * 1024 * 1024;
/// Délai pour envoyer le login après la connexion.
const LOGIN_DEADLINE: Duration = Duration::from_secs(10);
/// Délai pour apparaître dans le monde après la connexion.
const SPAWN_DEADLINE: Duration = Duration::from_secs(90);
/// Chunks envoyés par joueur et par tick quand il se déplace (4 = 80 par seconde).
const CHUNKS_PER_TICK: usize = 4;
/// Chunks d'un joueur confiés aux fils d'encodage et pas encore envoyés, au plus.
const MAX_CHUNKS_IN_FLIGHT: u64 = 16;
/// Vitesses horizontales maximales tolérées, en blocs par seconde (marge comprise).
const SPEED_SURVIVAL: f32 = 14.0;
const SPEED_CREATIVE: f32 = 30.0;
/// Déplacement maximal accepté en un seul paquet, en blocs.
const MAX_STEP: f32 = 5.0;
const MAX_STEP_VERTICAL: f32 = 8.0;
/// Les listes de joueurs visibles sont recalculées tous les N ticks.
const VISIBILITY_EVERY: u64 = 5;
/// Logins en attente de vérification ; au-delà, les nouveaux sont refusés poliment.
const MAX_PENDING_LOGINS: usize = 512;

struct LoginJob {
    addr: SocketAddr,
    body: Vec<u8>,
    keys: Option<Arc<KeySet>>,
    xbox_auth: bool,
    /// Joueur simulé admis sans compte Xbox (local-bots=true, depuis cette machine)
    local_bot: bool,
    now: u64,
    /// Clé prouvée par le joueur à l'ouverture de sa liaison NetherNet
    bound_cpk: Option<String>,
}

enum LoginOutcome {
    /// Login lu ; le booléen indique si le compte Xbox a été vérifié.
    Accepted(Box<bedrock::Login>, bool),
    Refused { message: &'static str, log: Option<String>, refresh_keys: bool },
}

/// Analyse un login et vérifie l'identité du joueur. S'exécute hors de la boucle principale.
fn process_login(job: &LoginJob) -> LoginOutcome {
    let refused = |message, log| LoginOutcome::Refused { message, log, refresh_keys: false };
    let mut login = match bedrock::parse_login(&job.body) {
        Ok(l) => l,
        Err(e) => return refused("Connexion invalide.", Some(format!("connexion refusée ({})", e))),
    };
    if !job.xbox_auth {
        return LoginOutcome::Accepted(Box::new(login), false);
    }
    if job.local_bot && login.name.len() > 3 && login.name.get(..3).map(|p| p.eq_ignore_ascii_case("bot")).unwrap_or(false) {
        return LoginOutcome::Accepted(Box::new(login), false);
    }
    let keys = match &job.keys {
        Some(k) => k,
        None => {
            return refused(
                "Le serveur ne peut pas vérifier les comptes Xbox pour le moment. Réessayez dans une minute.",
                None,
            )
        }
    };
    match auth::verify(login.token.as_deref(), &login.client_jwt, keys, job.now) {
        Ok(id) => {
            // Sur NetherNet, le login doit porter la clé déjà prouvée par la liaison chiffrée :
            // le login d'un autre joueur, même authentique, ne peut donc pas être rejoué.
            if let Some(bound) = &job.bound_cpk {
                if *bound != id.cpk {
                    return refused(
                        "Échec de la vérification du compte Xbox.",
                        Some(format!("login de « {} » : la clé ne correspond pas à celle de la liaison", id.name)),
                    );
                }
            }
            login.name = id.name;
            login.xuid = id.xuid;
            LoginOutcome::Accepted(Box::new(login), true)
        }
        Err(AuthError::NotSignedIn) => {
            refused("Connectez-vous à votre compte Xbox pour rejoindre ce serveur.", None)
        }
        Err(AuthError::UnknownKey) => LoginOutcome::Refused {
            message: "Vérification du compte Xbox impossible pour le moment. Réessayez dans une minute.",
            log: Some("jeton Xbox signé par une clé inconnue : mise à jour des clés demandée".to_string()),
            refresh_keys: true,
        },
        Err(AuthError::Invalid(why)) => refused(
            "Échec de la vérification du compte Xbox.",
            Some(format!("authentification Xbox refusée pour « {} » : {}", login.name, why)),
        ),
    }
}

/// Fils de travail qui traitent les logins en parallèle de la boucle principale.
struct LoginWorkers {
    jobs: Sender<LoginJob>,
    results: Receiver<(SocketAddr, LoginOutcome)>,
    pending: usize,
    threads: usize,
}

impl LoginWorkers {
    /// Fils de vérification des logins : la signature Xbox coûte cher, elle ne passe pas
    /// par la boucle du serveur.
    fn start() -> LoginWorkers {
        let (jobs, job_rx) = mpsc::channel::<LoginJob>();
        let (result_tx, results) = mpsc::channel();
        let job_rx = Arc::new(Mutex::new(job_rx));
        // Un cœur reste à la boucle principale
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        let threads = cores.saturating_sub(1).max(1).min(8);
        for _ in 0..threads {
            let job_rx = job_rx.clone();
            let result_tx = result_tx.clone();
            std::thread::spawn(move || loop {
                let job = match job_rx.lock() {
                    Ok(rx) => rx.recv(),
                    Err(_) => return,
                };
                let job = match job {
                    Ok(j) => j,
                    Err(_) => return, // serveur arrêté
                };
                let outcome = std::panic::catch_unwind(|| process_login(&job)).unwrap_or(LoginOutcome::Refused {
                    message: "Connexion invalide.",
                    log: Some("erreur interne pendant l'analyse du login".to_string()),
                    refresh_keys: false,
                });
                if result_tx.send((job.addr, outcome)).is_err() {
                    return;
                }
            });
        }
        LoginWorkers { jobs, results, pending: 0, threads }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Stage {
    Handshake,
    Login,
    /// Login reçu, vérification en cours sur un autre cœur
    Authenticating,
    /// Échange sur les packs de ressources
    Packs,
    /// StartGame envoyé, en attente de la demande de terrain
    Spawning,
    InGame,
    Done,
}

struct Client {
    stage: Stage,
    compression: bool,
    /// Chiffrement de la session (joueurs en RakNet ; NetherNet est déjà chiffré par DTLS)
    crypt: Option<crate::crypto::Crypt>,
    /// Jeton de chiffrement envoyé, réponse du jeu attendue : rien d'autre n'est accepté
    handshake_pending: bool,
    name: Option<String>,
    protocol: i32,
    connected_at: Instant,
    window: Instant,
    batches: u32,
    bytes: usize,
    id: i64,
    pack_responses: u8,
    chunks_sent: bool,
    joined: bool,
    /// Dernière position annoncée par le jeu
    pos: (f32, f32, f32),
    /// Chunk au centre de la zone chargée, et rayon accordé
    center: (i32, i32),
    radius: i32,
    /// Chunks déjà envoyés, et chunks restant à envoyer (les plus proches d'abord)
    loaded: HashSet<(i32, i32)>,
    queue: VecDeque<(i32, i32)>,
    pitch: f32,
    yaw: f32,
    head_yaw: f32,
    /// Tick du dernier déplacement, et tick de la dernière diffusion reçue des autres
    moved_tick: u64,
    /// Dernière orientation (regard, corps, tête) diffusée aux autres joueurs
    shown_look: (f32, f32, f32),
    /// Mise au point : paquets de mouvement reçus, sonde en cours et tick de la dernière sonde
    inputs: u64,
    /// Bloc donné par la commande « give », posé par les clics droits (inventaire provisoire)
    /// Inventaire complet du joueur (sac, barre, curseur, armure, seconde main)
    inv: PlayerWindows,
    /// Un inventaire non vide a déjà été envoyé (sert à interpréter un refus du jeu)
    inv_touched: bool,
    /// Mode de jeu du joueur (celui du serveur à son arrivée, puis /gamemode)
    mode: GameMode,
    /// Mort, en attente de réapparition (tick de la mort)
    dead: Option<u64>,
    /// Tick du dernier coup reçu : un joueur ne peut pas être frappé plus de 2 fois par seconde
    hurt_at: u64,
    /// Hauteur des pieds la plus haute depuis qu'il a quitté le sol (dégâts de chute)
    fall_peak: Option<f32>,
    /// Tick du dernier bloc actionné (porte, levier), pour ignorer les clics répétés
    last_toggle: u64,
    /// Tick de la dernière interaction avec un bloc (porte, coffre…)
    last_interact: u64,
    /// Accroupi / en course, d'après les mouvements envoyés par le jeu
    sneaking: bool,
    sprinting: bool,
    /// Apparence annoncée par le jeu à la connexion (None : apparence par défaut)
    skin: Option<Arc<crate::player::Skin>>,
    /// Tick du dernier geste du bras relayé aux autres joueurs
    last_swing: u64,
    /// Objet en main tel que les autres joueurs le voient (identifiant, présent ou non)
    shown_held: Option<(i16, u8)>,
    /// Anti-abus du chat : début de la fenêtre et messages envoyés dedans
    chat_window: Instant,
    chat_count: u32,
    /// Diagnostic : interactions déjà journalisées
    interactions_logged: u8,
    /// Diagnostic : interactions illisibles déjà journalisées
    unread_logged: u8,
    gesture_logged: u8,
    last_input: Instant,
    /// Le signal d'apparition reste à envoyer, une fois le terrain initial parti
    spawn_pending: bool,
    /// Chunks confiés aux fils d'encodage : numéro du prochain, du prochain à envoyer, et
    /// ceux déjà prêts (envoyés dans l'ordre, du centre vers l'extérieur)
    chunk_next: u64,
    chunk_sent: u64,
    chunk_ready: std::collections::BTreeMap<u64, EncodedChunk>,
    probe: u8,
    probe_tick: u64,
    synced_tick: u64,
    /// Distance horizontale encore autorisée (se recharge avec le temps)
    move_budget: f32,
    move_budget_at: Instant,
    corrections: u32,
    /// Autres joueurs actuellement affichés chez ce client
    visible: HashSet<i64>,
    /// Joueurs inscrits dans sa liste des joueurs : identifiant d'entité → identifiant universel
    listed: HashMap<i64, [u8; 16]>,
    /// Objets au sol que son jeu affiche
    items_seen: HashSet<i64>,
    /// Créatures que son jeu affiche
    mobs_seen: HashSet<i64>,
    /// Données du joueur (vie, expérience, noms…), chargées à son entrée
    data: Option<PlayerData>,
    /// Vrai si le joueur vient pour la première fois (aucune sauvegarde trouvée)
    first_visit: bool,
    /// Le jeu a signalé un paquet illisible : son départ ne dit rien sur la liste d'objets
    reported_error: bool,
}

pub struct Server {
    pub cfg: Config,
    rak: RakServer,
    clients: HashMap<SocketAddr, Client>,
    plugins: PluginHost,
    motd: String,
    tick: u64,
    running: bool,
    next_id: i64,
    /// Personnages non joueurs des plugins, par nom en minuscules
    npcs: HashMap<String, Npc>,
    /// Créatures (animaux) du monde
    mobs: HashMap<i64, mobs::Mob>,
    mob_rng: crate::world::generator::Rng,
    /// Le jeu a refusé l'apparition d'une créature : plus d'animaux
    mobs_refused: bool,
    /// Chunks déjà peuplés d'animaux (une fois par chunk, à la première visite)
    populated: HashSet<(i32, i32)>,
    /// Animaux changés depuis le dernier enregistrement
    mobs_dirty: bool,
    /// Objets posés au sol
    ground: HashMap<i64, GroundItem>,
    /// Noms des blocs, partagés avec les plugins
    plugin_blocks: Option<Arc<crate::data::BlockNames>>,
    /// Le jeu refuse les objets au sol : le butin retourne directement dans l'inventaire
    drops_refused: bool,
    #[allow(dead_code)]
    chunk_no_biomes: Vec<u8>,
    /// Écriture de LevelChunk en cours d'essai (mise au point avec le vrai jeu)
    chunk_variant: u8,
    /// Dernière connexion ayant fait changer de variante (une seule fois par connexion)
    chunk_variant_by: Option<SocketAddr>,
    /// Paquets d'état du joueur que le jeu a refusés : ils ne sont plus envoyés
    refused_extras: HashSet<u32>,
    /// Paquets relevés auprès du serveur officiel : protocole -> identifiant -> contenu
    captured: HashMap<i32, HashMap<u32, Vec<u8>>>,
    /// Les mêmes, déjà emballés et compressés (protocole, identifiant) : envoyés tels quels
    captured_ready: HashMap<(i32, u32), Arc<Vec<u8>>>,
    /// Paquets relevés que le jeu a refusés : ils ne sont plus envoyés
    refused_captured: HashSet<u32>,
    last_save: Instant,
    /// Opérateurs du serveur (pseudos en minuscules), enregistrés dans ops.txt
    ops: OpList,
    /// Liste blanche (pseudos en minuscules), enregistrée dans white-list.txt
    whitelist: NameList,
    /// Édition du monde en volume : presse-papiers et historique de chaque joueur (pseudo en minuscules)
    /// L'avertissement « téléportation indisponible » a déjà été affiché
    /// Heure du monde au démarrage (en ticks de jeu), décalée par /time
    time_base: i64,
    /// Le jeu a refusé le paquet qui fait apparaître un autre joueur : il n'est plus envoyé
    add_player_refused: bool,
    /// Le jeu a refusé le paquet de déplacement des autres joueurs : il n'est plus envoyé
    move_actor_refused: bool,
    /// Idem pour l'objet tenu en main et pour le geste du bras des autres joueurs
    equip_refused: bool,
    animate_refused: bool,
    /// Idem pour la liste des joueurs (noms et skins)
    player_list_refused: bool,
    /// Boutons enfoncés à relâcher : (tick, position, état au repos)
    releases: Vec<(u64, (i32, i32, i32), u32)>,
    started: Instant,
    world: crate::world::World,
    /// Point d'apparition du monde (bloc où se tiennent les pieds)
    spawn: (i32, i32, i32),
    /// Envoie la liste d'objets réduite (nécessaire à « give ») ; repasse à la liste vide
    /// si le jeu la refuse ou décroche juste après l'apparition.
    /// Liste d'objets envoyée : 2 complète (données du jeu), 1 réduite, 0 vide. Le serveur
    /// descend d'un cran si le jeu refuse la liste, gèle ou décroche juste après l'apparition.
    items_level: u8,
    /// Données du jeu (objets, blocs), si le dossier `data/` est présent
    data: Option<Arc<GameData>>,
    /// Tous les blocs du jeu, avec leurs règles (construit à partir des données)
    blocks: Option<BlockRegistry>,
    /// Eau et lave qui s'écoulent (absent sans les données du jeu)
    liquids: Option<crate::world::liquid::Liquids>,
    /// Prochain numéro de pile pour les objets créés par les blocs (fours)
    tile_net_id: i32,
    /// Gros paquets déjà compressés, par (identifiant, taille, empreinte du contenu)
    packet_cache: HashMap<(u32, usize, u64), Arc<Vec<u8>>>,
    /// Résultats des recherches `/locate`, faites sur un fil à part : (destinataire, message)
    locate_tx: std::sync::mpsc::Sender<(Option<SocketAddr>, String)>,
    locate_rx: std::sync::mpsc::Receiver<(Option<SocketAddr>, String)>,
    /// Recherches `/locate` en cours
    locate_busy: Arc<std::sync::atomic::AtomicUsize>,
    /// Envoie les numéros de pile avec les objets ; coupé si le jeu refuse ce format
    item_net_ids: bool,
    /// Expérience en cours (voir `experiment_name`) et verrou une fois qu'une expérience tient
    experiment: u8,
    experiment_locked: bool,
    global_window: Instant,
    global_bytes: usize,
    auth_keys: Option<Arc<KeySet>>,
    logins: LoginWorkers,
    key_fetcher: Option<KeyFetcher>,
    last_key_error: String,
    perf: Perf,
    /// Signalisation HTTP de NetherNet (port TCP du serveur)
    signaling: Option<Signaling>,
    /// Transport NetherNet (liaison WebRTC), si disponible
    nether: Option<crate::network::nethernet_thread::NetherNetHandle>,
    chunk_pool: ChunkPool,
    /// Demandes d'entrée NetherNet en attente de leur session (préparée sur un fil à part)
    join_waiting: HashMap<SocketAddr, Request>,
}

/// Mesure de la charge : le serveur vise 20 ticks par seconde.
struct Perf {
    window: Instant,
    ticks: u32,
    worst_late: Duration,
    /// Dernière fenêtre terminée : (ticks par seconde, pire retard d'un tick)
    last: (f32, Duration),
    traffic: crate::raknet::Traffic,
    /// Trafic par seconde sur la dernière fenêtre
    rates: (u64, u64, u64, u64),
    warned: Option<Instant>,
}

/// Mise au point du blocage du jeu après l'apparition : chaque expérience retire une
/// partie de ce que le serveur envoie, pour trouver laquelle fige le jeu.
const EXPERIMENTS: u8 = 6;

/// Nom d'une expérience, pour le journal.
fn experiment_name(exp: u8) -> &'static str {
    match exp {
        0 => "envoi complet, listes vides",
        1 => "plaines, terrain vide (aucun bloc)",
        2 => "plaines, sans les paquets d'état du joueur",
        3 => "plaines, sans la liste d'objets",
        4 => "plaines, terrain vide, sans état du joueur ni liste d'objets",
        _ => "plaines, envoi complet",
    }
}

/// Expériences qui retirent la liste des objets, ou l'état du joueur.
fn experiment_skips_items(exp: u8) -> bool {
    exp == 3 || exp == 4
}

fn experiment_skips_state(exp: u8) -> bool {
    exp == 2 || exp == 4
}

/// Objet posé au sol.
struct GroundItem {
    item: i16,
    /// Bloc correspondant à l'objet (pour son affichage)
    block: u32,
    count: u8,
    /// Position (centre de la base), vitesse, contact avec le sol
    body: item_physics::Body,
    /// Immobile au sol : recalculé seulement de temps en temps
    resting: bool,
    /// Dernière position envoyée aux joueurs
    sent: (f32, f32, f32),
    /// Tick d'apparition
    born: u64,
    /// Âge en ticks, compté seulement quand un joueur est assez près (comme dans le jeu, où un
    /// objet ne vieillit que dans une zone chargée) : disparition à 5 minutes
    age: u64,
    /// Ramassable à partir de ce tick
    pickup_at: u64,
}

/// Durée de vie d'un objet au sol : 5 minutes.
const ITEM_LIFETIME: u64 = 6000;
/// Objets au sol au plus : au-delà, les plus anciens disparaissent.
const MAX_GROUND_ITEMS: usize = 2000;

/// Ce qui se trouve sous les pieds d'un joueur.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Footing {
    Air,
    Solid,
    Liquid,
}

/// Cause d'un dégât ou d'une mort.
#[derive(Clone, PartialEq, Debug)]
enum Cause {
    Fall,
    Void,
    /// Frappé par ce joueur (nom affiché)
    Player(String),
    Command,
    Plugin,
}

/// L'événement a-t-il été annulé par un écouteur ?
fn cancelled(out: &Option<Json>) -> bool {
    out.as_ref().and_then(|v| v.get("cancelled")).and_then(|c| c.as_bool()).unwrap_or(false)
}

/// Texte rendu par les écouteurs (sans caractères de contrôle, longueur bornée).
fn text_of(out: &Option<Json>, key: &str, max: usize) -> Option<String> {
    let s = out.as_ref()?.get(key)?.as_str()?;
    Some(s.chars().filter(|c| !c.is_control()).take(max).collect())
}

/// Hauteur des yeux au-dessus des pieds.
fn player_eye_height() -> f32 {
    crate::player::EYE_HEIGHT
}

/// Dégâts d'une chute de `distance` blocs : un demi-cœur par bloc au-delà de 3.
fn fall_damage(distance: f32) -> f32 {
    (distance - 3.0).ceil().max(0.0)
}

/// Chunk encodé (contenu, puis paquet compressé) par un fil d'encodage.
struct EncodedChunk {
    x: i32,
    z: i32,
    /// Version du chunk au moment de l'encodage (voir `World::chunk_version`)
    version: u64,
    packet: Vec<u8>,
}

struct ChunkJob {
    addr: SocketAddr,
    /// Identifiant du joueur : une nouvelle connexion depuis la même adresse ne reçoit pas
    /// les chunks encodés pour l'ancienne
    owner: i64,
    seq: u64,
    x: i32,
    z: i32,
    version: u64,
    view: crate::world::WorldView,
    exp: u8,
    features: crate::versions::Features,
    compression: bool,
}

/// Fils qui encodent et compressent les chunks à envoyer (le terrain y est aussi généré au
/// besoin), pour que la boucle du jeu n'ait plus qu'à les expédier.
struct ChunkPool {
    jobs: Sender<ChunkJob>,
    done: Receiver<(SocketAddr, i64, u64, EncodedChunk)>,
}

impl ChunkPool {
    /// Fils qui encodent et compressent les chunks à envoyer.
    fn new() -> ChunkPool {
        let (jobs, inbox) = mpsc::channel::<ChunkJob>();
        let (outbox, done) = mpsc::channel();
        let inbox = Arc::new(Mutex::new(inbox));
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).saturating_sub(1).clamp(1, 4);
        for i in 0..threads {
            let (inbox, outbox): (Arc<Mutex<Receiver<ChunkJob>>>, Sender<(SocketAddr, i64, u64, EncodedChunk)>) = (inbox.clone(), outbox.clone());
            let _ = std::thread::Builder::new().name(format!("chunks-{}", i)).spawn(move || {
                crate::utils::lower_thread_priority();
                loop {
                    let job = match inbox.lock().unwrap_or_else(|e| e.into_inner()).recv() {
                        Ok(j) => j,
                        Err(_) => return,
                    };
                    let payload = job.view.chunk_payload(job.x, job.z);
                    let pk = build_chunk(job.exp, job.x, job.z, payload, job.features);
                    // Hors de la boucle du jeu : compression la plus forte (8 fois plus petit environ)
                    let packet = bedrock::encode_packet_with(bedrock::ID_LEVEL_CHUNK, &pk, job.compression, crate::utils::deflate::Level::Best);
                    if outbox.send((job.addr, job.owner, job.seq, EncodedChunk { x: job.x, z: job.z, version: job.version, packet })).is_err() {
                        return;
                    }
                }
            });
        }
        ChunkPool { jobs, done }
    }
}

/// Paquet LevelChunk complet, selon ce que gère la version du joueur.
fn build_chunk(exp: u8, x: i32, z: i32, payload: (u32, Vec<u8>), features: crate::versions::Features) -> Vec<u8> {
    if features.chunk_biome_update_flag && (exp == 1 || exp == 4) {
        // Expérience de mise au point : aucun sous-chunk, seulement les biomes et l'octet des
        // blocs de bordure
        let mut payload = Vec::new();
        for _ in 0..24 {
            payload.extend_from_slice(&[1, 2]);
        }
        payload.push(0);
        return bedrock::level_chunk_probe(x, z, 0, &payload, false);
    }
    let (sections, payload) = payload;
    if features.chunk_biome_update_flag {
        return bedrock::level_chunk_probe(x, z, sections, &payload, false);
    }
    bedrock::level_chunk_counted(x, z, sections, &payload, features)
}

/// Paquets propres à Bedrock, que la version Java ne connaît pas.
fn bedrock_extra(id: u32) -> bool {
    bedrock::EXTRA_IDS.contains(&id)
}

impl Server {
    /// Ouvre le réseau, charge les données du jeu et le monde, puis les plugins.
    pub fn new(cfg: Config) -> std::io::Result<Server> {
        let rak = RakServer::bind(
            &format!("{}:{}", cfg.ip, cfg.port),
            cfg.max_players as usize + 8,
            Duration::from_secs(cfg.timeout_secs),
        )?;
        let mut rak = rak;
        rak.max_sessions = cfg.max_players as usize + 32;
        rak.max_per_ip = cfg.max_per_ip;
        if cfg.port6 != 0 && cfg.port != 0 {
            rak.listen_v6(cfg.port6);
        }
        rak.debug = cfg.debug;
        rak.max_new_sessions = (cfg.max_players / 8).max(40);
        let plugins = PluginHost::new(Path::new(&cfg.plugins_dir), cfg.plugin_sandbox, &cfg.plugin_trusted, cfg.plugin_memory_mb);
        let (locate_tx, locate_rx) = std::sync::mpsc::channel();
        bedrock::COMPRESS.store(cfg.network_compression, std::sync::atomic::Ordering::Relaxed);
        let mut s = Server {
            motd: cfg.server_name.clone(),
            cfg,
            rak,
            clients: HashMap::new(),
            plugins,
            tick: 0,
            running: true,
            next_id: 1,
            npcs: HashMap::new(),
            mobs: HashMap::new(),
            mob_rng: mobs::mob_rng(),
            mobs_refused: false,
            populated: HashSet::new(),
            mobs_dirty: false,
            ground: HashMap::new(),
            plugin_blocks: None,
            drops_refused: false,
            global_window: Instant::now(),
            global_bytes: 0,
            chunk_no_biomes: bedrock::flat_chunk_payload_with(false),
            chunk_variant: 1, // la variante 0 a déjà été refusée par le jeu 1.26.65
            chunk_variant_by: None,
            refused_extras: HashSet::new(),
            captured: HashMap::new(),
            captured_ready: HashMap::new(),
            refused_captured: HashSet::new(),
            last_save: Instant::now(),
            ops: OpList::default(),
            whitelist: NameList::default(),
            time_base: 6000,
            add_player_refused: false,
            move_actor_refused: false,
            equip_refused: false,
            animate_refused: false,
            player_list_refused: false,
            releases: Vec::new(),
            started: Instant::now(),
            world: crate::world::World::new(),
            spawn: (0, -60, 0),
            items_level: 1,
            data: None,
            blocks: None,
            liquids: None,
            tile_net_id: 1_000_000_000,
            packet_cache: HashMap::new(),
            locate_tx,
            locate_rx,
            locate_busy: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            item_net_ids: true,
            experiment: 0,
            // Réglage retenu après essais avec le jeu 1.26.65 : l'expérience 0. Les listes
            // d'objets et de biomes doivent être envoyées, même vides ; sans elles le jeu gèle.
            // Une liste de biomes mal formée, ou une liste d'entités vide, le font décrocher.
            experiment_locked: true,
            auth_keys: None,
            logins: LoginWorkers::start(),
            key_fetcher: None,
            last_key_error: String::new(),
            signaling: None,
            nether: None,
            chunk_pool: ChunkPool::new(),
            join_waiting: HashMap::new(),
            perf: Perf {
                window: Instant::now(),
                ticks: 0,
                worst_late: Duration::from_millis(0),
                last: (20.0, Duration::from_millis(0)),
                traffic: Default::default(),
                rates: (0, 0, 0, 0),
                warned: None,
            },
        };
        s.start_auth();
        s.update_pong();
        Ok(s)
    }

    /// Charge les clés de vérification Xbox et lance leur mise à jour en tâche de fond.
    fn start_auth(&mut self) {
        if !self.cfg.xbox_auth {
            warn!("Authentification Xbox désactivée (xbox-auth=false) : les pseudos ne sont pas vérifiés");
            return;
        }
        if !self.cfg.auth_keys_file.is_empty() {
            match KeySet::load(Path::new(&self.cfg.auth_keys_file)) {
                Ok(k) => {
                    info!("Authentification Xbox : {} clé(s) lue(s) dans {}", k.len(), self.cfg.auth_keys_file);
                    self.auth_keys = Some(Arc::new(k));
                }
                Err(e) => error!("Authentification Xbox : {} — aucun joueur ne pourra se connecter", e),
            }
            return;
        }
        let cache = Path::new("auth-keys.json");
        if let Ok(k) = KeySet::load(cache) {
            info!("Authentification Xbox : {} clé(s) reprise(s) du cache", k.len());
            self.auth_keys = Some(Arc::new(k));
        }
        self.key_fetcher = Some(KeyFetcher::spawn(self.cfg.game_version.clone(), cache.to_path_buf()));
    }

    /// Clés Xbox téléchargées en arrière-plan : à prendre si elles sont arrivées.
    fn poll_auth(&mut self) {
        let result = match &self.key_fetcher {
            Some(f) => f.poll(),
            None => return,
        };
        match result {
            Some(Ok(keys)) => {
                if self.auth_keys.is_none() || !self.last_key_error.is_empty() {
                    info!("Authentification Xbox prête : {} clé(s) de vérification", keys.len());
                }
                self.last_key_error.clear();
                self.auth_keys = Some(Arc::new(keys));
            }
            Some(Err(e)) => {
                if e != self.last_key_error {
                    let effect = if self.auth_keys.is_some() {
                        "les clés en cache restent utilisées"
                    } else {
                        "les joueurs sont refusés en attendant"
                    };
                    warn!("Clés d'authentification Xbox indisponibles ({}) ; {}", e, effect);
                    self.last_key_error = e;
                }
            }
            None => {}
        }
    }

    /// Adresse réellement ouverte (utile quand le port 0 laisse le système choisir).
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.rak.socket.local_addr()
    }

    /// Pseudos des joueurs connectés.
    fn players(&self) -> Vec<String> {
        self.clients.values().filter_map(|c| c.name.clone()).collect()
    }

    /// Photo de l'état du serveur donnée aux plugins à chaque appel.
    fn info(&self) -> Info {
        let mut info = Info {
            motd: self.motd.clone(),
            players: Vec::new(),
            positions: Vec::new(),
            pdata: Vec::new(),
            perms: self.plugins.permissions(),
            world: Some(self.world.view()),
            blocks: self.plugin_blocks.clone(),
        };
        for c in self.clients.values() {
            if let Some(n) = &c.name {
                info.players.push(n.clone());
                info.positions.push(c.pos);
            }
            if let Some(d) = &c.data {
                info.pdata.push(PlayerSnap {
                    name: d.name.clone(),
                    display_name: d.display_name.clone(),
                    name_tag: d.name_tag.clone(),
                    health: d.health,
                    max_health: d.max_health,
                    food: d.food,
                    xp_level: d.xp_level,
                    xp_progress: d.xp_progress,
                    pos: c.pos,
                    first_played: d.first_played,
                    yaw: c.yaw,
                    pitch: c.pitch,
                    held: Some(c.inv.held()).filter(|s| !s.is_empty()).and_then(|s| self.data.as_ref().and_then(|g| g.item_by_id(s.id))).map(|i| i.name.clone()).unwrap_or_default(),
                    op: self.ops.is_op(&d.name),
                    gamemode: c.mode.save_id(),
                    perms: d.permissions.clone(),
                });
            }
        }
        info
    }

    /// Charge les listes du serveur (opérateurs et liste blanche), rangées à côté de
    /// server.properties : elles valent pour tous les mondes.
    fn load_ops(&mut self) {
        let path = self.cfg.root.join("ops.txt");
        // Une version précédente rangeait ops.txt dans le dossier du monde : on le déplace
        let old = std::path::Path::new(&self.cfg.world_dir).join("ops.txt");
        if !path.exists() && old.exists() {
            match std::fs::rename(&old, &path).or_else(|_| std::fs::copy(&old, &path).map(|_| ())) {
                Ok(()) => info!("Liste des opérateurs déplacée de {} vers {}", old.display(), path.display()),
                Err(e) => warn!("Liste des opérateurs non déplacée ({}) : {}", old.display(), e),
            }
        }
        self.ops = OpList::load(&path);
        self.whitelist = NameList::load(&self.cfg.root.join("white-list.txt"));
        if self.cfg.white_list {
            info!("Liste blanche active : {} joueur(s) inscrit(s), plus {} opérateur(s)", self.whitelist.len(), self.ops.len());
            if !self.cfg.xbox_auth {
                warn!("Liste blanche sans authentification Xbox (xbox-auth=false) : les pseudos ne sont pas vérifiés, n'importe qui peut prendre celui d'un joueur inscrit");
            }
        }
    }

    /// Ce pseudo peut-il entrer ? Toujours si la liste blanche est éteinte ; sinon il faut
    /// y être inscrit ou être opérateur.
    fn is_whitelisted(&self, name: &str) -> bool {
        !self.cfg.white_list || self.whitelist.contains(name) || self.ops.is_op(name)
    }

    /// Commande `whitelist` de la console.
    fn whitelist_command(&mut self, args: &[&str]) {
        match (args.first().copied().unwrap_or(""), args.get(1)) {
            (state @ ("on" | "off"), _) => {
                self.cfg.white_list = state == "on";
                if let Err(e) = self.cfg.store("white-list", if self.cfg.white_list { "true" } else { "false" }) {
                    warn!("Réglage non enregistré dans server.properties : {}", e);
                }
                info!("Liste blanche {}", if self.cfg.white_list { "activée : seuls les joueurs inscrits et les opérateurs peuvent entrer" } else { "désactivée : tout le monde peut entrer" });
                if self.cfg.white_list {
                    if !self.cfg.xbox_auth {
                        warn!("xbox-auth=false : les pseudos ne sont pas vérifiés, la liste blanche peut être contournée");
                    }
                    // Les joueurs déjà en jeu qui n'y figurent pas sont expulsés
                    let out: Vec<String> = self.players().into_iter().filter(|n| !self.is_whitelisted(n)).collect();
                    for name in out {
                        self.kick_player(&name, "Le serveur vient de passer en liste blanche.");
                    }
                }
            }
            (verb @ ("add" | "remove"), Some(name)) => match self.whitelist.set(name, verb == "add") {
                Ok(true) => {
                    info!("{} {} la liste blanche", name, if verb == "add" { "ajouté à" } else { "retiré de" });
                    if verb == "remove" && !self.is_whitelisted(name) {
                        self.kick_player(name, "Vous avez été retiré de la liste blanche.");
                    }
                }
                Ok(false) => info!("{} {} la liste blanche", name, if verb == "add" { "est déjà dans" } else { "n'est pas dans" }),
                Err(e) => warn!("Liste blanche non enregistrée : {}", e),
            },
            ("list", _) => {
                let list = self.whitelist.list();
                info!("Liste blanche {} — {} joueur(s) : {}", if self.cfg.white_list { "active" } else { "éteinte" }, list.len(), list.join(", "));
            }
            ("reload", _) => {
                self.whitelist.reload();
                info!("Liste blanche relue : {} joueur(s)", self.whitelist.len());
            }
            _ => info!("Usage : whitelist on | off | add <joueur> | remove <joueur> | list | reload"),
        }
    }

    /// Donne ou retire le rang d'opérateur, et l'enregistre.
    fn set_op(&mut self, name: &str, op: bool) {
        match self.ops.set(name, op) {
            Ok(true) => {
                info!("{} {} opérateur", name, if op { "est maintenant" } else { "n'est plus" });
                // Ses commandes changent : le jeu reçoit la nouvelle liste
                if let Some(addr) = self.addr_of(name) {
                    self.send_commands(&addr);
                }
            }
            // Rien n'a changé : on le dit, sinon la commande semble sans effet
            Ok(false) => info!("{} {}", name, if op { "est déjà opérateur" } else { "n'est pas opérateur" }),
            Err(e) => warn!("Liste des opérateurs non enregistrée : {}", e),
        }
    }

    /// Heure en secondes depuis 1970.
    fn now_unix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }

    /// Adresse réseau du joueur qui porte ce pseudo.
    fn addr_of(&self, name: &str) -> Option<SocketAddr> {
        self.clients.iter().find(|(_, c)| c.name.as_deref() == Some(name)).map(|(a, _)| *a)
    }

    /// Chargement d'un joueur à son entrée (ou initialisation à sa première visite).
    /// Renvoie vrai s'il vient pour la première fois.
    fn load_player(&mut self, addr: &SocketAddr) -> bool {
        let name = match self.clients.get(addr) {
            Some(c) if c.data.is_none() => match &c.name {
                Some(n) => n.clone(),
                None => return false,
            },
            _ => return false,
        };
        // Les données d'un joueur appartiennent au serveur, pas à un monde : elles sont rangées
        // dans `players/`, à côté de server.properties (comme ops.txt et la liste blanche).
        let dir = self.cfg.root.clone();
        let old_dir = std::path::PathBuf::from(&self.cfg.world_dir);
        let now = Self::now_unix();
        // Sa dernière sauvegarde (départ juste avant) peut être encore en cours d'écriture :
        // on l'attend, pour ne pas relire un fichier dépassé
        if crate::background::DISK.pending() > 0 {
            let _t = timings::scope("Connexion — attente d'une sauvegarde en cours");
            crate::background::DISK.flush(Duration::from_secs(5));
        }
        let mut moved = false;
        let found = match PlayerData::load(&dir, &name) {
            // Rien ici : peut-être un fichier d'avant, rangé dans le dossier du monde
            Ok(None) if old_dir != dir => {
                let old = PlayerData::load(&old_dir, &name);
                moved = matches!(old, Ok(Some(_)));
                old
            }
            other => other,
        };
        let (mut data, fresh) = match found {
            Ok(Some(d)) => {
                if moved {
                    // Repris de l'ancien emplacement : réenregistré au nouveau, l'ancien est retiré
                    match d.save(&dir) {
                        Ok(()) => {
                            let old = PlayerData::path(&old_dir, &name);
                            let _ = std::fs::remove_file(&old);
                            let _ = std::fs::remove_file(old.with_extension("nbt"));
                            let _ = std::fs::remove_file(old.with_extension("txt"));
                            // Le dossier disparaît s'il est vide (sans effet sinon)
                            let _ = std::fs::remove_dir(old_dir.join("players"));
                            info!("Données de {} déplacées du dossier du monde vers players/", name);
                        }
                        Err(e) => warn!("Données de {} : déplacement vers players/ impossible ({})", name, e),
                    }
                }
                (d, false)
            }
            Ok(None) => (PlayerData::new(&name, now), true),
            Err(e) => {
                warn!("Données de {} illisibles ({}) : le joueur repart de zéro", name, e);
                (PlayerData::new(&name, now), true)
            }
        };
        data.name = name.clone();
        data.last_played = now;
        // Parti mort (fichier d'une version précédente, ou arrêt brutal) : il revient en pleine santé
        if data.is_dead() {
            data.health = data.max_health;
        }
        // Le joueur retrouve le mode de jeu qu'il avait en partant
        let saved_mode = data.game_type;
        // Inventaire : les objets sont retrouvés par leur nom
        let items: Vec<(String, usize, i16, u8)> = data
            .items
            .iter()
            .filter_map(|(container, slot, item, count)| {
                let id = self.item_id(item)?;
                Some((container.clone(), *slot as usize, id, *count))
            })
            .collect();
        if let Some(c) = self.clients.get_mut(addr) {
            for (container, slot, id, count) in items {
                // Sac et barre à leur place, armure et seconde main sur le joueur
                match container.as_str() {
                    "main" => c.inv.restore(slot, id, count),
                    other => c.inv.restore_worn(other, slot, id, count),
                }
                c.inv_touched = true;
            }
            if let (Some(mode), false) = (saved_mode, fresh) {
                c.mode = GameMode::from_save_id(mode);
            }
            c.data = Some(data);
            c.first_visit = fresh;
        }
        fresh
    }

    /// État à jour d'un joueur, prêt à être enregistré.
    fn snapshot_player(&self, c: &Client) -> Option<PlayerData> {
        let mut d = c.data.clone()?;
        if c.joined && c.dead.is_some() {
            // Parti pendant l'écran de mort : il revient au point de réapparition, en pleine santé
            d.pos = self.spawn_point();
            d.health = d.max_health;
            d.has_pos = true;
        } else if c.joined {
            d.pos = c.pos;
            d.rotation = (c.yaw.rem_euclid(360.0), c.pitch.clamp(-90.0, 90.0));
            d.has_pos = true;
        }
        d.last_played = Self::now_unix();
        d.game_type = Some(c.mode.save_id());
        // Dossier du monde, comme PocketMine le note
        d.world = std::path::Path::new(&self.cfg.world_dir).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        d.items.clear();
        let name_of = |id: i16| self.data.as_ref().and_then(|g| g.item_by_id(id)).map(|i| i.name.clone());
        let groups: [(&str, &[inventory::ItemStack]); 3] =
            [("main", &c.inv.main.slots), ("armor", &c.inv.armor.slots), ("offhand", &c.inv.offhand.slots)];
        for (container, slots) in groups {
            for (i, s) in slots.iter().enumerate() {
                if !s.is_empty() {
                    if let Some(item) = name_of(s.id) {
                        d.items.push((container.to_string(), i as u8, item, s.count));
                    }
                }
            }
        }
        Some(d)
    }

    /// Sauvegarde d'un joueur : l'état est relevé ici, l'écriture se fait sur le fil des écritures.
    fn store_player(&self, c: &Client) {
        if let Some(d) = self.snapshot_player(c) {
            let root = self.cfg.root.clone();
            crate::background::DISK.run(move || {
                if let Err(e) = d.save(&root) {
                    warn!("Sauvegarde de {} impossible : {}", d.name, e);
                }
            });
        }
    }

    /// Sauvegarde de tous les joueurs en jeu (les plugins sont prévenus par l'événement « save »).
    fn save_players(&mut self) {
        let names: Vec<String> = self.clients.values().filter(|c| c.joined && c.data.is_some()).filter_map(|c| c.name.clone()).collect();
        for name in names {
            self.emit("save", &[LVal::Player(name.clone())]);
            if let Some(c) = self.addr_of(&name).and_then(|a| self.clients.get(&a)) {
                self.store_player(c);
            }
        }
    }

    /// Envoie au joueur sa vie, sa faim et son expérience.
    fn sync_attributes(&mut self, addr: &SocketAddr) {
        self.send_attributes(addr, false);
    }

    /// Tous les attributs (vitesses comprises) : seulement à l'arrivée et à la réapparition,
    /// sinon le sprint en cours serait annulé.
    fn send_attributes(&mut self, addr: &SocketAddr, full: bool) {
        let packet = match self.clients.get(addr) {
            Some(c) if c.protocol >= 2225 => match &c.data {
                Some(d) if full => bedrock::update_attributes_with(c.id as u64, d.health, d.max_health, d.food, d.xp_level, d.xp_progress),
                Some(d) => bedrock::update_vitals(c.id as u64, d.health, d.max_health, d.food, d.xp_level, d.xp_progress),
                None => return,
            },
            _ => return,
        };
        if !self.refused_extras.contains(&bedrock::ID_UPDATE_ATTRIBUTES) {
            self.send(addr, bedrock::ID_UPDATE_ATTRIBUTES, &packet);
        }
    }

    /// Déplace un joueur (plugin, ou retour à sa dernière position).
    fn teleport(&mut self, addr: &SocketAddr, pos: (f32, f32, f32)) {
        if !(pos.0.is_finite() && pos.1.is_finite() && pos.2.is_finite()) || pos.0.abs() > 3.0e7 || pos.2.abs() > 3.0e7 {
            return;
        }
        let pos = (pos.0, pos.1.clamp(-64.0, 400.0), pos.2);
        let packet = match self.clients.get_mut(addr) {
            Some(c) => {
                c.pos = pos;
                c.move_budget = SPEED_CREATIVE * 2.0;
                // Les autres joueurs le voient à sa nouvelle place, même s'il ne bouge plus
                c.moved_tick = self.tick + 1;
                bedrock::teleport_player(c.protocol, c.id, pos, c.pitch, c.yaw, c.head_yaw)
            }
            None => return,
        };
        self.send(addr, bedrock::ID_MOVE_PLAYER, &packet);
    }

    /// Chunks d'un disque de rayon donné, du centre vers l'extérieur.
    fn chunks_around(center: (i32, i32), radius: i32) -> Vec<(i32, i32)> {
        let mut coords = Vec::new();
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx * dx + dz * dz <= radius * radius {
                    coords.push((dx * dx + dz * dz, center.0 + dx, center.1 + dz));
                }
            }
        }
        coords.sort();
        coords.into_iter().map(|(_, x, z)| (x, z)).collect()
    }

    /// Vitesse maximale acceptée par le contrôle des déplacements.
    fn max_speed(&self, creative: bool) -> f32 {
        if creative {
            SPEED_CREATIVE
        } else {
            SPEED_SURVIVAL
        }
    }

    /// Le joueur annonce une nouvelle position. Un déplacement impossible est
    /// refusé et le joueur est replacé ; sinon le terrain manquant est préparé.
    fn on_move(&mut self, addr: &SocketAddr, input: bedrock::Input) {
        // Mort : il attend sur l'écran de mort, ses mouvements sont ignorés
        if self.clients.get(addr).map(|c| c.dead.is_some()).unwrap_or(true) {
            return;
        }
        let footing = self.footing(input.pos);
        let mut fall: Option<Cause> = None;
        let mut fall_amount = 0.0;
        let max_speed = self.max_speed(self.clients.get(addr).map(|c| c.mode.may_fly()).unwrap_or(false));
        let check = self.cfg.movement_check;
        // Le déplacement sera diffusé au prochain tick
        let tick = self.tick + 1;
        let mut correction = None;
        let mut publish = None;
        match self.clients.get_mut(addr) {
            Some(c) => {
                // Tourner la tête sans avancer se voit aussi chez les autres joueurs
                let turn = |a: f32, b: f32| ((a - b + 180.0).rem_euclid(360.0) - 180.0).abs() >= 1.0;
                if turn(input.pitch, c.shown_look.0) || turn(input.yaw, c.shown_look.1) || turn(input.head_yaw, c.shown_look.2) {
                    c.shown_look = (input.pitch, input.yaw, input.head_yaw);
                    c.moved_tick = tick;
                }
                c.pitch = input.pitch;
                c.yaw = input.yaw;
                c.head_yaw = input.head_yaw;
                let pos = input.pos;
                let (dx, dy, dz) = (pos.0 - c.pos.0, pos.1 - c.pos.1, pos.2 - c.pos.2);
                let horizontal = (dx * dx + dz * dz).sqrt();
                if check && c.chunks_sent {
                    // La distance autorisée se recharge à la vitesse maximale, jusqu'à 2 secondes d'avance
                    let now = Instant::now();
                    let cap = max_speed * 2.0;
                    c.move_budget = (c.move_budget + max_speed * now.duration_since(c.move_budget_at).as_secs_f32()).min(cap);
                    c.move_budget_at = now;
                    if horizontal > MAX_STEP || dy.abs() > MAX_STEP_VERTICAL || horizontal > c.move_budget {
                        c.corrections += 1;
                        if c.corrections == 1 || c.corrections % 200 == 0 {
                            warn!(
                                "{} : déplacement anormal ({:.1} blocs d'un coup, {} fois) — joueur replacé",
                                c.name.as_deref().unwrap_or("?"),
                                horizontal.max(dy.abs()),
                                c.corrections,
                            );
                        }
                        correction = Some(bedrock::teleport_player(c.protocol, c.id, c.pos, c.pitch, c.yaw, c.head_yaw));
                    } else {
                        c.move_budget -= horizontal;
                    }
                }
                if correction.is_none() {
                    if horizontal > 0.0 || dy != 0.0 {
                        c.moved_tick = tick;
                    }
                    // Chute : hauteur la plus haute en l'air, dégâts à l'arrivée au sol
                    let feet = pos.1 - player_eye_height();
                    if feet < -128.0 {
                        fall = Some(Cause::Void);
                    } else if !c.mode.takes_damage() || footing == Footing::Liquid {
                        c.fall_peak = None;
                    } else if footing == Footing::Air {
                        c.fall_peak = Some(c.fall_peak.map(|p| p.max(feet)).unwrap_or(feet));
                    } else if let Some(peak) = c.fall_peak.take() {
                        let damage = fall_damage(peak - feet);
                        if damage > 0.0 {
                            fall = Some(Cause::Fall);
                            fall_amount = damage;
                        }
                    }
                    c.pos = pos;
                    let center = ((pos.0.floor() as i32) >> 4, (pos.2.floor() as i32) >> 4);
                    if c.chunks_sent && center != c.center {
                        c.center = center;
                        // Le jeu oublie les chunks qui sortent de sa zone : le serveur aussi, pour
                        // les renvoyer si le joueur revient. (La zone annoncée au jeu dépasse d'un
                        // chunk celle-ci : il ne jette jamais un chunk que le serveur croit chargé.)
                        let r = c.radius;
                        c.loaded.retain(|(x, z)| (x - center.0) * (x - center.0) + (z - center.1) * (z - center.1) <= r * r);
                        let wanted = Self::chunks_around(center, c.radius);
                        c.queue = wanted.into_iter().filter(|xz| !c.loaded.contains(xz)).collect();
                        publish = Some((pos, c.radius));
                    }
                }
            }
            None => return,
        }
        if let Some(packet) = correction {
            self.send(addr, bedrock::ID_MOVE_PLAYER, &packet);
        }
        if let Some(cause) = fall {
            self.hurt(addr, fall_amount, cause);
        }
        if let Some((pos, radius)) = publish {
            self.send(
                addr,
                bedrock::ID_CHUNK_PUBLISHER_UPDATE,
                &bedrock::chunk_publisher_update(pos.0.floor() as i32, pos.1.floor() as i32, pos.2.floor() as i32, (radius as u32 + 1) * 16),
            );
        }
    }

    /// Ordre d'un plugin à ses PNJ. Un PNJ appartient au plugin qui l'a créé : les autres
    /// ne peuvent ni le déplacer ni le retirer.
    fn npc_order(&mut self, owner: String, order: NpcOrder) {
        const MAX_NPCS: usize = 512;
        let mine = |n: &Npc| n.owner == owner;
        match order {
            NpcOrder::Spawn(name, x, y, z) => {
                let key = name.to_lowercase();
                if let Some(n) = self.npcs.get(&key) {
                    if !mine(n) {
                        warn!("[{}] le PNJ « {} » appartient déjà au plugin {}", owner, name, n.owner);
                        return;
                    }
                }
                if !self.npcs.contains_key(&key) && self.npcs.len() >= MAX_NPCS {
                    warn!("[{}] trop de PNJ ({} au plus)", owner, MAX_NPCS);
                    return;
                }
                // Un PNJ replacé change d'identifiant : les joueurs le voient réapparaître
                let id = self.next_id;
                self.next_id += 1;
                self.npcs.insert(key, Npc { id, owner, name, pos: (x, y, z), velocity: (0.0, 0.0), fall: 0.0, yaw: 0.0, moved_tick: self.tick });
            }
            NpcOrder::Walk(name, vx, vz) => {
                if let Some(n) = self.npcs.get_mut(&name.to_lowercase()).filter(|n| mine(n)) {
                    n.velocity = (vx / 20.0, vz / 20.0);
                    if vx != 0.0 || vz != 0.0 {
                        n.yaw = (-vx).atan2(vz).to_degrees();
                    }
                    n.moved_tick = self.tick;
                }
            }
            NpcOrder::Teleport(name, x, y, z) => {
                if let Some(n) = self.npcs.get_mut(&name.to_lowercase()).filter(|n| mine(n)) {
                    n.pos = (x, y, z);
                    n.fall = 0.0;
                    n.moved_tick = self.tick;
                }
            }
            NpcOrder::Remove(name) => {
                let key = name.to_lowercase();
                if self.npcs.get(&key).map(|n| mine(n)).unwrap_or(false) {
                    self.npcs.remove(&key);
                }
            }
            NpcOrder::RemoveAll => self.npcs.retain(|_, n| n.owner != owner),
        }
    }

    /// Peut-on traverser ce bloc à pied (air, eau, herbes, fleurs…) ?
    fn npc_passable(&self, state: u32) -> bool {
        self.replaceable(state) || self.data.as_ref().and_then(|d| d.block_name(state)).map(washed_away).unwrap_or(false)
    }

    /// Fait avancer les PNJ qui marchent : ils montent les marches d'un bloc, tombent dans
    /// les trous et s'arrêtent devant un mur.
    fn move_npcs(&mut self) {
        let keys: Vec<String> = self.npcs.keys().cloned().collect();
        for key in keys {
            let n = match self.npcs.get(&key) {
                Some(n) => n.clone(),
                None => continue,
            };
            let (mut x, mut y, mut z) = n.pos;
            let mut fall = n.fall;
            // Jamais de génération de terrain dans le tick : le chunk où il se trouve et celui
            // où il va sont demandés aux fils de génération, et le PNJ attend qu'ils soient prêts
            let ahead = (x + n.velocity.0 * 60.0, z + n.velocity.1 * 60.0);
            let here = ((x.floor() as i32) >> 4, (z.floor() as i32) >> 4);
            let next = (((x + n.velocity.0).floor() as i32) >> 4, ((z + n.velocity.1).floor() as i32) >> 4);
            let far = ((ahead.0.floor() as i32) >> 4, (ahead.1.floor() as i32) >> 4);
            for (cx, cz) in [here, next, far] {
                self.world.prepare(cx, cz);
            }
            if !self.world.is_ready(here.0, here.1) || !self.world.is_ready(next.0, next.1) {
                continue;
            }
            let free = |s: &Self, x: f32, y: f32, z: f32| {
                let (bx, by, bz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
                s.npc_passable(s.world.get(bx, by, bz)) && s.npc_passable(s.world.get(bx, by + 1, bz))
            };
            let mut moved = false;
            if n.velocity != (0.0, 0.0) {
                let (nx, nz) = (x + n.velocity.0, z + n.velocity.1);
                if free(self, nx, y, nz) {
                    x = nx;
                    z = nz;
                    moved = true;
                } else if fall == 0.0 && free(self, nx, y.floor() + 1.0, nz) {
                    // Marche d'un bloc
                    x = nx;
                    z = nz;
                    y = y.floor() + 1.0;
                    moved = true;
                }
            }
            // Chute : le sol est-il sous les pieds ?
            let below = self.world.get(x.floor() as i32, (y - 0.01).floor() as i32, z.floor() as i32);
            if self.npc_passable(below) && y > -64.0 {
                fall = (fall + 0.08).min(3.0);
                let mut ny = y - fall;
                // Atterrit sur le premier bloc solide rencontré
                let mut by = (y - 0.01).floor() as i32;
                while by as f32 >= ny.floor() {
                    if !self.npc_passable(self.world.get(x.floor() as i32, by, z.floor() as i32)) {
                        ny = (by + 1) as f32;
                        fall = 0.0;
                        break;
                    }
                    by -= 1;
                }
                y = ny.max(-64.0);
                moved = true;
            } else {
                fall = 0.0;
                // Coincé dans un bloc (posé sur lui, terrain qui change) : remonte
                if !self.npc_passable(self.world.get(x.floor() as i32, y.floor() as i32, z.floor() as i32)) {
                    y = y.floor() + 1.0;
                    moved = true;
                }
            }
            if let Some(m) = self.npcs.get_mut(&key) {
                m.pos = (x, y, z);
                m.fall = fall;
                if moved {
                    m.moved_tick = self.tick;
                }
            }
        }
    }

    /// Montre à chaque joueur les joueurs proches de lui, et seulement ceux-là :
    /// c'est ce qui permet de tenir avec beaucoup de monde.
    fn broadcast_players(&mut self) {
        if !self.cfg.show_players {
            return;
        }
        struct Seen {
            id: i64,
            chunk: (i32, i32),
            pos: (f32, f32, f32),
            pitch: f32,
            yaw: f32,
            head_yaw: f32,
            moved_tick: u64,
            name: String,
            sneaking: bool,
            sprinting: bool,
            held: (i16, u8, u32),
            skin: Option<Arc<crate::player::Skin>>,
            mode: i32,
            npc: bool,
        }
        let refresh = self.tick % VISIBILITY_EVERY == 0;
        let mut players: Vec<Seen> = Vec::new();
        // Joueurs connectés, même morts : seuls ceux qui ont quitté le serveur sortent de la liste
        let online: HashSet<i64> = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame).map(|c| c.id).collect();
        let now_tick = self.tick;
        for c in self.clients.values() {
            // Un mort reste visible le temps de sa chute (1 seconde), puis disparaît
            let shown = c.dead.map(|t| now_tick.saturating_sub(t) <= 20).unwrap_or(true) && !c.mode.is_spectator();
            if c.joined && c.stage == Stage::InGame && shown {
                players.push(Seen {
                    id: c.id,
                    chunk: ((c.pos.0.floor() as i32) >> 4, (c.pos.2.floor() as i32) >> 4),
                    pos: c.pos,
                    pitch: c.pitch,
                    yaw: c.yaw,
                    head_yaw: c.head_yaw,
                    moved_tick: c.moved_tick,
                    name: if refresh { c.name.clone().unwrap_or_default() } else { String::new() },
                    sneaking: c.sneaking,
                    sprinting: c.sprinting,
                    skin: if refresh { c.skin.clone() } else { None },
                    mode: c.mode.bedrock_id(),
                    npc: false,
                    held: {
                        let h = c.inv.held();
                        (h.id, h.count.min(1), if h.is_empty() { 0 } else { self.block_for(h.id).unwrap_or(0) })
                    },
                });
            }
        }
        let mut online = online;
        for n in self.npcs.values() {
            online.insert(n.id);
            players.push(Seen {
                id: n.id,
                chunk: ((n.pos.0.floor() as i32) >> 4, (n.pos.2.floor() as i32) >> 4),
                pos: (n.pos.0, n.pos.1 + EYE, n.pos.2),
                pitch: 0.0,
                yaw: n.yaw,
                head_yaw: n.yaw,
                moved_tick: n.moved_tick,
                name: if refresh { n.name.clone() } else { String::new() },
                sneaking: false,
                sprinting: false,
                // Pas d'entrée dans la liste des joueurs : le jeu refuse un skin fabriqué
                // (et couperait alors la liste de tout le monde) ; il les montre avec
                // l'apparence par défaut
                skin: None,
                mode: 0,
                npc: true,
                held: (0, 0, 0),
            });
        }
        if players.len() < 2 && !refresh {
            return;
        }
        let tick = self.tick;
        let by_id: HashMap<i64, usize> = players.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        if refresh {
            for (i, p) in players.iter().enumerate() {
                grid.entry(p.chunk).or_insert_with(Vec::new).push(i);
            }
        }
        let range = self.cfg.player_view;
        let cap = self.cfg.max_visible;
        let hidden = self.add_player_refused;
        let still = self.move_actor_refused;
        let no_equipment = self.equip_refused;
        let no_list = self.player_list_refused;
        let mut out: Vec<(SocketAddr, Vec<u8>)> = Vec::new();
        for (addr, c) in self.clients.iter_mut() {
            if !(c.joined && c.stage == Stage::InGame) {
                continue;
            }
            let mut packets: Vec<(u32, Vec<u8>)> = Vec::new();
            let me = ((c.pos.0.floor() as i32) >> 4, (c.pos.2.floor() as i32) >> 4);
            if refresh {
                // Retire ceux qui sont partis ou trop loin (avec une marge pour éviter le clignotement)
                let gone: Vec<i64> = c
                    .visible
                    .iter()
                    .copied()
                    .filter(|id| match by_id.get(id) {
                        Some(&i) => {
                            let o = players[i].chunk;
                            (o.0 - me.0).abs() > range + 1 || (o.1 - me.1).abs() > range + 1
                        }
                        None => true,
                    })
                    .collect();
                for id in gone {
                    c.visible.remove(&id);
                    packets.push((bedrock::ID_REMOVE_ACTOR, bedrock::remove_actor(id)));
                }
                // Ceux qui ont quitté le serveur sortent aussi de la liste des joueurs
                let departed: Vec<i64> = c.listed.keys().copied().filter(|id| !online.contains(id)).collect();
                if !departed.is_empty() {
                    let uuids: Vec<[u8; 16]> = departed.iter().filter_map(|id| c.listed.remove(id)).collect();
                    packets.push((bedrock::ID_PLAYER_LIST, bedrock::player_list_remove(&uuids)));
                }
                // Ajoute les joueurs proches, dans la limite du plafond
                let mut appeared = 0;
                'scan: for dx in -range..=range {
                    for dz in -range..=range {
                        if let Some(cell) = grid.get(&(me.0 + dx, me.1 + dz)) {
                            for &i in cell {
                                // Un skin pèse jusqu'à plusieurs centaines de Ko : dans une foule, les
                                // nouveaux joueurs apparaissent par petits groupes, pas tous d'un coup
                                if c.visible.len() >= cap || appeared >= 3 {
                                    break 'scan;
                                }
                                let o = &players[i];
                                if o.id != c.id && !hidden && c.visible.insert(o.id) {
                                    appeared += 1;
                                    let key = if o.npc { format!("npc:{}", o.name.to_lowercase()) } else { o.name.to_lowercase() };
                                    let uuid = crate::crypto::sha256(key.as_bytes());
                                    let mut id16 = [0u8; 16];
                                    id16.copy_from_slice(&uuid[..16]);
                                    // Son apparence d'abord : le jeu l'associe au joueur qui
                                    // apparaît ensuite sous le même identifiant
                                    if let (Some(skin), false) = (&o.skin, no_list || c.protocol != 2225) {
                                        if c.listed.insert(o.id, id16).is_none() {
                                            packets.push((bedrock::ID_PLAYER_LIST, bedrock::player_list_add(&[(&id16, o.id, &o.name, skin, list_colour(o.id))])));
                                        }
                                    }
                                    packets.push((
                                        bedrock::ID_ADD_PLAYER,
                                        bedrock::add_player(&id16, &o.name, o.id, (o.pos.0, o.pos.1 - 1.62, o.pos.2), o.pitch, o.yaw, o.head_yaw, o.mode, Self::remote_flags(o.sneaking, o.sprinting), (0, 0, 0)),
                                    ));
                                    // L'objet qu'il tient suit dans un paquet à part : s'il était
                                    // refusé par le jeu, l'apparition du joueur ne le serait pas
                                    if o.held.1 > 0 && !no_equipment {
                                        packets.push((bedrock::ID_MOB_EQUIPMENT, bedrock::mob_equipment(o.id, o.held, 0)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Dans une foule, les positions des autres sont rafraîchies moins souvent :
            // 20 fois par seconde jusqu'à 16 joueurs visibles, 10 fois jusqu'à 32, puis environ 7 fois.
            let every = match c.visible.len() {
                0..=16 => 1,
                17..=32 => 2,
                _ => 3,
            };
            if tick >= c.synced_tick + every {
                for id in c.visible.iter() {
                    if let Some(&i) = by_id.get(id) {
                        let o = &players[i];
                        if o.moved_tick > c.synced_tick {
                            if move_player_known(c.protocol) {
                                packets.push((
                                    bedrock::ID_MOVE_PLAYER,
                                    bedrock::move_player(o.id, o.pos, o.pitch, o.yaw, o.head_yaw, false),
                                ));
                            } else if !still {
                                // Jeux récents : le paquet de déplacement des joueurs a changé de
                                // format ; celui des entités, plus simple, déplace aussi les joueurs
                                packets.push((
                                    bedrock::ID_MOVE_ACTOR_ABSOLUTE,
                                    bedrock::move_actor_absolute(o.id, o.pos, o.pitch, o.yaw, o.head_yaw, true),
                                ));
                            }
                        }
                    }
                }
                c.synced_tick = tick;
            }
            if !packets.is_empty() {
                out.push((*addr, bedrock::encode_batch(&packets, c.compression)));
            }
        }
        for (addr, batch) in out {
            self.net_send(&addr, &batch);
        }
    }

    /// Envoie quelques chunks en attente à chaque joueur, pour étaler la charge.
    fn send_queued_chunks(&mut self) {
        // 1. Chunks encodés par les fils : rangés par joueur, dans l'ordre de la file
        while let Ok((addr, owner, seq, chunk)) = self.chunk_pool.done.try_recv() {
            if let Some(c) = self.clients.get_mut(&addr).filter(|c| c.id == owner) {
                c.chunk_ready.insert(seq, chunk);
            }
        }
        // 2. Nouveaux chunks à encoder (quelques-uns par tick et par joueur, sans trop d'avance)
        let exp = self.experiment;
        let mut jobs = Vec::new();
        for (addr, c) in self.clients.iter_mut() {
            let features = crate::versions::features(c.protocol);
            for _ in 0..CHUNKS_PER_TICK {
                if c.chunk_next - c.chunk_sent >= MAX_CHUNKS_IN_FLIGHT {
                    break;
                }
                let (x, z) = match c.queue.pop_front() {
                    Some(xz) => xz,
                    None => break,
                };
                c.loaded.insert((x, z));
                jobs.push(ChunkJob {
                    addr: *addr,
                    owner: c.id,
                    seq: c.chunk_next,
                    x,
                    z,
                    version: self.world.chunk_version(x, z),
                    view: self.world.view(),
                    exp,
                    features,
                    compression: c.compression,
                });
                c.chunk_next += 1;
            }
        }
        for job in jobs {
            let _ = self.chunk_pool.jobs.send(job);
        }
        // 3. Envoi, dans l'ordre, de ceux qui sont prêts. Un chunk modifié pendant son
        // encodage est réencodé ici, pour ne jamais effacer un bloc déjà envoyé à part.
        let mut out: Vec<(SocketAddr, Vec<u8>)> = Vec::new();
        let mut stale: Vec<(SocketAddr, i32, i32, crate::versions::Features, bool)> = Vec::new();
        let mut spawn: Vec<SocketAddr> = Vec::new();
        for (addr, c) in self.clients.iter_mut() {
            while let Some(chunk) = c.chunk_ready.remove(&c.chunk_sent) {
                c.chunk_sent += 1;
                if chunk.version != self.world.chunk_version(chunk.x, chunk.z) {
                    stale.push((*addr, chunk.x, chunk.z, crate::versions::features(c.protocol), c.compression));
                } else {
                    out.push((*addr, chunk.packet));
                }
            }
            if c.spawn_pending && c.queue.is_empty() && c.chunk_sent == c.chunk_next {
                c.spawn_pending = false;
                spawn.push(*addr);
            }
        }
        for (addr, x, z, features, compression) in stale {
            let pk = build_chunk(self.experiment, x, z, self.world.chunk_payload(x, z), features);
            out.push((addr, bedrock::encode_packet(bedrock::ID_LEVEL_CHUNK, &pk, compression)));
        }
        for (addr, packet) in out {
            self.net_send(&addr, &packet);
        }
        for addr in spawn {
            self.send(&addr, bedrock::ID_PLAY_STATUS, &bedrock::play_status(bedrock::STATUS_PLAYER_SPAWN));
        }
    }

    /// Réponse au ping de la liste des serveurs (nom, joueurs, version).
    fn update_pong(&mut self) {
        let clean = |s: &str| s.replace(';', ",").replace('\n', " ");
        let port = self.local_addr().map(|a| a.port()).unwrap_or(self.cfg.port);
        self.rak.pong = format!(
            "MCPE;{};{};{};{};{};{};{};{};1;{};{};",
            clean(&self.motd),
            self.cfg.protocol,
            clean(&self.cfg.game_version),
            self.players().len(),
            self.cfg.max_players,
            self.rak.guid,
            clean(&self.cfg.sub_motd),
            clean(&self.cfg.gamemode),
            port,
            if self.rak.has_v6() { self.cfg.port6 } else { port }
        );
    }

    /// Charge les plugins avec les constantes du serveur (version, protocole…).
    fn load_plugins(&mut self) {
        let info = self.info();
        let constants = [
            ("version", LVal::Str(crate::VERSION.to_string())),
            ("game_version", LVal::Str(self.cfg.game_version.clone())),
            ("protocol", LVal::Int(self.cfg.protocol as i64)),
            ("max_players", LVal::Int(self.cfg.max_players as i64)),
        ];
        self.plugins.load_all(&info, &constants);
        self.apply_actions();
    }

    /// Applique ce que les plugins ont demandé pendant leurs appels.
    fn apply_actions(&mut self) {
        for a in self.plugins.take_actions() {
            match a {
                Action::SetMotd(m) => {
                    self.motd = crate::log::clean(&m, 64);
                    self.update_pong();
                }
                Action::Kick(name, reason) => {
                    let addr = self
                        .clients
                        .iter()
                        .find(|(_, c)| c.name.as_deref() == Some(name.as_str()))
                        .map(|(a, _)| *a);
                    if let Some(a) = addr {
                        info!("{} est expulsé : {}", name, crate::log::clean(&reason, 200));
                        self.kick(&a, &reason);
                    }
                }
                Action::Stop => self.running = false,
                Action::PlayerSet(name, key, value) => {
                    let addr = match self.addr_of(&name) {
                        Some(a) => a,
                        None => continue,
                    };
                    if let ("op", LVal::Bool(b)) = (key.as_str(), &value) {
                        self.set_op(&name, *b);
                        continue;
                    }
                    if let ("gamemode", LVal::Num(v)) = (key.as_str(), &value) {
                        self.set_gamemode(&addr, GameMode::from_save_id(*v as i32));
                        continue;
                    }
                    let mut attributes = false;
                    if let Some(d) = self.clients.get_mut(&addr).and_then(|c| c.data.as_mut()) {
                        match (key.as_str(), &value) {
                            (k, v) if k.starts_with("perm:") => {
                                let node = k[5..].to_string();
                                let ok = node.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
                                if ok && !node.is_empty() && node.len() <= 64 {
                                    d.permissions.retain(|(n, _)| *n != node);
                                    if let (LVal::Bool(b), true) = (v, d.permissions.len() < 256) {
                                        d.permissions.push((node, *b));
                                    }
                                }
                                continue;
                            }
                            ("display", LVal::Str(s)) => d.display_name = clean_text(s, 32),
                            ("tag", LVal::Str(s)) => d.name_tag = clean_text(s, 64),
                            ("health", LVal::Num(v)) => d.health = *v as f32,
                            ("max_health", LVal::Num(v)) => d.max_health = *v as f32,
                            ("food", LVal::Num(v)) => d.food = *v as f32,
                            ("xp_level", LVal::Num(v)) => d.xp_level = v.clamp(0.0, 24_791.0) as i32,
                            ("xp_progress", LVal::Num(v)) => d.xp_progress = *v as f32,
                            _ => continue,
                        }
                        d.sanitize();
                        attributes = key != "display" && key != "tag";
                    }
                    if attributes {
                        self.sync_attributes(&addr);
                        // Vie mise à zéro par un plugin : le joueur meurt
                        let dead = self.clients.get(&addr).and_then(|c| c.data.as_ref()).map(|d| d.is_dead()).unwrap_or(false);
                        if dead {
                            self.die(&addr, Cause::Plugin);
                        }
                    }
                }
                Action::PlayerMessage(name, text) => {
                    if let Some(addr) = self.addr_of(&name) {
                        let text: String = text.chars().filter(|c| !c.is_control() || *c == '\n').take(512).collect();
                        self.send(&addr, bedrock::ID_TEXT, &bedrock::text_raw(&text));
                    }
                }
                Action::PlayerTeleport(name, x, y, z) => {
                    if let Some(addr) = self.addr_of(&name) {
                        self.teleport(&addr, (x, y, z));
                    }
                }
                Action::SetBlocks(list) => self.set_blocks_from_plugin(list),
                Action::Npc(owner, order) => self.npc_order(owner, order),
                Action::PlayerGive(name, item, count) => {
                    if let (Some(addr), Some(id)) = (self.addr_of(&name), self.item_id(&item)) {
                        self.add_item(&addr, id, count);
                    }
                }
                Action::PlayerSave(name) => {
                    if let Some(c) = self.addr_of(&name).and_then(|a| self.clients.get(&a)) {
                        self.store_player(c);
                    }
                }
            }
        }
    }

    /// Après une édition en volume : les joueurs reçoivent les blocs changés, un par un s'ils
    /// sont peu nombreux, sinon par chunks entiers.
    /// Blocs posés par un plugin (`world:setBlock`, `world:setBlocks`). Les états inconnus du
    /// jeu sont ignorés.
    fn set_blocks_from_plugin(&mut self, list: Vec<(i32, i32, i32, u32)>) {
        let mut outcome = Outcome::default();
        let mut linked: Vec<(i32, i32, i32)> = Vec::new();
        let mut chunks: HashSet<(i32, i32)> = HashSet::new();
        for (x, y, z, state) in list {
            let known = self.data.as_ref().map(|d| d.block_name(state).is_some()).unwrap_or(false);
            if !known || self.world.get(x, y, z) == state || !self.world.set(x, y, z, state) {
                continue;
            }
            outcome.changed += 1;
            if outcome.changed <= 4096 {
                linked.push((x, y, z));
            }
            chunks.insert((x >> 4, z >> 4));
            if outcome.blocks.len() <= 512 {
                outcome.blocks.push(((x, y, z), state));
            }
        }
        if outcome.changed == 0 {
            return;
        }
        if outcome.changed > 512 {
            outcome.blocks.clear();
        }
        outcome.chunks = chunks.into_iter().collect();
        self.refresh_after_edit(&outcome);
        for p in linked {
            self.settle_links(p);
        }
    }

    /// Après une édition de plugin : envoie les blocs changés, ou les chunks entiers s'ils
    /// sont nombreux.
    fn refresh_after_edit(&mut self, outcome: &Outcome) {
        if !outcome.blocks.is_empty() {
            for ((x, y, z), block) in &outcome.blocks {
                self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(*x, *y, *z, *block), None);
            }
        } else {
            for c in self.clients.values_mut() {
                for chunk in &outcome.chunks {
                    if c.loaded.contains(chunk) && !c.queue.contains(chunk) {
                        c.queue.push_back(*chunk);
                    }
                }
            }
        }
        // Les coffres et fours dont le bloc a disparu perdent leurs données
        if !self.world.tiles.is_empty() {
            let world = &self.world;
            let (data, blocks) = (&self.data, &self.blocks);
            let still_there = |pos: (i32, i32, i32)| {
                let state = world.get(pos.0, pos.1, pos.2);
                match (data, blocks) {
                    (Some(d), Some(b)) => d.block_name(state).and_then(|n| b.get(n)).map(|b| b.tile().is_some()).unwrap_or(false),
                    _ => true,
                }
            };
            let doomed: Vec<(i32, i32, i32)> = self.world.tiles.positions().into_iter().filter(|p| !still_there(*p)).collect();
            for pos in doomed {
                self.world.tiles.remove(pos);
            }
        }
    }

    /// Commande tapée en jeu par un joueur (« /set stone », « //set stone »).
    fn run_player_command(&mut self, addr: &SocketAddr, line: &str) {
        let name = match self.clients.get(addr).and_then(|c| c.name.clone()) {
            Some(n) => n,
            None => return,
        };
        let mut parts = line.trim().trim_start_matches('/').split_whitespace();
        let command = match parts.next() {
            Some(c) => c.to_lowercase(),
            None => return,
        };
        if let Some(def) = VanillaCommands::get(&command) {
            if def.op && !self.ops.is_op(&name) {
                self.send(addr, bedrock::ID_TEXT, &bedrock::text_raw("§cVous n'avez pas la permission d'utiliser cette commande."));
                return;
            }
            if self.cfg.debug {
                info!("{} lance la commande /{}", name, crate::log::clean(line.trim().trim_start_matches('/'), 200));
            }
            let words = crate::command::tokens(line.trim().trim_start_matches('/'));
            self.run_vanilla(Some(*addr), &command, &words[1.min(words.len())..]);
            return;
        }
        let args: Vec<&str> = parts.take(32).collect();
        if !self.plugins.commands().iter().any(|c| c.0 == command) {
            self.send(addr, bedrock::ID_TEXT, &bedrock::text_raw(&format!("§cCommande inconnue : /{}", crate::log::clean(&command, 32))));
            return;
        }
        // La permission exigée par le plugin.yml du plugin, si la commande en a une
        if let Some(node) = self.plugins.command_permission(&command) {
            let own = self.clients.get(addr).and_then(|c| c.data.as_ref()).map(|d| d.permissions.clone()).unwrap_or_default();
            if !permission::check(self.ops.is_op(&name), &own, &self.plugins.permissions(), &node) {
                self.send(addr, bedrock::ID_TEXT, &bedrock::text_raw("§cVous n'avez pas la permission d'utiliser cette commande."));
                return;
            }
        }
        if self.cfg.debug {
            info!("{} lance la commande /{} {}", name, command, crate::log::clean(&args.join(" "), 200));
        }
        let info = self.info();
        self.plugins.set_info(&info, self.tick);
        self.plugins.run_command_as(&command, &args, Some(&name));
        self.apply_actions();
    }

    /// Heure du monde, en ticks de jeu (20 par seconde).
    fn world_time(&self) -> i64 {
        self.time_base + (self.started.elapsed().as_millis() / 50) as i64
    }

    /// Réponse à l'auteur d'une commande : dans son chat, ou dans la console.
    fn reply(&mut self, sender: Option<SocketAddr>, message: &str) {
        match sender {
            Some(addr) => self.send(&addr, bedrock::ID_TEXT, &bedrock::text_raw(message)),
            None => {
                // Sans les codes de couleur du jeu
                let mut plain = String::new();
                let mut skip = false;
                for c in message.chars() {
                    if skip {
                        skip = false;
                    } else if c == '§' {
                        skip = true;
                    } else {
                        plain.push(c);
                    }
                }
                info!("{}", plain);
            }
        }
    }

    /// Joueurs désignés par un argument : un pseudo, `@s` (soi-même), `@p` (le plus proche,
    /// donc soi-même) ou `@a` (tout le monde).
    fn targets(&self, sender: Option<SocketAddr>, word: &str) -> Vec<SocketAddr> {
        let in_game = |c: &Client| c.joined && c.stage == Stage::InGame;
        match word {
            "@a" | "@e" => self.clients.iter().filter(|(_, c)| in_game(c)).map(|(a, _)| *a).collect(),
            "@s" | "@p" | "@r" => sender.into_iter().collect(),
            name => self.clients.iter().filter(|(_, c)| in_game(c) && c.name.as_deref().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false)).map(|(a, _)| *a).collect(),
        }
    }

    /// Pseudo du joueur à cette adresse (vide avant le login).
    fn name_at(&self, addr: &SocketAddr) -> String {
        self.clients.get(addr).and_then(|c| c.name.clone()).unwrap_or_default()
    }

    /// Change le mode de jeu d'un joueur et l'en informe. Faux si un plugin s'y oppose.
    fn set_gamemode(&mut self, addr: &SocketAddr, mode: GameMode) -> bool {
        let out = self.fire("PlayerGameModeChangeEvent", vec![("player", self.name_json(addr)), ("newGamemode", Json::Num(mode.save_id() as f64))]);
        if cancelled(&out) {
            return false;
        }
        let id = match self.clients.get_mut(addr) {
            Some(c) => {
                c.mode = mode;
                c.fall_peak = None;
                c.id
            }
            None => return false,
        };
        self.send(addr, bedrock::ID_SET_PLAYER_GAME_TYPE, &bedrock::set_time(mode.bedrock_id()));
        self.send(addr, bedrock::ID_UPDATE_ABILITIES, &bedrock::update_abilities(id, mode));
        true
    }

    /// Exécute une commande du jeu pour un joueur (`sender`) ou pour la console (`None`).
    /// Les droits ont déjà été vérifiés.
    fn run_vanilla(&mut self, sender: Option<SocketAddr>, command: &str, args: &[String]) {
        let _t = timings::scope_with(|| format!("Commande /{}", command));
        const NO_TARGET: &str = "§cAucune cible ne correspond au sélecteur";
        let who = match sender {
            Some(a) => self.name_at(&a),
            None => "Serveur".to_string(),
        };
        let here = sender.and_then(|a| self.clients.get(&a)).map(|c| (c.pos.0, c.pos.1 - 1.62, c.pos.2)).unwrap_or((0.0, 0.0, 0.0));
        let usage = |s: &mut Self| {
            let text = VanillaCommands::get(command).map(|c| c.usage).unwrap_or("");
            s.reply(sender, &format!("§cErreur de syntaxe. Usage : {}", text));
        };
        let arg = |i: usize| args.get(i).map(|s| s.as_str()).unwrap_or("");
        match command {
            "help" => {
                let op = sender.map(|a| self.ops.is_op(&self.name_at(&a))).unwrap_or(true);
                let mut names: Vec<String> = VanillaCommands::all().iter().filter(|c| op || !c.op).map(|c| c.usage.split(" | ").next().unwrap_or(c.usage).to_string()).collect();
                names.extend(self.plugins.commands().into_iter().map(|c| format!("/{}", c.0)));
                self.reply(sender, &format!("§2--- Aide : {} commandes ---", names.len()));
                for line in names.chunks(4) {
                    self.reply(sender, &line.join("§7  ·  §f"));
                }
            }
            "list" => {
                let p = self.players();
                self.reply(sender, &format!("Il y a {}/{} joueurs en ligne :", p.len(), self.cfg.max_players));
                self.reply(sender, &p.join(", "));
            }
            "me" => {
                let text = crate::log::clean(&args.join(" "), 256);
                self.broadcast(bedrock::ID_TEXT, &bedrock::text_raw(&format!("* {} {}", who, text)), None);
            }
            "say" => {
                let text = crate::log::clean(&args.join(" "), 256);
                info!("[{}] {}", who, text);
                self.broadcast(bedrock::ID_TEXT, &bedrock::text_raw(&format!("[{}] {}", who, text)), None);
            }
            "tell" | "msg" | "w" => {
                let targets = self.targets(sender, arg(0));
                let text = crate::log::clean(&args.get(1..).unwrap_or(&[]).join(" "), 256);
                if args.len() < 2 {
                    usage(self);
                } else if targets.is_empty() {
                    self.reply(sender, NO_TARGET);
                } else {
                    for t in targets {
                        let to = self.name_at(&t);
                        self.send(&t, bedrock::ID_TEXT, &bedrock::text_raw(&format!("§7§o{} vous chuchote : {}", who, text)));
                        self.reply(sender, &format!("§7§oVous chuchotez à {} : {}", to, text));
                    }
                }
            }
            "gamemode" => {
                let mode = match arg(0).to_lowercase().as_str() {
                    // « default » : le mode du serveur (server.properties)
                    "default" | "d" | "5" => GameMode::parse(&self.cfg.gamemode).unwrap_or(GameMode::Survival),
                    other => match GameMode::parse(other) {
                        Some(m) => m,
                        None => return usage(self),
                    },
                };
                let targets = if args.len() > 1 { self.targets(sender, arg(1)) } else { sender.into_iter().collect() };
                if targets.is_empty() {
                    return self.reply(sender, NO_TARGET);
                }
                let label = mode.label();
                for t in targets {
                    if !self.set_gamemode(&t, mode) {
                        continue;
                    }
                    let mode = label;
                    if Some(t) == sender {
                        self.reply(sender, &format!("Votre mode de jeu a été mis à jour : {}", mode));
                    } else {
                        let name = self.name_at(&t);
                        self.send(&t, bedrock::ID_TEXT, &bedrock::text_raw(&format!("Votre mode de jeu a été mis à jour : {}", mode)));
                        self.reply(sender, &format!("Mode de jeu de {} défini sur {}", name, mode));
                    }
                }
            }
            "give" => {
                let targets = self.targets(sender, arg(0));
                let count: u16 = match args.get(2) {
                    Some(c) => match c.parse::<u16>() {
                        Ok(n) if (1..=32767).contains(&n) => n.min(2304),
                        _ => return usage(self),
                    },
                    None => 1,
                };
                let item = arg(1).to_lowercase();
                let id = match self.item_id(&item) {
                    Some(id) if args.len() >= 2 => id,
                    _ if args.len() < 2 => return usage(self),
                    _ => return self.reply(sender, &format!("§cObjet inconnu : {}", crate::log::clean(&item, 64))),
                };
                if targets.is_empty() {
                    return self.reply(sender, NO_TARGET);
                }
                for t in targets {
                    self.add_item(&t, id, count);
                    let name = self.name_at(&t);
                    self.reply(sender, &format!("{} a reçu {} * {}", name, item.trim_start_matches("minecraft:"), count));
                }
            }
            "clear" => {
                let targets = if args.is_empty() { sender.into_iter().collect() } else { self.targets(sender, arg(0)) };
                if targets.is_empty() {
                    return self.reply(sender, NO_TARGET);
                }
                let only = match args.get(1) {
                    Some(item) => match self.item_id(&item.to_lowercase()) {
                        Some(id) => Some(id),
                        None => return self.reply(sender, &format!("§cObjet inconnu : {}", crate::log::clean(item, 64))),
                    },
                    None => None,
                };
                for t in targets {
                    let mut removed = 0u32;
                    if let Some(c) = self.clients.get_mut(&t) {
                        c.inv_touched = true;
                        for slot in c.inv.main.slots.iter_mut().chain(c.inv.armor.slots.iter_mut()).chain(c.inv.offhand.slots.iter_mut()) {
                            if slot.count > 0 && only.map(|id| id == slot.id).unwrap_or(true) {
                                removed += slot.count as u32;
                                *slot = crate::item::ItemStack::default();
                            }
                        }
                    }
                    self.send_inventory(&t);
                    let name = self.name_at(&t);
                    self.reply(sender, &format!("Inventaire de {} vidé : {} objet(s) retiré(s)", name, removed));
                }
            }
            "setblock" | "fill" => {
                let fill = command == "fill";
                let from = match crate::command::position(args, 0, here) {
                    Some(p) => p,
                    None => return usage(self),
                };
                let to = if fill {
                    match crate::command::position(args, 3, here) {
                        Some(p) => p,
                        None => return usage(self),
                    }
                } else {
                    from
                };
                let block = match parse_pattern(self.data.as_deref(), arg(if fill { 6 } else { 3 })) {
                    Ok(b) if b.len() == 1 => b[0],
                    Ok(_) => return usage(self),
                    Err(e) => return self.reply(sender, &format!("§c{}", e)),
                };
                let cell = |p: (f32, f32, f32)| (p.0.floor() as i32, p.1.floor() as i32, p.2.floor() as i32);
                let (a, b) = (cell(from), cell(to));
                let (lo, hi) = ((a.0.min(b.0), a.1.min(b.1), a.2.min(b.2)), (a.0.max(b.0), a.1.max(b.1), a.2.max(b.2)));
                if lo.1 < -64 || hi.1 > 319 {
                    return self.reply(sender, "§cImpossible de placer des blocs en dehors du monde");
                }
                let volume = (hi.0 - lo.0 + 1) as i64 * (hi.1 - lo.1 + 1) as i64 * (hi.2 - lo.2 + 1) as i64;
                if volume > 32768 {
                    return self.reply(sender, &format!("§cTrop de blocs dans la zone indiquée ({} > 32768)", volume));
                }
                let mut changed: Vec<((i32, i32, i32), u32)> = Vec::new();
                let mut chunks: Vec<(i32, i32)> = Vec::new();
                for x in lo.0..=hi.0 {
                    for y in lo.1..=hi.1 {
                        for z in lo.2..=hi.2 {
                            if self.world.get(x, y, z) != block && self.world.set(x, y, z, block) {
                                changed.push(((x, y, z), block));
                                if !chunks.contains(&(x >> 4, z >> 4)) {
                                    chunks.push((x >> 4, z >> 4));
                                }
                            }
                        }
                    }
                }
                let count = changed.len();
                let outcome = Outcome { changed: count, blocks: if count <= 512 { changed } else { Vec::new() }, chunks };
                if count > 0 {
                    self.refresh_after_edit(&outcome);
                }
                match (fill, count) {
                    (_, 0) if !fill => self.reply(sender, "§cLe bloc n'a pas pu être placé"),
                    (false, _) => self.reply(sender, "Bloc placé"),
                    (true, n) => self.reply(sender, &format!("{} blocs remplis", n)),
                }
            }
            "time" => {
                let now = self.world_time();
                let wanted = match (arg(0), args.get(1)) {
                    ("query", _) => return self.reply(sender, &format!("L'heure du jour est {}", now.rem_euclid(24000))),
                    ("set", Some(w)) => crate::command::time_of(w),
                    ("add", Some(w)) => w.parse::<i64>().ok().filter(|n| (0..=i32::MAX as i64).contains(n)).map(|n| now + n),
                    _ => None,
                };
                match wanted {
                    Some(time) => {
                        self.time_base += time - now;
                        self.broadcast(bedrock::ID_SET_TIME, &bedrock::set_time(time as i32), None);
                        self.reply(sender, &format!("Heure définie sur {}", time));
                    }
                    None => usage(self),
                }
            }
            "tp" | "teleport" => {
                // Formes : <position> | <joueur> | <victime> <position>
                let (victims, destination) = if let Some(p) = crate::command::position(args, 0, here) {
                    (sender.into_iter().collect::<Vec<_>>(), Some(p))
                } else if args.len() == 1 {
                    let to = self.targets(sender, arg(0)).first().and_then(|a| self.clients.get(a)).map(|c| (c.pos.0, c.pos.1 - 1.62, c.pos.2));
                    (sender.into_iter().collect(), to)
                } else if args.len() >= 4 {
                    (self.targets(sender, arg(0)), crate::command::position(args, 1, here))
                } else {
                    return usage(self);
                };
                let destination = match destination {
                    Some(d) => d,
                    None => return self.reply(sender, NO_TARGET),
                };
                if victims.is_empty() {
                    return self.reply(sender, NO_TARGET);
                }
                for v in victims {
                    let name = self.name_at(&v);
                    self.teleport(&v, (destination.0, destination.1 + 1.62, destination.2));
                    self.reply(sender, &format!("{} téléporté en {:.2}, {:.2}, {:.2}", name, destination.0, destination.1, destination.2));
                }
            }
            "locate" => {
                use crate::world::structures::{self as st, Target};
                use std::sync::atomic::Ordering;
                let (mode, name) = (arg(0).to_ascii_lowercase(), arg(1).to_string());
                if name.is_empty() || (mode != "structure" && mode != "biome") {
                    return usage(self);
                }
                if self.world.generator().vanilla().is_none() {
                    return self.reply(sender, "§cCe monde n'a ni structures ni biomes du jeu (terrain plat ou ancien terrain de Basalt)");
                }
                enum Wanted {
                    Structure(Target),
                    Biome(crate::world::vanilla::B),
                }
                let wanted = if mode == "structure" {
                    match Target::parse(&name) {
                        Some(t) => Wanted::Structure(t),
                        None if st::NOT_GENERATED.contains(&name.trim_start_matches("minecraft:")) => {
                            return self.reply(sender, &format!("§cBasalt ne génère pas encore cette structure : {}", name));
                        }
                        None => return self.reply(sender, &format!("§cStructure inconnue : {}", name)),
                    }
                } else {
                    let short = name.trim_start_matches("minecraft:").to_ascii_lowercase();
                    match crate::world::vanilla::B::ALL.iter().find(|b| b.name() == short) {
                        Some(b) => Wanted::Biome(*b),
                        None => return self.reply(sender, &format!("§cBiome inconnu : {}", name)),
                    }
                };
                // La recherche peut parcourir des milliers de positions : sur un fil à part,
                // le résultat arrive quelques instants plus tard
                if self.locate_busy.load(Ordering::Relaxed) >= 4 {
                    return self.reply(sender, "§cTrop de recherches en cours, réessayez dans un instant");
                }
                self.locate_busy.fetch_add(1, Ordering::Relaxed);
                let (view, tx, busy) = (self.world.view(), self.locate_tx.clone(), self.locate_busy.clone());
                let (x, z) = (here.0.floor() as i32, here.2.floor() as i32);
                let spawned = std::thread::Builder::new().name("locate".into()).spawn(move || {
                    crate::utils::lower_thread_priority();
                    let message = match (view.vanilla(), wanted) {
                        (Some(v), Wanted::Structure(t)) => match st::locate(v, t, x, z) {
                            Some((px, py, pz)) => {
                                let d = (((px - x) as f64).powi(2) + ((pz - z) as f64).powi(2)).sqrt().round();
                                format!("{} {} en {}, {}, {} (à {} blocs)", t.label(), t.verb(), px, py, pz, d)
                            }
                            None => "§cAucune structure de ce type dans un rayon de 10 000 blocs".to_string(),
                        },
                        (Some(v), Wanted::Biome(b)) => match st::locate_biome(v, b, x, z) {
                            Some((px, py, pz)) => {
                                let d = (((px - x) as f64).powi(2) + ((pz - z) as f64).powi(2)).sqrt().round();
                                format!("Le biome {} le plus proche se trouve en {}, {}, {} (à {} blocs)", b.name(), px, py, pz, d)
                            }
                            None => format!("§cAucun biome {} dans un rayon de 6 400 blocs", b.name()),
                        },
                        (None, _) => "§cRecherche impossible sur ce monde".to_string(),
                    };
                    let _ = tx.send((sender, message));
                    busy.fetch_sub(1, Ordering::Relaxed);
                });
                if spawned.is_err() {
                    self.locate_busy.fetch_sub(1, Ordering::Relaxed);
                    self.reply(sender, "§cRecherche impossible pour le moment");
                }
            }
            "kill" => {
                let targets = if args.is_empty() { sender.into_iter().collect() } else { self.targets(sender, arg(0)) };
                if targets.is_empty() {
                    return self.reply(sender, NO_TARGET);
                }
                for t in targets {
                    let name = self.name_at(&t);
                    self.hurt(&t, 0.0, Cause::Command);
                    if Some(t) != sender {
                        self.reply(sender, &format!("{} a été tué", name));
                    }
                }
            }
            "kick" => {
                let targets = self.targets(sender, arg(0));
                if args.is_empty() {
                    return usage(self);
                }
                if targets.is_empty() {
                    return self.reply(sender, NO_TARGET);
                }
                let reason = crate::log::clean(&args[1..].join(" "), 128);
                for t in targets {
                    let name = self.name_at(&t);
                    let message = if reason.is_empty() { "Vous avez été expulsé du jeu".to_string() } else { format!("Vous avez été expulsé du jeu : {}", reason) };
                    self.kick(&t, &message);
                    info!("{} expulsé par {}", name, who);
                    self.reply(sender, &format!("{} a été expulsé du jeu", name));
                }
            }
            "op" | "deop" => {
                if args.is_empty() {
                    return usage(self);
                }
                let grant = command == "op";
                let name = crate::log::clean(arg(0), 32);
                let was = self.ops.is_op(&name);
                self.set_op(&name, grant);
                let text = match (grant, was) {
                    (true, false) => format!("Opérateur : {}", name),
                    (true, true) => format!("§c{} est déjà opérateur", name),
                    (false, true) => format!("{} n'est plus opérateur", name),
                    (false, false) => format!("§c{} n'est pas opérateur", name),
                };
                if sender.is_some() {
                    self.reply(sender, &text);
                }
                if let (Some(t), true) = (self.addr_of(&name), grant != was) {
                    let told = if grant { "Vous êtes maintenant opérateur" } else { "Vous n'êtes plus opérateur" };
                    self.send(&t, bedrock::ID_TEXT, &bedrock::text_raw(told));
                }
            }
            "allowlist" | "whitelist" => {
                let words: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                self.whitelist_command(&words);
                if sender.is_some() {
                    self.reply(sender, "Liste blanche mise à jour (détail dans la console du serveur)");
                }
            }
            "status" => {
                let (tps, worst) = (self.perf.last.0, self.perf.last.1.as_millis());
                // Tant qu'aucune mesure n'a été faite (première seconde), le serveur est à l'heure
                let tps = if tps <= 0.0 { 20.0 } else { tps.min(20.0) };
                let color = if tps >= 19.0 {
                    "§a"
                } else if tps >= 15.0 {
                    "§e"
                } else {
                    "§c"
                };
                let up = self.started.elapsed().as_secs();
                let uptime = match (up / 86_400, up / 3600 % 24, up / 60 % 60, up % 60) {
                    (0, 0, m, s) => format!("{} min {} s", m, s),
                    (0, h, m, _) => format!("{} h {} min", h, m),
                    (d, h, m, _) => format!("{} j {} h {} min", d, h, m),
                };
                let rates = self.perf.rates;
                let sessions = (self.rak.sessions.len(), self.nether.as_ref().map(|n| n.len()).unwrap_or(0));
                let lines = vec![
                    "§2---- §fÉtat du serveur §2----".to_string(),
                    format!("§6En marche depuis : §f{}", uptime),
                    format!("§6Charge : {}{:.1} ticks/s §7(objectif 20, pire retard {} ms)", color, tps, worst),
                    format!("§6Joueurs : §f{}/{} §7({} sessions RakNet, {} NetherNet)", self.players().len(), self.cfg.max_players, sessions.0, sessions.1),
                    format!("§6Réseau : §f{} Ko/s reçus, {} Ko/s envoyés §7({} et {} datagrammes/s)", rates.2 / 1024, rates.3 / 1024, rates.0, rates.1),
                    match memory_mb() {
                        Some(mb) => format!("§6Mémoire : §f{} Mo", mb),
                        None => "§6Mémoire : §7non mesurée sur ce système".to_string(),
                    },
                    format!("§6Monde : §f{} blocs modifiés, {} blocs à données (coffres, fours…)", self.world.edit_count(), self.world.tiles.len()),
                    format!("§6Plugins : §f{} §7— vérification des logins : {} fil(s), {} en attente", self.plugins.catalog().plugins.len(), self.logins.threads, self.logins.pending),
                ];
                for line in lines {
                    self.reply(sender, &line);
                }
            }
            "version" => {
                let build = match crate::BUILD {
                    Some(b) if !b.is_empty() => format!(" §7({})", b),
                    _ => String::new(),
                };
                let lines = vec![
                    format!("§fCe serveur utilise §aBasalt {}{}", crate::VERSION, build),
                    format!(
                        "§fVersions du jeu acceptées : §aBedrock {} à {} §7(protocoles {} à {})",
                        crate::versions::name(self.cfg.min_protocol),
                        crate::versions::name(self.cfg.max_protocol),
                        self.cfg.min_protocol,
                        self.cfg.max_protocol
                    ),
                    format!("§fSystème : §a{} {}", std::env::consts::OS, std::env::consts::ARCH),
                ];
                for line in lines {
                    self.reply(sender, &line);
                }
            }
            "timings" => match arg(0) {
                "on" => {
                    if timings::enable() {
                        self.reply(sender, "§aMesures allumées. Laissez tourner le serveur, puis lancez §f/timings report§a.");
                    } else {
                        // Déjà en marche : rien à faire, ce n'est pas une erreur
                        self.reply(sender, "§aLes mesures tournent déjà. §f/timings report§a pour le rapport, §f/timings reset§a pour repartir de zéro.");
                    }
                }
                "off" => {
                    if timings::disable() {
                        self.reply(sender, "§aMesures éteintes. Les chiffres relevés restent disponibles : §f/timings report§a.");
                    } else {
                        self.reply(sender, "§eLes mesures sont déjà éteintes.");
                    }
                }
                "reset" => {
                    timings::reset();
                    if timings::is_enabled() {
                        self.reply(sender, "§aCompteurs remis à zéro. Les mesures continuent : inutile de refaire §f/timings on§a.");
                    } else {
                        self.reply(sender, "§aCompteurs remis à zéro. Les mesures sont éteintes : §f/timings on§a pour les relancer.");
                    }
                }
                "report" | "paste" => {
                    let header = vec![
                        format!("Basalt {}{}", crate::VERSION, crate::BUILD.map(|b| format!(" ({})", b)).unwrap_or_default()),
                        format!("Système : {} {} — mémoire : {}", std::env::consts::OS, std::env::consts::ARCH, memory_mb().map(|m| format!("{} Mo", m)).unwrap_or_else(|| "non mesurée".into())),
                        format!("Joueurs : {}/{} — plugins : {} — blocs modifiés : {}", self.players().len(), self.cfg.max_players, self.plugins.catalog().plugins.len(), self.world.edit_count()),
                        format!("Mesures {}", if timings::is_enabled() { "en cours" } else { "arrêtées" }),
                    ];
                    let (text, top) = match (timings::report(&header), timings::snapshot()) {
                        (Some(t), Some(s)) if !s.2.is_empty() => (t, s),
                        _ => return self.reply(sender, "§cAucune mesure : lancez d'abord §f/timings on§c et laissez tourner le serveur."),
                    };
                    // Le rapport complet va dans un fichier ; l'essentiel est affiché tout de suite
                    let dir = self.cfg.root.join("timings");
                    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    let path = dir.join(format!("timings-{}.txt", stamp));
                    // Écrit sur le fil des écritures : le rapport peut être long
                    let target = path.clone();
                    crate::background::DISK.run(move || {
                        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&target, text.as_bytes())) {
                            warn!("Rapport de mesures non enregistré dans {} : {}", target.display(), e);
                        }
                    });
                    self.reply(sender, &format!("§2---- §fMesures sur {:.0} s §7({} ticks, {:.1} ticks/s) §2----", top.0, top.1, top.1 as f64 / top.0));
                    for line in top.2.iter().take(8) {
                        let per_tick = if top.1 == 0 { 0.0 } else { line.total_ms / top.1 as f64 };
                        let color = if per_tick >= 25.0 {
                            "§c"
                        } else if per_tick >= 5.0 {
                            "§e"
                        } else {
                            "§a"
                        };
                        self.reply(sender, &format!("{}{:>5.1} %§f  {} §7({:.2} ms par tick, pire {:.1} ms)", color, line.share, line.name, per_tick, line.worst_ms));
                    }
                    if let Some(p) = top.2.iter().find(|l| l.name.starts_with("Pause de 1 ms")).filter(|l| l.worst_ms >= 50.0) {
                        self.reply(sender, &format!("§cÀ-coups venant de la machine : une pause de 1 ms a duré jusqu'à {:.0} ms. Détails dans le rapport.", p.worst_ms));
                    }
                    self.reply(sender, &format!("§6Rapport complet : §f{}", path.display()));
                }
                _ => usage(self),
            },
            "stop" => {
                self.broadcast(bedrock::ID_TEXT, &bedrock::text_raw("Arrêt du serveur…"), None);
                info!("Arrêt du serveur demandé par {}", who);
                self.running = false;
            }
            _ => {}
        }
    }

    /// Annonce au jeu les commandes que ce joueur a le droit d'utiliser.
    fn send_commands(&mut self, addr: &SocketAddr) {
        let name = match self.clients.get(addr) {
            // Format connu pour ce seul protocole (relevé auprès du serveur officiel)
            Some(c) if c.stage == Stage::InGame && c.protocol == 2225 && self.cfg.commands_in_game => match &c.name {
                Some(n) => n.clone(),
                None => return,
            },
            _ => return,
        };
        let own = self.clients.get(addr).and_then(|c| c.data.as_ref()).map(|d| d.permissions.clone()).unwrap_or_default();
        let op = self.ops.is_op(&name);
        let known = self.plugins.permissions();
        // Les commandes du jeu d'abord, puis celles des plugins (qui ne les remplacent pas)
        let mut list: Vec<(String, String, Option<Vec<bedrock::Overload>>)> = VanillaCommands::all()
            .iter()
            .filter(|c| op || !c.op)
            .map(|c| (c.name.to_string(), c.description.to_string(), Some(bedrock::command_overloads(c.usage))))
            .collect();
        let from_plugins: Vec<(String, String, Option<Vec<bedrock::Overload>>)> = self
            .plugins
            .commands()
            .into_iter()
            .filter(|c| VanillaCommands::get(&c.0).is_none())
            .filter(|c| match self.plugins.command_permission(&c.0) {
                Some(node) => permission::check(op, &own, &known, &node),
                None => true,
            })
            .map(|c| match self.plugins.command_decl(&c.0) {
                // Décrite dans un plugin.yml : sa description et ses vrais paramètres
                Some((description, usage)) => {
                    let args = bedrock::command_overloads(&usage);
                    (c.0, if description.is_empty() { c.1 } else { description }, Some(args))
                }
                None => (c.0, c.1, None),
            })
            .collect();
        list.extend(from_plugins);
        if list.is_empty() && !op {
            return;
        }
        let (blocks, items) = match &self.data {
            Some(d) => (d.block_name_list(), d.item_name_list()),
            None => (Vec::new(), Vec::new()),
        };
        self.send(addr, bedrock::ID_AVAILABLE_COMMANDS, &bedrock::available_commands(&list, &blocks, &items));
    }

    /// Paquet envoyé par le jeu quand un joueur tape une commande : il commence par le texte tapé.
    fn on_command_request(&mut self, addr: &SocketAddr, body: &[u8]) {
        let allowed = match self.clients.get_mut(addr) {
            Some(c) => {
                if c.chat_window.elapsed() > Duration::from_secs(5) {
                    c.chat_window = Instant::now();
                    c.chat_count = 0;
                }
                c.chat_count += 1;
                c.chat_count <= 8
            }
            None => return,
        };
        let mut pos = 0;
        let len = bedrock::read_varu32(body, &mut pos).unwrap_or(0) as usize;
        let line = match body.get(pos..pos + len).map(|b| String::from_utf8_lossy(b).to_string()) {
            Some(l) if allowed && (1..=256).contains(&len) => l,
            _ => return,
        };
        let line: String = line.chars().filter(|c| !c.is_control()).collect();
        let out = self.fire("PlayerCommandPreprocessEvent", vec![("player", self.name_json(addr)), ("message", Json::Str(line.clone()))]);
        if cancelled(&out) {
            return;
        }
        let line = text_of(&out, "message", 256).unwrap_or(line);
        self.run_player_command(addr, &line);
    }

    /// Demande aux plugins s'ils laissent faire une action sur un bloc. Faux si l'un d'eux
    /// répond `false`.
    fn block_event(&mut self, event: &str, addr: &SocketAddr, pos: (i32, i32, i32)) -> bool {
        if !self.plugins.available() || self.plugins.catalog().plugins.is_empty() {
            return true;
        }
        let name = match self.clients.get(addr).and_then(|c| c.name.clone()) {
            Some(n) => n,
            None => return true,
        };
        let state = self.world.get(pos.0, pos.1, pos.2);
        let block = self.data.as_ref().and_then(|d| d.block_name(state)).unwrap_or("").to_string();
        let answers = self.emit(event, &[LVal::Player(name), LVal::Int(pos.0 as i64), LVal::Int(pos.1 as i64), LVal::Int(pos.2 as i64), LVal::Str(block)]);
        !answers.iter().any(|a| *a == LVal::Bool(false))
    }

    /// Événement à la PocketMine (écouteurs des plugins, voir docs/PLUGINS.md). Renvoie l'état
    /// final, ou None si aucun plugin n'écoute cet événement (rien n'est alors calculé).
    fn fire(&mut self, event: &str, fields: Vec<(&str, Json)>) -> Option<Json> {
        if !self.plugins.listens(event) {
            return None;
        }
        let _t = timings::scope_with(|| format!("Plugins — {}", event));
        let info = self.info();
        self.plugins.set_info(&info, self.tick);
        let state = Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
        let out = self.plugins.fire(event, &state);
        self.apply_actions();
        out
    }

    /// Objet tel que les écouteurs le voient : { name, count }.
    fn item_json(&self, id: i16, count: u8) -> Json {
        let name = self.data.as_ref().and_then(|d| d.item_by_id(id)).map(|i| i.name.clone()).unwrap_or_default();
        Json::Obj(vec![("name".into(), Json::Str(name)), ("count".into(), Json::Num(count as f64))])
    }

    /// Bloc tel que les écouteurs le voient : { name, x, y, z }.
    fn block_json(&self, state: u32, pos: (i32, i32, i32)) -> Json {
        let name = self.data.as_ref().and_then(|d| d.block_name(state)).unwrap_or("").to_string();
        Json::Obj(vec![
            ("name".into(), Json::Str(name)),
            ("x".into(), Json::Num(pos.0 as f64)),
            ("y".into(), Json::Num(pos.1 as f64)),
            ("z".into(), Json::Num(pos.2 as f64)),
        ])
    }

    /// Pseudo d'un joueur, pour les écouteurs.
    fn name_json(&self, addr: &SocketAddr) -> Json {
        Json::Str(self.clients.get(addr).and_then(|c| c.name.clone()).unwrap_or_default())
    }

    /// PlayerInteractEvent (clic droit sur un bloc) : vrai si un écouteur l'annule.
    fn interact_cancelled(&mut self, addr: &SocketAddr, pos: (i32, i32, i32)) -> bool {
        // Un spectateur n'ouvre ni porte ni coffre
        if self.clients.get(addr).map(|c| c.mode.is_spectator()).unwrap_or(true) {
            return true;
        }
        if !self.plugins.listens("PlayerInteractEvent") {
            return false;
        }
        let state = self.world.get(pos.0, pos.1, pos.2);
        let fields = vec![("player", self.name_json(addr)), ("block", self.block_json(state, pos)), ("action", Json::Num(1.0))];
        let out = self.fire("PlayerInteractEvent", fields);
        cancelled(&out)
    }

    /// Événement simple vers les plugins (server.on) ; renvoie leurs réponses.
    fn emit(&mut self, event: &str, args: &[LVal]) -> Vec<LVal> {
        let _t = timings::scope_with(|| format!("Plugins — événement {}", event));
        let info = self.info();
        self.plugins.set_info(&info, self.tick);
        let res = self.plugins.dispatch(event, args);
        self.apply_actions();
        res
    }

    /// Envoie un paquet de jeu à un joueur (compressé si sa session le demande).
    fn send(&mut self, addr: &SocketAddr, id: u32, body: &[u8]) {
        let compression = self.clients.get(addr).map(|c| c.compression).unwrap_or(false);
        if self.cfg.debug {
            if id != bedrock::ID_LEVEL_CHUNK {
                info!("[paquets] -> {} : id {} ({} octets)", addr, id, body.len());
            }
        }
        // Gros paquets identiques pour tous (recettes, objets, biomes, menu créatif : plus
        // d'un mégaoctet à chaque connexion) : compressés une fois, gardés pour les suivants
        if compression && body.len() >= CACHE_FROM && self.cfg.network_compression {
            let key = (id, body.len(), quick_hash(body));
            let packet = match self.packet_cache.get(&key) {
                Some(p) => p.clone(),
                None => {
                    let p = Arc::new(bedrock::encode_packet_with(id, body, true, crate::utils::deflate::Level::Best));
                    if self.packet_cache.len() >= 64 {
                        self.packet_cache.clear();
                    }
                    self.packet_cache.insert(key, p.clone());
                    p
                }
            };
            return self.net_send(addr, &packet);
        }
        self.net_send(addr, &bedrock::encode_packet(id, body, compression));
    }

    /// Expulse un joueur d'après son pseudo (sans effet s'il n'est pas en jeu).
    fn kick_player(&mut self, name: &str, message: &str) {
        let addr = self.clients.iter().find(|(_, c)| c.name.as_deref().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false)).map(|(a, _)| *a);
        if let Some(a) = addr {
            self.kick(&a, message);
        }
    }

    /// Message de déconnexion, puis fermeture un peu plus tard pour qu'il arrive.
    fn kick(&mut self, addr: &SocketAddr, message: &str) {
        self.send(addr, bedrock::ID_DISCONNECT, &bedrock::disconnect(message));
        self.net_close(addr, Duration::from_millis(1500));
        if let Some(c) = self.clients.get_mut(addr) {
            c.stage = Stage::Done;
        }
    }

    /// Coupe immédiatement un client qui dépasse une limite ou envoie des données invalides.
    fn violation(&mut self, addr: &SocketAddr, reason: &'static str) {
        if let Some(c) = self.clients.get_mut(addr) {
            c.stage = Stage::Done;
        }
        self.net_abort(addr, reason);
    }

    /// Ferme les connexions qui traînent sans s'identifier ni entrer en jeu.
    /// Mise au point : toutes les 6 secondes après l'entrée d'un joueur, envoie la sonde
    /// suivante. Le joueur indique à quel moment l'écran de chargement s'est fermé, et le
    /// journal montre ce que le jeu a envoyé après chaque sonde.
    fn run_probes(&mut self) {
        if !self.cfg.probes {
            return;
        }
        let tick = self.tick;
        let due: Vec<(SocketAddr, u8, (i32, i32), i32, u64)> = self
            .clients
            .iter_mut()
            .filter(|(_, c)| c.stage == Stage::InGame && c.protocol >= 2225 && c.probe < bedrock::PROBES)
            .filter(|(_, c)| tick >= c.probe_tick + 120)
            .map(|(a, c)| {
                c.probe += 1;
                c.probe_tick = tick;
                (*a, c.probe, c.center, c.radius.min(3).max(1), c.inputs)
            })
            .collect();
        for (addr, probe, center, radius, inputs) in due {
            warn!("{} : sonde {} : {} ({} mouvements reçus jusqu'ici)", addr, probe, bedrock::probe_name(probe), inputs);
            if let Some((count, payload, flag)) = bedrock::probe_chunk(probe) {
                for (x, z) in Self::chunks_around(center, radius) {
                    let pk = bedrock::level_chunk_probe(x, z, count, &payload, flag);
                    self.send(&addr, bedrock::ID_LEVEL_CHUNK, &pk);
                }
            }
        }
    }

    /// Envoie un paquet à tous les joueurs en jeu (jeux 1.26.60 et plus), sauf éventuellement un.
    fn broadcast(&mut self, id: u32, body: &[u8], except: Option<SocketAddr>) {
        let targets: Vec<SocketAddr> = self
            .clients
            .iter()
            .filter(|(a, c)| c.stage == Stage::InGame && c.protocol >= 2225 && Some(**a) != except)
            .map(|(a, _)| *a)
            .collect();
        for addr in targets {
            self.send(&addr, id, body);
        }
    }

    /// Message de chat d'un joueur : limité en fréquence et en longueur, nettoyé, puis diffusé.
    fn on_chat(&mut self, addr: &SocketAddr, body: &[u8]) {
        let (name, allowed) = match self.clients.get_mut(addr) {
            Some(c) if c.protocol >= 2225 => {
                if c.chat_window.elapsed() > Duration::from_secs(5) {
                    c.chat_window = Instant::now();
                    c.chat_count = 0;
                }
                c.chat_count += 1;
                (c.name.clone().unwrap_or_default(), c.chat_count <= 8)
            }
            _ => return,
        };
        if !allowed {
            return; // plus de 8 messages en 5 secondes : ignorés
        }
        let message = match bedrock::read_chat(body) {
            Some(m) => m,
            None => {
                if self.cfg.debug {
                    let hex: String = body.iter().take(96).map(|b| format!("{:02x}", b)).collect();
                    info!("{} : message de chat illisible ({} octets) {}", addr, body.len(), hex);
                }
                return;
            }
        };
        // Pas de caractères de contrôle. Les couleurs (§a, ou &a plus facile à taper) sont
        // réservées aux opérateurs ; chez les autres, les § sont retirés.
        let op = self.ops.is_op(&name);
        let clean: String = message.chars().filter(|c| !c.is_control() && (op || *c != '§')).take(256).collect();
        let clean = if op { ampersand_colors(&clean) } else { clean };
        let clean = clean.trim().to_string();
        if clean.is_empty() {
            return;
        }
        let out = self.fire("PlayerChatEvent", vec![("player", Json::Str(name.clone())), ("message", Json::Str(clean.clone()))]);
        if cancelled(&out) {
            return;
        }
        let clean = match text_of(&out, "message", 256) {
            Some(m) if !m.trim().is_empty() => m,
            Some(_) => return,
            None => clean,
        };
        info!("<{}> {}", name, clean);
        self.emit("chat", &[LVal::Player(name.clone()), LVal::Str(clean.clone())]);
        // Le nom affiché a pu être changé par un plugin (y compris pendant cet événement)
        let shown = self.addr_of(&name).and_then(|a| self.clients.get(&a)).and_then(|c| c.data.as_ref()).map(|d| d.display_name.clone()).unwrap_or(name);
        // Envoyé comme un message du serveur, « <pseudo> message », et non comme un message de
        // joueur : le jeu applique aux messages de joueurs son filtre de grossièretés (mots
        // remplacés par des #), que le serveur ne veut pas.
        self.broadcast(bedrock::ID_TEXT, &bedrock::text_raw(&format!("<{}§r> {}", shown, clean)), None);
    }

    /// Sous les pieds d'un joueur (position à hauteur des yeux) : sol, liquide ou vide.
    fn footing(&self, eye: (f32, f32, f32)) -> Footing {
        let feet = eye.1 - player_eye_height();
        let air = self.world.air;
        let mut found = Footing::Air;
        // Les quatre coins du joueur (0,3 bloc autour de son centre), juste sous ses pieds et à
        // hauteur de ses pieds (liquide)
        for (dx, dz) in [(-0.3, -0.3), (0.3, -0.3), (-0.3, 0.3), (0.3, 0.3)] {
            let (x, z) = ((eye.0 + dx).floor() as i32, (eye.2 + dz).floor() as i32);
            for (y, below) in [((feet + 0.01).floor() as i32, false), ((feet - 0.05).floor() as i32, true)] {
                let state = self.world.get(x, y, z);
                if state == air {
                    continue;
                }
                let liquid = self.data.as_ref().and_then(|d| d.block_name(state)).map(|n| n.contains("water") || n.contains("lava")).unwrap_or(false);
                if liquid {
                    return Footing::Liquid;
                }
                if below {
                    found = Footing::Solid;
                }
            }
        }
        found
    }

    /// Un joueur perd de la vie. Sans effet en créatif (sauf chute hors du monde et /kill),
    /// sur un joueur déjà mort, ou sur un joueur frappé il y a moins d'une demi-seconde.
    fn hurt(&mut self, addr: &SocketAddr, amount: f32, cause: Cause) {
        let tick = self.tick;
        let unstoppable = matches!(cause, Cause::Void | Cause::Command);
        match self.clients.get(addr) {
            Some(c) if c.joined && c.dead.is_none() => {
                if c.mode.is_spectator() || (!c.mode.takes_damage() && !unstoppable) {
                    return;
                }
                if matches!(cause, Cause::Player(_)) && c.hurt_at > 0 && tick.saturating_sub(c.hurt_at) < 10 {
                    return;
                }
            }
            _ => return,
        }
        // Écouteurs : annulation, dégâts modifiés
        let mut amount = amount;
        if self.plugins.listens("EntityDamageEvent") {
            let (label, damager) = match &cause {
                Cause::Fall => ("FALL", None),
                Cause::Void => ("VOID", None),
                Cause::Player(by) => ("ENTITY_ATTACK", Some(by.clone())),
                Cause::Command => ("SUICIDE", None),
                Cause::Plugin => ("CUSTOM", None),
            };
            let mut fields = vec![("entity", self.name_json(addr)), ("cause", Json::Str(label.into())), ("baseDamage", Json::Num(amount as f64))];
            if let Some(by) = damager {
                fields.push(("damager", Json::Str(by)));
            }
            let out = self.fire("EntityDamageEvent", fields);
            if cancelled(&out) {
                return;
            }
            if let Some(n) = out.as_ref().and_then(|v| v.get("baseDamage")).and_then(|n| n.as_f64()) {
                amount = n.clamp(0.0, 1.0e6) as f32;
            }
        }
        let (id, dead) = match self.clients.get_mut(addr) {
            Some(c) if c.joined && c.dead.is_none() => {
                c.hurt_at = tick;
                let d = match c.data.as_mut() {
                    Some(d) => d,
                    None => return,
                };
                if unstoppable {
                    d.health = 0.0;
                } else {
                    d.damage(amount);
                }
                (c.id, d.is_dead())
            }
            _ => return,
        };
        // Le joueur rougit, chez lui et chez ceux qui le voient
        self.actor_event_around(addr, id, bedrock::EVENT_HURT);
        self.sync_attributes(addr);
        if dead {
            self.die(addr, cause);
        }
    }

    /// Événement d'un joueur (dégât, mort…) envoyé à lui-même et à ceux qui le voient.
    fn actor_event_around(&mut self, addr: &SocketAddr, id: i64, event: u8) {
        if self.refused_extras.contains(&bedrock::ID_ACTOR_EVENT) {
            return;
        }
        let packet = bedrock::actor_event(id, event, 0);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| *a == addr || c.visible.contains(&id)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_ACTOR_EVENT, &packet);
        }
    }

    /// Mort d'un joueur : écran de mort chez lui, chute du personnage chez les autres, message
    /// à tous. Son inventaire tombe au sol, comme dans le jeu.
    fn die(&mut self, addr: &SocketAddr, cause: Cause) {
        let tick = self.tick;
        let (id, name) = match self.clients.get_mut(addr) {
            Some(c) if c.dead.is_none() => {
                c.dead = Some(tick);
                c.fall_peak = None;
                if let Some(d) = c.data.as_mut() {
                    d.health = 0.0;
                }
                let shown = c.data.as_ref().map(|d| d.display_name.clone()).or_else(|| c.name.clone()).unwrap_or_default();
                (c.id, shown)
            }
            _ => return,
        };
        let message = match &cause {
            Cause::Fall => format!("{} s'est écrasé au sol", name),
            Cause::Void => format!("{} est tombé hors du monde", name),
            Cause::Player(by) => {
                let shown = self.addr_of(by).and_then(|a| self.clients.get(&a)).and_then(|c| c.data.as_ref()).map(|d| d.display_name.clone()).unwrap_or_else(|| by.clone());
                format!("{} a été tué par {}", name, shown)
            }
            Cause::Command | Cause::Plugin => format!("{} est mort", name),
        };
        // Écouteurs : message de mort, inventaire gardé
        let out = self.fire(
            "PlayerDeathEvent",
            vec![("player", self.name_json(addr)), ("deathMessage", Json::Str(message.clone())), ("keepInventory", Json::Bool(self.cfg.keep_inventory))],
        );
        let message = text_of(&out, "deathMessage", 512).unwrap_or(message);
        let keep = out.as_ref().and_then(|v| v.get("keepInventory")).and_then(|k| k.as_bool()).unwrap_or(self.cfg.keep_inventory);
        self.actor_event_around(addr, id, bedrock::EVENT_DEATH);
        // Son inventaire tombe au sol autour de lui, sauf avec keep-inventory=true. Si le jeu
        // refuse les objets au sol, il est perdu (comme un objet tombé dans la lave).
        if !keep {
            let mut stuff: Vec<(i16, u8)> = Vec::new();
            let at = match self.clients.get_mut(addr) {
                Some(c) => {
                    for slots in [&mut c.inv.main.slots, &mut c.inv.armor.slots, &mut c.inv.offhand.slots] {
                        for s in slots.iter_mut() {
                            if !s.is_empty() {
                                stuff.push((s.id, s.count));
                                *s = inventory::ItemStack::EMPTY;
                            }
                        }
                    }
                    c.inv_touched = true;
                    c.pos
                }
                None => return,
            };
            if !stuff.is_empty() {
                for (n, (item, count)) in stuff.into_iter().enumerate() {
                    // Éparpillés autour du corps, avec un petit élan au hasard
                    let spot = (at.0, at.1 - player_eye_height() + 0.5, at.2);
                    let vel = self.scatter(n as u64, 0.2);
                    self.drop_item(spot, vel, item, count, 40);
                }
                self.send_inventory_with(addr, true);
            }
        }
        if !self.refused_extras.contains(&bedrock::ID_DEATH_INFO) {
            self.send(addr, bedrock::ID_DEATH_INFO, &bedrock::death_info(&message));
        }
        if !self.refused_extras.contains(&bedrock::ID_RESPAWN) {
            let spawn = self.spawn_point();
            self.send(addr, bedrock::ID_RESPAWN, &bedrock::respawn(spawn, bedrock::RESPAWN_SEARCHING, id));
        }
        if !message.is_empty() {
            info!("{}", crate::log::clean(&message, 128));
            self.broadcast(bedrock::ID_TEXT, &bedrock::text_raw(&message), None);
        }
        let player = self.clients.get(addr).and_then(|c| c.name.clone()).unwrap_or_default();
        self.emit("death", &[LVal::Player(player)]);
    }

    /// Choisit le terrain du monde (celui déjà enregistré, sinon `level-type` et `level-seed`),
    /// recharge les modifications et place le point d'apparition.
    fn open_world(&mut self) {
        use crate::world::generator::{parse_seed, Generator, Kind, Palette};
        let dir = std::path::PathBuf::from(&self.cfg.world_dir);
        let (kind, seed, created) = match crate::world::World::stored_generator(&dir) {
            Ok(Some((kind, seed))) => (kind, seed, false),
            Ok(None) => (Kind::from_config(&self.cfg.level_type), parse_seed(&self.cfg.level_seed), true),
            Err(e) => {
                warn!("Monde illisible ({}) : terrain plat, l'ancien fichier sera remplacé au prochain enregistrement", e);
                (Kind::Flat, 0, false)
            }
        };
        let palette = {
            let data = &self.data;
            let resolve = |name: &str| data.as_ref().and_then(|d| d.state_from_spec(name));
            // Plans officiels des structures (fichiers du serveur de Mojang), s'ils sont là
            let templates = ["structures", "structures.zip"].iter().map(Path::new).find(|p| p.exists()).and_then(|p| {
                match crate::world::templates::Templates::load(p, &resolve) {
                    Ok(t) => {
                        info!("Structures : {} plans officiels chargés depuis {}", t.count(), p.display());
                        Some(std::sync::Arc::new(t))
                    }
                    Err(e) => {
                        warn!("Structures : plans de {} illisibles ({}) ; structures faites à la main", p.display(), e);
                        None
                    }
                }
            });
            Palette::new(&resolve).with_templates(templates)
        };
        let generator = if kind == Kind::Plugin {
            // Générateur fourni par un plugin : plugins/<plugin>/generators/<nom>.lua
            let name = if created {
                self.cfg.level_type.trim().splitn(2, ':').nth(1).unwrap_or("").trim().to_string()
            } else {
                crate::world::World::stored_plugin(&dir).unwrap_or_default()
            };
            let loaded = match self.data.clone() {
                Some(data) => crate::plugin::generator::LuaGenerator::load(Path::new(&self.cfg.plugins_dir), &name, seed, data, self.cfg.plugin_memory_mb),
                None => Err("données du jeu absentes".to_string()),
            };
            let loaded = match loaded {
                Ok(g) => {
                    info!("Générateur « {} » fourni par le plugin {}", g.name, g.plugin);
                    Some(g)
                }
                Err(e) => {
                    error!("Générateur « {} » : {} — le terrain reste vide tant qu'il manque (rien n'est perdu : le terrain n'est jamais enregistré)", name, e);
                    None
                }
            };
            Generator::with_plugin(&name, seed, palette, loaded)
        } else {
            Generator::new(kind, seed, palette)
        };
        let stored = generator.stored_name();
        let tiles = std::mem::take(&mut self.world.tiles);
        self.world = crate::world::World::with_generator(generator);
        self.world.tiles = tiles;
        match self.world.load(&dir) {
            Ok(0) => {}
            Ok(n) => info!("Monde rechargé : {} blocs modifiés", n),
            Err(e) => warn!("Monde non rechargé ({}) : le serveur repart d'un terrain neuf, l'ancien fichier sera remplacé au prochain enregistrement", e),
        }
        if created {
            // Le type de terrain et la graine sont enregistrés tout de suite
            self.world.dirty = true;
            let _ = std::fs::create_dir_all(&dir);
            let _ = self.world.save(&dir);
            info!("Nouveau monde : terrain {}, graine {}", stored, seed as i64);
        } else {
            info!("Terrain du monde : {}, graine {}", stored, seed as i64);
        }
        self.spawn = self.find_spawn();
        self.liquids = self.data.as_deref().and_then(crate::world::liquid::Liquids::new);
    }

    /// Un bloc a changé : l'eau et la lave voisines seront recalculées.
    fn liquid_neighbours(&mut self, pos: (i32, i32, i32)) {
        if let Some(l) = self.liquids.as_mut() {
            l.block_changed(&self.world, self.tick, pos);
        }
    }

    /// Fait couler l'eau et la lave dont la mise à jour est arrivée à échéance.
    fn flow_liquids(&mut self) {
        let Some(mut liquids) = self.liquids.take() else { return };
        let changed = {
            let data = self.data.as_ref();
            let washable = |state: u32| data.and_then(|d| d.block_name(state)).map(washed_away).unwrap_or(false);
            liquids.tick(&mut self.world, self.tick, &washable)
        };
        self.liquids = Some(liquids);
        for (x, y, z, state) in changed {
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, state), None);
        }
    }

    /// Point de réapparition (à hauteur des yeux).
    fn spawn_point(&self) -> (f32, f32, f32) {
        (self.spawn.0 as f32 + 0.5, self.spawn.1 as f32 + player_eye_height(), self.spawn.2 as f32 + 0.5)
    }

    /// Point d'apparition du monde : sur le terrain généré, au-dessus du premier bloc plein
    /// (pas sur les feuilles d'un arbre), près de l'origine.
    fn find_spawn(&self) -> (i32, i32, i32) {
        let (x, _, z) = self.world.generator().spawn();
        let y = self.world.highest(x, z, &|state| self.blocks_movement(state) && !self.is_leaves(state));
        (x, y + 1, z)
    }

    /// Bloc dans lequel on ne peut pas se tenir (ni air, ni liquide, ni plante, ni neige fine).
    fn blocks_movement(&self, state: u32) -> bool {
        if state == self.world.air {
            return false;
        }
        match self.data.as_ref().and_then(|d| d.block_name(state)) {
            Some(n) => !(n.contains("water") || n.contains("lava") || (n.contains("grass") && !n.contains("grass_block")) || n.contains("snow_layer") || n.contains("dandelion") || n.contains("poppy") || n.contains("sapling")),
            None => true,
        }
    }

    /// Bloc de feuilles ?
    fn is_leaves(&self, state: u32) -> bool {
        self.data.as_ref().and_then(|d| d.block_name(state)).map(|n| n.contains("leaves")).unwrap_or(false)
    }

    /// Le joueur a cliqué sur « Réapparaître » : il revient au point de réapparition, en pleine
    /// santé. Ceux qui le voyaient le voient disparaître, puis réapparaître là-bas.
    fn respawn(&mut self, addr: &SocketAddr) {
        let mut spawn = self.spawn_point();
        if self.clients.get(addr).map(|c| c.dead.is_none()).unwrap_or(true) {
            return;
        }
        // Écouteurs : point de réapparition (aux pieds du joueur, comme dans PocketMine)
        if self.plugins.listens("PlayerRespawnEvent") {
            let feet = Json::Obj(vec![
                ("x".into(), Json::Num(spawn.0 as f64)),
                ("y".into(), Json::Num((spawn.1 - player_eye_height()) as f64)),
                ("z".into(), Json::Num(spawn.2 as f64)),
            ]);
            let out = self.fire("PlayerRespawnEvent", vec![("player", self.name_json(addr)), ("respawnPosition", feet)]);
            let at = out.as_ref().and_then(|v| v.get("respawnPosition"));
            let coord = |k: &str| at.and_then(|p| p.get(k)).and_then(|n| n.as_f64()).filter(|n| n.abs() < 3.0e7);
            if let (Some(x), Some(y), Some(z)) = (coord("x"), coord("y"), coord("z")) {
                spawn = (x as f32, (y as f32).clamp(-64.0, 320.0) + player_eye_height(), z as f32);
            }
        }
        let id = match self.clients.get_mut(addr) {
            Some(c) if c.dead.is_some() => {
                c.dead = None;
                c.fall_peak = None;
                c.pos = spawn;
                c.moved_tick = self.tick + 1;
                if let Some(d) = c.data.as_mut() {
                    d.health = d.max_health;
                }
                c.id
            }
            _ => return,
        };
        if !self.refused_extras.contains(&bedrock::ID_RESPAWN) {
            self.send(addr, bedrock::ID_RESPAWN, &bedrock::respawn(spawn, bedrock::RESPAWN_READY, id));
        }
        self.send_attributes(addr, true);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.visible.contains(&id)).map(|(a, _)| *a).collect();
        for v in viewers {
            if let Some(c) = self.clients.get_mut(&v) {
                c.visible.remove(&id);
            }
            self.send(&v, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id));
        }
        let player = self.clients.get(addr).and_then(|c| c.name.clone()).unwrap_or_default();
        self.emit("respawn", &[LVal::Player(player)]);
    }

    /// Un joueur en frappe un autre.
    fn on_attack(&mut self, addr: &SocketAddr, target: u64) {
        if self.cfg.debug {
            let found = self.clients.values().find(|c| c.id as u64 == target).map(|c| (c.name.clone().unwrap_or_default(), c.mode.label(), c.pos));
            info!("[combat] {} frappe l'entité {} : {:?}", addr, target, found);
        }
        let (from, eye, name, damage) = match self.clients.get(addr) {
            Some(c) if c.joined && c.dead.is_none() => {
                let held = c.inv.held();
                let tool = match (held.is_empty(), self.data.as_ref().and_then(|d| d.item_by_id(held.id))) {
                    (false, Some(def)) => VanillaItems::get(&def.name),
                    _ => VanillaItems::hand(),
                };
                let damage = tool.attack_damage().max(1.0);
                (c.id, c.pos, c.name.clone().unwrap_or_default(), damage)
            }
            _ => return,
        };
        if self.clients.get(addr).map(|c| c.mode.is_spectator()).unwrap_or(true) {
            return;
        }
        // Une créature ?
        if self.hit_mob(target as i64, (eye.0, eye.1 - EYE, eye.2), damage) {
            return;
        }
        let victim = self.clients.iter().find(|(_, c)| c.id as u64 == target && c.id != from && c.joined).map(|(a, c)| (*a, c.pos, !c.mode.takes_damage(), c.id));
        let (victim, pos, creative, victim_id) = match victim {
            Some(v) => v,
            None => return,
        };
        // Portée d'un coup : 6 blocs au plus entre les deux joueurs (marge pour le décalage réseau)
        let (dx, dy, dz) = (pos.0 - eye.0, pos.1 - eye.1, pos.2 - eye.2);
        if dx * dx + dy * dy + dz * dz > 6.0 * 6.0 || creative {
            return;
        }
        let before = self.clients.get(&victim).map(|c| c.hurt_at).unwrap_or(0);
        self.hurt(&victim, damage, Cause::Player(name));
        let hit = self.clients.get(&victim).map(|c| c.hurt_at != before).unwrap_or(false);
        // Recul : poussé dans le sens du coup, et un peu vers le haut
        let flat = (dx * dx + dz * dz).sqrt();
        if hit && flat > 0.01 && !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) {
            let motion = (dx / flat * 0.4, 0.4, dz / flat * 0.4);
            self.send(&victim, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(victim_id, motion));
        }
    }

    /// Vie regagnée peu à peu, comme dans le jeu quand la barre de faim est presque pleine :
    /// un demi-cœur toutes les 4 secondes.
    fn regenerate(&mut self) {
        let ready: Vec<SocketAddr> = self
            .clients
            .iter_mut()
            .filter(|(_, c)| c.joined && c.dead.is_none())
            .filter_map(|(a, c)| {
                let d = c.data.as_mut()?;
                if d.health < d.max_health && d.food >= 18.0 {
                    d.heal(1.0);
                    Some(*a)
                } else {
                    None
                }
            })
            .collect();
        for a in ready {
            self.sync_attributes(&a);
        }
    }

    /// Ce qu'un objet au sol trouve à un endroit du monde.
    fn cell(&self, x: i32, y: i32, z: i32) -> Cell {
        if !crate::world::World::in_bounds(y) {
            return if y < 0 { Cell::Solid(1.0) } else { Cell::Empty };
        }
        let state = self.world.get(x, y, z);
        if state == self.world.air {
            return Cell::Empty;
        }
        let d = match &self.data {
            Some(d) => d,
            None => return Cell::Solid(1.0),
        };
        let name = d.block_name(state).unwrap_or("");
        let short = name.trim_start_matches("minecraft:");
        if short.contains("water") {
            return Cell::Water;
        }
        if short.contains("lava") || short == "fire" || short == "soul_fire" || short == "cactus" {
            return Cell::Deadly;
        }
        // Ce qu'un objet traverse : plantes, torches, rails, panneaux…
        const THROUGH: [&str; 20] = [
            "flower", "sapling", "torch", "button", "lever", "rail", "sign", "vine", "short_grass", "tall_grass", "fern", "dead_bush", "redstone_wire",
            "tripwire", "seagrass", "kelp", "pressure_plate", "web", "banner", "tulip",
        ];
        if THROUGH.iter().any(|w| short.contains(w))
            || matches!(short, "dandelion" | "poppy" | "allium" | "azure_bluet" | "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "wheat" | "carrots" | "potatoes" | "beetroot" | "brown_mushroom" | "red_mushroom")
        {
            return Cell::Empty;
        }
        if short.ends_with("_slab") && !short.contains("double") {
            let top = matches!(d.prop(state, "minecraft:vertical_half"), Some(crate::data::Prop::Str(s)) if s == "top");
            return Cell::Solid(if top { 1.0 } else { 0.5 });
        }
        if short.ends_with("carpet") {
            return Cell::Solid(0.0625);
        }
        if short == "snow_layer" {
            let h = match d.prop(state, "height") {
                Some(crate::data::Prop::Int(h)) => h as f32,
                _ => 0.0,
            };
            return Cell::Solid((h + 1.0) / 8.0);
        }
        Cell::Solid(1.0)
    }

    /// Fait tomber un objet. Faux si le jeu refuse les objets au sol (l'appelant le garde
    /// alors ailleurs). `delay` : ticks avant de pouvoir être ramassé.
    fn drop_item(&mut self, at: (f32, f32, f32), vel: (f32, f32, f32), item: i16, count: u8, delay: u64) -> bool {
        if self.drops_refused {
            return false;
        }
        if count == 0 {
            return true;
        }
        // Trop d'objets au sol : le plus ancien disparaît
        if self.ground.len() >= MAX_GROUND_ITEMS {
            if let Some(oldest) = self.ground.iter().min_by_key(|(_, g)| g.born).map(|(id, _)| *id) {
                self.remove_ground_item(oldest, None);
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        let block = self.block_for(item).unwrap_or(0);
        if self.cfg.debug {
            info!("[objets] objet {} lâché : objet n°{} ×{} en {:.1} {:.1} {:.1}", id, item, count, at.0, at.1, at.2);
        }
        let body = item_physics::Body { pos: at, vel, on_ground: false };
        self.ground.insert(id, GroundItem { item, block, count, body, resting: false, sent: at, born: self.tick, age: 0, pickup_at: self.tick + delay });
        self.show_ground_items();
        true
    }

    /// Petit élan au hasard (butin d'un bloc, inventaire lâché à la mort), comme dans le jeu.
    fn scatter(&self, salt: u64, strength: f32) -> (f32, f32, f32) {
        let r = |k: u64| -> f32 {
            let mut h = (self.next_id as u64 ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ k.wrapping_mul(0xc2b2_ae3d_27d4_eb4f)).wrapping_add(self.tick);
            h ^= h >> 31;
            h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
            h ^= h >> 29;
            (h % 10_000) as f32 / 10_000.0
        };
        ((r(1) - 0.5) * strength, 0.2, (r(2) - 0.5) * strength)
    }

    /// Retire un objet du sol ; `taker` : le joueur qui l'a ramassé (l'objet vole vers lui).
    fn remove_ground_item(&mut self, id: i64, taker: Option<i64>) {
        if self.ground.remove(&id).is_none() {
            return;
        }
        let viewers: Vec<SocketAddr> = self.clients.iter_mut().filter_map(|(a, c)| if c.items_seen.remove(&id) { Some(*a) } else { None }).collect();
        for v in viewers {
            if let Some(by) = taker {
                self.send(&v, bedrock::ID_TAKE_ITEM_ACTOR, &bedrock::take_item_actor(id, by));
            }
            self.send(&v, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(bedrock::item_unique_id(id)));
        }
    }

    /// Montre à chaque joueur les objets au sol proches de lui (64 blocs), retire les lointains.
    fn show_ground_items(&mut self) {
        let mut out: Vec<(SocketAddr, u32, Vec<u8>)> = Vec::new();
        for (addr, c) in self.clients.iter_mut() {
            if !c.joined || c.stage != Stage::InGame {
                continue;
            }
            for (id, g) in &self.ground {
                let (dx, dz) = (g.body.pos.0 - c.pos.0, g.body.pos.2 - c.pos.2);
                let far = dx * dx + dz * dz;
                if far < 64.0 * 64.0 {
                    if c.items_seen.insert(*id) {
                        out.push((*addr, bedrock::ID_ADD_ITEM_ACTOR, bedrock::add_item_actor(*id, g.item, g.count, g.block, g.body.pos, g.body.vel)));
                    }
                } else if far > 80.0 * 80.0 && c.items_seen.remove(id) {
                    out.push((*addr, bedrock::ID_REMOVE_ACTOR, bedrock::remove_actor(bedrock::item_unique_id(*id))));
                }
            }
        }
        for (addr, id, packet) in out {
            self.send(&addr, id, &packet);
        }
    }

    /// Physique des objets au sol (chaque tick) : ceux qui bougent sont recalculés et leur
    /// position envoyée ; ceux au repos sont vérifiés deux fois par seconde (bloc cassé en
    /// dessous, bloc posé dessus).
    fn move_ground_items(&mut self) {
        let recheck = self.tick % 10 == 0;
        let ids: Vec<i64> = self.ground.iter().filter(|(_, g)| !g.resting || recheck).map(|(id, _)| *id).collect();
        let mut destroyed = Vec::new();
        let mut moves: Vec<(i64, (f32, f32, f32), bool)> = Vec::new();
        for id in ids {
            let mut body = match self.ground.get(&id) {
                Some(g) => g.body,
                None => continue,
            };
            let outcome = item_physics::step(&mut body, &|x, y, z| self.cell(x, y, z));
            if let Some(g) = self.ground.get_mut(&id) {
                g.body = body;
                g.resting = outcome == Step::Resting;
                let (dx, dy, dz) = (body.pos.0 - g.sent.0, body.pos.1 - g.sent.1, body.pos.2 - g.sent.2);
                if dx * dx + dy * dy + dz * dz > 0.0001 {
                    g.sent = body.pos;
                    moves.push((id, body.pos, body.on_ground));
                }
            }
            if outcome == Step::Destroyed {
                destroyed.push(id);
            }
        }
        for id in destroyed {
            self.remove_ground_item(id, None);
        }
        for (id, pos, on_ground) in moves {
            let packet = bedrock::move_actor_absolute(id, pos, 0.0, 0.0, 0.0, on_ground);
            let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.items_seen.contains(&id)).map(|(a, _)| *a).collect();
            for v in viewers {
                self.send(&v, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
            }
        }
    }

    /// Objets au sol : vieillissement et disparition, fusion des tas identiques, ramassage.
    fn update_ground_items(&mut self) {
        if self.ground.is_empty() {
            return;
        }
        let tick = self.tick;
        // Vieillissement (toutes les secondes) : seulement près d'un joueur
        if tick % 20 == 0 {
            let players: Vec<(f32, f32)> = self.clients.values().filter(|c| c.joined).map(|c| (c.pos.0, c.pos.2)).collect();
            let mut expired = Vec::new();
            for (id, g) in self.ground.iter_mut() {
                if players.iter().any(|p| (p.0 - g.body.pos.0).powi(2) + (p.1 - g.body.pos.2).powi(2) < 128.0 * 128.0) {
                    g.age += 20;
                }
                if g.age > ITEM_LIFETIME {
                    expired.push(*id);
                }
            }
            for id in expired {
                self.remove_ground_item(id, None);
            }
        }
        // Fusion (quatre fois par seconde) : deux tas du même objet posés au sol, à moins d'un
        // bloc l'un de l'autre, ne font plus qu'un, dans la limite de 64
        if tick % 5 == 0 {
            self.merge_ground_items();
        }
        // Ramassage : objet dans la zone du joueur élargie d'un bloc (et d'un demi en hauteur)
        let mut taken: Vec<(SocketAddr, i64, i64)> = Vec::new();
        for (addr, c) in self.clients.iter() {
            // Un spectateur ne ramasse rien
            if !c.joined || c.stage != Stage::InGame || c.dead.is_some() || c.mode.is_spectator() {
                continue;
            }
            let feet = c.pos.1 - player_eye_height();
            for (id, g) in &self.ground {
                let (dx, dz) = (g.body.pos.0 - c.pos.0, g.body.pos.2 - c.pos.2);
                let y = g.body.pos.1;
                if tick >= g.pickup_at && dx.abs() <= 1.3 && dz.abs() <= 1.3 && y >= feet - 0.75 && y <= feet + 2.3 && !taken.iter().any(|t| t.1 == *id) {
                    taken.push((*addr, *id, c.id));
                }
            }
        }
        for (addr, id, by) in taken {
            let (item, count) = match self.ground.get(&id) {
                Some(g) => (g.item, g.count),
                None => continue,
            };
            if self.plugins.listens("EntityItemPickupEvent") {
                let out = self.fire("EntityItemPickupEvent", vec![("entity", self.name_json(&addr)), ("item", self.item_json(item, count))]);
                if cancelled(&out) {
                    continue;
                }
            }
            let left = match self.clients.get_mut(&addr) {
                Some(c) => {
                    c.inv_touched = true;
                    c.inv.add(item, count as u32)
                }
                None => continue,
            };
            if left as u8 == count {
                continue; // inventaire plein : l'objet reste au sol
            }
            self.send_inventory(&addr);
            self.show_held(&addr);
            if left == 0 {
                self.remove_ground_item(id, Some(by));
            } else {
                self.recount_ground_item(id, left as u8);
            }
        }
    }

    /// Change la quantité d'un tas au sol : le jeu ne sait pas la mettre à jour, l'objet est
    /// donc retiré puis réaffiché chez ceux qui le voient.
    fn recount_ground_item(&mut self, id: i64, count: u8) {
        match self.ground.get_mut(&id) {
            Some(g) => g.count = count,
            None => return,
        }
        // Comme le jeu officiel : l'objet reste en place, seule la taille de sa pile change
        // (un objet retiré puis recréé aussitôt clignoterait)
        let packet = bedrock::actor_event(id, bedrock::EVENT_ITEM_STACK_SIZE, count as i32);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.items_seen.contains(&id)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_ACTOR_EVENT, &packet);
        }
    }

    /// Fusionne les tas identiques et proches, posés au sol. Le plus ancien absorbe les autres.
    fn merge_ground_items(&mut self) {
        /// Écart maximal sur x et sur z entre deux tas qui fusionnent
        const MERGE_RADIUS: f32 = 1.0;
        // Rangés par bloc : seuls les tas d'un même bloc ou de blocs voisins sont comparés
        let mut cells: HashMap<(i32, i32, i32), Vec<i64>> = HashMap::new();
        for (id, g) in &self.ground {
            if g.body.on_ground && g.count < 64 {
                let p = g.body.pos;
                cells.entry((p.0.floor() as i32, p.1.floor() as i32, p.2.floor() as i32)).or_default().push(*id);
            }
        }
        let mut gone: HashSet<i64> = HashSet::new();
        let mut grown: Vec<(i64, u8)> = Vec::new();
        let mut ids: Vec<i64> = cells.values().flatten().copied().collect();
        ids.sort_unstable();
        for a in ids {
            if gone.contains(&a) {
                continue;
            }
            let (pa, item, mut count) = match self.ground.get(&a) {
                Some(g) => (g.body.pos, g.item, g.count),
                None => continue,
            };
            let base = (pa.0.floor() as i32, pa.1.floor() as i32, pa.2.floor() as i32);
            let mut absorbed = false;
            for (dx, dy, dz) in (-1..=1).flat_map(|dx| (-1..=1).flat_map(move |dy| (-1..=1).map(move |dz| (dx, dy, dz)))) {
                {
                    for b in cells.get(&(base.0 + dx, base.1 + dy, base.2 + dz)).into_iter().flatten() {
                        if *b <= a || gone.contains(b) {
                            continue;
                        }
                        let g = match self.ground.get(b) {
                            Some(g) => g,
                            None => continue,
                        };
                        let (ex, ey, ez) = (g.body.pos.0 - pa.0, g.body.pos.1 - pa.1, g.body.pos.2 - pa.2);
                        if g.item == item && ex.abs() <= MERGE_RADIUS && ez.abs() <= MERGE_RADIUS && ey.abs() <= 0.5 && count as u16 + g.count as u16 <= 64 {
                            count += g.count;
                            gone.insert(*b);
                            absorbed = true;
                        }
                    }
                }
            }
            if absorbed {
                grown.push((a, count));
            }
        }
        for id in gone {
            self.remove_ground_item(id, None);
        }
        for (id, count) in grown {
            self.recount_ground_item(id, count);
        }
    }

    /// Enregistre les objets au sol dans le dossier du monde (`items.json`), pour les retrouver
    /// après un redémarrage.
    fn ground_items_json(&self) -> Option<String> {
        let d = self.data.as_ref()?;
        let list: Vec<Json> = self
            .ground
            .values()
            .filter_map(|g| {
                let name = d.item_by_id(g.item)?.name.clone();
                let n = |v: f32| Json::Num((v as f64 * 1000.0).round() / 1000.0);
                Some(Json::Obj(vec![
                    ("item".into(), Json::Str(name)),
                    ("count".into(), Json::Num(g.count as f64)),
                    ("x".into(), n(g.body.pos.0)),
                    ("y".into(), n(g.body.pos.1)),
                    ("z".into(), n(g.body.pos.2)),
                    ("age".into(), Json::Num(g.age as f64)),
                ]))
            })
            .collect();
        Some(Json::Arr(list).to_json())
    }

    /// Recharge les objets au sol enregistrés.
    fn load_ground_items(&mut self) {
        let path = std::path::Path::new(&self.cfg.world_dir).join("items.json");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return,
        };
        let list = match crate::json::parse(&text) {
            Ok(Json::Arr(list)) => list,
            _ => {
                warn!("{} illisible : objets au sol non rechargés", path.display());
                return;
            }
        };
        let mut loaded = 0;
        for v in list.iter().take(MAX_GROUND_ITEMS) {
            let item = match v.get("item").and_then(|i| i.as_str()).and_then(|n| self.item_id(n)) {
                Some(i) => i,
                None => continue,
            };
            let num = |k: &str| v.get(k).and_then(|n| n.as_f64()).filter(|n| n.abs() < 3.0e7).map(|n| n as f32);
            let (x, y, z) = match (num("x"), num("y"), num("z")) {
                (Some(x), Some(y), Some(z)) => (x, y, z),
                _ => continue,
            };
            let count = num("count").unwrap_or(1.0).clamp(1.0, 64.0) as u8;
            let id = self.next_id;
            self.next_id += 1;
            let block = self.block_for(item).unwrap_or(0);
            let body = item_physics::Body { pos: (x, y, z), vel: (0.0, 0.0, 0.0), on_ground: true };
            let age = num("age").unwrap_or(0.0).max(0.0) as u64;
            self.ground.insert(id, GroundItem { item, block, count, body, resting: false, sent: (x, y, z), born: self.tick, age, pickup_at: 0 });
            loaded += 1;
        }
        if loaded > 0 {
            info!("Objets au sol rechargés : {}", loaded);
        }
    }

    /// Un joueur casse (ou tente de poser) un bloc.
    fn on_block_use(&mut self, addr: &SocketAddr, block_use: bedrock::BlockUse) {
        let (x, y, z) = block_use.pos;
        let near = match self.clients.get(addr) {
            // Aventure et spectateur ne cassent rien
            Some(c) if c.stage == Stage::InGame && c.dead.is_none() && c.mode.can_build() => {
                let (dx, dy, dz) = (c.pos.0 - x as f32 - 0.5, c.pos.1 - y as f32 - 0.5, c.pos.2 - z as f32 - 0.5);
                dx * dx + dy * dy + dz * dz <= 8.0 * 8.0
            }
            _ => return,
        };
        if !near || !crate::world::World::in_bounds(y) {
            return; // trop loin ou hors du monde : ignoré
        }
        if block_use.action != 2 {
            return;
        }
        let air = self.world.air;
        let old = self.world.get(x, y, z);
        if old == air {
            return;
        }
        // Les plugins peuvent s'y opposer (terrain protégé, baguette d'EasyEdit…)
        if !self.block_event("block_break", addr, (x, y, z)) {
            self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, old));
            return;
        }
        let old = self.world.get(x, y, z);
        if old == air {
            return;
        }
        // En survie, les règles du bloc s'appliquent : incassable ou non, butin selon l'outil
        let survival = self.clients.get(addr).map(|c| c.mode == GameMode::Survival).unwrap_or(false);
        let loot: Option<Vec<(i16, u8)>> = match (&self.data, &self.blocks, self.items_level, survival) {
            (Some(d), Some(blocks), 2, true) => d.block_name(old).and_then(|name| blocks.get(name).map(|b| (name, b))).map(|(name, block)| {
                if !block.is_breakable() {
                    return None;
                }
                let held = self.clients.get(addr).map(|c| c.inv.held()).filter(|s| !s.is_empty());
                let tool = match held.and_then(|s| d.item_by_id(s.id)) {
                    Some(def) => VanillaItems::get(&def.name),
                    None => VanillaItems::hand(),
                };
                let found = block
                    .drops(tool.as_ref())
                    .iter()
                    .filter_map(|drop| {
                        // L'objet du même nom d'abord ; à défaut, celui que les données associent au bloc
                        let id = d.item(&drop.item).map(|i| i.id).or_else(|| if drop.item == block.name() { d.item_for_block_name(name) } else { None });
                        id.map(|id| (id, drop.count))
                    })
                    .collect();
                Some(found)
            }),
            _ => None,
        }
        .unwrap_or_else(|| Some(if survival { self.drop_for(old).map(|id| vec![(id, 1)]).unwrap_or_default() } else { Vec::new() }));
        let loot = match loot {
            Some(l) => l,
            None => {
                // Bloc incassable : le jeu l'a déjà retiré de son côté, on le lui rend
                self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, old));
                return;
            }
        };
        // Écouteurs : annulation, butin modifié
        let mut loot = loot;
        if self.plugins.listens("BlockBreakEvent") {
            let drops = Json::Arr(loot.iter().map(|(id, n)| self.item_json(*id, *n)).collect());
            let fields = vec![("player", self.name_json(addr)), ("block", self.block_json(old, (x, y, z))), ("drops", drops)];
            let out = self.fire("BlockBreakEvent", fields);
            if cancelled(&out) {
                self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, old));
                return;
            }
            if let Some(list) = out.as_ref().and_then(|v| v.get("drops")).and_then(|d| d.as_arr()) {
                loot = list
                    .iter()
                    .take(64)
                    .filter_map(|d| {
                        let id = self.item_id(d.get("name")?.as_str()?)?;
                        let n = d.get("count").and_then(|c| c.as_f64()).unwrap_or(1.0).clamp(1.0, 64.0) as u8;
                        Some((id, n))
                    })
                    .collect();
            }
        }
        // Coffre de structure jamais ouvert : son butin tombe quand on le casse
        let unopened_loot = if self.world.tiles.get((x, y, z)).is_none() { self.world.structure_chest((x, y, z)) } else { None };
        // Une plante aquatique (ou un corail immergé) laisse son eau derrière elle
        let after = self.world.left_behind(x, y, z);
        if !self.world.set(x, y, z, after) {
            return;
        }
        if self.cfg.debug {
            info!("{} : bloc cassé en {} {} {}", addr, x, y, z);
        }
        self.liquid_neighbours((x, y, z));
        self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, after), Some(*addr));
        if after != air {
            // L'eau du bloc revient sur la couche principale : la seconde couche se vide
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block_layer(x, y, z, air, 1), None);
        }
        self.settle_stairs((x, y, z));
        self.settle_links((x, y, z));
        // Une porte part en entier : l'autre moitié disparaît avec elle
        let other_half = self.data.as_ref().and_then(|d| {
            let p = match d.prop(old, "upper_block_bit")? {
                Prop::Int(1) => (x, y - 1, z),
                _ => (x, y + 1, z),
            };
            if d.block_name(self.world.get(p.0, p.1, p.2)) == d.block_name(old) {
                Some(p)
            } else {
                None
            }
        });
        if let Some(p) = other_half {
            self.change_block(p, air);
        }
        // Un coffre ou un four cassé rend ce qu'il contenait
        let mut loot = loot;
        if let Some(kind) = unopened_loot {
            let seed = self.world.generator().seed;
            for (_, name, count) in crate::world::structures::loot(kind, seed, (x, y, z)) {
                if let Some(id) = self.item_id(name) {
                    loot.push((id, count));
                }
            }
        }
        if let Some(tile) = self.world.tiles.remove((x, y, z)) {
            if let Some(inv) = tile.inventory() {
                loot.extend(inv.contents().into_iter().map(|(_, item)| (item.id, item.count)));
            }
        }
        // Le butin saute du centre du bloc et retombe (ou va dans l'inventaire si le jeu refuse
        // les objets au sol)
        for (n, (id, count)) in loot.into_iter().enumerate() {
            let vel = self.scatter(n as u64, 0.2);
            if !self.drop_item((x as f32 + 0.5, y as f32 + 0.25, z as f32 + 0.5), vel, id, count, 10) {
                self.add_item(addr, id, count as u16);
            }
        }
    }

    /// Bloc posé par un objet : d'après les données du jeu si elles sont chargées et
    /// annoncées au jeu, sinon d'après la petite liste connue par cœur.
    fn block_for(&self, id: i16) -> Option<u32> {
        match (&self.data, self.items_level) {
            (Some(d), 2) => d.block_for_item(id),
            _ => bedrock::block_for_item(id),
        }
    }

    /// Objet obtenu en cassant un bloc.
    fn drop_for(&self, block: u32) -> Option<i16> {
        match (&self.data, self.items_level) {
            (Some(d), 2) => d.drop_for_block(block),
            _ => bedrock::drop_for_block(block),
        }
    }

    /// Identifiant d'un objet d'après son nom, parmi ceux que le jeu connaît.
    fn item_id(&self, name: &str) -> Option<i16> {
        let short = name.trim_start_matches("minecraft:");
        match (&self.data, self.items_level) {
            (Some(d), 2) => d.item(short).map(|i| i.id),
            (_, 0) => None,
            _ => bedrock::GIVABLE_BLOCKS.iter().find(|(n, _)| *n == short).map(|(_, id)| *id),
        }
    }

    /// Ajoute des objets à l'inventaire d'un joueur et lui renvoie son inventaire.
    /// Faux si tout n'a pas pu être rangé.
    fn add_item(&mut self, addr: &SocketAddr, id: i16, count: u16) -> bool {
        let left = match self.clients.get_mut(addr) {
            Some(c) => {
                c.inv_touched = true;
                c.inv.add(id, count as u32)
            }
            None => return false,
        };
        self.send_inventory(addr);
        left == 0
    }

    /// Envoie au joueur le contenu de son sac et de sa barre d'objets, de son armure et de sa
    /// seconde main.
    fn send_inventory(&mut self, addr: &SocketAddr) {
        self.send_inventory_with(addr, false);
    }

    /// Idem ; `worn` : l'armure et la seconde main sont envoyées même vides (après les avoir vidées).
    fn send_inventory_with(&mut self, addr: &SocketAddr, worn: bool) {
        let (windows, protocol) = match self.clients.get(addr) {
            Some(c) if c.protocol >= 2225 => {
                let of = |slots: &[inventory::ItemStack]| -> Vec<(i16, u8, i32)> { slots.iter().map(|s| (s.id, s.count, s.net_id)).collect() };
                let mut windows = vec![(0u32, of(&c.inv.main.slots))];
                // Armure et seconde main : envoyées seulement si elles portent quelque chose
                if worn || c.inv.armor.slots.iter().any(|s| !s.is_empty()) {
                    windows.push((120, of(&c.inv.armor.slots)));
                }
                if worn || c.inv.offhand.slots.iter().any(|s| !s.is_empty()) {
                    windows.push((119, of(&c.inv.offhand.slots)));
                }
                (windows, c.protocol)
            }
            _ => return,
        };
        for (window, slots) in windows {
            let slots: Vec<(i16, u8, i32, u32)> =
                slots.iter().map(|(id, count, net)| (*id, *count, *net, self.block_for(*id).unwrap_or(0))).collect();
            let pk = bedrock::container_items(window, &slots, self.item_net_ids, crate::versions::features(protocol));
            self.send(addr, bedrock::ID_INVENTORY_CONTENT, &pk);
        }
    }

    /// Le joueur ouvre son inventaire : le serveur confirme l'ouverture de l'écran.
    fn on_open_inventory(&mut self, addr: &SocketAddr) {
        let id = match self.clients.get_mut(addr) {
            Some(c) if c.protocol >= 2225 && c.inv.open_window == 0 => {
                c.inv.open_window = 1;
                c.id
            }
            _ => return,
        };
        self.send(addr, bedrock::ID_CONTAINER_OPEN, &bedrock::container_open_inventory(1, id));
    }

    /// Le joueur ferme un écran de conteneur : le serveur confirme. Un objet resté au bout
    /// du curseur retourne dans l'inventaire.
    fn on_close_container(&mut self, addr: &SocketAddr, body: &[u8]) {
        let (window, kind) = match bedrock::read_container_close(body) {
            Some(v) => v,
            None => return,
        };
        let mut resend = false;
        if let Some(c) = self.clients.get_mut(addr) {
            c.inv.open_window = 0;
            c.inv.block_pos = None;
            c.inv.block.slots.clear();
            c.inv.block_is_furnace = false;
            let held = c.inv.cursor.slots[0];
            if !held.is_empty() {
                c.inv.cursor.slots[0] = inventory::ItemStack::EMPTY;
                c.inv.add(held.id, held.count as u32);
                resend = true;
            }
            // Ce qui reste dans la grille de fabrication retourne dans le sac
            for i in 0..c.inv.craft.slots.len() {
                let left = c.inv.craft.slots[i];
                if !left.is_empty() {
                    c.inv.craft.slots[i] = inventory::ItemStack::EMPTY;
                    c.inv.add(left.id, left.count as u32);
                    resend = true;
                }
            }
        }
        self.send(addr, bedrock::ID_CONTAINER_CLOSE, &bedrock::container_close(window, kind, false));
        if resend {
            self.send_inventory(addr);
        }
    }

    /// Demandes de déplacement d'objets : chacune est vérifiée et appliquée en entier, ou refusée.
    fn on_stack_request(&mut self, addr: &SocketAddr, body: &[u8]) {
        let requests = match bedrock::read_stack_requests(body) {
            Some(r) => r,
            None => {
                if self.cfg.debug {
                    let hex: String = body.iter().take(200).map(|b| format!("{:02x}", b)).collect();
                    info!("{} : demande d'inventaire illisible ({} octets) {}", addr, body.len(), hex);
                }
                // Le jeu a déjà appliqué sa demande de son côté : on lui renvoie l'état réel
                self.send_inventory(addr);
                if self.load_block_view(addr) {
                    let view = self.clients.get(addr).filter(|c| c.inv.block_pos.is_some()).map(|c| c.inv.block.slots.clone());
                    if let Some(slots) = view {
                        self.send_block_content(addr, &slots);
                    }
                }
                return;
            }
        };
        if self.cfg.debug {
            let hex: String = body.iter().take(200).map(|b| format!("{:02x}", b)).collect();
            info!("{} : demande d'inventaire {:?} — {}", addr, requests, hex);
        }
        for request in requests {
            // Menu créatif : l'objet demandé doit exister, et le serveur être en mode créatif
            let creative = self.clients.get(addr).map(|c| c.mode == GameMode::Creative).unwrap_or(false);
            let from_menu = request.actions.iter().any(|a| matches!(a, bedrock::StackAction::CraftCreative { .. }));
            let created = match &request.created {
                Some(name) if creative || !from_menu => self.item_id(name),
                _ => None,
            };
            // Le bloc ouvert (four qui cuit…) peut avoir changé depuis la dernière demande
            if !self.load_block_view(addr) {
                return;
            }
            // Fabrication : le résultat annoncé doit venir d'une vraie recette qui correspond à la
            // grille ; le serveur en déduit ce que chaque case doit perdre
            let crafting = request.actions.iter().any(|a| matches!(a, bedrock::StackAction::CraftRecipe { .. }));
            let craft = if crafting { self.check_craft(addr, &request) } else { None };
            if crafting && craft.is_none() && self.cfg.debug {
                info!("{} : fabrication refusée ({:?} ×{} ne correspond à aucune recette pour cette grille)", addr, request.created, request.created_count);
            }
            let mut dropped: Vec<(i16, u8)> = Vec::new();
            let outcome = match self.clients.get_mut(addr) {
                Some(c) if c.stage == Stage::InGame => Self::run_stack_request(&mut c.inv, &request, created, craft.as_ref(), &mut dropped),
                _ => return,
            };
            if !dropped.is_empty() {
                // Lancé depuis les yeux du joueur, dans la direction de son regard
                let at = self.throw_from(addr);
                if let Some((at, vel)) = at {
                    for (id, count) in dropped {
                        // Écouteurs : un objet dont le lâcher est annulé revient dans l'inventaire
                        let out = self.fire("PlayerDropItemEvent", vec![("player", self.name_json(addr)), ("item", self.item_json(id, count))]);
                        if cancelled(&out) {
                            self.add_item(addr, id, count as u16);
                            continue;
                        }
                        // Jeu qui refuse les objets au sol : l'objet est perdu, comme avant
                        self.drop_item(at, vel, id, count, 40);
                    }
                }
            }
            if outcome.is_some() {
                self.store_block_view(addr);
                // L'objet en main a pu changer (pile déplacée dans la barre)
                self.show_held(addr);
            }
            let response = match &outcome {
                Some(changed) => bedrock::item_stack_response(request.id, Some(changed)),
                None => bedrock::item_stack_response(request.id, None),
            };
            self.send(addr, bedrock::ID_ITEM_STACK_RESPONSE, &response);
            if outcome.is_none() {
                // Refus : on renvoie l'état exact pour que le jeu et le serveur restent d'accord
                self.send_inventory(addr);
                let view = self.clients.get(addr).filter(|c| c.inv.block_pos.is_some()).map(|c| c.inv.block.slots.clone());
                if let Some(slots) = view {
                    self.send_block_content(addr, &slots);
                }
            }
        }
    }

    /// Point de départ et élan d'un objet lancé par un joueur : juste devant ses yeux, un peu
    /// plus bas, dans la direction du regard. Si ce point est dans un bloc (joueur collé à un
    /// mur), l'objet part de sa tête.
    fn throw_from(&self, addr: &SocketAddr) -> Option<((f32, f32, f32), (f32, f32, f32))> {
        let c = self.clients.get(addr)?;
        let (y, p) = (c.yaw.to_radians(), c.pitch.to_radians());
        let look = (-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos());
        let ahead = (c.pos.0 + look.0 * 0.4, c.pos.1 - 0.3 + look.1 * 0.4, c.pos.2 + look.2 * 0.4);
        let free = |q: (f32, f32, f32)| matches!(self.cell(q.0.floor() as i32, q.1.floor() as i32, q.2.floor() as i32), item_physics::Cell::Empty | item_physics::Cell::Water);
        let at = if free(ahead) { ahead } else { (c.pos.0, c.pos.1 - 0.3, c.pos.2) };
        if self.cfg.debug {
            info!("[objets] lancer : yeux {:.2} {:.2} {:.2}, regard {:.0}° / {:.0}°, départ {:.2} {:.2} {:.2}", c.pos.0, c.pos.1, c.pos.2, c.yaw, c.pitch, at.0, at.1, at.2);
        }
        Some((at, item_physics::throw_velocity(c.yaw, c.pitch)))
    }

    /// Touche « lâcher » (Q) : l'objet doit être dans la case indiquée (ou en main), en
    /// quantité suffisante ; sinon le joueur retrouve l'inventaire exact du serveur.
    fn on_key_drop(&mut self, addr: &SocketAddr, d: bedrock::WorldDrop) {
        let debug = self.cfg.debug;
        let taken = match self.clients.get_mut(addr) {
            Some(c) if c.stage == Stage::InGame && c.dead.is_none() && !c.mode.is_spectator() => {
                // Q jette l'objet en main : la case tenue d'abord, puis celle lue dans le paquet
                // (sa place exacte dans le paquet n'est pas certaine), puis toute case qui porte
                // cet objet en quantité suffisante
                let fits = |s: usize| c.inv.get((Target::Player, s)).map(|st| !st.is_empty() && st.id == d.id && d.count >= 1 && d.count <= st.count as u16).unwrap_or(false);
                let held = c.inv.selected as usize;
                let slot = [Some(held), d.slot].into_iter().flatten().find(|s| fits(*s)).or_else(|| (0..36).find(|s| fits(*s))).unwrap_or(held);
                if debug {
                    info!("[objets] Q : objet n°{} ×{}, case lue {:?}, case tenue {}, case retenue {}", d.id, d.count, d.slot, held, slot);
                }
                match c.inv.get((Target::Player, slot)) {
                    Some(stack) if !stack.is_empty() && stack.id == d.id && d.count >= 1 && d.count as u8 <= stack.count && d.count <= 255 => {
                        let ok = c.inv.apply(inventory::Action::Remove { count: d.count as u8, from: (Target::Player, slot) }).is_some();
                        if ok {
                            c.inv_touched = true;
                        }
                        ok
                    }
                    _ => false,
                }
            }
            _ => return,
        };
        if !taken {
            self.send_inventory(addr);
            return;
        }
        // Le jeu a déjà retiré l'objet de son côté ; on renvoie l'état du serveur (même
        // contenu, identifiants de piles à jour)
        self.send_inventory(addr);
        self.show_held(addr);
        let out = self.fire("PlayerDropItemEvent", vec![("player", self.name_json(addr)), ("item", self.item_json(d.id, d.count as u8))]);
        if cancelled(&out) {
            self.add_item(addr, d.id, d.count);
            return;
        }
        let at = self.throw_from(addr);
        if let Some((at, vel)) = at {
            self.drop_item(at, vel, d.id, d.count as u8, 40);
        }
    }

    /// Applique une demande sur une copie de l'inventaire ; rien n'est gardé si une action échoue.
    /// Recette correspondant à la grille de fabrication du joueur et au résultat annoncé.
    fn check_craft(&self, addr: &SocketAddr, request: &bedrock::StackRequest) -> Option<crate::data::Craft> {
        let data = self.data.as_ref()?;
        let output = request.created.as_deref()?;
        let c = self.clients.get(addr)?;
        let mut grid: [Option<(String, u8)>; 13] = Default::default();
        for (i, cell) in grid.iter_mut().enumerate() {
            let stack = c.inv.craft.slots.get(28 + i).copied().unwrap_or(inventory::ItemStack::EMPTY);
            if !stack.is_empty() {
                *cell = Some((data.item_by_id(stack.id)?.name.clone(), stack.count));
            }
        }
        // Le jeu annonce la quantité d'une fabrication, et le nombre de fabrications à part
        data.recipes.check(&grid, output, request.created_count.saturating_mul(Self::craft_times(request)))
    }

    /// Nombre de fabrications d'affilée demandées (fabriquer tout d'un coup), 1 au moins.
    fn craft_times(request: &bedrock::StackRequest) -> u16 {
        request
            .actions
            .iter()
            .find_map(|a| match a {
                bedrock::StackAction::CraftRecipe { times, .. } => Some((*times).max(1) as u16),
                _ => None,
            })
            .unwrap_or(1)
    }

    /// Applique une demande d'inventaire sur une copie ; le joueur n'est modifié que si
    /// tout est valide.
    fn run_stack_request(inv: &mut PlayerWindows, request: &bedrock::StackRequest, created: Option<i16>, craft: Option<&crate::data::Craft>, dropped: &mut Vec<(i16, u8)>) -> Option<Vec<(u8, u8, u8, i32)>> {
        let mut work = inv.clone();
        let mut thrown: Vec<(i16, u8)> = Vec::new();
        let mut touched: Vec<(Target, usize)> = Vec::new();
        let at = |r: &bedrock::SlotRef| inventory::resolve(r.container, r.slot);
        for action in &request.actions {
            let step = match action {
                bedrock::StackAction::Take { count, from, to } | bedrock::StackAction::Place { count, from, to } => {
                    inventory::Action::Move { count: *count, from: at(from)?, to: at(to)? }
                }
                bedrock::StackAction::Swap { a, b } => inventory::Action::Swap { a: at(a)?, b: at(b)? },
                // Objets jetés : ils tomberont au sol devant le joueur
                bedrock::StackAction::Drop { count, from } => {
                    let slot = at(from)?;
                    let stack = work.get(slot)?;
                    if !stack.is_empty() {
                        thrown.push((stack.id, (*count).min(stack.count)));
                    }
                    inventory::Action::Remove { count: *count, from: slot }
                }
                bedrock::StackAction::Destroy { count, from } => inventory::Action::Remove { count: *count, from: at(from)? },
                bedrock::StackAction::CraftCreative { .. } => {
                    work.create(created?, inventory::MAX_STACK);
                    continue;
                }
                bedrock::StackAction::CraftRecipe { .. } => {
                    // Vérifiée avant (`check_craft`) : sans recette correspondante, refus
                    craft?;
                    let total = (request.created_count as u32 * Self::craft_times(request) as u32).clamp(1, 255) as u8;
                    work.create(created?, total);
                    continue;
                }
                bedrock::StackAction::Results => continue,
                bedrock::StackAction::Unsupported(_) => return None,
            };
            for slot in work.apply(step)? {
                if !touched.contains(&slot) {
                    touched.push(slot);
                }
            }
        }
        // Après une fabrication, chaque case de la grille doit avoir perdu exactement ce que la
        // recette consomme : ni plus (objets détruits pour rien), ni moins (fabrication gratuite)
        if let Some(craft) = craft {
            for slot in 28..=40usize {
                let before = inv.craft.slots.get(slot).copied().unwrap_or(inventory::ItemStack::EMPTY);
                let after = work.craft.slots.get(slot).copied().unwrap_or(inventory::ItemStack::EMPTY);
                let lose = craft.consumed.iter().find(|(s, _)| *s == slot).map(|(_, n)| *n).unwrap_or(0);
                let expected = before.count.saturating_sub(lose);
                if after.count != expected || (expected > 0 && after.id != before.id) {
                    return None;
                }
            }
        }
        work.clear_created();
        let changed = touched
            .iter()
            .filter(|(target, _)| *target != Target::Created)
            .map(|(target, slot)| {
                let stack = work.get((*target, *slot)).unwrap_or(inventory::ItemStack::EMPTY);
                let name = match (*target, work.block_is_furnace) {
                    // four : ingrédient, combustible, résultat
                    (Target::Block, true) => [25u8, 24, 26][(*slot).min(2)],
                    _ => inventory::protocol_name(*target, *slot),
                };
                (name, *slot as u8, stack.count, stack.net_id)
            })
            .collect();
        *inv = work;
        dropped.extend(thrown);
        Some(changed)
    }

    /// Le joueur pose le bloc qu'il tient en main.
    /// Règles du bloc désigné par son identifiant réseau.
    fn block_at_state(&self, state: u32) -> Option<&dyn Block> {
        let name = self.data.as_ref()?.block_name(state)?;
        self.blocks.as_ref()?.get(name)
    }

    /// Un joueur clique sur un bloc. Renvoie vrai si le bloc réagit au clic : dans ce cas,
    /// l'objet tenu en main n'est pas posé.
    fn on_interact(&mut self, addr: &SocketAddr, pos: (i32, i32, i32)) -> bool {
        let state = self.world.get(pos.0, pos.1, pos.2);
        let (name, action, kind) = match self.block_at_state(state) {
            Some(b) if b.interaction() != Interaction::None => (b.short_name().to_string(), b.interaction(), b.tile()),
            _ => return false,
        };
        // Un bloc posé avant cette version n'a pas encore ses données : on les crée au premier clic
        if let Some(kind) = kind {
            if self.world.tiles.get(pos).is_none() {
                if let Some(mut new) = tile::create(kind) {
                    // Coffre d'un village, d'un temple ou d'une mine : son butin, tiré une fois
                    if let Some(loot) = self.world.structure_chest(pos) {
                        self.fill_loot(new.as_mut(), loot, pos);
                    }
                    self.world.tiles.insert(pos, new);
                }
            }
        }
        match action {
            Interaction::Open(container) => self.open_container(addr, pos, container, &name),
            Interaction::Toggle => self.toggle_block(addr, pos),
            other => {
                if self.cfg.debug {
                    info!("{} : clic sur {} en {} {} {} — {:?} (pas encore géré)", addr, name, pos.0, pos.1, pos.2, other);
                }
            }
        }
        true
    }

    /// Remplit un coffre de structure avec son butin (le même pour une graine et une place
    /// données). Les objets inconnus de cette version du jeu sont ignorés.
    fn fill_loot(&mut self, tile: &mut dyn tile::Tile, loot: crate::world::structures::Loot, pos: (i32, i32, i32)) {
        let seed = self.world.generator().seed;
        let items: Vec<(usize, i16, u8)> = crate::world::structures::loot(loot, seed, pos).into_iter().filter_map(|(slot, name, count)| Some((slot, self.item_id(name)?, count))).collect();
        if let Some(inv) = tile.inventory_mut() {
            for (slot, id, count) in items {
                self.tile_net_id = if self.tile_net_id >= i32::MAX - 1 { 1_000_000_000 } else { self.tile_net_id + 1 };
                inv.set_item(slot, crate::item::ItemStack { id, count, net_id: self.tile_net_id });
            }
        }
        if self.cfg.debug {
            info!("Coffre de structure ({:?}) ouvert pour la première fois en {} {} {}", loot, pos.0, pos.1, pos.2);
        }
    }

    /// Ouvre chez le joueur l'écran d'un bloc : coffre, tonneau, four, établi…
    fn open_container(&mut self, addr: &SocketAddr, pos: (i32, i32, i32), container: crate::block::ContainerType, name: &str) {
        use crate::block::ContainerType as C;
        // Type d'écran du jeu ; None : écran pas encore géré par le serveur
        let (screen, furnace) = match container {
            C::Chest | C::Barrel | C::ShulkerBox => (Some(0u8), false),
            C::CraftingTable => (Some(1), false),
            C::Furnace => (Some(2), true),
            C::BlastFurnace => (Some(27), true),
            C::Smoker => (Some(28), true),
            C::Dispenser => (Some(6), false),
            C::Dropper => (Some(7), false),
            C::Hopper => (Some(8), false),
            _ => (None, false),
        };
        let screen = match screen {
            Some(s) => s,
            None => {
                if self.cfg.debug {
                    info!("{} : clic sur {} — cet écran n'est pas encore géré", addr, name);
                }
                return;
            }
        };
        match self.clients.get(addr) {
            // Le jeu répète le clic tant que le bouton est tenu : un seul écran à la fois
            Some(c) if c.stage == Stage::InGame && c.inv.open_window == 0 => {}
            _ => return,
        }
        // Un seul joueur à la fois dans un même bloc, pour que personne ne voie un contenu périmé
        let has_inventory = container != C::CraftingTable;
        if has_inventory && self.clients.values().any(|c| c.inv.block_pos == Some(pos)) {
            self.send(addr, bedrock::ID_TEXT, &bedrock::text_raw("§7Quelqu'un d'autre utilise déjà ce bloc."));
            return;
        }
        let mut slots: Vec<inventory::ItemStack> = Vec::new();
        if has_inventory {
            // Numéros de pile propres au serveur : aucun risque de doublon avec ceux du joueur
            let next = &mut self.tile_net_id;
            match self.world.tiles.get_mut(pos).and_then(|t| t.inventory_mut()) {
                Some(inv) => {
                    for i in 0..inv.slots().len() {
                        let mut s = inv.slots()[i];
                        if !s.is_empty() {
                            *next = if *next >= i32::MAX - 1 { 1_000_000_000 } else { *next + 1 };
                            s.net_id = *next;
                            inv.set_item(i, s);
                        }
                        slots.push(s);
                    }
                }
                None => return,
            }
        }
        const WINDOW: u8 = 2;
        if let Some(c) = self.clients.get_mut(addr) {
            c.inv.open_window = WINDOW;
            c.inv.block_is_furnace = furnace;
            if has_inventory {
                c.inv.block.slots = slots.clone();
                c.inv.block_pos = Some(pos);
            }
        }
        if self.cfg.debug {
            info!("{} : ouverture de {} en {} {} {} (écran {}, {} emplacements)", addr, name, pos.0, pos.1, pos.2, screen, slots.len());
        }
        self.send(addr, bedrock::ID_CONTAINER_OPEN, &bedrock::container_open_block(WINDOW, screen, pos));
        if has_inventory {
            self.send_block_content(addr, &slots);
        }
    }

    /// Envoie au joueur le contenu du bloc qu'il a ouvert.
    fn send_block_content(&mut self, addr: &SocketAddr, slots: &[inventory::ItemStack]) {
        let (protocol, window) = match self.clients.get(addr) {
            Some(c) => (c.protocol, c.inv.open_window),
            None => return,
        };
        let list: Vec<(i16, u8, i32, u32)> = slots.iter().map(|s| (s.id, s.count, s.net_id, self.block_for(s.id).unwrap_or(0))).collect();
        let pk = bedrock::container_items(window as u32, &list, self.item_net_ids, crate::versions::features(protocol));
        self.send(addr, bedrock::ID_INVENTORY_CONTENT, &pk);
    }

    /// Recopie le contenu réel du bloc ouvert dans la vue du joueur. Faux si le bloc a
    /// disparu : l'écran est alors fermé.
    fn load_block_view(&mut self, addr: &SocketAddr) -> bool {
        let pos = match self.clients.get(addr).and_then(|c| c.inv.block_pos) {
            Some(p) => p,
            None => return true,
        };
        let slots: Option<Vec<inventory::ItemStack>> = self.world.tiles.get(pos).and_then(|t| t.inventory()).map(|i| i.slots().to_vec());
        match (slots, self.clients.get_mut(addr)) {
            (Some(slots), Some(c)) => {
                c.inv.block.slots = slots;
                true
            }
            _ => {
                self.force_close(addr);
                false
            }
        }
    }

    /// Recopie la vue du joueur dans le bloc, après une demande acceptée.
    fn store_block_view(&mut self, addr: &SocketAddr) {
        let (pos, slots) = match self.clients.get(addr) {
            Some(c) => match c.inv.block_pos {
                Some(p) => (p, c.inv.block.slots.clone()),
                None => return,
            },
            None => return,
        };
        if let Some(inv) = self.world.tiles.get_mut(pos).and_then(|t| t.inventory_mut()) {
            for (i, s) in slots.iter().enumerate() {
                if inv.slots().get(i) != Some(s) {
                    inv.set_item(i, *s);
                }
            }
        }
    }

    /// Ferme d'autorité l'écran ouvert chez un joueur (bloc cassé, joueur trop loin).
    fn force_close(&mut self, addr: &SocketAddr) {
        let window = match self.clients.get_mut(addr) {
            Some(c) if c.inv.open_window != 0 => {
                let w = c.inv.open_window;
                c.inv.block_pos = None;
                c.inv.block.slots.clear();
                w
            }
            _ => return,
        };
        self.send(addr, bedrock::ID_CONTAINER_CLOSE, &bedrock::container_close(window, 0, true));
    }

    /// Tient à jour les écrans ouverts : contenu du four qui change, jauges, joueur parti
    /// trop loin ou bloc disparu.
    fn watch_containers(&mut self) {
        let viewers: Vec<(SocketAddr, (i32, i32, i32), (f32, f32, f32), Vec<inventory::ItemStack>, bool, u8)> = self
            .clients
            .iter()
            .filter_map(|(a, c)| c.inv.block_pos.map(|p| (*a, p, c.pos, c.inv.block.slots.clone(), c.inv.block_is_furnace, c.inv.open_window)))
            .collect();
        for (addr, pos, at, seen, furnace, window) in viewers {
            let (dx, dy, dz) = (at.0 - pos.0 as f32 - 0.5, at.1 - pos.1 as f32 - 0.5, at.2 - pos.2 as f32 - 0.5);
            let state = self.world.tiles.get(pos).and_then(|t| t.inventory().map(|i| (i.slots().to_vec(), t.gauges())));
            let (now, gauges) = match state {
                Some(s) if dx * dx + dy * dy + dz * dz <= 10.0 * 10.0 => s,
                _ => {
                    self.force_close(&addr);
                    continue;
                }
            };
            if now != seen {
                if let Some(c) = self.clients.get_mut(&addr) {
                    c.inv.block.slots = now.clone();
                }
                self.send_block_content(&addr, &now);
            }
            if let (true, Some((cook, _total, burn, burn_total))) = (furnace, gauges) {
                // Propriétés de l'écran du four : 0 cuisson, 1 feu restant, 2 durée du feu
                if self.tick % 10 == 0 {
                    for (property, value) in [(0, cook), (1, burn), (2, burn_total)] {
                        self.send(&addr, bedrock::ID_CONTAINER_SET_DATA, &bedrock::container_set_data(window, property, value as i32));
                    }
                }
            }
        }
    }

    /// Clic droit sur un bloc : ouvrir, utiliser un objet ou poser un bloc.
    fn on_place(&mut self, addr: &SocketAddr, target: (i32, i32, i32), face: u8, click: (f32, f32, f32)) {
        let (x, y, z) = target;
        let (near, item, looks, yaw) = match self.clients.get(addr) {
            // Aventure et spectateur ne posent rien
            Some(c) if c.stage == Stage::InGame && c.dead.is_none() && c.mode.can_build() => {
                let (dx, dy, dz) = (c.pos.0 - x as f32 - 0.5, c.pos.1 - y as f32 - 0.5, c.pos.2 - z as f32 - 0.5);
                // Sens du regard : l'angle annoncé par le jeu, comme en vanilla. C'est ce qui permet
                // de poser un escalier de biais par rapport au bloc visé, pour former un coin.
                let looks = placement::facing(c.yaw);
                (dx * dx + dy * dy + dz * dz <= 8.0 * 8.0, c.inv.held(), looks, c.yaw)
            }
            _ => return,
        };
        // Seau : remplir à une source, ou vider de l'eau ou de la lave
        if near && item.count > 0 && self.use_bucket(addr, item.id, (x, y, z), face) {
            return;
        }
        let block = match self.block_for(item.id) {
            Some(b) if item.count > 0 => b,
            _ => return, // rien de posable en main
        };
        let air = self.world.air;
        // Orientation selon la face cliquée et le regard du joueur ; une porte prend deux blocs
        let default = block;
        let (block, upper) = match (&self.data, self.items_level) {
            (Some(d), 2) => {
                let mut placed = placement::placed_state(d, default, placement::Placement { face, click, looks });
                let mut upper = None;
                if placement::is_door(d, placed) {
                    // Charnière : à droite si une porte se trouve à gauche (double porte), ou si
                    // le côté droit offre plus d'appui que le gauche
                    let left = placement::step(placement::left_of(looks));
                    let door_at = |dx: i32, dz: i32| d.block_name(self.world.get(x + dx, y, z + dz)).map(|n| n.ends_with("door") && !n.ends_with("trapdoor")).unwrap_or(false);
                    let solid = |dx: i32, dz: i32| (0..2).filter(|h| self.world.get(x + dx, y + h, z + dz) != air).count();
                    let right_hinge = door_at(left.0, left.1) || (!door_at(-left.0, -left.1) && solid(-left.0, -left.1) > solid(left.0, left.1));
                    if right_hinge {
                        placed = d.with_props(placed, &[("door_hinge_bit", Prop::Int(1))]).unwrap_or(placed);
                    }
                    upper = d.with_props(placed, &[("upper_block_bit", Prop::Int(1))]);
                }
                (placed, upper)
            }
            _ => (default, None),
        };
        // Dalle posée sur une dalle du même bois ou de la même pierre : une double dalle
        if near {
            if let Some((pos, double)) = self.double_slab_for(block, (x, y, z), face) {
                self.make_double_slab(addr, pos, double);
                return;
            }
        }
        let same_block = |s: &Self, state: u32| match &s.data {
            Some(d) => d.block_name(state).is_some() && d.block_name(state) == d.block_name(default),
            None => state == default,
        };
        if near && same_block(self, self.world.get(x, y, z)) {
            // Le jeu répète parfois la même pose : elle est déjà enregistrée. Mais en la
            // répétant, il a pu redessiner le bloc avec sa propre orientation : il reçoit de
            // nouveau celle que le serveur a retenue, pour que l'affichage reste fidèle.
            let (current, above) = (self.world.get(x, y, z), self.world.get(x, y + 1, z));
            self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, current));
            if upper.is_some() {
                self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y + 1, z, above));
            }
            return;
        }
        let room_above = upper.is_none() || (y < 319 && self.replaceable(self.world.get(x, y + 1, z)));
        let refused = near && room_above && self.plugins.listens("BlockPlaceEvent") && {
            let fields = vec![("player", self.name_json(addr)), ("block", self.block_json(block, (x, y, z)))];
            let out = self.fire("BlockPlaceEvent", fields);
            cancelled(&out)
        };
        // Posé dans de l'eau : le bloc la garde s'il le peut (dalle, escalier, clôture…)
        let was = self.world.get(x, y, z);
        let soak = self.is_water_source(was) && self.holds_water(block);
        if refused || !near || !room_above || !self.replaceable(was) || !self.world.set(x, y, z, block) {
            // Pose refusée : le joueur retrouve son inventaire exact et le bloc d'origine
            self.send_inventory(addr);
            let current = self.world.get(x, y, z);
            self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, current));
            return;
        }
        // En créatif, poser un bloc ne le retire pas de la main (comme dans le jeu : le jeu ne
        // le retire pas non plus de son côté, et les deux inventaires resteraient d'accord)
        if let Some(c) = self.clients.get_mut(addr) {
            if c.mode != GameMode::Creative {
                c.inv.consume_held();
            }
        }
        // Dernier bloc de la pile posé : les autres joueurs voient la main vide
        self.show_held(addr);
        // Coffre, four, panneau… : le bloc reçoit ses données
        if let Some(kind) = self.block_at_state(block).and_then(|b| b.tile()) {
            if let Some(new) = tile::create(kind) {
                self.world.tiles.insert((x, y, z), new);
            }
        }
        if self.cfg.debug {
            let name = self.data.as_ref().and_then(|d| d.block_name(block)).unwrap_or("?").to_string();
            let side = ["bas", "haut", "nord", "sud", "ouest", "est"][looks.min(5) as usize];
            info!("{} : bloc posé en {} {} {} — {}, face cliquée {}, regard vers {} (angle annoncé {:.0})", addr, x, y, z, name, face, side, yaw);
        }
        // Le poseur reçoit aussi l'état retenu : son jeu a pu prévoir une autre orientation
        self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, block), None);
        self.world.set_wet(x, y, z, soak);
        self.liquid_neighbours((x, y, z));
        if soak {
            let water = self.world.generator().blocks.water;
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block_layer(x, y, z, water, 1), None);
        }
        self.settle_stairs((x, y, z));
        self.settle_links((x, y, z));
        if let Some(upper) = upper {
            if self.world.set(x, y + 1, z, upper) {
                self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y + 1, z, upper), None);
            }
        }
    }

    /// Montre aux joueurs qui le voient l'objet qu'un joueur tient en main, s'il a changé.
    fn show_held(&mut self, addr: &SocketAddr) {
        if self.equip_refused {
            return;
        }
        let (id, held, slot) = match self.clients.get(addr) {
            Some(c) if c.stage == Stage::InGame => {
                let h = c.inv.held();
                let block = if h.is_empty() { 0 } else { self.block_for(h.id).unwrap_or(0) };
                (c.id, (h.id, h.count.min(1), block), c.inv.selected)
            }
            _ => return,
        };
        match self.clients.get_mut(addr) {
            // Même objet qu'au dernier envoi : rien à redire
            Some(c) if c.shown_held == Some((held.0, held.1)) => return,
            Some(c) => c.shown_held = Some((held.0, held.1)),
            None => return,
        }
        let packet = bedrock::mob_equipment(id, held, slot);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| *a != addr && c.visible.contains(&id)).map(|(a, _)| *a).collect();
        for viewer in viewers {
            self.send(&viewer, bedrock::ID_MOB_EQUIPMENT, &packet);
        }
    }

    /// Indicateurs d'un joueur vu par les autres, selon sa posture.
    fn remote_flags(sneaking: bool, sprinting: bool) -> i64 {
        bedrock::REMOTE_PLAYER_FLAGS | if sneaking { bedrock::FLAG_SNEAKING } else { 0 } | if sprinting { bedrock::FLAG_SPRINTING } else { 0 }
    }

    /// Change un bloc et l'annonce à tout le monde.
    /// Coins d'escaliers : le bloc qui vient de changer et ses quatre voisins prennent la forme
    /// que leur donne leur entourage. Depuis la 1.26, le coin fait partie de l'état du bloc :
    /// c'est au serveur de le calculer, sinon il disparaît au prochain chargement du terrain.
    fn settle_stairs(&mut self, pos: (i32, i32, i32)) {
        let mut changes: Vec<((i32, i32, i32), u32)> = Vec::new();
        if let Some(d) = &self.data {
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (x, y, z) = (pos.0 + dx, pos.1, pos.2 + dz);
                let at = |ax: i32, az: i32| placement::stair_of(d, self.world.get(x + ax, y, z + az)).map(|(f, top, _)| (f, top));
                if let Some(new) = placement::settled_stair(d, self.world.get(x, y, z), &at) {
                    changes.push(((x, y, z), new));
                }
            }
        }
        for (p, state) in changes {
            self.change_block(p, state);
        }
    }

    /// Barrières, murets, vitres et barreaux autour d'un bloc changé : chacun se relie (ou se
    /// détache) de ses voisins. Le bloc lui-même, ses quatre voisins, et celui du dessous (un
    /// muret devient haut ou garde son poteau selon ce qu'il porte).
    fn settle_links(&mut self, pos: (i32, i32, i32)) {
        let mut changes: Vec<((i32, i32, i32), u32)> = Vec::new();
        if let Some(d) = &self.data {
            for (dx, dy, dz) in [(0, 0, 0), (1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, -1, 0)] {
                let (x, y, z) = (pos.0 + dx, pos.1 + dy, pos.2 + dz);
                let at = |ax: i32, ay: i32, az: i32| self.world.get(x + ax, y + ay, z + az);
                if let Some(new) = placement::linked_state(d, self.world.get(x, y, z), &at) {
                    changes.push(((x, y, z), new));
                }
            }
        }
        for (p, state) in changes {
            self.change_block(p, state);
        }
    }

    /// Double dalle formée par cette dalle posée (`placed` : état prévu à la place `target`,
    /// contre la face `face`) : sur la dalle cliquée (face du dessus d'une dalle du bas, ou
    /// du dessous d'une dalle du haut), ou dans la moitié libre d'une dalle déjà là.
    fn double_slab_for(&self, placed: u32, target: (i32, i32, i32), face: u8) -> Option<((i32, i32, i32), u32)> {
        let d = self.data.as_ref()?;
        let name = d.block_name(placed)?;
        let double = placement::double_slab_of(d, placed)?;
        let half = |s: u32| match d.prop(s, "minecraft:vertical_half") {
            Some(Prop::Str(h)) => Some(h == "top"),
            _ => None,
        };
        let same = |s: u32| d.block_name(s) == Some(name);
        // 1. La dalle cliquée elle-même
        let clicked = bedrock::unplace_target(target, face);
        let c = self.world.get(clicked.0, clicked.1, clicked.2);
        if same(c) && ((face == 1 && half(c) == Some(false)) || (face == 0 && half(c) == Some(true))) {
            return Some((clicked, double));
        }
        // 2. Une dalle déjà à l'endroit visé, dont la moitié libre est celle que vise le joueur
        let t = self.world.get(target.0, target.1, target.2);
        if same(t) && half(t).is_some() && half(t) != half(placed) {
            return Some((target, double));
        }
        None
    }

    /// Transforme une dalle en double dalle (la dalle tenue est consommée).
    fn make_double_slab(&mut self, addr: &SocketAddr, pos: (i32, i32, i32), double: u32) {
        let (x, y, z) = pos;
        let current = self.world.get(x, y, z);
        let refused = self.plugins.listens("BlockPlaceEvent") && {
            let fields = vec![("player", self.name_json(addr)), ("block", self.block_json(double, pos))];
            let out = self.fire("BlockPlaceEvent", fields);
            cancelled(&out)
        };
        if refused || !self.world.set(x, y, z, double) {
            self.send_inventory(addr);
            self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, current));
            return;
        }
        if let Some(c) = self.clients.get_mut(addr) {
            if c.mode != GameMode::Creative {
                c.inv.consume_held();
            }
        }
        self.show_held(addr);
        self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, double), None);
        self.liquid_neighbours((x, y, z));
        // Une double dalle est un bloc plein : elle ne garde pas d'eau
        if self.world.is_wet(x, y, z) {
            self.world.set_wet(x, y, z, false);
            let air = self.world.air;
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block_layer(x, y, z, air, 1), None);
        }
        if self.cfg.debug {
            info!("{} : double dalle en {} {} {}", addr, x, y, z);
        }
    }

    /// Seau tenu en main, clic sur un bloc (`target` : la case devant la face cliquée).
    /// Renvoie vrai si le seau a servi (ou si l'objet tenu est un seau, même sans effet).
    fn use_bucket(&mut self, addr: &SocketAddr, item: i16, target: (i32, i32, i32), face: u8) -> bool {
        use crate::world::liquid::Fluid;
        let name = match self.data.as_ref().and_then(|d| d.item_by_id(item)) {
            Some(def) => def.name.trim_start_matches("minecraft:").to_string(),
            None => return false,
        };
        let filled = match name.as_str() {
            "bucket" => None,
            "water_bucket" => Some(Fluid::Water),
            "lava_bucket" => Some(Fluid::Lava),
            _ => return false,
        };
        let Some(liquids) = self.liquids.as_ref() else { return true };
        let clicked = bedrock::unplace_target(target, face);
        let creative = self.clients.get(addr).map(|c| c.mode == GameMode::Creative).unwrap_or(false);
        let at = |s: &Self, p: (i32, i32, i32)| s.world.get(p.0, p.1, p.2);
        // Ce que l'opération change : (position, nouveau bloc, eau gardée dans le bloc ?)
        let (pos, state, wet, give): ((i32, i32, i32), u32, Option<bool>, &str) = match filled {
            None => {
                // Remplir : la source cliquée, sinon celle devant la face (le jeu vise parfois
                // le bloc sous l'eau), ou l'eau d'un bloc posé dans l'eau
                let source = [clicked, target].into_iter().find_map(|p| match liquids.fluid(at(self, p)) {
                    Some((f, 0)) => Some((p, f)),
                    _ => None,
                });
                match source {
                    Some((p, f)) => (p, self.world.air, None, if f == Fluid::Water { "water_bucket" } else { "lava_bucket" }),
                    None if self.world.is_wet(clicked.0, clicked.1, clicked.2) => (clicked, at(self, clicked), Some(false), "water_bucket"),
                    None => {
                        self.send_inventory(addr);
                        return true;
                    }
                }
            }
            Some(fluid) => {
                let source = liquids.source(fluid);
                let c = at(self, clicked);
                if fluid == Fluid::Water && self.holds_water(c) && !self.world.is_wet(clicked.0, clicked.1, clicked.2) {
                    // Vidé sur une dalle, un escalier… : le bloc garde l'eau
                    (clicked, c, Some(true), "bucket")
                } else if self.replaceable(c) && liquids.fluid(c) != Some((fluid, 0)) {
                    (clicked, source, None, "bucket")
                } else if self.replaceable(at(self, target)) {
                    (target, source, None, "bucket")
                } else {
                    self.send_inventory(addr);
                    let current = at(self, target);
                    self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(target.0, target.1, target.2, current));
                    return true;
                }
            }
        };
        let event_block = if filled.is_some() { state } else { at(self, pos) };
        let event = if filled.is_some() { "BlockPlaceEvent" } else { "BlockBreakEvent" };
        let refused = self.plugins.listens(event) && {
            let fields = vec![("player", self.name_json(addr)), ("block", self.block_json(event_block, pos))];
            let out = self.fire(event, fields);
            cancelled(&out)
        };
        let before = at(self, pos);
        if refused || (state != before && !self.world.set(pos.0, pos.1, pos.2, state)) {
            self.send_inventory(addr);
            self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(pos.0, pos.1, pos.2, before));
            return true;
        }
        if state != before {
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(pos.0, pos.1, pos.2, state), None);
        }
        if let Some(wet) = wet {
            self.world.set_wet(pos.0, pos.1, pos.2, wet);
            let layer = if wet { self.world.generator().blocks.water } else { self.world.air };
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block_layer(pos.0, pos.1, pos.2, layer, 1), None);
        }
        self.liquid_neighbours(pos);
        // En survie, le seau change ; en créatif, il reste tel quel
        if !creative {
            if let Some(id) = self.item_id(give) {
                if let Some(c) = self.clients.get_mut(addr) {
                    c.inv.replace_held(id);
                }
            }
        }
        self.send_inventory(addr);
        self.show_held(addr);
        if self.cfg.debug {
            info!("{} : seau ({}) en {} {} {}", addr, name, pos.0, pos.1, pos.2);
        }
        true
    }

    /// Seau vide utilisé sans viser de bloc : la première source de liquide sur la ligne du
    /// regard (5 blocs au plus) est ramassée.
    fn bucket_in_air(&mut self, addr: &SocketAddr) {
        let Some((eye, pitch, yaw, item)) = self.clients.get(addr).map(|c| (c.pos, c.pitch, c.yaw, c.inv.held())) else { return };
        let empty_bucket = self.data.as_ref().and_then(|d| d.item_by_id(item.id)).map(|d| d.name == "minecraft:bucket").unwrap_or(false);
        let Some(liquids) = self.liquids.as_ref() else { return };
        if !empty_bucket || item.count == 0 {
            return;
        }
        let (p, y) = (pitch.to_radians(), yaw.to_radians());
        let dir = (-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos());
        let mut hit = None;
        for i in 1..=50 {
            let d = i as f32 * 0.1;
            let b = ((eye.0 + dir.0 * d).floor() as i32, (eye.1 + dir.1 * d).floor() as i32, (eye.2 + dir.2 * d).floor() as i32);
            let s = self.world.get(b.0, b.1, b.2);
            if let Some((_, depth)) = liquids.fluid(s) {
                if depth == 0 {
                    hit = Some(b);
                    break;
                }
            } else if s != self.world.air {
                break;
            }
        }
        if let Some(b) = hit {
            // Même chemin qu'un clic sur la face du dessus de la source
            self.use_bucket(addr, item.id, (b.0, b.1 + 1, b.2), 1);
        }
    }

    /// Un bloc peut-il être posé à la place de celui-ci (air, eau, lave) ?
    fn replaceable(&self, state: u32) -> bool {
        if state == self.world.air {
            return true;
        }
        match self.data.as_ref().and_then(|d| d.block_name(state)) {
            Some(n) => matches!(n, "minecraft:water" | "minecraft:flowing_water" | "minecraft:lava" | "minecraft:flowing_lava"),
            None => state == self.world.generator().blocks.water,
        }
    }

    /// Eau immobile (une source) : c'est elle qu'un bloc posé dedans peut garder.
    fn is_water_source(&self, state: u32) -> bool {
        match &self.data {
            Some(d) => matches!(d.block_name(state), Some("minecraft:water" | "minecraft:flowing_water")) && matches!(d.prop(state, "liquid_depth"), Some(Prop::Int(0)) | None),
            None => state == self.world.generator().blocks.water,
        }
    }

    /// Le bloc peut-il contenir de l'eau, comme dans le jeu ?
    fn holds_water(&self, state: u32) -> bool {
        let Some(name) = self.data.as_ref().and_then(|d| d.block_name(state)) else { return false };
        let n = name.trim_start_matches("minecraft:");
        if n.contains("double_slab") || n.ends_with("_double_slab") {
            return false;
        }
        const ENDS: [&str; 18] = [
            "_slab", "_stairs", "_fence", "_fence_gate", "_wall", "_trapdoor", "_sign", "_hanging_sign", "_coral", "_coral_fan", "glass_pane", "_bars",
            "lantern", "chain", "_candle", "_rod", "_grate", "_bud",
        ];
        ENDS.iter().any(|e| n.ends_with(e))
            || matches!(
                n,
                "ladder" | "chest" | "trapped_chest" | "ender_chest" | "scaffolding" | "sea_pickle" | "conduit" | "candle" | "hopper" | "amethyst_cluster" | "pointed_dripstone" | "mangrove_roots" | "heavy_core" | "barrier" | "copper_chain" | "decorated_pot"
            )
    }

    /// Change un bloc, prévient l'eau et la lave voisines, et l'envoie à tout le monde.
    fn change_block(&mut self, pos: (i32, i32, i32), state: u32) {
        if self.world.set(pos.0, pos.1, pos.2, state) {
            self.liquid_neighbours(pos);
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(pos.0, pos.1, pos.2, state), None);
        }
    }

    /// Porte, trappe, portillon, levier ou bouton actionné par un clic.
    fn toggle_block(&mut self, addr: &SocketAddr, pos: (i32, i32, i32)) {
        // Le jeu répète le clic tant que le bouton de la souris est tenu : un seul changement
        let now = self.tick;
        match self.clients.get_mut(addr) {
            Some(c) => {
                let gap = now.saturating_sub(c.last_toggle);
                c.last_toggle = now;
                if gap < 5 {
                    return;
                }
            }
            None => return,
        }
        let state = self.world.get(pos.0, pos.1, pos.2);
        let (next, kind, other_half) = match &self.data {
            Some(d) => match placement::toggled(d, state) {
                Some((next, kind)) => {
                    // Une porte : l'autre moitié suit
                    let other = match d.prop(state, "upper_block_bit") {
                        Some(Prop::Int(1)) => Some((pos.0, pos.1 - 1, pos.2)),
                        Some(Prop::Int(0)) => Some((pos.0, pos.1 + 1, pos.2)),
                        _ => None,
                    };
                    let other = other.and_then(|p| {
                        let s = self.world.get(p.0, p.1, p.2);
                        if d.block_name(s) != d.block_name(state) {
                            return None;
                        }
                        placement::toggled(d, s).map(|(n, _)| (p, n))
                    });
                    (next, kind, other)
                }
                None => return,
            },
            None => return,
        };
        self.change_block(pos, next);
        if let Some((p, n)) = other_half {
            self.change_block(p, n);
        }
        if kind == placement::Toggle::Momentary && self.releases.len() < 4096 {
            // Un bouton se relâche tout seul (une seconde et demie pour le bois, une pour la pierre)
            self.releases.push((self.tick + 30, pos, state));
        }
    }

    /// Relâche les boutons dont le délai est écoulé.
    fn release_buttons(&mut self) {
        if self.releases.is_empty() {
            return;
        }
        let now = self.tick;
        let due: Vec<((i32, i32, i32), u32)> = self.releases.iter().filter(|r| r.0 <= now).map(|r| (r.1, r.2)).collect();
        self.releases.retain(|r| r.0 > now);
        for (pos, rest) in due {
            // Seulement si le bouton est toujours là
            let current = self.world.get(pos.0, pos.1, pos.2);
            let same = self.data.as_ref().map(|d| d.block_name(current) == d.block_name(rest)).unwrap_or(false);
            if same {
                self.change_block(pos, rest);
            }
        }
    }

    /// Repère un jeu figé (plus aucun mouvement alors que la liaison tient) et passe à
    /// l'expérience suivante ; repère aussi une expérience qui tient et la conserve.
    fn watch_freezes(&mut self) {
        let now = Instant::now();
        let mut frozen = Vec::new();
        let mut steady = false;
        for (addr, c) in self.clients.iter() {
            if c.protocol < 2225 || c.inputs == 0 || !(c.stage == Stage::Spawning || c.stage == Stage::InGame) {
                continue;
            }
            if now.duration_since(c.last_input) > Duration::from_secs(15) {
                frozen.push((*addr, c.inputs));
            } else if c.inputs == 400 {
                steady = true;
            }
        }
        let _ = steady;
        for (addr, inputs) in frozen {
            warn!(
                "{} : le jeu s'est figé après {} mouvements — expérience {} ({})",
                addr, inputs, self.experiment, experiment_name(self.experiment)
            );
            if self.items_level > 0 && inputs < 200 {
                self.items_level -= 1;
                warn!("Gel juste après l'apparition : retour à la liste d'objets vide. Reconnectez-vous.");
            }
            if !self.experiment_locked && self.cfg.debug {
                self.experiment = (self.experiment + 1) % EXPERIMENTS;
                warn!("Prochaine connexion : expérience {} ({}). Relancez le jeu et reconnectez-vous.", self.experiment, experiment_name(self.experiment));
            }
            self.net_abort(&addr, "jeu figé");
        }
    }

    /// Joueurs bloqués trop longtemps à une étape de la connexion : déconnectés.
    fn check_deadlines(&mut self) {
        let now = Instant::now();
        let late: Vec<SocketAddr> = self
            .clients
            .iter()
            .filter(|(_, c)| {
                let age = now.duration_since(c.connected_at);
                match c.stage {
                    Stage::Handshake | Stage::Login | Stage::Authenticating => age > LOGIN_DEADLINE,
                    Stage::Packs | Stage::Spawning => age > SPAWN_DEADLINE,
                    _ => false,
                }
            })
            .map(|(a, _)| *a)
            .collect();
        for a in late {
            self.violation(&a, "connexion trop lente");
        }
    }

    /// Suite du login une fois l'identité établie par un fil de travail.
    fn finish_login(&mut self, addr: SocketAddr, login: bedrock::Login, authenticated: bool) {
        if self.cfg.debug {
            // Modèle pour l'outil de capture : les données client d'un vrai jeu (skin, appareil).
            // Ce fichier reste sur cette machine.
            if let Some(payload) = login.client_jwt.split('.').nth(1) {
                let _ = std::fs::create_dir_all("logs");
                let _ = std::fs::write("logs/client-data.b64", payload);
            }
        }
        // Apparence du joueur, gardée pour la montrer aux autres (ignorée si elle est incohérente)
        let skin = crate::player::Skin::from_client_jwt(&login.client_jwt).map(Arc::new);
        if let Some(c) = self.clients.get_mut(&addr) {
            c.skin = skin;
        }
        let proto = self.clients.get(&addr).map(|c| c.protocol).unwrap_or(0);
        info!(
            "{} se connecte depuis {} (Bedrock {}, protocole {}, {})",
            login.name,
            addr,
            if login.game_version.is_empty() { crate::versions::name(proto) } else { &login.game_version },
            proto,
            if authenticated { "compte Xbox vérifié" } else { "non authentifié" }
        );
        let player = LVal::Map(vec![
            ("name", LVal::Str(login.name.clone())),
            ("xuid", LVal::Str(login.xuid.clone())),
            ("address", LVal::Str(addr.ip().to_string())),
            ("game_version", LVal::Str(login.game_version.clone())),
            ("protocol", LVal::Int(proto as i64)),
            ("authenticated", LVal::Bool(authenticated)),
        ]);
        let mut refusal = None;
        if !self.is_whitelisted(&login.name) {
            info!("{} refusé : absent de la liste blanche", login.name);
            refusal = Some("Ce serveur est en liste blanche : vous n'y êtes pas inscrit.".to_string());
        }
        let answers = if refusal.is_none() { self.emit("login", &[player]) } else { Vec::new() };
        for r in answers {
            match r {
                LVal::Str(s) => refusal = Some(s),
                LVal::Bool(false) => refusal = Some("Connexion refusée.".to_string()),
                _ => continue,
            }
            break;
        }
        if refusal.is_none() {
            let out = self.fire("PlayerLoginEvent", vec![("player", Json::Str(login.name.clone())), ("kickMessage", Json::Str("Connexion refusée.".into()))]);
            if cancelled(&out) {
                refusal = Some(text_of(&out, "kickMessage", 256).unwrap_or_else(|| "Connexion refusée.".into()));
            }
        }
        let names = self.players();
        if refusal.is_none() && names.len() >= self.cfg.max_players as usize {
            refusal = Some("Le serveur est plein.".to_string());
        }
        if refusal.is_none() && names.iter().any(|n| n.eq_ignore_ascii_case(&login.name)) {
            refusal = Some("Un joueur porte déjà ce pseudo.".to_string());
        }
        if let Some(message) = refusal {
            info!("{} : connexion refusée ({})", login.name, crate::log::clean(&message, 200));
            self.kick(&addr, &message);
            return;
        }
        if let Some(c) = self.clients.get_mut(&addr) {
            c.name = Some(login.name.clone());
            c.stage = Stage::Packs;
        }
        self.net_set_game_limits(&addr);
        self.update_pong();
        // Chiffrement (RakNet seulement : NetherNet passe déjà par une liaison DTLS chiffrée).
        // Il prouve aussi que le joueur possède la clé de son jeton : un login intercepté puis
        // rejoué ne mène à rien. Un joueur authentifié a toujours une clé ; sans
        // authentification (xbox-auth=false), un client sans clé reste en clair.
        let raknet = !self.nether.as_ref().map(|n| n.has(&addr)).unwrap_or(false);
        if raknet {
            let key = if authenticated {
                login.token.as_deref().and_then(auth::token_client_key)
            } else {
                auth::client_data_key(&login.client_jwt)
            };
            match key.as_ref().map(auth::server_handshake) {
                Some(Some((jwt, session_key))) => {
                    let mut body = Vec::with_capacity(jwt.len() + 3);
                    let mut w = bedrock::W::default();
                    w.varu32(jwt.len() as u32);
                    body.extend_from_slice(&w.0);
                    body.extend_from_slice(jwt.as_bytes());
                    self.send(&addr, bedrock::ID_SERVER_TO_CLIENT_HANDSHAKE, &body);
                    if let Some(c) = self.clients.get_mut(&addr) {
                        c.crypt = Some(crate::crypto::Crypt::new(session_key));
                        c.handshake_pending = true;
                    }
                    if self.cfg.debug {
                        info!("{} : chiffrement proposé, réponse du jeu attendue", addr);
                    }
                    return;
                }
                Some(None) => {
                    warn!("{} : chiffrement impossible (aléa du système indisponible ou clé du joueur inutilisable)", addr);
                    self.kick(&addr, "Connexion sécurisée impossible.");
                    return;
                }
                None if authenticated => {
                    self.kick(&addr, "Clé du joueur illisible.");
                    return;
                }
                None => {}
            }
        }
        self.send_login_success(&addr, login.protocol);
    }

    /// Login accepté (et chiffrement en place s'il y en a un) : début des packs de ressources.
    fn send_login_success(&mut self, addr: &SocketAddr, protocol: i32) {
        self.send(addr, bedrock::ID_PLAY_STATUS, &bedrock::play_status(bedrock::STATUS_LOGIN_SUCCESS));
        self.send(addr, bedrock::ID_RESOURCE_PACKS_INFO, &bedrock::resource_packs_info(crate::versions::features(protocol)));
    }

    /// Traite les logins analysés et vérifiés en arrière-plan.
    fn poll_logins(&mut self) {
        while let Ok((addr, outcome)) = self.logins.results.try_recv() {
            self.logins.pending = self.logins.pending.saturating_sub(1);
            let waiting = self.clients.get(&addr).map(|c| c.stage == Stage::Authenticating).unwrap_or(false);
            if !waiting {
                continue; // le client est parti entre-temps
            }
            match outcome {
                LoginOutcome::Accepted(login, authenticated) => {
                    if let Some(c) = self.clients.get_mut(&addr) {
                        c.stage = Stage::Login;
                    }
                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.finish_login(addr, *login, authenticated)
                    }));
                    if res.is_err() {
                        error!("Erreur interne pendant le login de {} : joueur déconnecté", addr);
                        self.violation(&addr, "erreur interne");
                    }
                }
                LoginOutcome::Refused { message, log, refresh_keys } => {
                    if let Some(text) = log {
                        warn!("{} : {}", addr, text);
                    }
                    if refresh_keys {
                        if let Some(f) = &self.key_fetcher {
                            f.refresh();
                        }
                    }
                    self.kick(&addr, message);
                }
            }
        }
    }

    /// Paquet reçu d'un joueur : décodage, puis traitement selon l'étape de sa connexion.
    fn on_packet(&mut self, addr: SocketAddr, data: Vec<u8>) {
        let (stage, compression) = match self.clients.get(&addr) {
            Some(c) => (c.stage, c.compression),
            None => return,
        };
        if stage == Stage::Done {
            return;
        }
        // Quotas : nombre de lots et volume de données par seconde
        let now = Instant::now();
        if now.duration_since(self.global_window) >= Duration::from_secs(1) {
            self.global_window = now;
            self.global_bytes = 0;
        }
        if self.global_bytes > MAX_GLOBAL_BYTES_PER_SEC {
            return; // serveur saturé : on ignore plutôt que de ralentir tout le monde
        }
        let over = match self.clients.get_mut(&addr) {
            Some(c) => {
                if now.duration_since(c.window) >= Duration::from_secs(1) {
                    c.window = now;
                    c.batches = 0;
                    c.bytes = 0;
                }
                c.batches += 1;
                c.batches > MAX_BATCHES_PER_SEC
            }
            None => return,
        };
        if over {
            self.violation(&addr, "trop de paquets par seconde");
            return;
        }
        let max = if stage == Stage::Login {
            crate::raknet::MAX_PACKET_LOGIN
        } else {
            crate::raknet::MAX_PACKET_GAME
        };
        // Session chiffrée : déchiffrement et somme de contrôle (un lot altéré, rejoué ou
        // envoyé en clair est refusé)
        let opened = match self.clients.get_mut(&addr).and_then(|c| c.crypt.as_mut()) {
            Some(crypt) => match data.get(1..).map(|d| crypt.open(d)) {
                Some(Ok(plain)) => Some(plain),
                _ => {
                    self.violation(&addr, "lot chiffré invalide");
                    return;
                }
            },
            None => None,
        };
        let batch: &[u8] = match &opened {
            Some(p) => p,
            None => &data[1..],
        };
        let packets = match bedrock::decode_batch(batch, compression, max) {
            Ok(p) => p,
            Err(_) => {
                self.violation(&addr, "paquet illisible ou trop volumineux");
                return;
            }
        };
        let size: usize = packets.iter().map(|(_, b)| b.len()).sum();
        self.global_bytes += size;
        let over = match self.clients.get_mut(&addr) {
            Some(c) => {
                c.bytes += size;
                c.bytes > MAX_BYTES_PER_SEC
            }
            None => return,
        };
        if over {
            self.violation(&addr, "trop de données par seconde");
            return;
        }
        for (id, body) in packets {
            let _t = timings::scope_with(|| format!("Paquet reçu n° {}{}", id, packet_label(id)));
            let stage = self.clients.get(&addr).map(|c| c.stage).unwrap_or(Stage::Done);
            if self.cfg.debug && id != 144 {
                if body.len() <= 48 {
                    let hex: String = body.iter().map(|b| format!("{:02x}", b)).collect();
                    info!("[paquets] <- {} : id {} ({} octets) {}", addr, id, body.len(), hex);
                } else {
                    info!("[paquets] <- {} : id {} ({} octets)", addr, id, body.len());
                }
            }
            // Chiffrement proposé : seule la réponse du jeu est attendue
            if self.clients.get(&addr).map(|c| c.handshake_pending).unwrap_or(false) {
                if id == bedrock::ID_CLIENT_TO_SERVER_HANDSHAKE {
                    let protocol = match self.clients.get_mut(&addr) {
                        Some(c) => {
                            c.handshake_pending = false;
                            c.protocol
                        }
                        None => return,
                    };
                    if self.cfg.debug {
                        info!("{} : chiffrement en place", addr);
                    }
                    self.send_login_success(&addr, protocol);
                }
                continue;
            }
            if id == 156 {
                // PacketViolationWarning : le jeu signale qu'il n'a pas pu lire un de nos paquets.
                // C'est l'information la plus utile pour corriger le protocole : toujours journalisée.
                let mut pos = 0;
                let kind = bedrock::read_vari32(&body, &mut pos).unwrap_or(-99);
                let severity = bedrock::read_vari32(&body, &mut pos).unwrap_or(-99);
                let packet = bedrock::read_vari32(&body, &mut pos).unwrap_or(-99);
                let len = bedrock::read_varu32(&body, &mut pos).unwrap_or(0) as usize;
                let context = body.get(pos..pos.saturating_add(len)).unwrap_or(&[]);
                let hex: String = body.iter().take(160).map(|b| format!("{:02x}", b)).collect();
                warn!(
                    "{} : le jeu n'a pas pu lire le paquet {} envoyé par le serveur (type {}, gravité {}) : {}",
                    addr,
                    packet,
                    kind,
                    severity,
                    crate::log::clean(&String::from_utf8_lossy(context), 300)
                );
                warn!("{} : détail brut : {}", addr, hex);
                if let Some(c) = self.clients.get_mut(&addr) {
                    c.reported_error = true;
                }
                if packet == bedrock::ID_PLAYER_LIST as i32 && !self.player_list_refused {
                    self.player_list_refused = true;
                    warn!("La liste des joueurs (noms et skins) est refusée par le jeu : elle n'est plus envoyée. Les joueurs peuvent se reconnecter ; ils garderont l'apparence par défaut.");
                }
                if packet == bedrock::ID_MOB_EQUIPMENT as i32 && !self.equip_refused {
                    self.equip_refused = true;
                    warn!("L'objet tenu par les autres joueurs est refusé par le jeu : il n'est plus envoyé. Reconnectez-vous.");
                }
                if packet == bedrock::ID_ANIMATE as i32 && !self.animate_refused {
                    self.animate_refused = true;
                    warn!("Le geste du bras des autres joueurs est refusé par le jeu : il n'est plus envoyé. Reconnectez-vous.");
                }
                if packet == bedrock::ID_MOVE_ACTOR_ABSOLUTE as i32 && !self.move_actor_refused {
                    self.move_actor_refused = true;
                    warn!("Le déplacement des autres joueurs est refusé par le jeu : il est désactivé. Les joueurs peuvent se reconnecter ; ils se verront, immobiles.");
                }
                if packet == bedrock::ID_ADD_ACTOR as i32 && !self.mobs_refused {
                    // Format des créatures pas encore relevé : en cas de refus, plus d'animaux
                    self.mobs_refused = true;
                    let ids: Vec<i64> = self.mobs.keys().copied().collect();
                    for id in ids {
                        self.remove_mob(id);
                    }
                    warn!("Les animaux sont refusés par le jeu : ils sont désactivés. Lancez la capture (Capturer-les-donnees.cmd) avec « summon cow » pour relever leur format.");
                }
                if packet == bedrock::ID_ADD_PLAYER as i32 && !self.add_player_refused {
                    // Sans ce repli, chaque arrivée d'un second joueur déconnecterait tout le monde
                    self.add_player_refused = true;
                    warn!("L'apparition des autres joueurs est refusée par le jeu : elle est désactivée. Les joueurs peuvent se reconnecter et jouer, mais ne se verront pas.");
                }
                let from_give = packet == 49 && self.clients.get(&addr).map(|c| c.inv_touched).unwrap_or(false);
                let relayed = packet >= 0 && self.captured(&addr, packet as u32).is_some();
                if relayed {
                    self.refused_captured.insert(packet as u32);
                    warn!("Le paquet {} relevé auprès du serveur officiel a été refusé par le jeu : Basalt reprend sa propre version. Reconnectez-vous.", packet);
                } else if from_give && self.item_net_ids {
                    self.item_net_ids = false;
                    warn!("Inventaire refusé avec les numéros de pile : ils ne seront plus envoyés. Reconnectez-vous.");
                } else if from_give {
                    warn!("L'objet donné par « give » a été refusé par le jeu : son format reste à corriger.");
                } else if packet == 162 {
                    self.items_level = self.items_level.saturating_sub(1);
                    warn!("Liste d'objets refusée : retour à la liste vide pour les prochaines connexions.");
                } else if packet >= 0 && bedrock_extra(packet as u32) && self.refused_extras.insert(packet as u32) {
                    warn!("Le paquet {} ne sera plus envoyé aux prochaines connexions. Reconnectez-vous.", packet);
                } else if (packet as u32 == bedrock::ID_ADD_ITEM_ACTOR || packet as u32 == bedrock::ID_TAKE_ITEM_ACTOR) && !self.drops_refused {
                    self.drops_refused = true;
                    self.ground.clear();
                    for c in self.clients.values_mut() {
                        c.items_seen.clear();
                    }
                    warn!("Les objets au sol sont refusés par le jeu : le butin ira de nouveau directement dans l'inventaire.");
                } else if [bedrock::ID_ACTOR_EVENT, bedrock::ID_SET_ACTOR_MOTION, bedrock::ID_RESPAWN, bedrock::ID_DEATH_INFO, bedrock::ID_LEVEL_SOUND_EVENT].contains(&(packet as u32))
                    && self.refused_extras.insert(packet as u32)
                {
                    warn!("Le paquet {} (vie, mort ou sons) est refusé par le jeu : il n'est plus envoyé.", packet);
                }
                if packet == 122 && self.cfg.debug && !self.experiment_locked {
                    self.experiment = 0;
                    warn!("Liste de biomes refusée. Prochaine connexion : expérience {} ({}).", self.experiment, experiment_name(self.experiment));
                }
                if packet == 58 && self.chunk_variant_by != Some(addr) {
                    self.chunk_variant_by = Some(addr);
                    warn!("Variante de chunks refusée : {} ({})", self.chunk_variant, bedrock::chunk_variant_name(self.chunk_variant));
                    // Mise au point : on essaie l'écriture suivante à la prochaine connexion.
                    self.chunk_variant = (self.chunk_variant + 1) % bedrock::CHUNK_VARIANTS;
                    warn!(
                        "Chunks refusés : la prochaine connexion essaiera la variante {} ({}). Reconnectez-vous.",
                        self.chunk_variant,
                        bedrock::chunk_variant_name(self.chunk_variant)
                    );
                }
                continue;
            }
            match (stage, id) {
                (Stage::Handshake, bedrock::ID_REQUEST_NETWORK_SETTINGS) => {
                    let proto = match body.get(..4) {
                        Some(b) => i32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                        None => return,
                    };
                    if proto < self.cfg.min_protocol || proto > self.cfg.max_protocol {
                        info!(
                            "{} : protocole {} refusé (acceptés : {} à {})",
                            addr, proto, self.cfg.min_protocol, self.cfg.max_protocol
                        );
                        let status = if proto < self.cfg.min_protocol {
                            bedrock::STATUS_CLIENT_OUTDATED
                        } else {
                            bedrock::STATUS_SERVER_OUTDATED
                        };
                        self.send(&addr, bedrock::ID_PLAY_STATUS, &bedrock::play_status(status));
                        self.net_close(&addr, Duration::from_millis(1500));
                        if let Some(c) = self.clients.get_mut(&addr) {
                            c.stage = Stage::Done;
                        }
                        return;
                    }
                    if self.cfg.debug {
                        info!("{} : client Bedrock {} (protocole {})", addr, crate::versions::name(proto), proto);
                    }
                    self.send(
                        &addr,
                        bedrock::ID_NETWORK_SETTINGS,
                        &bedrock::network_settings(crate::versions::features(proto)),
                    );
                    if let Some(c) = self.clients.get_mut(&addr) {
                        c.compression = true;
                        c.protocol = proto;
                        c.stage = Stage::Login;
                    }
                }
                (Stage::Login, bedrock::ID_LOGIN) => {
                    // L'analyse du login et la vérification des signatures (environ 2 ms)
                    // sont confiées aux autres cœurs : la boucle principale ne s'arrête pas.
                    if self.logins.pending >= MAX_PENDING_LOGINS {
                        self.kick(&addr, "Le serveur est très sollicité. Réessayez dans un instant.");
                        return;
                    }
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let job = LoginJob {
                        addr,
                        body,
                        keys: self.auth_keys.clone(),
                        xbox_auth: self.cfg.xbox_auth,
                        local_bot: self.cfg.local_bots && addr.ip().is_loopback() && !self.nether.as_ref().map(|n| n.has(&addr)).unwrap_or(false),
                        now,
                        bound_cpk: self.nether.as_ref().and_then(|n| n.bound_cpk(&addr)),
                    };
                    if self.logins.jobs.send(job).is_err() {
                        self.kick(&addr, "Connexion impossible pour le moment.");
                        return;
                    }
                    self.logins.pending += 1;
                    if let Some(c) = self.clients.get_mut(&addr) {
                        c.stage = Stage::Authenticating;
                    }
                    return;
                }
                // Le client répond deux fois : « j'ai tous les packs », puis « terminé ».
                (Stage::Packs, bedrock::ID_RESOURCE_PACK_RESPONSE) => {
                    let (n, id, features) = match self.clients.get_mut(&addr) {
                        Some(c) => {
                            c.pack_responses += 1;
                            (c.pack_responses, c.id, crate::versions::features(c.protocol))
                        }
                        None => return,
                    };
                    if n == 1 {
                        self.send(&addr, bedrock::ID_RESOURCE_PACK_STACK, &bedrock::resource_pack_stack());
                        continue;
                    }
                    // Le joueur est chargé dès maintenant : il apparaît directement là où il
                    // s'était arrêté, sans qu'il faille le déplacer après coup
                    self.load_player(&addr);
                    let spawn = self
                        .clients
                        .get(&addr)
                        .and_then(|c| c.data.as_ref())
                        .filter(|d| d.has_pos && d.pos.0.abs() < 3.0e7 && d.pos.2.abs() < 3.0e7)
                        // Hauteur exacte, même dans un trou creusé sous le niveau du sol
                        .map(|d| (d.pos.0, d.pos.1.clamp(-64.0 + 1.62, 400.0), d.pos.2))
                        // Position enregistrée dans un autre monde (ou bouchée depuis) : un joueur
                        // n'apparaît jamais coincé dans la roche
                        .filter(|p| {
                            let (x, y, z) = (p.0.floor() as i32, (p.1 - 1.62 + 0.01).floor() as i32, p.2.floor() as i32);
                            !self.blocks_movement(self.world.get(x, y, z)) && !self.blocks_movement(self.world.get(x, y + 1, z))
                        })
                        .unwrap_or_else(|| self.spawn_point());
                    // Il regarde aussi dans la direction où il regardait en partant
                    let (yaw, pitch) = self.clients.get(&addr).and_then(|c| c.data.as_ref()).map(|d| d.rotation).unwrap_or((0.0, 0.0));
                    if let Some(c) = self.clients.get_mut(&addr) {
                        c.pos = spawn;
                        c.yaw = yaw;
                        c.head_yaw = yaw;
                        c.pitch = pitch;
                    }
                    let start = bedrock::start_game(&bedrock::StartGame {
                        entity_id: id,
                        gamemode: self.clients.get(&addr).map(|c| c.mode.bedrock_id()).unwrap_or(0),
                        position: spawn,
                        rotation: (pitch, yaw),
                        world_name: &self.cfg.world_name.clone(),
                        game_version: &self.cfg.game_version.clone(),
                        seed: self.world.generator().seed,
                        flat: self.world.generator().kind == crate::world::generator::Kind::Flat,
                        spawn: self.spawn,
                        features,
                    });
                    if features.jigsaw_before_start {
                        self.send(&addr, bedrock::ID_JIGSAW_STRUCTURE_DATA, &bedrock::jigsaw_structure_data());
                    }
                    if features.voxel_shapes {
                        self.send(&addr, bedrock::ID_VOXEL_SHAPES, &bedrock::voxel_shapes());
                    }
                    self.send(&addr, bedrock::ID_START_GAME, &start);
                    if self.cfg.debug {
                        info!("{} : envoi du monde, variante de chunks {} ({})", addr, self.chunk_variant, bedrock::chunk_variant_name(self.chunk_variant));
                    }
                    let exp = if features.chunk_biome_update_flag { self.experiment } else { 0 };
                    if features.chunk_biome_update_flag {
                        if exp != 0 {
                            warn!("{} : expérience {} : {}", addr, exp, experiment_name(exp));
                        }
                    }
                    if !experiment_skips_items(exp) {
                        let level = if features.editor_migration_version { self.items_level } else { 0 };
                        let sent = self.send_captured(&addr, bedrock::ID_ITEM_REGISTRY);
                        let registry = match (level, &self.data) {
                            _ if sent => Vec::new(),
                            (2, Some(d)) => {
                                let list: Vec<(&str, i16, i32, Option<&[u8]>)> =
                                    d.items.iter().map(|i| (i.name.as_str(), i.id, i.version, i.components.as_deref())).collect();
                                bedrock::item_registry_full(&list)
                            }
                            (0, _) => bedrock::item_registry(),
                            _ => bedrock::item_registry_basic(),
                        };
                        if !registry.is_empty() {
                            self.send(&addr, bedrock::ID_ITEM_REGISTRY, &registry);
                        }
                    }
                    match exp {
                        0 => {
                            if !self.send_captured(&addr, bedrock::ID_BIOME_DEFINITION_LIST) {
                                self.send(&addr, bedrock::ID_BIOME_DEFINITION_LIST, &bedrock::biome_definition_list())
                            }
                        }
                        _ => self.send(&addr, bedrock::ID_BIOME_DEFINITION_LIST, &bedrock::biome_definition_list_plains()),
                    }
                    if !experiment_skips_state(exp) {
                        self.send_player_state(&addr, features);
                    }
                    if let Some(c) = self.clients.get_mut(&addr) {
                        c.stage = Stage::Spawning;
                    }
                }
                (Stage::Spawning, bedrock::ID_REQUEST_CHUNK_RADIUS)
                | (Stage::InGame, bedrock::ID_REQUEST_CHUNK_RADIUS) => {
                    let mut pos = 0;
                    let asked = bedrock::read_vari32(&body, &mut pos).unwrap_or(4);
                    let radius = asked.max(1).min(self.cfg.view_distance);
                    self.send(&addr, bedrock::ID_CHUNK_RADIUS_UPDATED, &bedrock::chunk_radius_updated(radius));
                    let features = self.clients.get(&addr).map(|c| crate::versions::features(c.protocol));
                    let features = match features {
                        Some(f) => f,
                        None => return,
                    };
                    let first = match self.clients.get_mut(&addr) {
                        Some(c) => !std::mem::replace(&mut c.chunks_sent, true),
                        None => return,
                    };
                    if first {
                        // Autour de l'endroit où le joueur apparaît
                        let at = self.clients.get(&addr).map(|c| c.pos).unwrap_or((0.5, 0.0, 0.5));
                        let center = ((at.0.floor() as i32) >> 4, (at.2.floor() as i32) >> 4);
                        self.send(
                            &addr,
                            bedrock::ID_CHUNK_PUBLISHER_UPDATE,
                            &bedrock::chunk_publisher_update(at.0.floor() as i32, at.1.floor() as i32, at.2.floor() as i32, (radius as u32 + 1) * 16),
                        );
                        // Du centre vers l'extérieur, pour que le sol sous le joueur arrive d'abord.
                        // Les chunks partent au fil de leur génération (voir send_queued_chunks).
                        let coords = Self::chunks_around(center, radius);
                        for (x, z) in &coords {
                            self.world.prepare(*x, *z);
                        }
                        let _ = features;
                        if let Some(c) = self.clients.get_mut(&addr) {
                            c.radius = radius;
                            c.center = center;
                            c.move_budget = SPEED_CREATIVE * 2.0;
                            c.move_budget_at = Instant::now();
                            c.loaded.clear();
                            c.queue = coords.into_iter().collect();
                        }
                        // Le signal d'apparition part quand tout le terrain initial est envoyé
                        // (voir send_queued_chunks) : le joueur doit apparaître dans un monde chargé.
                        if let Some(c) = self.clients.get_mut(&addr) {
                            c.spawn_pending = true;
                        }
                    }
                }
                (Stage::Spawning, bedrock::ID_PLAYER_AUTH_INPUT) | (Stage::InGame, bedrock::ID_PLAYER_AUTH_INPUT) => {
                    let parsed = bedrock::read_input(&body);
                    // Accroupi (8) et en course (20) : état annoncé par le jeu à chaque mouvement
                    let moves = bedrock::input_flags(&body);
                    let posture = (moves.contains(&8), moves.contains(&20));
                    let mut changed_posture = None;
                    if let Some(c) = self.clients.get_mut(&addr) {
                        if (c.sneaking, c.sprinting) != posture && c.stage == Stage::InGame {
                            c.sneaking = posture.0;
                            c.sprinting = posture.1;
                            changed_posture = Some(c.id);
                        }
                    }
                    if let Some(id) = changed_posture {
                        // Les joueurs qui le voient reçoivent sa nouvelle posture
                        let flags = Self::remote_flags(posture.0, posture.1);
                        let packet = bedrock::set_actor_flags(id as u64, flags);
                        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| **a != addr && c.visible.contains(&id)).map(|(a, _)| *a).collect();
                        for viewer in viewers {
                            self.send(&viewer, bedrock::ID_SET_ACTOR_DATA, &packet);
                        }
                    }
                    let debug = self.cfg.debug;
                    if let Some(c) = self.clients.get_mut(&addr) {
                        c.inputs += 1;
                        c.last_input = Instant::now();
                        if debug && c.inputs % 100 == 0 {
                            if let Some(i) = &parsed {
                                info!("{} : position envoyée par le jeu : {:.2} {:.2} {:.2} ({} mouvements)", addr, i.pos.0, i.pos.1, i.pos.2, c.inputs);
                            }
                        }
                        if debug && c.inputs == 1 {
                            let hex: String = body.iter().take(64).map(|b| format!("{:02x}", b)).collect();
                            info!(
                                "{} : premier paquet de mouvement reçu (après la sonde {}, lisible : {}, {} octets) {}",
                                addr,
                                c.probe,
                                parsed.is_some(),
                                body.len(),
                                hex
                            );
                        }
                    }
                    if let Some(input) = parsed {
                        self.on_move(&addr, input);
                    }
                    if self.cfg.debug {
                        // Mise au point : gestes sur les blocs (35) et demandes d'objets (36) joints au mouvement
                        let flags = bedrock::input_flags(&body);
                        if flags.contains(&34) || flags.contains(&36) {
                            if let Some(c) = self.clients.get_mut(&addr) {
                                if c.gesture_logged < 12 {
                                    c.gesture_logged += 1;
                                    let hex: String = body.iter().skip(32).take(300).map(|b| format!("{:02x}", b)).collect();
                                    info!("{} : mouvement avec gestes, drapeaux {:?} ({} octets) {}", addr, flags, body.len(), hex);
                                }
                            }
                        }
                    }
                    if bedrock::input_has_interaction(&body) {
                        // Lecture directe, puis recherche du geste par la position du joueur qu'il
                        // contient : avec un outil en main, le début du geste varie.
                        let player = self.clients.get(&addr).map(|c| c.pos).unwrap_or((0.0, 0.0, 0.0));
                        let close = |u: &bedrock::BlockUse| {
                            (u.pos.0 as f32 - player.0).abs() < 9.0 && (u.pos.1 as f32 - player.1).abs() < 9.0 && (u.pos.2 as f32 - player.2).abs() < 9.0
                        };
                        let block_use = bedrock::read_block_use(&body)
                            .filter(|u| close(u))
                            .or_else(|| bedrock::read_use_item_in_input(&body, player));
                        if self.cfg.debug {
                            if let Some(c) = self.clients.get_mut(&addr) {
                                if block_use.is_none() && c.unread_logged < 8 {
                                    c.unread_logged += 1;
                                    let hex: String = body.iter().take(400).map(|b| format!("{:02x}", b)).collect();
                                    info!("{} : interaction reçue, lue comme {:?} ({} octets) {}", addr, block_use, body.len(), hex);
                                }
                            }
                        }
                        if let Some(u) = block_use {
                            self.on_block_use(&addr, u);
                        }
                    }
                }
                (Stage::InGame, bedrock::ID_TEXT) => self.on_chat(&addr, &body),
                (Stage::InGame, 77) => self.on_command_request(&addr, &body),
                (Stage::InGame, bedrock::ID_MOB_EQUIPMENT) => {
                    if let (Some(slot), Some(c)) = (bedrock::read_selected_slot(&body), self.clients.get_mut(&addr)) {
                        c.inv.selected = slot;
                    }
                    self.show_held(&addr);
                }
                (Stage::InGame, bedrock::ID_ANIMATE) => {
                    // Geste du bras (frapper, miner, poser) : montré aux joueurs qui le voient,
                    // au plus quatre fois par seconde
                    let now = self.tick;
                    let relayed = match self.clients.get_mut(&addr) {
                        Some(c) if !self.animate_refused && now.saturating_sub(c.last_swing) >= 5 => {
                            c.last_swing = now;
                            bedrock::relay_arm_swing(&body, c.id).map(|p| (c.id, p))
                        }
                        _ => None,
                    };
                    if let Some((id, packet)) = relayed {
                        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| **a != addr && c.visible.contains(&id)).map(|(a, _)| *a).collect();
                        for viewer in viewers {
                            self.send(&viewer, bedrock::ID_ANIMATE, &packet);
                        }
                    }
                }
                (Stage::InGame, bedrock::ID_INTERACT) => {
                    if bedrock::read_open_inventory(&body) {
                        self.on_open_inventory(&addr);
                    }
                }
                (Stage::InGame, bedrock::ID_CONTAINER_CLOSE) => self.on_close_container(&addr, &body),
                (Stage::InGame, bedrock::ID_ITEM_STACK_REQUEST) => self.on_stack_request(&addr, &body),
                (Stage::InGame, bedrock::ID_INVENTORY_TRANSACTION) if bedrock::read_attack(&body).is_some() => {
                    // Coup porté à un autre joueur
                    if let Some((target, action)) = bedrock::read_attack(&body) {
                        if bedrock::is_attack(action) {
                            self.on_attack(&addr, target);
                        } else if action == 0 {
                            // Clic droit sur une entité : nourrir, tondre, traire un animal
                            self.interact_mob(&addr, target as i64);
                        }
                    }
                }
                (Stage::InGame, bedrock::ID_INVENTORY_TRANSACTION) if bedrock::read_world_drop(&body).is_some() => {
                    // Objet jeté avec la touche « lâcher » (Q)
                    if let Some(d) = bedrock::read_world_drop(&body) {
                        self.on_key_drop(&addr, d);
                    }
                }
                (Stage::InGame, bedrock::ID_PLAYER_ACTION) => {
                    if let Some((_, bedrock::ACTION_RESPAWN)) = bedrock::read_player_action(&body) {
                        self.respawn(&addr);
                    }
                }
                (Stage::InGame, bedrock::ID_RESPAWN) => {
                    // Le joueur a cliqué sur « Réapparaître »
                    if bedrock::read_respawn_state(&body) == Some(bedrock::RESPAWN_CLIENT_READY) {
                        self.respawn(&addr);
                    }
                }
                (Stage::InGame, bedrock::ID_INVENTORY_TRANSACTION) => {
                    // Une transaction par bloc posé, y compris quand le clic droit reste appuyé
                    let player = self.clients.get(&addr).map(|c| c.pos).unwrap_or((0.0, 0.0, 0.0));
                    // La lecture directe peut se tromper quand la transaction commence par des
                    // objets (seau rempli ou vidé…) : un bloc hors de portée du joueur est
                    // écarté au profit de la lecture de secours, qui se cale sur sa position.
                    let in_reach = |u: &bedrock::BlockUse| {
                        let (x, y, z) = u.pos;
                        u.action == 1 || ((x as f32 - player.0).abs() < 9.0 && (y as f32 - player.1).abs() < 9.0 && (z as f32 - player.2).abs() < 9.0)
                    };
                    let block_use = bedrock::read_use_item(&body).filter(|u| in_reach(u)).or_else(|| bedrock::read_use_item_scan(&body, player));
                    if self.cfg.debug {
                        if let Some(c) = self.clients.get_mut(&addr) {
                            // Les transactions longues (avec des objets) sont les plus instructives
                            if body.len() > 60 && c.interactions_logged < 40 {
                                c.interactions_logged += 1;
                                let hex: String = body.iter().take(260).map(|b| format!("{:02x}", b)).collect();
                                info!("{} : transaction reçue, lue comme {:?} ({} octets) {}", addr, block_use, body.len(), hex);
                            }
                        }
                    }
                    if let Some(u) = block_use {
                        if u.action == 0 {
                            let target = bedrock::place_target(u.pos, u.face);
                            if !self.block_event("block_click", &addr, u.pos) || self.interact_cancelled(&addr, u.pos) {
                                // Refusé par un plugin : le joueur retrouve son inventaire et le bloc d'origine
                                self.send_inventory(&addr);
                                if let Some((x, y, z)) = target {
                                    let current = self.world.get(x, y, z);
                                    self.send(&addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, current));
                                }
                            } else if self.on_interact(&addr, u.pos) {
                                let now = self.tick;
                                let (first, holds_block) = match self.clients.get_mut(&addr) {
                                    Some(c) => {
                                        let first = now.saturating_sub(c.last_interact) >= 6;
                                        c.last_interact = now;
                                        (first, !c.inv.held().is_empty())
                                    }
                                    None => (false, false),
                                };
                                // Si le jeu a quand même dessiné un bloc contre celui-ci, on le
                                // lui retire : le serveur n'a rien posé
                                if let (Some(target), true) = (target, holds_block) {
                                    let current = self.world.get(target.0, target.1, target.2);
                                    self.send(&addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(target.0, target.1, target.2, current));
                                    if first {
                                        self.send_inventory(&addr);
                                    }
                                }
                            } else if let Some(target) = target {
                                // Le jeu répète le clic à chaque tick tant que le bouton est tenu.
                                // Juste après avoir ouvert une porte, le clic suivant traverse
                                // l'ouverture et viserait un autre bloc : comme dans le jeu, on
                                // ne pose rien pendant un court instant après une interaction.
                                let just_interacted = self.clients.get(&addr).map(|c| self.tick.saturating_sub(c.last_interact) < 6).unwrap_or(false);
                                if just_interacted {
                                    // Le jeu a déjà dessiné le bloc de son côté : on le lui retire
                                    self.send_inventory(&addr);
                                    let current = self.world.get(target.0, target.1, target.2);
                                    self.send(&addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(target.0, target.1, target.2, current));
                                } else {
                                    self.on_place(&addr, target, u.face, u.click);
                                }
                            }
                        } else if u.action == 1 {
                            // Clic dans le vide : un seau vide peut viser une source d'eau ou de lave
                            self.bucket_in_air(&addr);
                        }
                    }
                }
                (Stage::Spawning, bedrock::ID_SET_LOCAL_PLAYER_INITIALIZED) => {
                    let name = match self.clients.get_mut(&addr) {
                        Some(c) => {
                            c.stage = Stage::InGame;
                            c.joined = true;
                            c.name.clone().unwrap_or_default()
                        }
                        None => return,
                    };
                    if let Some(c) = self.clients.get_mut(&addr) {
                        c.probe_tick = self.tick;
                    }
                    // Chargement (ou initialisation) du joueur, puis les plugins ont la main
                    self.load_player(&addr);
                    let fresh = self.clients.get(&addr).map(|c| c.first_visit).unwrap_or(false);
                    self.emit("load", &[LVal::Player(name.clone()), LVal::Bool(fresh)]);
                    self.send_attributes(&addr, true);
                    if self.clients.get(&addr).map(|c| c.inv_touched).unwrap_or(false) {
                        self.send_inventory(&addr);
                    }
                    let shown = self.clients.get(&addr).and_then(|c| c.data.as_ref()).map(|d| d.display_name.clone()).unwrap_or_else(|| name.clone());
                    info!("{} a rejoint la partie", name);
                    let mut message = format!("§e{} a rejoint la partie", shown);
                    let out = self.fire("PlayerJoinEvent", vec![("player", Json::Str(name.clone())), ("joinMessage", Json::Str(message.clone()))]);
                    if let Some(m) = text_of(&out, "joinMessage", 512) {
                        message = m;
                    }
                    if !message.is_empty() {
                        self.broadcast(bedrock::ID_TEXT, &bedrock::text_raw(&message), None);
                    }
                    self.send_commands(&addr);
                    // Le jeu s'inscrit lui-même dans sa liste des joueurs (« Vous ») : lui envoyer sa
                    // propre entrée l'y ferait figurer deux fois
                    self.emit("join", &[LVal::Player(name)]);
                }
                _ => {}
            }
        }
    }

    /// Événements réseau RakNet : connexion, paquet, déconnexion.
    fn on_event(&mut self, ev: RakEvent) {
        match ev {
            RakEvent::Connected(addr) => {
                if self.cfg.debug {
                    info!("{} : connexion réseau établie", addr);
                }
                self.clients
                    .insert(addr, Client {
                        stage: Stage::Handshake,
                        compression: false,
                        crypt: None,
                        handshake_pending: false,
                        name: None,
                        protocol: 0,
                        connected_at: Instant::now(),
                        window: Instant::now(),
                        batches: 0,
                        bytes: 0,
                        id: self.next_id,
                        pack_responses: 0,
                        chunks_sent: false,
                        joined: false,
                        pos: self.spawn_point(),
                        center: (0, 0),
                        radius: 0,
                        loaded: HashSet::new(),
                        queue: VecDeque::new(),
                        pitch: 0.0,
                        yaw: 0.0,
                        head_yaw: 0.0,
                        moved_tick: 0,
                        shown_look: (0.0, 0.0, 0.0),
                        inputs: 0,
                        inv: PlayerWindows::new(),
                        inv_touched: false,
                        mode: GameMode::parse(&self.cfg.gamemode).unwrap_or(GameMode::Survival),
                        dead: None,
                        hurt_at: 0,
                        fall_peak: None,
                        last_toggle: 0,
                        last_interact: 0,
                        sneaking: false,
                        sprinting: false,
                        skin: None,
                        last_swing: 0,
                        shown_held: None,
                        chat_window: Instant::now(),
                        chat_count: 0,
                        interactions_logged: 0,
                        unread_logged: 0,
                        gesture_logged: 0,
                        last_input: Instant::now(),
                        spawn_pending: false,
                        chunk_next: 0,
                        chunk_sent: 0,
                        chunk_ready: std::collections::BTreeMap::new(),
                        probe: 0,
                        probe_tick: 0,
                        synced_tick: 0,
                        move_budget: 0.0,
                        move_budget_at: Instant::now(),
                        corrections: 0,
                        visible: HashSet::new(),
                        listed: HashMap::new(),
                        items_seen: HashSet::new(),
                        mobs_seen: HashSet::new(),
                        data: None,
                        first_visit: false,
                        reported_error: false,
                    });
                self.next_id += 1;
                self.emit("connect", &[LVal::Str(addr.ip().to_string())]);
            }
            RakEvent::Packet(addr, data) => self.on_packet(addr, data),
            RakEvent::Disconnected(addr, fault) => {
                let _t = timings::scope("Départ d'un joueur (sauvegarde, annonce)");
                if let Some(c) = self.clients.remove(&addr) {
                    // Mise au point : le jeu est parti de lui-même juste après l'apparition
                    // (écran d'erreur « Block ») : on passe à l'expérience suivante.
                    if self.items_level > 0
                        && fault.is_none()
                        && c.protocol >= 2225
                        && !c.inv_touched
                        && !c.reported_error
                        && c.inputs < 200
                        && (c.stage == Stage::Spawning || (c.stage == Stage::InGame && c.inputs > 0))
                    {
                        self.items_level -= 1;
                        warn!("Le jeu est parti juste après l'apparition : retour à la liste d'objets vide. Reconnectez-vous.");
                    }
                    if self.cfg.debug
                        && !self.experiment_locked
                        && fault.is_none()
                        && c.protocol >= 2225
                        && c.inputs > 0
                        && c.inputs < 400
                        && (c.stage == Stage::Spawning || c.stage == Stage::InGame)
                    {
                        warn!(
                            "{} : le jeu s'est déconnecté après {} mouvements — expérience {} ({})",
                            addr, c.inputs, self.experiment, experiment_name(self.experiment)
                        );
                        self.experiment = (self.experiment + 1) % EXPERIMENTS;
                        warn!("Prochaine connexion : expérience {} ({}).", self.experiment, experiment_name(self.experiment));
                    }
                    // Sur un serveur public, des inconnus sondent le port sans arrêt : seuls les
                    // joueurs identifiés sont signalés hors mise au point
                    if !c.joined && fault.is_none() && (self.cfg.debug || c.name.is_some()) {
                        let step = match c.stage {
                            Stage::Handshake => "avant l'annonce de sa version",
                            Stage::Login => "après les réglages réseau, avant le login",
                            Stage::Authenticating => "pendant la vérification du login",
                            Stage::Packs => "pendant l'échange des packs de ressources",
                            Stage::Spawning => "pendant le chargement du monde",
                            Stage::InGame => "en jeu",
                            Stage::Done => "après un refus du serveur",
                        };
                        info!(
                            "{} ({}) : connexion terminée {}",
                            c.name.as_deref().unwrap_or("client non identifié"),
                            addr,
                            step
                        );
                    }
                    if let Some(reason) = fault {
                        warn!(
                            "{} ({}) déconnecté par sécurité : {}",
                            c.name.as_deref().unwrap_or("client non identifié"),
                            addr,
                            reason
                        );
                    }
                    if let (true, Some(n)) = (c.joined, &c.name) {
                        self.store_player(&c);
                        let shown = c.data.as_ref().map(|d| d.display_name.clone()).unwrap_or_else(|| n.clone());
                        info!("{} a quitté la partie", n);
                        let mut message = format!("§e{} a quitté la partie", shown);
                        let reason = fault.map(|f| f.to_string()).unwrap_or_else(|| "Déconnexion".into());
                        let out = self.fire(
                            "PlayerQuitEvent",
                            vec![("player", Json::Str(n.clone())), ("quitMessage", Json::Str(message.clone())), ("quitReason", Json::Str(reason))],
                        );
                        if let Some(m) = text_of(&out, "quitMessage", 512) {
                            message = m;
                        }
                        if !message.is_empty() {
                            self.broadcast(bedrock::ID_TEXT, &bedrock::text_raw(&message), None);
                        }
                    }
                    let name = c.name.map(LVal::Str).unwrap_or(LVal::Nil);
                    self.update_pong();
                    self.emit("disconnect", &[LVal::Str(addr.ip().to_string()), name]);
                }
            }
        }
    }

    /// Commande de la console (ou d'un plugin).
    pub fn command(&mut self, line: &str) {
        let out = self.fire("ServerCommandEvent", vec![("command", Json::Str(line.trim().trim_start_matches('/').to_string()))]);
        if cancelled(&out) {
            return;
        }
        let changed = text_of(&out, "command", 1024);
        let line = changed.as_deref().unwrap_or(line);
        let line = line.trim().trim_start_matches('/');
        // « /op Enzo » est accepté comme « op Enzo » : l'habitude du jeu
        let mut parts = line.trim().trim_start_matches('/').split_whitespace();
        let name = match parts.next() {
            Some(n) => n.to_lowercase(),
            None => return,
        };
        let args: Vec<&str> = parts.collect();
        match name.as_str() {
            "stop" => {
                info!("Arrêt du serveur…");
                self.running = false;
            }
            // Essai : renvoie tel quel l'objet au sol relevé auprès du serveur officiel (un bloc de
            // terre en 0,8 -59,3 0,8). Sert à savoir si le jeu l'affiche venant de Basalt.
            "testitem" => {
                let packet = bedrock::add_item_actor_recorded();
                let targets: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined).map(|(a, _)| *a).collect();
                for a in &targets {
                    self.send(a, bedrock::ID_ADD_ITEM_ACTOR, &packet);
                }
                info!("Objet de terre du relevé envoyé à {} joueur(s), en 0.8 -59.3 0.8 (allez voir près de 0 -60 0)", targets.len());
            }
            "help" | "aide" | "?" => {
                info!("Commandes disponibles :");
                info!("  help      affiche cette aide");
                info!("  list      joueurs connectés");
                info!("  give      give <joueur> <objet> [quantité] : donne des objets");
                info!("  status    charge du serveur (ticks/s, trafic)");
                info!("  plugins   plugins chargés");
                info!("  reload    recharge les plugins");
                info!("  op        op <joueur> : donne le rang d'opérateur (deop pour le retirer, ops pour la liste)");
                info!("  whitelist whitelist on | off | add <joueur> | remove <joueur> | list | reload");
                info!("  motd      affiche ou change le nom du serveur");
                info!("  version   version du serveur");
                info!("  timings   timings on | off | reset | report : mesure où passe le temps du serveur");
                info!("  stop      arrête le serveur");
                for (n, d, p) in self.plugins.commands() {
                    info!("  {:<9} {} [{}]", n, d, p);
                }
            }
            "give" => {
                let wanted = args.get(1).copied().unwrap_or("stone");
                let count: u16 = args.get(2).and_then(|c| c.parse().ok()).unwrap_or(64).clamp(1, 2304);
                let id = match (args.is_empty(), self.item_id(wanted)) {
                    (false, Some(id)) => id,
                    _ => {
                        info!("Usage : give <joueur> <objet> [quantité] — exemple : give Alex diamond_pickaxe 1");
                        if !args.is_empty() {
                            info!("Objet inconnu : {}", wanted);
                        }
                        return;
                    }
                };
                let target = self
                    .clients
                    .iter()
                    .find(|(_, c)| c.stage == Stage::InGame && c.name.as_deref().map(|n| n.eq_ignore_ascii_case(args[0])).unwrap_or(false))
                    .map(|(a, c)| (*a, c.protocol));
                match target {
                    Some((addr, protocol)) if protocol >= 2225 => {
                        if self.add_item(&addr, id, count) {
                            info!("{} × {} donnés à {}", count, wanted, args[0]);
                        } else {
                            info!("Inventaire de {} plein : tout n'a pas pu être donné.", args[0]);
                        }
                    }
                    Some(_) => info!("Ce joueur utilise une version du jeu trop ancienne pour cette commande."),
                    None => info!("Joueur introuvable : {}", args[0]),
                }
            }
            "list" => {
                let p = self.players();
                info!("{}/{} joueur(s) : {}", p.len(), self.cfg.max_players, p.join(", "));
            }
            "status" | "tps" => self.run_vanilla(None, "status", &[]),
            "plugins" => {
                if !self.plugins.available() {
                    info!("Plugins désactivés (Lua 5.4 introuvable)");
                } else {
                    info!("{} plugin(s) :", self.plugins.catalog().plugins.len());
                    for (m, _) in &self.plugins.catalog().plugins {
                        let by = if m.authors.is_empty() { String::new() } else { format!(" par {}", m.authors.join(", ")) };
                        let about = if m.description.is_empty() { String::new() } else { format!(" — {}", m.description) };
                        info!("  {} v{}{}{}", m.name, m.version, by, about);
                    }
                }
            }
            "op" | "deop" => {
                let grant = name == "op";
                match args.first() {
                    Some(player) => self.set_op(player, grant),
                    None => info!("Usage : {} <joueur>", if grant { "op" } else { "deop" }),
                }
            }
            "whitelist" => self.whitelist_command(&args),
            "ops" => {
                let list = self.ops.list();
                info!("{} opérateur(s) : {}", list.len(), list.join(", "));
            }
            "reload" => {
                self.plugins.unload_all();
                self.npcs.clear();
                self.load_plugins();
                info!("Plugins rechargés");
            }
            "motd" => {
                if !args.is_empty() {
                    self.motd = crate::log::clean(&args.join(" "), 64);
                    self.update_pong();
                }
                info!("Nom du serveur : {}", self.motd);
            }
            "version" | "ver" | "about" => {
                // Même contenu que /version en jeu, puis le détail des protocoles
                self.run_vanilla(None, "version", &[]);
                info!("Version annoncée aux jeux : Bedrock {}", self.cfg.game_version);
                for (p, n) in crate::versions::KNOWN {
                    if *p >= self.cfg.min_protocol && *p <= self.cfg.max_protocol {
                        info!("  {} = Bedrock {}", p, n);
                    }
                }
            }
            other if VanillaCommands::get(other).is_some() => {
                let words = crate::command::tokens(line.trim().trim_start_matches('/'));
                self.run_vanilla(None, other, &words[1.min(words.len())..]);
            }
            other => {
                let info = self.info();
                self.plugins.set_info(&info, self.tick);
                if !self.plugins.run_command(other, &args) {
                    warn!("Commande inconnue : {} (tapez « help »)", other);
                }
                self.apply_actions();
            }
        }
    }

    /// Ouvre la signalisation HTTP de NetherNet sur le port TCP du serveur.
    fn start_signaling(&mut self) {
        if !self.cfg.http_signaling {
            return;
        }
        let port = match self.local_addr() {
            Ok(a) => a.port(),
            Err(_) => return,
        };
        match Signaling::bind(&self.cfg.ip, port) {
            Ok(mut sig) => {
                sig.debug = self.cfg.debug;
                info!("Signalisation NetherNet active sur le port {} (TCP)", port);
                self.signaling = Some(sig);
                self.start_nethernet();
            }
            Err(e) => warn!(
                "Signalisation NetherNet impossible sur le port TCP {} ({}) : les jeux récents ne verront pas le serveur",
                port, e
            ),
        }
    }

    /// Ouvre la liaison WebRTC de NetherNet, par laquelle passent les jeux récents.
    fn start_nethernet(&mut self) {
        if !NetherNet::AVAILABLE {
            warn!("Basalt est compilé sans le transport NetherNet : les jeux 1.26.60 et plus ne pourront pas entrer");
            return;
        }
        let c = &self.cfg;
        match NetherNet::bind(&c.ip, c.nethernet_udp_port, &c.public_ip, &c.identity_domain, c.identity_cpk_jwk) {
            Ok(mut n) => {
                n.max_peers = c.max_players as usize + 32;
                n.debug = c.debug;
                info!(
                    "Transport NetherNet prêt (UDP, port {}) — empreinte de la clé du serveur : {}",
                    c.nethernet_udp_port,
                    n.key_fingerprint()
                );
                // La liaison WebRTC tourne sur son propre fil (voir nethernet_thread)
                self.nether = Some(crate::network::nethernet_thread::NetherNetHandle::start(n));
            }
            Err(e) => error!("Transport NetherNet indisponible : {}", e),
        }
    }

    /// Envoi par RakNet ou NetherNet, selon la connexion du joueur.
    fn net_send(&mut self, addr: &SocketAddr, packet: &[u8]) {
        // Session chiffrée : le lot (après l'octet 0xfe) reçoit sa somme de contrôle puis est
        // chiffré. Tous les envois passent par ici, dans l'ordre : le flux reste cohérent.
        let sealed = match self.clients.get_mut(addr).and_then(|c| c.crypt.as_mut()) {
            Some(crypt) if packet.first() == Some(&0xfe) => {
                let mut out = Vec::with_capacity(packet.len() + 9);
                out.push(0xfe);
                out.extend_from_slice(&crypt.seal(&packet[1..]));
                Some(out)
            }
            _ => None,
        };
        let packet: &[u8] = sealed.as_deref().unwrap_or(packet);
        match self.nether.as_mut() {
            // NetherNet transporte le même contenu que RakNet, sans l'octet 0xfe de tête
            Some(n) if n.has(addr) => n.send(addr, &packet[1..]),
            _ => self.rak.send(addr, packet),
        }
    }

    /// Fermeture, coupure et limites de jeu : même chose, sur le bon réseau.
    fn net_close(&mut self, addr: &SocketAddr, delay: Duration) {
        match self.nether.as_mut() {
            Some(n) if n.has(addr) => n.close(addr, delay),
            _ => self.rak.close(addr, delay),
        }
    }

    fn net_abort(&mut self, addr: &SocketAddr, reason: &'static str) {
        match self.nether.as_mut() {
            Some(n) if n.has(addr) => n.abort(addr, reason),
            _ => self.rak.abort(addr, reason),
        }
    }

    fn net_set_game_limits(&mut self, addr: &SocketAddr) {
        match self.nether.as_mut() {
            Some(n) if n.has(addr) => n.set_game_limits(addr),
            _ => self.rak.set_game_limits(addr),
        }
    }

    /// État initial du joueur (vie, capacités, inventaire vide…), envoyé avant le terrain.
    fn send_player_state(&mut self, addr: &SocketAddr, features: crate::versions::Features) {
        if !features.editor_migration_version {
            return; // formats vérifiés uniquement pour les jeux 1.26.60 et plus
        }
        let id = match self.clients.get(addr) {
            Some(c) => c.id,
            None => return,
        };
        let mode = self.clients.get(addr).map(|c| c.mode).unwrap_or(GameMode::Survival);
        // `None` : paquet relevé auprès du serveur officiel, envoyé déjà compressé
        let list: Vec<(u32, Option<Vec<u8>>)> = vec![
            (bedrock::ID_UPDATE_ATTRIBUTES, Some(bedrock::update_attributes(id as u64))),
            (bedrock::ID_UPDATE_ABILITIES, Some(bedrock::update_abilities(id, mode))),
            (bedrock::ID_SET_ACTOR_DATA, Some(bedrock::set_actor_data_player(id as u64))),
            (bedrock::ID_SET_TIME, Some(bedrock::set_time(self.world_time() as i32))),
            (bedrock::ID_CREATIVE_CONTENT, if self.has_captured(addr, bedrock::ID_CREATIVE_CONTENT) { None } else { Some(bedrock::creative_content_empty()) }),
            (bedrock::ID_INVENTORY_CONTENT, Some(bedrock::inventory_content_empty(0, 36, features))),
            (bedrock::ID_PLAYER_HOTBAR, Some(bedrock::player_hotbar(0))),
            (bedrock::ID_SET_PLAYER_GAME_TYPE, Some(bedrock::set_time(mode.bedrock_id()))),
            (bedrock::ID_SET_DIFFICULTY, Some(vec![1])),
            (bedrock::ID_SET_COMMANDS_ENABLED, Some(vec![1])),
            (bedrock::ID_UPDATE_ADVENTURE_SETTINGS, Some(bedrock::update_adventure_settings())),
            (bedrock::ID_GAME_RULES_CHANGED, Some(vec![0])),
        ];
        let mut list = list;
        // Liste complète des entités, d'après les données du jeu
        if self.has_captured(addr, bedrock::ID_AVAILABLE_ACTOR_IDENTIFIERS) {
            list.push((bedrock::ID_AVAILABLE_ACTOR_IDENTIFIERS, None));
        } else if let (Some(ids), 2) = (self.data.as_ref().and_then(|d| d.entity_identifiers.clone()), self.items_level) {
            list.push((bedrock::ID_AVAILABLE_ACTOR_IDENTIFIERS, Some(ids)));
        }
        // Recettes et ornements d'armure, tels que le serveur officiel les envoie
        for id in [52u32, 302] {
            if self.has_captured(addr, id) {
                list.push((id, None));
            }
        }
        for (packet, body) in list {
            if !self.refused_extras.contains(&packet) {
                match body {
                    Some(body) => self.send(addr, packet, &body),
                    None => {
                        self.send_captured(addr, packet);
                    }
                }
            }
        }
    }

    /// Demande d'entrée d'un joueur : vérifie son identité, puis ouvre la liaison WebRTC.
    fn on_join_request(&mut self, req: Request) {
        let offer = match std::str::from_utf8(&req.body) {
            Ok(text) => text.to_string(),
            Err(_) => return req.respond(400, "text/plain", b"invalid offer"),
        };
        if self.cfg.debug {
            let _ = std::fs::create_dir_all("logs");
            let _ = std::fs::write("logs/last-offer.sdp", &req.body);
        }
        if self.nether.is_none() {
            info!("{} : demande de connexion NetherNet refusée (transport indisponible)", req.addr);
            return req.respond(503, "text/plain", b"NetherNet transport not available");
        }
        // Limite de connexions par adresse, comme pour RakNet
        let same_ip = self.clients.keys().filter(|a| a.ip() == req.addr.ip()).count();
        if same_ip >= self.cfg.max_per_ip {
            return req.respond(503, "text/plain", b"too many connections");
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Identité : le jeton Xbox et la signature qui le lie à la liaison chiffrée
        let mut cpk = None;
        if self.cfg.xbox_auth {
            let keys = match &self.auth_keys {
                Some(k) => k.clone(),
                None => return req.respond(503, "text/plain", b"authentication keys not ready"),
            };
            let identity = match sdp_identity(&offer) {
                Some(i) => i,
                None => {
                    warn!("{} : offre sans identité Xbox, refusée", req.addr);
                    return req.respond(403, "text/plain", b"identity required");
                }
            };
            match auth::verify_offer(identity, &sdp_fingerprints(&offer), &keys, now) {
                Ok(id) => {
                    if self.cfg.debug {
                        info!("[nethernet] {} : identité de {} vérifiée", req.addr, id.name);
                    }
                    cpk = Some(id.cpk);
                }
                Err(AuthError::UnknownKey) => {
                    if let Some(f) = &self.key_fetcher {
                        f.refresh();
                    }
                    warn!("{} : jeton signé par une clé inconnue, mise à jour des clés demandée", req.addr);
                    return req.respond(503, "text/plain", b"authentication keys not ready");
                }
                Err(e) => {
                    warn!("{} : identité de l'offre refusée ({:?})", req.addr, e);
                    return req.respond(403, "text/plain", b"identity rejected");
                }
            }
        }
        let local_ip = match req.local_ip {
            Some(ip) => ip,
            None => return req.respond(503, "text/plain", b"no local address"),
        };
        let clean = sdp_without_identity(&offer);
        let result = match self.nether.as_mut() {
            Some(n) => n.accept(req.addr, local_ip, &clean, cpk, now),
            None => Err((503, "NetherNet transport not available")),
        };
        match result {
            // La session se prépare sur un fil à part : la réponse partira de `answer_joins`
            Ok(()) => {
                self.join_waiting.insert(req.addr, req);
            }
            Err((status, why)) => {
                warn!("{} : demande de connexion NetherNet refusée ({})", req.addr, why);
                req.respond(status, "text/plain", why.as_bytes());
            }
        }
    }

    /// Répond aux demandes d'entrée dont la session WebRTC est prête.
    fn answer_joins(&mut self) {
        let done = match self.nether.as_mut() {
            Some(n) => n.prepared(),
            None => return,
        };
        for (key, result) in done {
            let req = match self.join_waiting.remove(&key) {
                Some(r) => r,
                None => continue,
            };
            match result {
                Ok(answer) => {
                    info!("{} : demande de connexion NetherNet acceptée, liaison en cours", key);
                    if self.cfg.debug {
                        let _ = std::fs::write("logs/last-answer.sdp", answer.as_bytes());
                    }
                    req.respond(200, "application/sdp", answer.as_bytes());
                }
                Err((status, why)) => {
                    warn!("{} : demande de connexion NetherNet refusée ({})", key, why);
                    req.respond(status, "text/plain", why.as_bytes());
                }
            }
        }
    }

    /// Traite les requêtes HTTP du jeu : informations du serveur et demandes de connexion.
    fn poll_signaling(&mut self) {
        let requests = match self.signaling.as_mut() {
            Some(sig) => sig.poll(),
            None => return,
        };
        for req in requests {
            if self.cfg.debug {
                info!(
                    "[signalisation] {} : {} {}{}{}",
                    req.addr,
                    crate::log::clean(&req.method, 8),
                    crate::log::clean(&req.path, 80),
                    if req.query.is_empty() { "" } else { "?" },
                    crate::log::clean(&req.query, 120)
                );
            }
            let join = req.path == "/v1/join" || req.path == "/v1/join/";
            if join && req.method == "GET" {
                // Le jeu annonce sa version : on lui répond dans la sienne si elle est acceptée.
                let asked: Option<i32> = req.param("protocol").and_then(|p| p.parse().ok());
                let (protocol, version) = match asked {
                    Some(p) if p >= self.cfg.min_protocol && p <= self.cfg.max_protocol => {
                        let v = req.param("version").unwrap_or("");
                        let sane = !v.is_empty() && v.len() <= 16 && v.bytes().all(|b| b.is_ascii_digit() || b == b'.');
                        (p, if sane { v.to_string() } else { self.cfg.game_version.clone() })
                    }
                    _ => (self.cfg.protocol, self.cfg.game_version.clone()),
                };
                let body = format!(
                    "{{\"name\":{},\"protocol\":{},\"version\":{},\"level\":{},\"players\":{},\"maxPlayers\":{},\"gameType\":{}}}",
                    json_string(&self.motd),
                    protocol,
                    json_string(&version),
                    json_string(&self.cfg.world_name),
                    self.players().len(),
                    self.cfg.max_players,
                    GameMode::parse(&self.cfg.gamemode).unwrap_or(GameMode::Survival).bedrock_id()
                );
                req.respond(200, "application/json", body.as_bytes());
            } else if req.path.starts_with("/v1/join/") && req.method == "POST" {
                self.on_join_request(req);
            } else if join || req.path.starts_with("/v1/join/") {
                req.respond(405, "text/plain", b"");
            } else {
                req.respond(404, "text/plain", b"");
            }
        }
    }

    /// Auto-test au démarrage : un ping envoyé au serveur depuis cette machine.
    /// S'il obtient une réponse, le serveur reçoit et émet correctement ; si un jeu
    /// lancé sur le même ordinateur ne le joint pas, la cause est donc ailleurs.
    fn self_test(&self) {
        let port = match self.local_addr() {
            Ok(a) => a.port(),
            Err(_) => return,
        };
        let debug = self.cfg.debug;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            let run = || -> std::io::Result<bool> {
                let sock = std::net::UdpSocket::bind("127.0.0.1:0")?;
                sock.set_read_timeout(Some(Duration::from_secs(2)))?;
                let mut ping = vec![0x01];
                ping.extend_from_slice(&0i64.to_be_bytes());
                ping.extend_from_slice(&crate::raknet::MAGIC);
                ping.extend_from_slice(&0i64.to_be_bytes());
                sock.send_to(&ping, ("127.0.0.1", port))?;
                let mut buf = [0u8; 1024];
                let (n, _) = sock.recv_from(&mut buf)?;
                Ok(n > 0 && buf[0] == 0x1c)
            };
            match run() {
                Ok(true) if !debug => {}
                Ok(true) => info!("Auto-test réseau réussi : le serveur répond sur 127.0.0.1:{}", port),
                Ok(false) => warn!("Auto-test réseau : réponse inattendue sur 127.0.0.1:{}", port),
                Err(e) => warn!(
                    "Auto-test réseau ÉCHOUÉ sur 127.0.0.1:{} ({}) : le serveur ne reçoit pas les paquets de cette machine",
                    port, e
                ),
            }
        });
    }

    /// Tient les statistiques de charge et avertit si le serveur ne suit plus.
    fn measure(&mut self, now: Instant, late: Duration) {
        let p = &mut self.perf;
        p.ticks += 1;
        p.worst_late = p.worst_late.max(late);
        let elapsed = now.duration_since(p.window);
        if elapsed < Duration::from_secs(5) {
            return;
        }
        let secs = elapsed.as_secs_f32();
        p.last = (p.ticks as f32 / secs, p.worst_late);
        let t = self.rak.traffic;
        let per = |a: u64, b: u64| ((a - b) as f32 / secs) as u64;
        p.rates = (
            per(t.datagrams_in, p.traffic.datagrams_in),
            per(t.datagrams_out, p.traffic.datagrams_out),
            per(t.bytes_in, p.traffic.bytes_in),
            per(t.bytes_out, p.traffic.bytes_out),
        );
        p.traffic = t;
        p.window = now;
        p.ticks = 0;
        p.worst_late = Duration::from_millis(0);
        let quiet = p.warned.map(|w| now.duration_since(w) > Duration::from_secs(30)).unwrap_or(true);
        if p.last.0 < 18.0 && quiet {
            p.warned = Some(now);
            warn!(
                "Serveur en surcharge : {:.1} ticks/s au lieu de 20 (pire retard {} ms, {} sessions)",
                p.last.0,
                p.last.1.as_millis(),
                self.rak.sessions.len()
            );
        }
    }

    /// Paquet relevé auprès du serveur officiel pour la version de ce joueur, s'il existe
    /// et que le jeu ne l'a pas refusé.
    /// Envoie un paquet relevé auprès du serveur officiel, déjà compressé s'il est gros.
    /// Faux s'il n'y en a pas pour la version du joueur.
    fn send_captured(&mut self, addr: &SocketAddr, id: u32) -> bool {
        if self.items_level != 2 || self.refused_captured.contains(&id) {
            return false;
        }
        let (protocol, compression) = match self.clients.get(addr) {
            Some(c) => (c.protocol, c.compression),
            None => return false,
        };
        if compression {
            if let Some(ready) = self.captured_ready.get(&(protocol, id)).cloned() {
                if self.cfg.debug {
                    info!("[paquets] -> {} : id {} (relevé, {} octets compressés)", addr, id, ready.len());
                }
                self.net_send(addr, &ready);
                return true;
            }
        }
        match self.captured(addr, id) {
            Some(body) => {
                self.send(addr, id, &body);
                true
            }
            None => false,
        }
    }

    /// Ce paquet relevé sur un vrai serveur peut-il être envoyé à ce joueur ?
    fn has_captured(&self, addr: &SocketAddr, id: u32) -> bool {
        self.items_level == 2
            && !self.refused_captured.contains(&id)
            && self.clients.get(addr).and_then(|c| self.captured.get(&c.protocol)).map(|m| m.contains_key(&id)).unwrap_or(false)
    }

    /// Paquet relevé, si le jeu ne l'a pas refusé et que la version du joueur correspond.
    fn captured(&self, addr: &SocketAddr, id: u32) -> Option<Vec<u8>> {
        if self.items_level != 2 || self.refused_captured.contains(&id) {
            return None;
        }
        let protocol = self.clients.get(addr)?.protocol;
        self.captured.get(&protocol)?.get(&id).cloned()
    }

    /// Charge `data/<protocole>/packets/*.bin`.
    fn load_captured(&mut self, dir: &std::path::Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let protocol: i32 = match entry.file_name().to_string_lossy().parse() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let mut found = HashMap::new();
            for (id, name) in crate::tools::capture::WANTED.iter() {
                if *id == bedrock::ID_START_GAME || *id == bedrock::ID_AVAILABLE_COMMANDS || *id == bedrock::ID_ADD_PLAYER || *id == 63 || *id == bedrock::ID_ADD_ITEM_ACTOR || *id == 19 || *id == 161 {
                    continue; // propre au monde du serveur officiel
                }
                if let Ok(body) = std::fs::read(entry.path().join("packets").join(format!("{}.bin", name))) {
                    if !body.is_empty() && body.len() < 64 * 1024 * 1024 {
                        found.insert(*id, body);
                    }
                }
            }
            if !found.is_empty() {
                info!("Données relevées auprès du serveur officiel pour le protocole {} : {} paquets (recettes, objets, biomes…)", protocol, found.len());
                // Compressés une fois pour toutes (plus d'un mégaoctet par connexion sinon)
                if self.cfg.network_compression {
                    for (id, body) in &found {
                        if body.len() >= CACHE_FROM {
                            self.captured_ready.insert((protocol, *id), Arc::new(bedrock::encode_packet_with(*id, body, true, crate::utils::deflate::Level::Best)));
                        }
                    }
                }
                self.captured.insert(protocol, found);
            }
        }
    }

    /// Enregistre le monde. Le document est préparé ici ; la compression et l'écriture se
    /// font sur le fil des écritures, sans retenir les joueurs.
    fn save_world(&mut self) {
        let dir = std::path::PathBuf::from(&self.cfg.world_dir);
        // Objets au sol (petit fichier, réécrit à chaque enregistrement)
        if let Some(text) = self.ground_items_json() {
            let path = dir.join("items.json");
            crate::background::DISK.run(move || {
                let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
                let tmp = path.with_extension("json.tmp");
                if let Err(e) = std::fs::write(&tmp, text.as_bytes()).and_then(|_| std::fs::rename(&tmp, &path)) {
                    warn!("Objets au sol non enregistrés : {}", e);
                }
            });
        }
        // Animaux (réécrits quand ils ont changé)
        if self.mobs_dirty {
            self.mobs_dirty = false;
            let text = self.mobs_json();
            let path = dir.join("mobs.json");
            crate::background::DISK.run(move || {
                let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
                let tmp = path.with_extension("json.tmp");
                if let Err(e) = std::fs::write(&tmp, text.as_bytes()).and_then(|_| std::fs::rename(&tmp, &path)) {
                    warn!("Animaux non enregistrés : {}", e);
                }
            });
        }
        if let (true, Some(d)) = (self.world.tiles.dirty, &self.data) {
            let ctx = ItemCatalog { data: d, next: &mut self.tile_net_id };
            let doc = self.world.tiles.snapshot(&ctx);
            let path = dir.join("tiles.nbt");
            crate::background::DISK.run(move || {
                if let Err(e) = crate::nbt::save_file(&path, &doc) {
                    warn!("Contenu des coffres et des fours non enregistré : {}", e);
                }
            });
        }
        if !self.world.dirty {
            return;
        }
        // Un enregistrement précédent encore en cours (disque très lent) : on attend le suivant
        // plutôt que d'empiler des copies du monde en mémoire
        if self.running && crate::background::DISK.pending() > 4 {
            return;
        }
        let doc = self.world.snapshot();
        let (count, announce) = (self.world.edit_count(), self.cfg.debug || !self.running);
        crate::background::DISK.run(move || match crate::world::World::write_snapshot(&dir, &doc) {
            Ok(()) if announce => info!("Monde enregistré ({} blocs modifiés)", count),
            Ok(()) => {}
            Err(e) => warn!("Enregistrement du monde impossible dans {} : {}", dir.display(), e),
        });
    }

    /// Fait tourner le serveur jusqu'à `stop`, la commande `stop` ou un plugin.
    pub fn run(&mut self, stop: &AtomicBool, console: &Receiver<String>) {
        self.load_plugins();
        if let Ok(a) = self.local_addr() {
            info!("Serveur prêt sur {} (UDP) — tapez « help » pour l'aide", a);
            if self.rak.has_v6() {
                info!("Écoute IPv6 active sur le port {}", self.cfg.port6);
            }
        }
        match GameData::load(std::path::Path::new("data")) {
            Ok(d) => {
                info!("Données du jeu chargées : {} objets, {} blocs ({} états)", d.items.len(), d.block_count(), d.state_count());
                self.blocks = Some(d.block_registry());
                self.plugin_blocks = Some(Arc::new(d.block_names_for_plugins()));
                self.data = Some(Arc::new(d));
                self.items_level = 2;
            }
            Err(e) => warn!("Données du jeu absentes ({}) : seuls quelques blocs sont disponibles. Placez le dossier « data » à côté du serveur.", e),
        }
        self.load_captured(std::path::Path::new("data"));
        self.load_ops();
        self.open_world();
        if let Some(d) = &self.data {
            let mut ctx = ItemCatalog { data: d, next: &mut self.tile_net_id };
            match self.world.tiles.load(std::path::Path::new(&self.cfg.world_dir), &mut ctx) {
                Ok(0) => {}
                Ok(n) => info!("Coffres, fours et panneaux rechargés : {}", n),
                Err(e) => warn!("Contenu des coffres et des fours non rechargé : {}", e),
            }
        }
        self.load_ground_items();
        self.load_mobs();
        self.self_test();
        self.start_signaling();
        let mut events = Vec::new();
        let mut next_tick = Instant::now() + TICK;
        while self.running && !stop.load(Ordering::Relaxed) {
            let read = {
                let mut read = {
                    let _t = timings::scope("Réseau — lecture RakNet");
                    self.rak.poll(&mut events)
                };
                if let Some(n) = self.nether.as_mut() {
                    let _t = timings::scope("Réseau — lecture NetherNet");
                    read += n.poll(&mut events);
                }
                read
            };
            let _received = if events.is_empty() { None } else { Some(timings::scope("Paquets reçus — traitement")) };
            for ev in events.drain(..).collect::<Vec<_>>() {
                let addr = match &ev {
                    RakEvent::Connected(a) | RakEvent::Packet(a, _) | RakEvent::Disconnected(a, _) => *a,
                };
                // Filet de sécurité : un bug déclenché par un paquet ne ferme que ce joueur.
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.on_event(ev)));
                if res.is_err() {
                    error!("Erreur interne en traitant un paquet de {} : joueur déconnecté", addr);
                    self.violation(&addr, "erreur interne");
                }
            }
            drop(_received);
            {
                let _t = timings::scope("Logins — vérification");
                self.poll_logins();
            }
            while let Ok(line) = console.try_recv() {
                let _t = timings::scope("Console");
                self.command(&line);
            }
            let now = Instant::now();
            if now >= next_tick {
                let _tick = timings::scope("Tick complet");
                timings::tick();
                self.measure(now, now.duration_since(next_tick));
                self.tick += 1;
                next_tick += TICK;
                if now.duration_since(self.last_save) > Duration::from_secs(60) {
                    let _t = timings::scope("Tick — sauvegarde du monde et des joueurs");
                    self.last_save = now;
                    self.save_world();
                    self.save_players();
                }
                // Retard de quelques ticks (pause du système, pic de travail) : les ticks manqués
                // sont rattrapés aussitôt, l'un après l'autre, comme dans le jeu. Au-delà d'une
                // seconde de retard, on repart de maintenant plutôt que d'enchaîner les ticks.
                if next_tick + MAX_CATCH_UP < now {
                    next_tick = now + TICK;
                }
                let info = self.info();
                self.plugins.set_info(&info, self.tick);
                if let Some(d) = &self.data {
                    // Les fours cuisent, qu'un joueur les regarde ou non
                    let _t = timings::scope("Tick — blocs à données (fours…)");
                    let mut ctx = ItemCatalog { data: d, next: &mut self.tile_net_id };
                    self.world.tiles.tick(&mut ctx);
                }
                if !self.ground.is_empty() {
                    let _t = timings::scope("Tick — objets au sol (physique)");
                    self.move_ground_items();
                }
                if self.tick % 2 == 0 {
                    let _t = timings::scope("Tick — objets au sol (ramassage, fusion)");
                    self.update_ground_items();
                }
                if self.tick % 10 == 0 && !self.ground.is_empty() {
                    let _t = timings::scope("Tick — objets au sol (affichage)");
                    self.show_ground_items();
                }
                if self.tick % 80 == 0 {
                    let _t = timings::scope("Tick — vie regagnée");
                    self.regenerate();
                }
                {
                    let _t = timings::scope("Tick — eau et lave");
                    self.flow_liquids();
                }
                if !self.npcs.is_empty() {
                    let _t = timings::scope("Tick — PNJ des plugins");
                    self.move_npcs();
                }
                {
                    let _t = timings::scope("Tick — animaux");
                    if self.tick % 20 == 7 {
                        self.populate_chunks();
                    }
                    if !self.mobs.is_empty() {
                        self.move_mobs();
                    }
                    if self.tick % 10 == 3 {
                        self.show_mobs();
                    }
                }
                // Résultats des recherches /locate
                while let Ok((to, message)) = self.locate_rx.try_recv() {
                    if to.map(|a| self.clients.contains_key(&a)).unwrap_or(true) {
                        self.reply(to, &message);
                    }
                }
                if self.tick % 4 == 0 {
                    let _t = timings::scope("Tick — écrans ouverts (coffres, fours)");
                    self.watch_containers();
                }
                self.release_buttons();
                {
                    let _t = timings::scope("Tick — minuteries des plugins");
                    self.plugins.run_timers(self.tick);
                }
                {
                    let _t = timings::scope("Tick — actions demandées par les plugins");
                    self.apply_actions();
                }
                {
                    let _t = timings::scope("Tick — envoi du terrain (chunks)");
                    self.send_queued_chunks();
                }
                {
                    let _t = timings::scope("Tick — surveillance des connexions");
                    self.watch_freezes();
                    self.run_probes();
                }
                {
                    let _t = timings::scope("Tick — joueurs vus par les autres");
                    self.broadcast_players();
                }
                {
                    let _t = timings::scope("Tick — signalisation NetherNet");
                    self.poll_signaling();
                    self.answer_joins();
                }
                if self.tick % 20 == 0 {
                    let _t = timings::scope("Tick — clés Xbox et délais de connexion");
                    self.poll_auth();
                    self.check_deadlines();
                }
            }
            {
                let _t = timings::scope("Réseau — envoi");
                self.rak.flush();
                if let Some(n) = self.nether.as_mut() {
                    n.flush(&mut events);
                }
            }
            if read == 0 {
                // Rien à lire : on rend la main au système au lieu de tourner à vide.
                // Cette pause est mesurée : si le système met bien plus d'une milliseconde à
                // rendre la main, c'est la machine (processeur partagé) qui cause les à-coups.
                let _t = timings::scope("Pause de 1 ms (au-delà : la machine fait attendre)");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let addrs: Vec<SocketAddr> = self.clients.keys().copied().collect();
        for a in addrs {
            self.send(&a, bedrock::ID_DISCONNECT, &bedrock::disconnect("Serveur arrêté."));
        }
        self.save_world();
        self.save_players();
        // Tout ce qui attend d'être écrit sur le disque l'est avant de s'arrêter
        if !crate::background::DISK.flush(Duration::from_secs(60)) {
            warn!("Certaines sauvegardes n'ont pas pu être terminées à temps");
        }
        self.plugins.unload_all();
        self.rak.shutdown();
        if let Some(n) = self.nether.as_mut() {
            n.shutdown();
        }
        info!("Serveur arrêté");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_hurt_like_in_the_game() {
        // Un demi-cœur par bloc au-delà de 3 blocs
        assert_eq!(fall_damage(2.0), 0.0);
        assert_eq!(fall_damage(3.0), 0.0);
        assert_eq!(fall_damage(3.4), 1.0);
        assert_eq!(fall_damage(10.0), 7.0);
        assert_eq!(fall_damage(23.0), 20.0);
    }
}

/// Blocs emportés par l'eau ou brûlés par la lave qui coule : herbes, fleurs, torches,
/// pousses, cultures, rails, poudre de redstone…
/// Codes de couleur tapés avec & (« &aBonjour ») -> codes du jeu (« §aBonjour »). Seuls
/// les codes connus, en minuscules, sont convertis : « R&D » reste tel quel.
fn ampersand_colors(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if let Some(&next) = chars.peek() {
                if "0123456789abcdefghijklmnopqrstuv".contains(next) {
                    out.push('§');
                    out.push(next);
                    chars.next();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// Hauteur des yeux d'un joueur au-dessus de ses pieds.
const EYE: f32 = crate::player::EYE_HEIGHT;

/// Personnage non joueur créé par un plugin : vu par les joueurs comme un joueur.
#[derive(Clone)]
struct Npc {
    id: i64,
    owner: String,
    name: String,
    /// Position des pieds
    pos: (f32, f32, f32),
    /// Déplacement par tick sur x et z
    velocity: (f32, f32),
    /// Vitesse de chute (blocs par tick)
    fall: f32,
    yaw: f32,
    moved_tick: u64,
}

fn washed_away(name: &str) -> bool {
    let n = name.trim_start_matches("minecraft:");
    if n.contains("seagrass") || n.contains("kelp") || n.contains("coral") || n == "sea_pickle" {
        return false; // plantes aquatiques : déjà dans l'eau
    }
    const PARTS: [&str; 16] = ["flower", "sapling", "torch", "rail", "short_grass", "tall_grass", "fern", "deadbush", "dead_bush", "redstone_wire", "tripwire", "tulip", "mushroom", "petals", "leaf_litter", "dry_grass"];
    PARTS.iter().any(|p| n.contains(p))
        || matches!(
            n,
            "dandelion" | "poppy" | "allium" | "azure_bluet" | "oxeye_daisy" | "cornflower" | "lily_of_the_valley" | "wither_rose" | "wheat" | "carrots" | "potatoes"
                | "beetroot" | "snow_layer" | "web" | "bush" | "firefly_bush" | "wildflowers" | "sweet_berry_bush" | "lever" | "unpowered_repeater" | "unpowered_comparator"
        )
}

/// Paquets à partir de cette taille : compressés une fois et gardés (voir `Server::send`).
const CACHE_FROM: usize = 16 * 1024;

/// Empreinte rapide d'un contenu (FNV-1a sur des mots de 8 octets) : assez pour
/// reconnaître un paquet déjà compressé, avec sa taille et son identifiant.
fn quick_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut chunks = data.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])).wrapping_mul(0x0100_0000_01b3);
    }
    for &b in chunks.remainder() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    h
}
