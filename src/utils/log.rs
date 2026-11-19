//! Journal : console en couleur (voir `console`) et fichier `logs/latest.log` sans couleur.

use std::fs::File;
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub enum Level {
    Info,
    Warn,
    Error,
}

static FILE: Mutex<Option<File>> = Mutex::new(None);

/// Enregistre aussi le journal dans `logs/latest.log` ; le journal de l'exécution
/// précédente est gardé dans `logs/previous.log`.
/// Journal de l'outil de capture : `logs/capture.log`, sans toucher au journal du serveur.
pub fn init_capture_file() {
    let _ = std::fs::create_dir_all("logs");
    if let Ok(f) = File::create("logs/capture.log") {
        if let Ok(mut slot) = FILE.lock() {
            *slot = Some(f);
        }
    }
}

pub fn init_file() {
    let _ = std::fs::create_dir_all("logs");
    let _ = std::fs::rename("logs/latest.log", "logs/previous.log");
    match File::create("logs/latest.log") {
        Ok(f) => {
            if let Ok(mut slot) = FILE.lock() {
                *slot = Some(f);
            }
        }
        Err(e) => write(Level::Warn, &format!("Journal sur disque indisponible : {}", e)),
    }
}

pub fn write(level: Level, msg: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86_400;
    // Heure en gris ; niveau en couleur ; le texte d'un avertissement ou d'une erreur aussi
    let (label, color, text_color) = match level {
        Level::Info => ("INFO", "\x1b[36m", ""),
        Level::Warn => ("ATTENTION", "\x1b[1;33m", "\x1b[33m"),
        Level::Error => ("ERREUR", "\x1b[1;31m", "\x1b[31m"),
    };
    let time = format!("[{:02}:{:02}:{:02}]", secs / 3600, secs / 60 % 60, secs % 60);
    if let Ok(mut slot) = FILE.lock() {
        if let Some(f) = slot.as_mut() {
            let _ = writeln!(f, "{} [{}] {}", time, label, msg);
        }
    }
    let line = if crate::console::colors() {
        format!("\x1b[90m{}\x1b[0m {}[{}]\x1b[0m {}{}\x1b[0m", time, color, label, text_color, strip_game_colors(msg))
    } else {
        format!("{} [{}] {}", time, label, msg)
    };
    crate::console::print_line(&line);
}

/// Retire d'un message les codes de couleur de Minecraft (« §e », « §6 »…), que la console
/// ne comprend pas.
fn strip_game_colors(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut chars = msg.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// Retire les caractères de contrôle d'un texte venu du réseau avant affichage.
pub fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

#[macro_export]
macro_rules! info {
    ($($a:tt)*) => { $crate::log::write($crate::log::Level::Info, &format!($($a)*)) };
}
#[macro_export]
macro_rules! warn {
    ($($a:tt)*) => { $crate::log::write($crate::log::Level::Warn, &format!($($a)*)) };
}
#[macro_export]
macro_rules! error {
    ($($a:tt)*) => { $crate::log::write($crate::log::Level::Error, &format!($($a)*)) };
}
