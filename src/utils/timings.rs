//! Mesures de temps (« timings »), comme sur PocketMine : où passe le temps du serveur ?
//!
//! Éteintes, elles ne coûtent qu'un test par point de mesure. Allumées par `/timings on`,
//! elles chronomètrent chaque étape d'un tick, chaque type de paquet, chaque événement de
//! plugin et chaque commande ; `/timings report` en tire un rapport.

use std::collections::HashMap;
use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<State>> = Mutex::new(None);
/// Change à chaque remise à zéro : ce que les fils gardent d'avant est alors jeté
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Mesures d'un fil, pas encore versées au compte commun. Chaque fil compte dans son coin
/// et ne prend le verrou commun que dix fois par seconde : avec un verrou par mesure, les
/// fils du réseau (des centaines de milliers de mesures par seconde) se le disputaient, ce
/// qui ralentissait le serveur et gonflait les temps mesurés eux-mêmes.
struct Local {
    generation: u64,
    entries: HashMap<Cow<'static, str>, Entry>,
    pending: u32,
    flushed: Option<Instant>,
}

thread_local! {
    static LOCAL: RefCell<Local> = RefCell::new(Local { generation: 0, entries: HashMap::new(), pending: 0, flushed: None });
}

/// Délai entre deux versements d'un fil au compte commun
const FLUSH_EVERY: Duration = Duration::from_millis(100);

/// Verse les mesures d'un fil au compte commun.
fn flush(local: &mut Local) {
    local.pending = 0;
    if local.entries.is_empty() {
        return;
    }
    let mut guard = match STATE.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if local.generation != GENERATION.load(Ordering::Relaxed) {
        local.entries.clear();
        return;
    }
    if let Some(state) = guard.as_mut() {
        for (name, add) in local.entries.drain() {
            if !state.entries.contains_key(name.as_ref()) && state.entries.len() >= MAX_ENTRIES {
                continue;
            }
            let e = state.entries.entry(name.into_owned()).or_default();
            e.total += add.total;
            e.calls += add.calls;
            e.slow += add.slow;
            if add.worst > e.worst {
                e.worst = add.worst;
            }
        }
    } else {
        local.entries.clear();
    }
}

/// Verse tout de suite les mesures du fil courant (avant un rapport).
fn flush_current() {
    let _ = LOCAL.try_with(|l| {
        if let Ok(mut l) = l.try_borrow_mut() {
            flush(&mut l);
        }
    });
}

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
    /// Compteurs de la machine au début des mesures (Linux)
    machine: Option<Machine>,
}

/// Compteurs processeur relevés dans /proc (Linux), en unités du système.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Machine {
    /// Temps total de tous les cœurs
    pub total: u64,
    pub idle: u64,
    /// Attente du disque
    pub iowait: u64,
    /// Temps volé par l'hébergeur (machine virtuelle)
    pub steal: u64,
    /// Temps processeur de Basalt
    pub own: u64,
}

impl Machine {
    /// Relève les compteurs (None hors Linux).
    pub fn sample() -> Option<Machine> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let own = std::fs::read_to_string("/proc/self/stat").ok()?;
        Machine::parse(&stat, &own)
    }

    fn parse(stat: &str, own: &str) -> Option<Machine> {
        let line = stat.lines().find(|l| l.starts_with("cpu "))?;
        let v: Vec<u64> = line.split_whitespace().skip(1).map(|x| x.parse().unwrap_or(0)).collect();
        if v.len() < 5 {
            return None;
        }
        // user nice system idle iowait irq softirq steal (guest est déjà compté dans user)
        let total: u64 = v.iter().take(8).sum();
        // Les champs de /proc/self/stat après le nom (entre parenthèses) : utime et stime sont les 12e et 13e
        let after = &own[own.rfind(')')? + 1..];
        let f: Vec<&str> = after.split_whitespace().collect();
        let own = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
        Some(Machine { total, idle: v[3], iowait: v[4], steal: v.get(7).copied().unwrap_or(0), own })
    }
}

/// Bilan de la machine entre deux relevés, en pourcentages.
#[derive(Debug, Clone, PartialEq)]
pub struct MachineUse {
    pub cores: usize,
    /// Processeur de toute la machine occupé (hors attente)
    pub busy: f64,
    pub iowait: f64,
    pub steal: f64,
    /// Basalt, en % d'un cœur
    pub own_core: f64,
}

