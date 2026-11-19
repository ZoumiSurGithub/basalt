//! Console du serveur : couleurs, et une ligne de saisie qui reste lisible.
//!
//! Sans précaution, un message du journal qui arrive pendant qu'on tape une commande se
//! mêle au texte tapé. Ici la console lit le clavier touche par touche et connaît donc la
//! saisie en cours : à chaque message, elle efface la ligne de saisie, écrit le message,
//! puis réaffiche la flèche et ce qui était tapé.
//!
//! Hors d'un terminal (journal redirigé vers un fichier, service, tests), rien de tout cela
//! ne s'active : les messages sortent tels quels et les commandes sont lues ligne par ligne.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;

/// Flèche affichée devant la saisie.
const PROMPT: &str = "\x1b[1;32m\u{25ba}\x1b[0m ";
const PROMPT_PLAIN: &str = "> ";
const MAX_LINE: usize = 512;
const MAX_HISTORY: usize = 100;

struct State {
    /// Vrai quand la ligne de saisie est affichée en bas de la console
    interactive: bool,
    buffer: String,
    history: Vec<String>,
    /// Position dans l'historique pendant qu'on le parcourt (None : saisie neuve)
    browsing: Option<usize>,
}

static STATE: Mutex<State> = Mutex::new(State { interactive: false, buffer: String::new(), history: Vec::new(), browsing: None });
/// Couleurs acceptées par la console (réglé par `init`).
static COLORS: AtomicBool = AtomicBool::new(false);

pub fn colors() -> bool {
    COLORS.load(Ordering::Relaxed)
}

fn prompt() -> &'static str {
    if colors() {
        PROMPT
    } else {
        PROMPT_PLAIN
    }
}

/// Écrit une ligne du journal sans abîmer la saisie en cours.
pub fn print_line(text: &str) {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let out = std::io::stdout();
    let mut out = out.lock();
    if state.interactive {
        // Efface la ligne de saisie, écrit le message, puis réaffiche la flèche et la saisie
        let _ = write!(out, "\r\x1b[2K{}\n{}{}", text, prompt(), state.buffer);
    } else {
        let _ = writeln!(out, "{}", text);
    }
    let _ = out.flush();
}

fn redraw(state: &State) {
    let out = std::io::stdout();
    let mut out = out.lock();
    let _ = write!(out, "\r\x1b[2K{}{}", prompt(), state.buffer);
    let _ = out.flush();
}

/// Prépare la console : couleurs, accents, saisie touche par touche. À appeler une fois au
/// démarrage ; `restore` remet la console dans son état d'origine.
pub fn init() {
    let terminal = platform::enable();
    let wanted = std::env::var_os("NO_COLOR").is_none();
    COLORS.store(terminal.colors && wanted, Ordering::Relaxed);
    if terminal.raw_input {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.interactive = true;
        redraw(&state);
    }
}

/// Remet la console comme elle était (à l'arrêt du serveur).
pub fn restore() {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.interactive {
        state.interactive = false;
        let out = std::io::stdout();
        let mut out = out.lock();
        let _ = write!(out, "\r\x1b[2K");
        let _ = out.flush();
    }
    drop(state);
    platform::restore();
}

