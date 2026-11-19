//! Mesures de temps (« timings »), comme sur PocketMine : où passe le temps du serveur ?
//!
//! Éteintes, elles ne coûtent qu'un test par point de mesure. Allumées par `/timings on`,
//! elles chronomètrent chaque étape d'un tick, chaque type de paquet, chaque événement de
//! plugin et chaque commande ; `/timings report` en tire un rapport.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Durée d'un tick : au-delà, le serveur prend du retard.
const TICK: Duration = Duration::from_millis(50);
/// Nombre maximal de lignes distinctes (les noms viennent en partie des plugins et des joueurs).
const MAX_ENTRIES: usize = 512;

#[derive(Default, Clone)]
struct Entry {
    total: Duration,
    calls: u64,
    worst: Duration,
    /// Appels qui ont duré plus d'un tick à eux seuls
    slow: u64,
}

struct State {
    since: Instant,
    ticks: u64,
    entries: HashMap<String, Entry>,
}

/// Une mesure en cours ; elle s'enregistre quand elle sort de portée.
pub struct Scope {
    name: Option<(std::borrow::Cow<'static, str>, Instant)>,
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some((name, start)) = self.name.take() {
            record(&name, start.elapsed());
        }
    }
}

/// Chronomètre la portée courante sous un nom fixe.
pub fn scope(name: &'static str) -> Scope {
    if !ENABLED.load(Ordering::Relaxed) {
        return Scope { name: None };
    }
    Scope { name: Some((std::borrow::Cow::Borrowed(name), Instant::now())) }
}

/// Idem avec un nom construit à la demande (il n'est construit que si les mesures sont allumées).
pub fn scope_with(name: impl FnOnce() -> String) -> Scope {
    if !ENABLED.load(Ordering::Relaxed) {
        return Scope { name: None };
    }
    Scope { name: Some((std::borrow::Cow::Owned(name()), Instant::now())) }
}

fn record(name: &str, spent: Duration) {
    let mut guard = match STATE.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let Some(state) = guard.as_mut() {
        if !state.entries.contains_key(name) && state.entries.len() >= MAX_ENTRIES {
            return;
        }
        let e = state.entries.entry(name.to_string()).or_default();
        e.total += spent;
        e.calls += 1;
        if spent > e.worst {
            e.worst = spent;
        }
        if spent > TICK {
            e.slow += 1;
        }
    }
}

/// À appeler une fois par tick du serveur.
pub fn tick() {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut guard) = STATE.lock() {
        if let Some(state) = guard.as_mut() {
            state.ticks += 1;
        }
    }
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Allume les mesures (et repart de zéro). Faux si elles l'étaient déjà.
pub fn enable() -> bool {
    if ENABLED.load(Ordering::Relaxed) {
        return false;
    }
    reset();
    ENABLED.store(true, Ordering::Relaxed);
    true
}

/// Éteint les mesures ; les chiffres relevés restent disponibles pour un rapport.
pub fn disable() -> bool {
    ENABLED.swap(false, Ordering::Relaxed)
}

/// Remet les compteurs à zéro.
pub fn reset() {
    if let Ok(mut guard) = STATE.lock() {
        *guard = Some(State { since: Instant::now(), ticks: 0, entries: HashMap::new() });
    }
}

/// Une ligne du rapport.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub name: String,
    pub total_ms: f64,
    /// Part du temps écoulé depuis le début des mesures
    pub share: f64,
    pub calls: u64,
    pub average_us: f64,
    pub worst_ms: f64,
    pub slow: u64,
}

/// Résumé chiffré : (secondes mesurées, ticks, lignes triées de la plus coûteuse à la moins coûteuse).
pub fn snapshot() -> Option<(f64, u64, Vec<Line>)> {
    let guard = STATE.lock().ok()?;
    let state = guard.as_ref()?;
    let elapsed = state.since.elapsed().as_secs_f64().max(0.001);
    let mut lines: Vec<Line> = state
        .entries
        .iter()
        .map(|(name, e)| Line {
            name: name.clone(),
            total_ms: e.total.as_secs_f64() * 1000.0,
            share: e.total.as_secs_f64() / elapsed * 100.0,
            calls: e.calls,
            average_us: if e.calls == 0 { 0.0 } else { e.total.as_secs_f64() * 1_000_000.0 / e.calls as f64 },
            worst_ms: e.worst.as_secs_f64() * 1000.0,
            slow: e.slow,
        })
        .collect();
    lines.sort_by(|a, b| b.total_ms.partial_cmp(&a.total_ms).unwrap_or(std::cmp::Ordering::Equal).then(a.name.cmp(&b.name)));
    Some((elapsed, state.ticks, lines))
}

