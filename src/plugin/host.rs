//! Les plugins tournent sur leur propre fil d'exécution, à l'écart de la boucle du serveur.
//!
//! Le serveur envoie ses demandes (événements, minuteries, commandes) dans une file et
//! n'attend la réponse qu'un instant : un plugin en règle répond en quelques microsecondes
//! et tout se passe comme s'il était appelé directement. S'il traîne ou reste coincé, le
//! serveur continue sans lui ; ses effets seront appliqués quand il aura fini, et les
//! demandes en trop sont abandonnées plutôt qu'accumulées sans fin.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use crate::lua::{self, Action, Info, LVal, PluginManager};
use crate::plugin::manifest::{Manifest, PermDefault};

/// Attente maximale du serveur pour un événement ordinaire, et pour un événement dont il
/// lit la réponse (acceptation d'un joueur).
const WAIT: Duration = Duration::from_millis(25);
const WAIT_ANSWER: Duration = Duration::from_millis(150);
/// Attente au chargement et au déchargement (console, démarrage, arrêt).
const WAIT_LOAD: Duration = Duration::from_secs(10);
const WAIT_UNLOAD: Duration = Duration::from_secs(2);
/// Demandes en attente tolérées avant d'en abandonner.
const QUEUE: usize = 1024;

/// Ce que le serveur sait des plugins chargés, sans toucher à leur fil.
#[derive(Clone, Default)]
pub struct Catalog {
    pub available: bool,
    /// (fiche, protégé)
    pub plugins: Vec<(Manifest, bool)>,
    /// (nom, description, plugin)
    pub commands: Vec<(String, String, String)>,
    pub permissions: Vec<(String, PermDefault)>,
    /// (commande, permission exigée)
    pub command_permissions: Vec<(String, String)>,
    /// (commande, description, usage) d'après les plugin.yml
    pub command_decls: Vec<(String, String, String)>,
}

