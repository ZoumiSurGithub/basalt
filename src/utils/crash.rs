//! Rapport de plantage. Quand le serveur rencontre une erreur interne (un « panic »), un
//! fichier `crashdumps/crash-<date>.txt` est écrit : l'erreur, l'endroit du code, la pile
//! d'appels, la version, l'état du serveur et les dernières lignes du journal. C'est le
//! fichier à joindre pour signaler le problème.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Dernières lignes du journal, gardées pour le rapport.
static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// État du serveur, tenu à jour par la boucle principale (joueurs, mondes, plugins…).
static CONTEXT: Mutex<String> = Mutex::new(String::new());
/// Heure du dernier rapport (secondes) : pas plus d'un toutes les dix secondes.
static LAST: AtomicU64 = AtomicU64::new(0);
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

const KEPT_LINES: usize = 80;

/// Note une ligne du journal (appelé par `log::write`).
pub fn remember(line: &str) {
    if let Ok(mut recent) = RECENT.try_lock() {
        if recent.len() >= KEPT_LINES {
            recent.pop_front();
        }
        recent.push_back(line.chars().take(400).collect());
    }
}

/// Met à jour l'état du serveur joint au rapport.
pub fn set_context(text: String) {
    if let Ok(mut c) = CONTEXT.try_lock() {
        *c = text;
    }
}

/// Date et heure (UTC) d'un nombre de secondes depuis 1970 : (année, mois, jour, h, min, s).
pub fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rest = (secs % 86_400) as u32;
    // (algorithme des « jours civils » de Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day, rest / 3600, rest / 60 % 60, rest % 60)
}

/// Le texte du rapport.
fn report(message: &str, location: &str, thread: &str, backtrace: &str, fatal: bool) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil(now);
    let uptime = STARTED.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);
    let mut out = String::new();
    out.push_str("===== Rapport de plantage de Basalt =====\n\n");
    out.push_str(&format!("Date        : {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC\n", y, mo, d, h, mi, s));
    out.push_str(&format!("Version     : Basalt {} {}\n", crate::VERSION, option_env!("BASALT_BUILD").unwrap_or("")));
    out.push_str(&format!("Système     : {} ({})\n", std::env::consts::OS, std::env::consts::ARCH));
    out.push_str(&format!("En marche   : depuis {} h {:02} min {:02} s\n", uptime / 3600, uptime / 60 % 60, uptime % 60));
    out.push_str(&format!("Gravité     : {}\n", if fatal { "le serveur s'est arrêté" } else { "erreur rattrapée, le serveur a continué (le joueur ou le plugin concerné a été écarté)" }));
    out.push_str(&format!("\n--- Erreur ---\n{}\n  à : {}\n  fil : {}\n", message, location, thread));
    let context = CONTEXT.try_lock().map(|c| c.clone()).unwrap_or_default();
    if !context.is_empty() {
        out.push_str(&format!("\n--- État du serveur ---\n{}\n", context));
    }
    out.push_str(&format!("\n--- Pile d'appels ---\n{}\n", backtrace));
    out.push_str("\n--- Dernières lignes du journal ---\n");
    if let Ok(recent) = RECENT.try_lock() {
        for line in recent.iter() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str("\n(Ce fichier ne contient ni mot de passe ni jeton ; les lignes du journal peuvent\nmontrer des pseudos et des adresses IP : relisez-le avant de le partager.)\n");
    out
}

/// Installe l'écriture du rapport. `fatal` dit si l'erreur vient du fil principal hors des
/// filets de sécurité (auquel cas le serveur s'arrête).
pub fn install() {
    let _ = STARTED.set(Instant::now());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let message = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "erreur interne sans message".to_string());
        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_else(|| "endroit inconnu".to_string());
        let thread = std::thread::current().name().unwrap_or("sans nom").to_string();
        // (une rafale d'erreurs identiques ne remplit pas le disque)
        if now.saturating_sub(LAST.swap(now, Ordering::Relaxed)) >= 10 {
            let backtrace = std::backtrace::Backtrace::force_capture().to_string();
            let fatal = FATAL.load(Ordering::Relaxed) == 0 && thread == "main";
            let text = report(&message, &location, &thread, &backtrace, fatal);
            let (y, mo, d, h, mi, s) = civil(now);
            let path = std::path::Path::new("crashdumps").join(format!("crash-{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.txt", y, mo, d, h, mi, s));
            let written = std::fs::create_dir_all("crashdumps").and_then(|_| std::fs::write(&path, text));
            match written {
                Ok(()) => eprintln!("\n[ERREUR] Erreur interne : {} ({}). Rapport écrit dans {} — joignez ce fichier pour signaler le problème.", message, location, path.display()),
                Err(e) => eprintln!("\n[ERREUR] Erreur interne : {} ({}). Rapport non écrit : {}", message, location, e),
            }
        }
        previous(info);
    }));
}

/// Nombre de filets de sécurité actifs sur le fil principal (une erreur y est rattrapée).
static FATAL: AtomicU64 = AtomicU64::new(0);

/// Exécute `f` sous un filet de sécurité : une erreur interne y est rattrapée, et son
/// rapport la note comme non fatale.
pub fn guarded<T>(f: impl FnOnce() -> T) -> std::thread::Result<T> {
    FATAL.fetch_add(1, Ordering::Relaxed);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    FATAL.fetch_sub(1, Ordering::Relaxed);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_computed_without_a_library() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0));
        assert_eq!(civil(1_795_719_845), (2026, 11, 26, 19, 4, 5));
    }

    #[test]
    fn the_report_says_what_happened_and_where() {
        remember("[12:00:00] [INFO] ligne du journal");
        set_context("Joueurs : 3\nMondes : world, lobby".to_string());
        let text = report("index hors limites", "src/server.rs:42:7", "main", "0: basalt::server::tick", true);
        for needle in ["Basalt", "index hors limites", "src/server.rs:42:7", "Joueurs : 3", "basalt::server::tick", "ligne du journal", "le serveur s'est arrêté"] {
            assert!(text.contains(needle), "« {} » absent du rapport", needle);
        }
    }
}