/// Rapport complet, en texte. `header` : lignes de contexte (version, joueurs…).
pub fn report(header: &[String]) -> Option<String> {
    let (elapsed, ticks, lines) = snapshot()?;
    let mut out = String::new();
    out.push_str("Rapport de mesures de Basalt\n");
    out.push_str("============================\n");
    for h in header {
        out.push_str(h);
        out.push('\n');
    }
    out.push_str(&format!("Durée mesurée : {:.1} s — {} ticks, soit {:.2} ticks/s (objectif 20)\n", elapsed, ticks, ticks as f64 / elapsed));
    out.push_str("\nComment lire ce tableau\n");
    out.push_str("  % du temps : part du temps écoulé passée dans cette étape. Le serveur a du retard\n");
    out.push_str("               quand « Tick complet » et les étapes réseau approchent 100 % à eux tous.\n");
    out.push_str("  par tick   : temps moyen par tick, à comparer aux 50 ms dont dispose un tick.\n");
    out.push_str("  pire       : appel le plus long. « lents » : appels de plus de 50 ms.\n");
    out.push_str("  « Pause de 1 ms » n'est pas du travail : c'est le repos du serveur. Son « pire » doit\n");
    out.push_str("  rester de quelques millisecondes ; s'il atteint des dizaines de ms, le système\n");
    out.push_str("  d'exploitation ou l'hébergeur ne rend pas la main à temps (processeur partagé).\n");
    out.push_str("  Les lignes s'emboîtent : un paquet est compté dans « Paquets reçus », un événement\n");
    out.push_str("  de plugin dans le paquet ou l'étape qui l'a déclenché.\n\n");
    out.push_str(&format!("{:<44} {:>10} {:>9} {:>10} {:>9} {:>11} {:>9} {:>6}\n", "Étape", "total ms", "% temps", "par tick", "appels", "moyenne", "pire ms", "lents"));
    out.push_str(&format!("{}\n", "-".repeat(116)));
    for l in &lines {
        let per_tick = if ticks == 0 { 0.0 } else { l.total_ms / ticks as f64 };
        let average = if l.average_us >= 1000.0 { format!("{:.2} ms", l.average_us / 1000.0) } else { format!("{:.0} µs", l.average_us) };
        let name: String = l.name.chars().take(44).collect();
        out.push_str(&format!("{:<44} {:>10.1} {:>8.2}% {:>7.3} ms {:>9} {:>11} {:>9.2} {:>6}\n", name, l.total_ms, l.share, per_tick, l.calls, average, l.worst_ms, l.slow));
    }
    if lines.is_empty() {
        out.push_str("(aucune mesure : laissez tourner le serveur quelques secondes après « timings on »)\n");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un seul test : les mesures sont globales au processus, deux tests se marcheraient dessus.
    #[test]
    fn timings_measure_only_when_enabled() {
        disable();
        reset();
        {
            let _t = scope("éteint");
        }
        assert!(snapshot().map(|s| s.2.is_empty()).unwrap_or(true), "rien n'est mesuré tant que c'est éteint");

        assert!(enable());
        assert!(!enable(), "déjà allumé");
        {
            let _t = scope("étape lente");
            std::thread::sleep(Duration::from_millis(12));
        }
        for i in 0..3 {
            let _t = scope_with(|| format!("paquet {}", i % 2));
        }
        tick();
        tick();
        let (_, ticks, lines) = snapshot().unwrap();
        assert_eq!(ticks, 2);
        assert_eq!(lines[0].name, "étape lente", "la plus coûteuse en premier");
        assert!(lines[0].total_ms >= 12.0 && lines[0].calls == 1 && lines[0].slow == 0);
        assert_eq!(lines.iter().find(|l| l.name == "paquet 0").map(|l| l.calls), Some(2));
        let text = report(&["Basalt de test".to_string()]).unwrap();
        assert!(text.contains("étape lente") && text.contains("Basalt de test") && text.contains("2 ticks"));

        assert!(disable());
        {
            let _t = scope("après extinction");
        }
        assert!(snapshot().unwrap().2.iter().all(|l| l.name != "après extinction"), "les chiffres restent, sans nouvelle mesure");
        reset();
        assert!(snapshot().unwrap().2.is_empty());
    }
}