enum Job {
    Load(Info, Vec<(&'static str, LVal)>),
    Event(String, Vec<LVal>, Info, u64),
    /// Événement à la PocketMine : nom, état JSON
    Fire(String, String, Info, u64),
    Timers(Info, u64),
    Command(String, Vec<String>, Option<String>, Info, u64),
    Unload,
    Quit,
}

struct Done {
    id: u64,
    results: Vec<LVal>,
    actions: Vec<Action>,
    catalog: Option<Catalog>,
    /// Événements écoutés après ce travail
    listening: Vec<String>,
}

pub struct PluginHost {
    tx: SyncSender<(u64, Job)>,
    rx: Receiver<Done>,
    next_id: u64,
    /// Demandes envoyées dont la fin n'a pas encore été reçue
    pending: usize,
    /// Depuis quand le fil des plugins est en retard
    late_since: Option<Instant>,
    last_warning: Option<Instant>,
    dropped: u64,
    catalog: Catalog,
    actions: Vec<Action>,
    info: Info,
    tick: u64,
    /// Gestionnaires enregistrés (« nom » ou « nom#PRIORITÉ »)
    listening: Vec<String>,
}

/// Ce que le serveur peut savoir des plugins sans attendre leur fil.
fn catalog_of(m: &PluginManager) -> Catalog {
    Catalog {
        available: m.available(),
        plugins: m.plugins.iter().map(|p| (p.manifest.clone(), p.sandboxed())).collect(),
        commands: m.commands(),
        permissions: m.permissions(),
        command_permissions: m
            .plugins
            .iter()
            .flat_map(|p| p.manifest.commands.iter())
            .filter_map(|c| c.permission.clone().map(|p| (c.name.clone(), p)))
            .collect(),
        command_decls: m.plugins.iter().flat_map(|p| p.manifest.commands.iter()).map(|c| (c.name.clone(), c.description.clone(), c.usage.clone())).collect(),
    }
}

/// Fil des plugins : il charge les plugins puis traite les demandes du serveur une par une.
fn worker(dir: PathBuf, sandbox: bool, trusted: String, memory_mb: usize, jobs: Receiver<(u64, Job)>, done: mpsc::Sender<Done>) {
    let mut manager = PluginManager::new(&dir);
    manager.set_sandbox(sandbox, &trusted, memory_mb);
    let _ = done.send(Done { id: 0, results: Vec::new(), actions: Vec::new(), catalog: Some(catalog_of(&manager)), listening: Vec::new() });
    while let Ok((id, job)) = jobs.recv() {
        let mut results = Vec::new();
        let mut catalog = None;
        match job {
            Job::Load(info, constants) => {
                manager.load_all(&info, &constants);
                catalog = Some(catalog_of(&manager));
            }
            Job::Event(event, args, info, tick) => {
                manager.set_info(&info, tick);
                results = manager.dispatch(&event, &args);
            }
            Job::Fire(event, state, info, tick) => {
                manager.set_info(&info, tick);
                if let Some(s) = manager.fire(&event, &state) {
                    results = vec![LVal::Str(s)];
                }
            }
            Job::Timers(info, tick) => {
                manager.set_info(&info, tick);
                manager.run_timers(tick);
            }
            Job::Command(name, args, sender, info, tick) => {
                manager.set_info(&info, tick);
                let list: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                manager.run_command_as(&name, &list, sender.as_deref());
                catalog = Some(catalog_of(&manager));
            }
            Job::Unload => {
                manager.unload_all();
                catalog = Some(catalog_of(&manager));
            }
            Job::Quit => break,
        }
        manager.clear_world();
        let actions = manager.take_actions();
        let listening = manager.listening();
        if done.send(Done { id, results, actions, catalog, listening }).is_err() {
            break;
        }
    }
}

impl PluginHost {
    /// Lance le fil des plugins. Le serveur ne fait que poster des demandes et lire les réponses.
    pub fn new(dir: &Path, sandbox: bool, trusted: &str, memory_mb: usize) -> PluginHost {
        let (tx, jobs) = mpsc::sync_channel(QUEUE);
        let (done_tx, rx) = mpsc::channel();
        let (dir, trusted) = (dir.to_path_buf(), trusted.to_string());
        let spawned = std::thread::Builder::new().name("plugins".into()).spawn(move || worker(dir, sandbox, trusted, memory_mb, jobs, done_tx));
        if let Err(e) = spawned {
            warn!("Plugins désactivés : fil d'exécution impossible à créer ({})", e);
        }
        let mut host = PluginHost {
            tx,
            rx,
            next_id: 1,
            pending: 1, // l'annonce de démarrage du fil
            late_since: None,
            last_warning: None,
            dropped: 0,
            catalog: Catalog::default(),
            actions: Vec::new(),
            info: Info::default(),
            tick: 0,
            listening: Vec::new(),
        };
        host.wait_for(0, Duration::from_secs(5));
        host
    }

    /// Range une réponse du fil : actions à appliquer, catalogue mis à jour.
    fn absorb(&mut self, d: Done) -> (u64, Vec<LVal>) {
        self.pending = self.pending.saturating_sub(1);
        self.actions.extend(d.actions);
        if let Some(c) = d.catalog {
            self.catalog = c;
        }
        if d.id != 0 {
            self.listening = d.listening;
        }
        if self.pending == 0 {
            if let Some(since) = self.late_since.take() {
                if since.elapsed() > Duration::from_secs(1) {
                    info!("Les plugins répondent de nouveau (retard de {:.1} s)", since.elapsed().as_secs_f32());
                }
            }
        }
        (d.id, d.results)
    }

    /// Ramasse ce que le fil des plugins a terminé, sans attendre.
    fn drain(&mut self) {
        while let Ok(d) = self.rx.try_recv() {
            self.absorb(d);
        }
    }

    /// Attend au plus `wait` la fin de la demande `id` ; renvoie ses résultats si elle a fini.
    fn wait_for(&mut self, id: u64, wait: Duration) -> Option<Vec<LVal>> {
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(d) => {
                    let (done, results) = self.absorb(d);
                    if done == id {
                        return Some(results);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.note_late();
                    return None;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.pending = 0;
                    return None;
                }
            }
        }
    }

    /// Les plugins prennent du retard : avertit au plus toutes les 10 secondes.
    fn note_late(&mut self) {
        let since = *self.late_since.get_or_insert_with(Instant::now);
        let quiet = self.last_warning.map(|w| w.elapsed() > Duration::from_secs(10)).unwrap_or(true);
        if quiet && since.elapsed() >= Duration::from_millis(500) {
            self.last_warning = Some(Instant::now());
            let culprit = lua::current_plugin();
            warn!(
                "Le plugin {} occupe le fil des plugins depuis {:.1} s : le serveur continue sans l'attendre ({} demande(s) en attente, {} abandonnée(s))",
                if culprit.is_empty() { "en cours".to_string() } else { format!("« {} »", culprit) },
                since.elapsed().as_secs_f32(),
                self.pending,
                self.dropped
            );
        }
    }

    /// Envoie une demande ; None si la file est pleine ou le fil arrêté.
    fn submit(&mut self, job: Job) -> Option<u64> {
        let id = self.next_id;
        match self.tx.try_send((id, job)) {
            Ok(()) => {
                self.next_id += 1;
                self.pending += 1;
                Some(id)
            }
            Err(TrySendError::Full(_)) => {
                self.dropped += 1;
                self.note_late();
                None
            }
            Err(TrySendError::Disconnected(_)) => None,
        }
    }

    /// Envoie puis attend brièvement — seulement si le fil était libre : inutile d'attendre
    /// derrière un plugin déjà en retard.
    fn call(&mut self, job: Job, wait: Duration) -> Option<Vec<LVal>> {
        self.drain();
        let free = self.pending == 0;
        let id = self.submit(job)?;
        if !free {
            self.note_late();
            return None;
        }
        self.wait_for(id, wait)
    }

    /// Lectures du catalogue (copie tenue à jour après chaque réponse du fil).
    pub fn available(&self) -> bool {
        self.catalog.available
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn commands(&self) -> Vec<(String, String, String)> {
        self.catalog.commands.clone()
    }

    pub fn permissions(&self) -> Vec<(String, PermDefault)> {
        self.catalog.permissions.clone()
    }

    /// Informations pour un travail : la vue du monde y part, le serveur n'en garde pas de
    /// copie (sinon il recopierait des blocs à chaque modification).
    fn job_info(&mut self) -> Info {
        let info = self.info.clone();
        self.info.world = None;
        info
    }

    /// Infos du monde transmises aux plugins avec la prochaine demande.
    pub fn set_info(&mut self, info: &Info, tick: u64) {
        self.info = info.clone();
        self.tick = tick;
    }

    /// Charge tous les plugins et attend la fin (avec une limite, voir WAIT_LOAD).
    pub fn load_all(&mut self, info: &Info, constants: &[(&'static str, LVal)]) {
        self.info = info.clone();
        if self.call(Job::Load(info.clone(), constants.to_vec()), WAIT_LOAD).is_none() {
            warn!("Le chargement des plugins prend plus de {} s : le serveur démarre sans attendre la fin", WAIT_LOAD.as_secs());
        }
    }

    /// Décharge les plugins (événement disable compris).
    pub fn unload_all(&mut self) {
        self.call(Job::Unload, WAIT_UNLOAD);
    }

    /// Événement dont le serveur n'attend pas de réponse.
    pub fn dispatch(&mut self, event: &str, args: &[LVal]) -> Vec<LVal> {
        // Événements dont le serveur lit la réponse (refus, annulation)
        let wait = if matches!(event, "login" | "block_break" | "block_click") { WAIT_ANSWER } else { WAIT };
        let info = self.job_info();
        self.call(Job::Event(event.to_string(), args.to_vec(), info, self.tick), wait).unwrap_or_default()
    }

    /// Minuteries : rien n'est envoyé tant que le fil est occupé (elles se rattrapent ensuite).
    /// Un plugin écoute-t-il cet événement (à la PocketMine) ?
    pub fn listens(&self, event: &str) -> bool {
        self.listening.iter().any(|k| k.len() > event.len() && k.starts_with(event) && k.as_bytes()[event.len()] == b'#')
    }

    /// Événement à la PocketMine : l'état passe d'écouteur en écouteur, par priorité. Renvoie
    /// l'état final, ou None si personne n'écoute (ou si les plugins ne répondent pas à temps).
    pub fn fire(&mut self, event: &str, state: &crate::json::Value) -> Option<crate::json::Value> {
        if !self.listens(event) {
            return None;
        }
        let info = self.job_info();
        let results = self.call(Job::Fire(event.to_string(), state.to_json(), info, self.tick), WAIT_ANSWER)?;
        match results.first() {
            Some(LVal::Str(s)) => crate::json::parse(s).ok(),
            _ => None,
        }
    }

    /// Fait avancer les minuteries ; ne poste rien si le fil n'a pas fini la demande précédente.
    pub fn run_timers(&mut self, tick: u64) {
        self.drain();
        self.tick = tick;
        if self.pending == 0 {
            let info = self.job_info();
            self.submit(Job::Timers(info, tick));
        } else {
            self.note_late();
        }
    }

    /// Lance une commande de plugin ; faux si aucun plugin ne la propose.
    pub fn run_command(&mut self, name: &str, args: &[&str]) -> bool {
        self.run_command_as(name, args, None)
    }

    /// Même chose, pour une commande tapée en jeu par `sender`.
    pub fn run_command_as(&mut self, name: &str, args: &[&str], sender: Option<&str>) -> bool {
        if !self.catalog.commands.iter().any(|c| c.0 == name) {
            return false;
        }
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let info = self.job_info();
        self.call(Job::Command(name.to_string(), args, sender.map(|s| s.to_string()), info, self.tick), WAIT_ANSWER);
        true
    }

    /// Permission exigée par une commande de plugin, d'après son plugin.yml.
    pub fn command_permission(&self, name: &str) -> Option<String> {
        self.catalog.command_permissions.iter().find(|(c, _)| c == name).map(|(_, p)| p.clone())
    }

    /// Description et usage d'une commande d'après son plugin.yml.
    pub fn command_decl(&self, name: &str) -> Option<(String, String)> {
        self.catalog.command_decls.iter().find(|c| c.0 == name).map(|c| (c.1.clone(), c.2.clone()))
    }

    /// Effets demandés par les plugins depuis le dernier appel.
    pub fn take_actions(&mut self) -> Vec<Action> {
        self.drain();
        std::mem::take(&mut self.actions)
    }
}

impl Drop for PluginHost {
    /// Arrêt du serveur.
    fn drop(&mut self) {
        // Le fil s'arrête de lui-même ; s'il est coincé dans un plugin, on ne l'attend pas
        let _ = self.tx.try_send((u64::MAX, Job::Quit));
    }
}
