//! NetherNet sur son propre fil.
//!
//! La liaison WebRTC (chiffrement DTLS, SCTP, lecture et envoi des datagrammes) est le plus
//! gros poste du serveur. Elle tourne ici sur un fil à part : la boucle du jeu ne fait plus
//! que déposer les paquets à envoyer et relever ce qui est arrivé, par deux files.
//!
//! Le fil garde la main sur les sessions ; la boucle du jeu tient une copie de la liste des
//! sessions (pour savoir si un joueur passe par NetherNet ou par RakNet), mise à jour dans
//! l'ordre des événements : une session apparaît avant son premier paquet et disparaît après
//! sa déconnexion.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

use super::nethernet::NetherNet;
use crate::raknet::RakEvent;

enum Command {
    Accept { key: SocketAddr, local_ip: IpAddr, offer: String, cpk: Option<String>, now: u64 },
    Send(SocketAddr, Vec<u8>),
    Close(SocketAddr, Duration),
    Abort(SocketAddr, &'static str),
    Limits(SocketAddr),
    Shutdown,
}

enum Output {
    Prepared(SocketAddr, Result<String, (u16, &'static str)>),
    Added(SocketAddr, Option<String>),
    Event(RakEvent),
    Removed(SocketAddr),
}

/// Poignée sur le fil NetherNet, avec la même interface que `NetherNet`.
pub struct NetherNetHandle {
    commands: Sender<Command>,
    outputs: Receiver<Output>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Sessions en cours, d'après le fil : clé publique prouvée par le joueur
    peers: HashMap<SocketAddr, Option<String>>,
    prepared: Vec<(SocketAddr, Result<String, (u16, &'static str)>)>,
    fingerprint: String,
}

impl NetherNetHandle {
    pub const AVAILABLE: bool = NetherNet::AVAILABLE;

    /// Lance le fil, qui prend la main sur la liaison.
    pub fn start(mut net: NetherNet) -> NetherNetHandle {
        let fingerprint = net.key_fingerprint().to_string();
        let (commands, inbox) = mpsc::channel::<Command>();
        let (outbox, outputs) = mpsc::channel::<Output>();
        let thread = std::thread::Builder::new()
            .name("nethernet".into())
            .spawn(move || run(&mut net, inbox, outbox))
            .ok();
        NetherNetHandle { commands, outputs, thread, peers: HashMap::new(), prepared: Vec::new(), fingerprint }
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

    /// Confie l'offre au fil. Un refus arrive ensuite par `prepared`, comme une réponse.
    pub fn accept(&mut self, key: SocketAddr, local_ip: IpAddr, offer: &str, cpk: Option<String>, now: u64) -> Result<(), (u16, &'static str)> {
        self.commands
            .send(Command::Accept { key, local_ip, offer: offer.to_string(), cpk, now })
            .map_err(|_| (503, "server busy"))
    }

    /// Réponses aux offres WebRTC préparées par le fil.
    pub fn prepared(&mut self) -> Vec<(SocketAddr, Result<String, (u16, &'static str)>)> {
        std::mem::take(&mut self.prepared)
    }

    /// Relève ce que le fil a reçu. Renvoie le nombre d'éléments relevés.
    pub fn poll(&mut self, events: &mut Vec<RakEvent>) -> usize {
        let mut count = 0;
        while let Ok(out) = self.outputs.try_recv() {
            count += 1;
            match out {
                Output::Prepared(key, result) => self.prepared.push((key, result)),
                Output::Added(key, cpk) => {
                    self.peers.insert(key, cpk);
                }
                Output::Event(ev) => events.push(ev),
                Output::Removed(key) => {
                    self.peers.remove(&key);
                }
            }
        }
        count
    }

    /// Les envois partent depuis le fil, sans attendre : rien à faire ici.
    pub fn flush(&mut self, _events: &mut Vec<RakEvent>) {}

    /// Cette adresse est-elle une connexion NetherNet ?
    pub fn has(&self, key: &SocketAddr) -> bool {
        self.peers.contains_key(key)
    }

    /// Clé publique du client liée à la connexion, si elle a été vérifiée.
    pub fn bound_cpk(&self, key: &SocketAddr) -> Option<String> {
        self.peers.get(key).cloned().flatten()
    }

    /// Ordres transmis au fil NetherNet : envoi, fermeture, coupure, limites de jeu.
    pub fn send(&mut self, key: &SocketAddr, message: &[u8]) {
        let _ = self.commands.send(Command::Send(*key, message.to_vec()));
    }

    pub fn close(&mut self, key: &SocketAddr, delay: Duration) {
        let _ = self.commands.send(Command::Close(*key, delay));
    }

    pub fn abort(&mut self, key: &SocketAddr, reason: &'static str) {
        let _ = self.commands.send(Command::Abort(*key, reason));
    }

    pub fn set_game_limits(&mut self, key: &SocketAddr) {
        let _ = self.commands.send(Command::Limits(*key));
    }

    /// Ferme toutes les sessions (les derniers paquets partent d'abord) et arrête le fil.
    pub fn shutdown(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.peers.clear();
    }
}

impl Drop for NetherNetHandle {
    /// Arrête le fil NetherNet.
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Boucle du fil NetherNet.
fn run(net: &mut NetherNet, inbox: Receiver<Command>, outbox: Sender<Output>) {
    let mut known: HashMap<SocketAddr, Option<String>> = HashMap::new();
    let mut events = Vec::new();
    loop {
        let mut busy = false;
        loop {
            match inbox.try_recv() {
                Ok(cmd) => {
                    busy = true;
                    if !apply(net, cmd, &outbox) {
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
                    if !apply(net, cmd, &outbox) {
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
fn apply(net: &mut NetherNet, cmd: Command, outbox: &Sender<Output>) -> bool {
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
        Command::Shutdown => {
            net.shutdown();
            return false;
        }
    }
    true
}