/// Traite une touche (ou un caractère). Renvoie la commande validée par Entrée, s'il y en a une.
fn feed(state: &mut State, key: Key) -> Option<String> {
    match key {
        Key::Char(c) => {
            if state.buffer.chars().count() < MAX_LINE {
                state.buffer.push(c);
            }
            state.browsing = None;
        }
        Key::Backspace => {
            state.buffer.pop();
        }
        Key::ClearLine => state.buffer.clear(),
        Key::Up => {
            if !state.history.is_empty() {
                let at = match state.browsing {
                    Some(i) => i.saturating_sub(1),
                    None => state.history.len() - 1,
                };
                state.browsing = Some(at);
                state.buffer = state.history[at].clone();
            }
        }
        Key::Down => match state.browsing {
            Some(i) if i + 1 < state.history.len() => {
                state.browsing = Some(i + 1);
                state.buffer = state.history[i + 1].clone();
            }
            Some(_) => {
                state.browsing = None;
                state.buffer.clear();
            }
            None => {}
        },
        Key::Enter => {
            let line = std::mem::take(&mut state.buffer);
            state.browsing = None;
            let trimmed = line.trim().to_string();
            if !trimmed.is_empty() {
                if state.history.last() != Some(&trimmed) {
                    state.history.push(trimmed.clone());
                    if state.history.len() > MAX_HISTORY {
                        state.history.remove(0);
                    }
                }
                return Some(trimmed);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Key {
    Char(char),
    Backspace,
    Enter,
    Up,
    Down,
    /// Ctrl+U : efface la saisie
    ClearLine,
}

/// Transforme les octets du clavier en touches : texte UTF-8, Entrée, Retour arrière,
/// flèches (séquences d'échappement). Le reste est ignoré.
#[derive(Default)]
struct Decoder {
    pending: Vec<u8>,
    /// 0 : texte ; 1 : après ESC ; 2 : dans une séquence « ESC [ … »
    escape: u8,
    last_was_cr: bool,
}

impl Decoder {
    fn push(&mut self, byte: u8, keys: &mut Vec<Key>) {
        match self.escape {
            1 => {
                self.escape = if byte == b'[' || byte == b'O' { 2 } else { 0 };
                return;
            }
            2 => {
                // La séquence se termine par une lettre ou « ~ »
                if (0x40..=0x7e).contains(&byte) {
                    self.escape = 0;
                    match byte {
                        b'A' => keys.push(Key::Up),
                        b'B' => keys.push(Key::Down),
                        _ => {}
                    }
                }
                return;
            }
            _ => {}
        }
        let was_cr = std::mem::replace(&mut self.last_was_cr, byte == b'\r');
        match byte {
            0x1b => self.escape = 1,
            b'\r' => keys.push(Key::Enter),
            b'\n' => {
                if !was_cr {
                    keys.push(Key::Enter); // « \r\n » ne compte que pour une Entrée
                }
            }
            0x7f | 0x08 => keys.push(Key::Backspace),
            0x15 => keys.push(Key::ClearLine),
            b'\t' => keys.push(Key::Char(' ')),
            0..=0x1f => {}
            _ => {
                self.pending.push(byte);
                match std::str::from_utf8(&self.pending) {
                    Ok(text) => {
                        keys.extend(text.chars().map(Key::Char));
                        self.pending.clear();
                    }
                    Err(e) if e.error_len().is_some() || self.pending.len() > 4 => self.pending.clear(),
                    Err(_) => {} // caractère incomplet : on attend la suite
                }
            }
        }
    }
}

/// Lit le clavier jusqu'à la fermeture de l'entrée et envoie chaque commande validée.
/// À lancer sur son propre fil d'exécution.
pub fn read_commands(tx: Sender<String>) {
    let interactive = STATE.lock().map(|s| s.interactive).unwrap_or(false);
    if !interactive {
        // Entrée redirigée : lecture ligne par ligne, sans affichage
        use std::io::BufRead;
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
        return;
    }
    let mut decoder = Decoder::default();
    let mut bytes = [0u8; 256];
    let mut keys = Vec::new();
    let mut stdin = std::io::stdin();
    loop {
        let n = match stdin.read(&mut bytes) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        keys.clear();
        for b in &bytes[..n] {
            decoder.push(*b, &mut keys);
        }
        let mut commands = Vec::new();
        {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            if !state.interactive {
                return; // la console a été rendue (arrêt du serveur)
            }
            for key in &keys {
                if *key == Key::Enter {
                    // La commande validée reste visible au-dessus, comme une ligne du journal
                    let out = std::io::stdout();
                    let mut out = out.lock();
                    let _ = writeln!(out, "\r\x1b[2K{}{}", prompt(), state.buffer);
                    let _ = out.flush();
                }
                if let Some(command) = feed(&mut state, *key) {
                    commands.push(command);
                }
            }
            redraw(&state);
        }
        for command in commands {
            if tx.send(command).is_err() {
                return;
            }
        }
    }
}

/// Ce que la console accepte.
struct Terminal {
    colors: bool,
    raw_input: bool,
}

#[cfg(unix)]
mod platform {
    use super::Terminal;
    use std::sync::Mutex;

    /// Réglages du terminal (disposition de Linux ; seule cible Unix prise en charge pour la
    /// saisie touche par touche).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Termios {
        iflag: u32,
        oflag: u32,
        cflag: u32,
        lflag: u32,
        line: u8,
        cc: [u8; 32],
        ispeed: u32,
        ospeed: u32,
    }

    extern "C" {
        fn isatty(fd: i32) -> i32;
        fn tcgetattr(fd: i32, termios: *mut Termios) -> i32;
        fn tcsetattr(fd: i32, actions: i32, termios: *const Termios) -> i32;
    }

    const ICANON: u32 = 0o2;
    const ECHO: u32 = 0o10;
    const VTIME: usize = 5;
    const VMIN: usize = 6;

    static SAVED: Mutex<Option<Termios>> = Mutex::new(None);

    pub fn enable() -> Terminal {
        let out_tty = unsafe { isatty(1) } == 1;
        let in_tty = unsafe { isatty(0) } == 1;
        let dumb = std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false);
        let mut raw_input = false;
        if cfg!(target_os = "linux") && in_tty && out_tty && !dumb {
            let mut t = Termios { iflag: 0, oflag: 0, cflag: 0, lflag: 0, line: 0, cc: [0; 32], ispeed: 0, ospeed: 0 };
            if unsafe { tcgetattr(0, &mut t) } == 0 {
                if let Ok(mut saved) = SAVED.lock() {
                    *saved = Some(t);
                }
                // Lecture touche par touche, sans écho ; Ctrl+C garde son effet
                t.lflag &= !(ICANON | ECHO);
                t.cc[VMIN] = 1;
                t.cc[VTIME] = 0;
                raw_input = unsafe { tcsetattr(0, 0, &t) } == 0;
            }
        }
        Terminal { colors: out_tty && !dumb, raw_input }
    }

    pub fn restore() {
        if let Ok(mut saved) = SAVED.lock() {
            if let Some(t) = saved.take() {
                unsafe { tcsetattr(0, 0, &t) };
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::Terminal;
    use std::sync::Mutex;

    type Handle = *mut std::ffi::c_void;

    extern "system" {
        fn GetStdHandle(which: u32) -> Handle;
        fn GetConsoleMode(handle: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: Handle, mode: u32) -> i32;
        fn SetConsoleOutputCP(code_page: u32) -> i32;
        fn SetConsoleCP(code_page: u32) -> i32;
    }

    const STD_INPUT: u32 = -10i32 as u32;
    const STD_OUTPUT: u32 = -11i32 as u32;
    const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
    const ENABLE_LINE_INPUT: u32 = 0x0002;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    /// Modes d'origine : (entrée, sortie)
    static SAVED: Mutex<(Option<u32>, Option<u32>)> = Mutex::new((None, None));

    pub fn enable() -> Terminal {
        let mut terminal = Terminal { colors: false, raw_input: false };
        unsafe {
            let output = GetStdHandle(STD_OUTPUT);
            let mut out_mode = 0u32;
            if GetConsoleMode(output, &mut out_mode) != 0 {
                // Couleurs et effacement de ligne (séquences ANSI), accents en UTF-8
                SetConsoleOutputCP(65001);
                if SetConsoleMode(output, out_mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0 {
                    terminal.colors = true;
                    if let Ok(mut saved) = SAVED.lock() {
                        saved.1 = Some(out_mode);
                    }
                }
            }
            let input = GetStdHandle(STD_INPUT);
            let mut in_mode = 0u32;
            if terminal.colors && GetConsoleMode(input, &mut in_mode) != 0 {
                SetConsoleCP(65001);
                // Touche par touche, sans écho ; Ctrl+C garde son effet ; flèches en séquences ANSI
                let raw = (in_mode & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT)) | ENABLE_PROCESSED_INPUT | ENABLE_VIRTUAL_TERMINAL_INPUT;
                if SetConsoleMode(input, raw) != 0 {
                    terminal.raw_input = true;
                    if let Ok(mut saved) = SAVED.lock() {
                        saved.0 = Some(in_mode);
                    }
                }
            }
        }
        terminal
    }

    pub fn restore() {
        if let Ok(mut saved) = SAVED.lock() {
            unsafe {
                if let Some(mode) = saved.0.take() {
                    SetConsoleMode(GetStdHandle(STD_INPUT), mode);
                }
                if let Some(mode) = saved.1.take() {
                    SetConsoleMode(GetStdHandle(STD_OUTPUT), mode);
                }
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::Terminal;

    pub fn enable() -> Terminal {
        Terminal { colors: false, raw_input: false }
    }

    pub fn restore() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_of(bytes: &[u8]) -> Vec<Key> {
        let mut d = Decoder::default();
        let mut keys = Vec::new();
        for b in bytes {
            d.push(*b, &mut keys);
        }
        keys
    }

    fn type_all(state: &mut State, bytes: &[u8]) -> Vec<String> {
        keys_of(bytes).into_iter().filter_map(|k| feed(state, k)).collect()
    }

    #[test]
    fn keystrokes_become_commands() {
        let mut s = State { interactive: true, buffer: String::new(), history: Vec::new(), browsing: None };
        // Texte, accents (plusieurs octets), retour arrière, Entrée sous Windows (\r\n) et sous Linux (\n)
        assert_eq!(type_all(&mut s, "whitelist add Zoé".as_bytes()), Vec::<String>::new());
        assert_eq!(s.buffer, "whitelist add Zoé");
        assert_eq!(type_all(&mut s, &[0x7f, 0x08]), Vec::<String>::new());
        assert_eq!(s.buffer, "whitelist add Z");
        assert_eq!(type_all(&mut s, b"ed\r\n"), vec!["whitelist add Zed".to_string()]);
        assert_eq!(s.buffer, "");
        assert_eq!(type_all(&mut s, b"list\n"), vec!["list".to_string()]);
        // Entrée seule : rien ; Ctrl+U efface la saisie
        assert_eq!(type_all(&mut s, b"\r\n   \r"), Vec::<String>::new());
        type_all(&mut s, b"brouillon\x15");
        assert_eq!(s.buffer, "");
        // Flèches : historique vers le haut puis vers le bas ; les autres séquences sont ignorées
        type_all(&mut s, b"\x1b[A");
        assert_eq!(s.buffer, "list");
        type_all(&mut s, b"\x1b[A");
        assert_eq!(s.buffer, "whitelist add Zed");
        type_all(&mut s, b"\x1b[A\x1b[B");
        assert_eq!(s.buffer, "list");
        type_all(&mut s, b"\x1b[B\x1b[C\x1b[D\x1b[3~\x1b[1;5C");
        assert_eq!(s.buffer, "");
        // Une commande répétée n'encombre pas l'historique ; la saisie est bornée
        type_all(&mut s, b"list\rlist\r");
        assert_eq!(s.history, vec!["whitelist add Zed".to_string(), "list".to_string()]);
        type_all(&mut s, &vec![b'a'; 2000]);
        assert_eq!(s.buffer.len(), MAX_LINE);
        // Octets invalides : ignorés sans rien casser
        assert_eq!(keys_of(&[0xff, 0xfe, b'o', b'k']), vec![Key::Char('o'), Key::Char('k')]);
    }
}
