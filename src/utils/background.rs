//! Files de travail en arrière-plan : ce qui n'a pas besoin de la boucle principale
//! (écritures sur disque, journal) est confié à un fil à part, pour qu'un disque lent ou
//! un gros enregistrement ne retarde jamais les joueurs.
//!
//! Chaque file a un seul fil et traite ses travaux dans l'ordre d'arrivée : deux
//! enregistrements du même fichier ne peuvent donc pas s'intervertir. Si le fil ne peut pas
//! être lancé, le travail est fait sur place, comme avant.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send>;

enum Msg {
    Job(Job),
    /// Prévient quand tout ce qui précède est terminé
    Flush(Sender<()>),
}

pub struct Queue {
    name: &'static str,
    tx: Mutex<Option<Sender<Msg>>>,
    started: Mutex<bool>,
    pending: AtomicUsize,
}

impl Queue {
    /// File de travail nommée ; son fil démarre au premier envoi.
    pub const fn new(name: &'static str) -> Queue {
        Queue { name, tx: Mutex::new(None), started: Mutex::new(false), pending: AtomicUsize::new(0) }
    }

    /// Canal vers le fil, démarré si besoin.
    fn sender(&'static self) -> Option<Sender<Msg>> {
        let mut tx = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        if tx.is_none() {
            let mut started = self.started.lock().unwrap_or_else(|e| e.into_inner());
            if *started {
                return None; // le fil n'a pas pu être lancé : travail sur place
            }
            *started = true;
            let (s, r) = channel::<Msg>();
            let pending = &self.pending;
            let spawned = std::thread::Builder::new().name(self.name.to_string()).spawn(move || {
                super::lower_thread_priority();
                while let Ok(msg) = r.recv() {
                    match msg {
                        Msg::Job(job) => {
                            // Un travail qui panique ne doit pas arrêter la file
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                            pending.fetch_sub(1, Ordering::Relaxed);
                        }
                        Msg::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            });
            if spawned.is_err() {
                return None;
            }
            *tx = Some(s);
        }
        tx.clone()
    }

    /// Confie un travail à la file.
    pub fn run(&'static self, job: impl FnOnce() + Send + 'static) {
        match self.sender() {
            Some(tx) => {
                self.pending.fetch_add(1, Ordering::Relaxed);
                if let Err(e) = tx.send(Msg::Job(Box::new(job))) {
                    self.pending.fetch_sub(1, Ordering::Relaxed);
                    if let Msg::Job(job) = e.0 {
                        job();
                    }
                }
            }
            None => job(),
        }
    }

    /// Travaux confiés et pas encore terminés.
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::Relaxed)
    }

    /// Attend que tout ce qui a été confié soit fait (au plus `timeout`). Vrai si c'est le cas.
    pub fn flush(&self, timeout: Duration) -> bool {
        let tx = match self.tx.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            Some(tx) => tx,
            None => return true,
        };
        let (done_tx, done_rx) = channel();
        if tx.send(Msg::Flush(done_tx)).is_err() {
            return true;
        }
        done_rx.recv_timeout(timeout).is_ok()
    }

    /// Attend, sans limite de temps, que tout ce qui est en file soit fait (arrêt du serveur :
    /// rien ne doit rester à moitié écrit). `waiting` est appelé toutes les 10 secondes
    /// d'attente, avec le temps écoulé.
    pub fn flush_all(&self, mut waiting: impl FnMut(u64)) {
        let tx = match self.tx.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            Some(tx) => tx,
            None => return,
        };
        let (done_tx, done_rx) = channel();
        if tx.send(Msg::Flush(done_tx)).is_err() {
            return;
        }
        let started = std::time::Instant::now();
        loop {
            match done_rx.recv_timeout(Duration::from_secs(10)) {
                Ok(_) => return,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => waiting(started.elapsed().as_secs()),
                // (le fil d'écriture a disparu : plus rien à attendre)
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

/// Écritures sur disque : monde, coffres et fours, joueurs, rapports de mesures.
pub static DISK: Queue = Queue::new("basalt-disque");
/// Journal : console et fichier `logs/latest.log`.
pub static LOG: Queue = Queue::new("basalt-journal");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_run_in_order_and_flush_waits_for_them() {
        static Q: Queue = Queue::new("basalt-test-file");
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        for i in 0..50 {
            let seen = seen.clone();
            Q.run(move || {
                std::thread::sleep(Duration::from_micros(200));
                seen.lock().unwrap().push(i);
            });
        }
        // Un travail qui panique n'arrête pas la file
        Q.run(|| panic!("travail en échec"));
        let last = seen.clone();
        Q.run(move || last.lock().unwrap().push(99));
        assert!(Q.flush(Duration::from_secs(10)));
        let got = seen.lock().unwrap().clone();
        let mut expected: Vec<i32> = (0..50).collect();
        expected.push(99);
        assert_eq!(got, expected);
        assert_eq!(Q.pending(), 0);
    }
}
