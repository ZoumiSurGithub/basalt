//! NetherNet sur son propre fil.
//!
//! La liaison WebRTC (chiffrement DTLS, SCTP, lecture et envoi des datagrammes) est le plus
//! gros poste du serveur. Elle tourne ici sur un fil à part : la boucle du jeu ne fait plus
//! que déposer les paquets à envoyer et relever ce qui est arrivé, par deux files.
//!
//! Le fil garde la main sur les sessions ; la boucle du jeu tient une copie de la liste des
//! sessions (pour savoir si un joueur passe par NetherNet ou par le transport de test), mise à jour dans
//! l'ordre des événements : une session apparaît avant son premier paquet et disparaît après
//! sa déconnexion.
//!
//! **Plusieurs fils** (`nethernet-threads`) : chaque fil a sa propre instance et ses propres
//! sessions, toutes sur le même port UDP. Un fil de lecture relève les datagrammes et les
//! remet au fil qui tient la session de leur expéditeur ; ceux d'une origine encore inconnue
//! (le tout début d'une liaison) sont montrés à tous, et celui qui la reconnaît la garde.
//! Chaque session reste ainsi sur un seul fil, du début à la fin.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::nethernet::NetherNet;
use crate::transport::NetEvent;

enum Command {
    Accept { key: SocketAddr, local_ip: IpAddr, offer: String, cpk: Option<String>, now: u64 },
    Send(SocketAddr, Vec<u8>),
    Close(SocketAddr, Duration),
    Abort(SocketAddr, &'static str),
    Limits(SocketAddr),
    /// Datagramme relevé par le fil de lecture (plusieurs fils) : contenu, expéditeur
    Datagram(Vec<u8>, SocketAddr),
    Shutdown,
}

/// Expéditeur d'un datagramme -> fil qui tient sa session (plusieurs fils seulement).
type Routes = Arc<Mutex<HashMap<SocketAddr, usize>>>;

enum Output {
    Prepared(SocketAddr, Result<String, (u16, &'static str)>),
    Added(SocketAddr, Option<String>),
    Event(NetEvent),
    Removed(SocketAddr),
}

/// Poignée sur les fils NetherNet, avec la même interface que `NetherNet`.
pub struct NetherNetHandle {
    /// Un canal d'ordres par fil
    commands: Vec<Sender<Command>>,
    outputs: Receiver<Output>,
    threads: Vec<std::thread::JoinHandle<()>>,
    /// Arrêt du fil de lecture (plusieurs fils)
    stop: Arc<AtomicBool>,
    /// Sessions en cours, d'après les fils : clé publique prouvée par le joueur
    peers: HashMap<SocketAddr, Option<String>>,
    /// Fil de chaque session, de la demande d'entrée à sa fin
    shard_of: HashMap<SocketAddr, usize>,
    prepared: Vec<(SocketAddr, Result<String, (u16, &'static str)>)>,
    fingerprint: String,
}

impl NetherNetHandle {
    pub const AVAILABLE: bool = NetherNet::AVAILABLE;

    /// Lance les fils, qui prennent la main sur la liaison. `threads` : nombre de fils entre
    /// lesquels répartir les sessions (1 : un seul fil lit, chiffre et envoie tout).
    pub fn start(mut net: NetherNet, threads: usize) -> NetherNetHandle {
        let fingerprint = net.key_fingerprint().to_string();
        let (outbox, outputs) = mpsc::channel::<Output>();
        let stop = Arc::new(AtomicBool::new(false));
        let mut commands = Vec::new();
        let mut handles = Vec::new();
        // Plusieurs fils : des instances sœurs sur le même port, et un fil de lecture. Si
        // quoi que ce soit manque (partage du socket refusé), on reste sur un seul fil.
        let mut siblings: Vec<NetherNet> = Vec::new();
        let mut reader = None;
        if threads > 1 {
            let made: Result<Vec<NetherNet>, String> = (1..threads).map(|_| net.sibling()).collect();
            match made.and_then(|list| net.external_reads().map(|socket| (list, socket))) {
                Ok((list, socket)) => {
                    siblings = list;
                    for s in siblings.iter_mut() {
                        s.fed_from_outside();
                    }
                    reader = Some(socket);
                }
                Err(e) => warn!("NetherNet : un seul fil ({})", e),
            }
        }
        let routes: Routes = Arc::new(Mutex::new(HashMap::new()));
        let shared = reader.is_some();
        for (index, mut instance) in std::iter::once(net).chain(siblings).enumerate() {
            let (tx, inbox) = mpsc::channel::<Command>();
            let (out, routes) = (outbox.clone(), if shared { Some(routes.clone()) } else { None });
            let name = if shared { format!("nethernet-{}", index) } else { "nethernet".to_string() };
            if let Ok(h) = std::thread::Builder::new().name(name).spawn(move || run(&mut instance, inbox, out, index, routes)) {
                commands.push(tx);
                handles.push(h);
            }
        }
        if let Some(socket) = reader {
            let (senders, routes, stop) = (commands.clone(), routes.clone(), stop.clone());
            if let Ok(h) = std::thread::Builder::new().name("nethernet-lecture".into()).spawn(move || read_loop(socket, senders, routes, stop)) {
                handles.push(h);
            }
            info!("NetherNet : sessions réparties sur {} fils (même port)", commands.len());
        }
        NetherNetHandle { commands, outputs, threads: handles, stop, peers: HashMap::new(), shard_of: HashMap::new(), prepared: Vec::new(), fingerprint }
    }

    /// Canal du fil qui tient cette session.
    fn channel(&self, key: &SocketAddr) -> Option<&Sender<Command>> {
        self.shard_of.get(key).and_then(|i| self.commands.get(*i))
    }

    /// Empreinte de la clé du serveur.
    pub fn key_fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Nombre de connexions NetherNet.
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Confie l'offre au fil le moins chargé. Un refus arrive ensuite par `prepared`, comme
    /// une réponse.
    pub fn accept(&mut self, key: SocketAddr, local_ip: IpAddr, offer: &str, cpk: Option<String>, now: u64) -> Result<(), (u16, &'static str)> {
        let mut load = vec![0usize; self.commands.len()];
        for i in self.shard_of.values() {
            if let Some(n) = load.get_mut(*i) {
                *n += 1;
            }
        }
        // (une nouvelle tentative de la même origine retourne au même fil : c'est lui qui
        // sait qu'une session est déjà en cours pour elle)
        let shard = match self.shard_of.get(&key) {
            Some(i) => *i,
            None => load.iter().enumerate().min_by_key(|(_, n)| **n).map(|(i, _)| i).ok_or((503, "server busy"))?,
        };
        let channel = self.commands.get(shard).ok_or((503, "server busy"))?;
        channel.send(Command::Accept { key, local_ip, offer: offer.to_string(), cpk, now }).map_err(|_| (503, "server busy"))?;
        self.shard_of.insert(key, shard);
        Ok(())
    }

    /// Réponses aux offres WebRTC préparées par le fil.
    pub fn prepared(&mut self) -> Vec<(SocketAddr, Result<String, (u16, &'static str)>)> {
        std::mem::take(&mut self.prepared)
    }

    /// Relève ce que le fil a reçu. Renvoie le nombre d'éléments relevés.
    pub fn poll(&mut self, events: &mut Vec<NetEvent>) -> usize {
        let mut count = 0;
        while let Ok(out) = self.outputs.try_recv() {
            count += 1;
            match out {
                Output::Prepared(key, result) => {
                    // Refusée : cette origine n'a plus de fil attitré (sauf session déjà en cours)
                    if result.is_err() && !self.peers.contains_key(&key) {
                        self.shard_of.remove(&key);
                    }
                    self.prepared.push((key, result));
                }
                Output::Added(key, cpk) => {
                    self.peers.insert(key, cpk);
                }
                Output::Event(ev) => events.push(ev),
                Output::Removed(key) => {
                    self.peers.remove(&key);
                    self.shard_of.remove(&key);
                }
            }
        }
        count
    }

    /// Les envois partent depuis le fil, sans attendre : rien à faire ici.
    pub fn flush(&mut self, _events: &mut Vec<NetEvent>) {}

    /// Cette adresse est-elle une connexion NetherNet ?
    pub fn has(&self, key: &SocketAddr) -> bool {
        self.peers.contains_key(key)
    }

    /// Clé publique du client liée à la connexion, si elle a été vérifiée.
    pub fn bound_cpk(&self, key: &SocketAddr) -> Option<String> {
        self.peers.get(key).cloned().flatten()
    }

    /// Ordres transmis au fil de la session : envoi, fermeture, coupure, limites de jeu.
    pub fn send(&mut self, key: &SocketAddr, message: &[u8]) {
        if let Some(c) = self.channel(key) {
            let _ = c.send(Command::Send(*key, message.to_vec()));
        }
    }

    pub fn close(&mut self, key: &SocketAddr, delay: Duration) {
        if let Some(c) = self.channel(key) {
            let _ = c.send(Command::Close(*key, delay));
        }
    }

    pub fn abort(&mut self, key: &SocketAddr, reason: &'static str) {
        if let Some(c) = self.channel(key) {
            let _ = c.send(Command::Abort(*key, reason));
        }
    }

    pub fn set_game_limits(&mut self, key: &SocketAddr) {
        if let Some(c) = self.channel(key) {
            let _ = c.send(Command::Limits(*key));
        }
    }

    /// Ferme toutes les sessions (les derniers paquets partent d'abord) et arrête les fils.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for c in &self.commands {
            let _ = c.send(Command::Shutdown);
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        self.peers.clear();
        self.shard_of.clear();
    }
}

/// Fil de lecture (plusieurs fils) : relève les datagrammes et les remet au fil qui tient la
/// session de leur expéditeur, ou à tous si cette origine n'est pas encore connue.
fn read_loop(socket: std::net::UdpSocket, shards: Vec<Sender<Command>>, routes: Routes, stop: Arc<AtomicBool>) {
    let mut buf = [0u8; 2048];
    while !stop.load(Ordering::Relaxed) {
        // (lecture bloquante, 50 ms au plus : le temps de voir une demande d'arrêt. Une
        // erreur — délai écoulé, ou « connexion réinitialisée » sous Windows — n'arrête rien.)
        let (n, source) = match socket.recv_from(&mut buf) {
            Ok(v) if v.0 > 0 => v,
            _ => continue,
        };
        let _t = crate::timings::scope("NetherNet (fil de lecture) — répartition");
        let owner = routes.lock().ok().and_then(|r| r.get(&source).copied());
        match owner.and_then(|i| shards.get(i)) {
            Some(shard) => {
                let _ = shard.send(Command::Datagram(buf[..n].to_vec(), source));
            }
            None => {
                for shard in &shards {
                    let _ = shard.send(Command::Datagram(buf[..n].to_vec(), source));
                }
            }
        }
    }
}

impl Drop for NetherNetHandle {
    /// Arrête les fils NetherNet.
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Boucle du fil NetherNet.
fn run(net: &mut NetherNet, inbox: Receiver<Command>, outbox: Sender<Output>, index: usize, routes: Option<Routes>) {
    let mut known: HashMap<SocketAddr, Option<String>> = HashMap::new();
    let mut events = Vec::new();
    loop {
        let mut busy = false;
        // (au plus quelques centaines d'ordres d'affilée : sous un flot continu de datagrammes,
        // les envois et les minuteries doivent garder leur tour)
        for _ in 0..512 {
            match inbox.try_recv() {
                Ok(cmd) => {
                    busy = true;
                    if !apply(net, cmd, &outbox, &mut events, index, routes.as_ref()) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    net.shutdown();
                    return;
                }
            }
        }
        for (key, result) in net.prepared() {
            let _ = outbox.send(Output::Prepared(key, result));
        }
        let read = {
            let _t = crate::timings::scope("NetherNet (fil à part) — lecture et réception");
            net.poll(&mut events)
        };
        {
            let _t = crate::timings::scope("NetherNet (fil à part) — envoi");
            net.flush(&mut events);
        }
        // Sessions apparues : annoncées avant leurs paquets ; disparues : après leur fin
        let now: HashMap<SocketAddr, Option<String>> = net.peer_list().into_iter().collect();
        for (key, cpk) in &now {
            if !known.contains_key(key) {
                let _ = outbox.send(Output::Added(*key, cpk.clone()));
            }
        }
        for ev in events.drain(..) {
            let _ = outbox.send(Output::Event(ev));
        }
        for key in known.keys() {
            if !now.contains_key(key) {
                let _ = outbox.send(Output::Removed(*key));
            }
        }
        known = now;
        if read == 0 && !busy {
            // Rien à faire : on attend un ordre de la boucle du jeu, une milliseconde au plus
            match inbox.recv_timeout(Duration::from_millis(1)) {
                Ok(cmd) => {
                    if !apply(net, cmd, &outbox, &mut events, index, routes.as_ref()) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    net.shutdown();
                    return;
                }
            }
        }
    }
}

/// Exécute un ordre. Faux : il faut s'arrêter.
fn apply(net: &mut NetherNet, cmd: Command, outbox: &Sender<Output>, events: &mut Vec<NetEvent>, index: usize, routes: Option<&Routes>) -> bool {
    match cmd {
        Command::Accept { key, local_ip, offer, cpk, now } => {
            if let Err(e) = net.accept(key, local_ip, &offer, cpk, now) {
                let _ = outbox.send(Output::Prepared(key, Err(e)));
            }
        }
        Command::Send(key, message) => net.send(&key, &message),
        Command::Close(key, delay) => net.close(&key, delay),
        Command::Abort(key, reason) => net.abort(&key, reason),
        Command::Limits(key) => net.set_game_limits(&key),
        Command::Datagram(data, source) => {
            let mine = {
                let _t = crate::timings::scope("NetherNet (fil à part) — lecture et réception");
                net.feed(&data, source, events)
            };
            // Cette origine est à moi : les suivants me viendront directement. Elle ne l'est
            // pas (ou plus) : la route est oubliée, et le prochain sera montré à tous.
            if let Some(Ok(mut r)) = routes.map(|r| r.lock()) {
                match (mine, r.get(&source).copied()) {
                    (true, None) if r.len() < 65_536 => {
                        r.insert(source, index);
                    }
                    (false, Some(owner)) if owner == index => {
                        r.remove(&source);
                    }
                    _ => {}
                }
            }
        }
        Command::Shutdown => {
            net.shutdown();
            return false;
        }
    }
    true
}