fn machine_use(a: Machine, b: Machine, cores: usize) -> Option<MachineUse> {
    let total = b.total.checked_sub(a.total)? as f64;
    if total <= 0.0 {
        return None;
    }
    let pct = |x: u64, y: u64| y.saturating_sub(x) as f64 / total * 100.0;
    let idle = pct(a.idle, b.idle);
    let iowait = pct(a.iowait, b.iowait);
    let steal = pct(a.steal, b.steal);
    Some(MachineUse {
        cores,
        busy: (100.0 - idle - iowait - steal).max(0.0),
        iowait,
        steal,
        own_core: b.own.saturating_sub(a.own) as f64 / (total / cores.max(1) as f64) * 100.0,
    })
}

/// Mémoire de la machine : (disponible, totale, swap utilisé), en Mo.
fn memory() -> Option<(u64, u64, u64)> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let get = |k: &str| info.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok());
    let swap = get("SwapTotal:")?.saturating_sub(get("SwapFree:")?);
    Some((get("MemAvailable:")? / 1024, get("MemTotal:")? / 1024, swap / 1024))
}

/// Une mesure en cours ; elle s'enregistre quand elle sort de portée.
pub struct Scope {
    name: Option<(std::borrow::Cow<'static, str>, Instant)>,
}

thread_local! {
    /// Étapes ouvertes sur ce fil, de la plus large à la plus fine
    static OPEN: std::cell::RefCell<Vec<Cow<'static, str>>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn opened(name: &Cow<'static, str>) {
    let _ = OPEN.try_with(|o| {
        if let Ok(mut o) = o.try_borrow_mut() {
            if o.len() < 64 {
                o.push(name.clone());
            }
        }
    });
}

/// L'étape la plus fine ouverte sur ce fil (pour savoir qui a déclenché une étape commune).
pub fn current() -> Option<String> {
    OPEN.try_with(|o| o.try_borrow().ok().and_then(|o| o.last().map(|n| n.to_string()))).ok().flatten()
}

impl Drop for Scope {
    /// Fin de la mesure : le temps passé est ajouté à l'étape.
    fn drop(&mut self) {
        if let Some((name, start)) = self.name.take() {
            let _ = OPEN.try_with(|o| {
                if let Ok(mut o) = o.try_borrow_mut() {
                    if let Some(at) = o.iter().rposition(|n| *n == name) {
                        o.remove(at);
                    }
                }
            });
            record(name, start);
        }
    }
}

/// Chronomètre la portée courante sous un nom fixe.
pub fn scope(name: &'static str) -> Scope {
    if !ENABLED.load(Ordering::Relaxed) {
        return Scope { name: None };
    }
    let name = std::borrow::Cow::Borrowed(name);
    opened(&name);
    Scope { name: Some((name, Instant::now())) }
}

/// Idem avec un nom construit à la demande (il n'est construit que si les mesures sont allumées).
pub fn scope_with(name: impl FnOnce() -> String) -> Scope {
    if !ENABLED.load(Ordering::Relaxed) {
        return Scope { name: None };
    }
    let name: Cow<'static, str> = std::borrow::Cow::Owned(name());
    opened(&name);
    Scope { name: Some((name, Instant::now())) }
}

/// Ajoute à l'étape `name` le temps écoulé depuis `start`.
fn record(name: Cow<'static, str>, start: Instant) {
    let now = Instant::now();
    let spent = now.duration_since(start);
    let _ = LOCAL.try_with(|l| {
        let mut l = match l.try_borrow_mut() {
            Ok(l) => l,
            Err(_) => return,
        };
        let generation = GENERATION.load(Ordering::Relaxed);
        if l.generation != generation {
            l.entries.clear();
            l.generation = generation;
            l.pending = 0;
        }
        if l.entries.len() < MAX_ENTRIES || l.entries.contains_key(&name) {
            let e = l.entries.entry(name).or_default();
            e.total += spent;
            e.calls += 1;
            if spent > e.worst {
                e.worst = spent;
            }
            if spent > TICK {
                e.slow += 1;
            }
        }
        l.pending += 1;
        let due = l.flushed.map(|t| now.duration_since(t) >= FLUSH_EVERY).unwrap_or(true);
        if due || l.pending >= 65_536 {
            l.flushed = Some(now);
            flush(&mut l);
        }
    });
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

/// Les mesures sont-elles en cours (/timings on) ?
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
        GENERATION.fetch_add(1, Ordering::Relaxed);
        *guard = Some(State { since: Instant::now(), ticks: 0, entries: HashMap::new(), machine: Machine::sample() });
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
    flush_current();
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
    let start = STATE.lock().ok().and_then(|g| g.as_ref().and_then(|s| s.machine));
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let usage = start.zip(Machine::sample()).and_then(|(a, b)| machine_use(a, b, cores));
    let pause_worst = lines.iter().find(|l| l.name.starts_with("Pause de 1 ms")).map(|l| l.worst_ms).unwrap_or(0.0);
    let pause_slow = lines.iter().find(|l| l.name.starts_with("Pause de 1 ms")).map(|l| l.slow).unwrap_or(0);
    let mut out = String::new();
    out.push_str("Rapport de mesures de Basalt\n");
    out.push_str("============================\n");
    for h in header {
        out.push_str(h);
        out.push('\n');
    }
    out.push_str(&format!("Durée mesurée : {:.1} s — {} ticks, soit {:.2} ticks/s (objectif 20)\n", elapsed, ticks, ticks as f64 / elapsed));
    if let Some(u) = &usage {
        out.push_str(&format!(
            "Machine pendant les mesures : {} cœur(s) occupé(s) à {:.0} % — Basalt {:.0} % d'un cœur — attente disque {:.1} % — temps volé par l'hébergeur {:.1} %\n",
            u.cores, u.busy, u.own_core, u.iowait, u.steal
        ));
    }
    if let Some((free, total, swap)) = memory() {
        out.push_str(&format!("Mémoire de la machine : {} Mo libres sur {} Mo — swap utilisé : {} Mo\n", free, total, swap));
    }
    out.push_str(&verdict(pause_worst, pause_slow, usage.as_ref(), memory()));
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

/// Explication en clair des à-coups, d'après la pause et les compteurs de la machine.
fn verdict(pause_worst: f64, pause_slow: u64, usage: Option<&MachineUse>, mem: Option<(u64, u64, u64)>) -> String {
    if pause_worst < 50.0 {
        return "Diagnostic : la machine rend la main à temps ; les à-coups éventuels viennent des étapes ci-dessous.\n".into();
    }
    let mut why = Vec::new();
    if let Some(u) = usage {
        if u.steal >= 2.0 {
            why.push(format!("l'hébergeur prend du processeur à la machine virtuelle ({:.1} % volé)", u.steal));
        }
        if u.iowait >= 5.0 {
            why.push(format!("le disque est lent ou saturé ({:.1} % d'attente disque)", u.iowait));
        }
        if u.busy >= 85.0 {
            why.push(format!("le processeur est plein ({:.0} %) : d'autres programmes tournent", u.busy));
        }
    }
    if let Some((free, total, swap)) = mem {
        if swap >= 64 || (total > 0 && free * 10 < total) {
            why.push(format!("la mémoire manque ({} Mo libres, {} Mo en swap)", free, swap));
        }
    }
    let mut out = format!(
        "Diagnostic : une simple pause de 1 ms a duré jusqu'à {:.0} ms ({} fois plus de 50 ms). Ce n'est pas du travail de Basalt : le système ne lui rendait pas la main, et tout s'est figé en même temps (d'où les « pire » élevés sur des étapes sans rapport).\n",
        pause_worst, pause_slow
    );
    if why.is_empty() {
        out.push_str("  Cause probable : la machine elle-même (machine virtuelle mise en pause, mise en veille, gros programme lancé à côté). Regardez « top » (colonne st et wa) pendant un à-coup.\n");
    } else {
        out.push_str(&format!("  Cause probable : {}.\n", why.join(" ; ")));
    }
    out
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

    #[test]
    fn machine_counters_and_verdict() {
        let stat = "cpu  100 0 50 800 20 0 0 30 0 0\ncpu0 1 2 3\n";
        let own = "1234 (basalt (dev)) S 1 2 3 4 5 6 7 8 9 10 40 10 0 0";
        let a = Machine::parse(stat, own).unwrap();
        assert_eq!(a, Machine { total: 1000, idle: 800, iowait: 20, steal: 30, own: 50 });
        let b = Machine { total: 2000, idle: 1500, iowait: 120, steal: 130, own: 150 };
        let u = machine_use(a, b, 2).unwrap();
        assert!((u.steal - 10.0).abs() < 1e-9 && (u.iowait - 10.0).abs() < 1e-9 && (u.busy - 10.0).abs() < 1e-9);
        assert!((u.own_core - 20.0).abs() < 1e-9, "100 unités sur 500 par cœur");
        assert!(verdict(3.0, 0, Some(&u), None).contains("rend la main à temps"));
        let v = verdict(3762.0, 16, Some(&u), Some((100, 2000, 500)));
        assert!(v.contains("3762") && v.contains("volé") && v.contains("disque") && v.contains("mémoire"), "{}", v);
    }
}
