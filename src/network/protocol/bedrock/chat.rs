//! Chat et mise à jour de blocs.

#[allow(unused_imports)]
use super::*;

// ------------------------------------------------------------------ Chat ----

pub const ID_TEXT: u32 = 9;
pub const ID_UPDATE_BLOCK: u32 = 21;

/// Message de chat d'un joueur (format 1.26.60 : variante « auteur et message », type 1).
pub fn text_chat(author: &str, message: &str) -> Vec<u8> {
    let mut w = W::default();
    w.bool(false); // pas de traduction
    w.varu32(1); // variante : auteur et message
    w.u8(1); // type : chat
    w.string(author);
    w.string(message);
    w.string(""); // XUID de l'auteur
    w.string(""); // identifiant de plateforme
    w.bool(false); // message filtré : absent
    w.0
}

/// Message du serveur, sans auteur (variante « message seul », type 0).
pub fn text_raw(message: &str) -> Vec<u8> {
    let mut w = W::default();
    w.bool(false);
    w.varu32(0);
    w.u8(0);
    w.string(message);
    w.string("");
    w.string("");
    w.bool(false);
    w.0
}

pub const ID_AVAILABLE_COMMANDS: u32 = 76;

/// Types de paramètres compris par le jeu (valeurs relevées auprès du serveur officiel).
const ARG_INT: u32 = 0x0010_0001;
const ARG_FLOAT: u32 = 0x0010_0003;
const ARG_PLAYER: u32 = 0x0010_0008;
const ARG_STRING: u32 = 0x0010_0038;
/// Une position : trois coordonnées, éventuellement relatives (`~ ~1 ~`)
const ARG_POSITION: u32 = 0x0010_0040;
const ARG_TEXT: u32 = 0x0010_0044;
/// Un paramètre pris dans une liste : ce drapeau, plus le numéro de la liste.
const ARG_ENUM: u32 = 0x0030_0000;

/// Ce qu'un paramètre de commande accepte.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgKind {
    Int,
    Float,
    Player,
    /// Un mot
    String,
    /// Tout le reste de la ligne
    Text,
    /// Un bloc du jeu, proposé par le jeu pendant la frappe
    Block,
    /// Un objet du jeu
    Item,
    /// Une position dans le monde (trois nombres)
    Position,
    /// Un choix parmi quelques mots
    Choice(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandArg {
    pub name: String,
    pub kind: ArgKind,
    pub optional: bool,
}

/// Une forme d'une commande : la suite de ses paramètres.
pub type Overload = Vec<CommandArg>;

/// Formes décrites par la ligne `usage` d'un plugin.yml. `<nom>` est obligatoire, `[nom]`
/// facultatif ; un type peut suivre le nom (`<rayon:int>`) parmi `string` (par défaut), `int`,
/// `float`, `player`, `text` (tout le reste de la ligne), `block`, `item`, ou une liste de
/// choix (`<mode:on|off>`). Plusieurs formes se séparent par « | » :
/// `/set <bloc:block> | /set <motif:text>`.
pub fn command_overloads(usage: &str) -> Vec<Overload> {
    // Découpage sur les « | » qui ne sont pas à l'intérieur d'un paramètre
    let mut forms = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in usage.char_indices() {
        match c {
            '<' | '[' => depth += 1,
            '>' | ']' => depth -= 1,
            '|' if depth <= 0 => {
                forms.push(&usage[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    forms.push(&usage[start..]);
    let mut out: Vec<Overload> = forms.into_iter().take(8).map(command_args).collect();
    out.dedup();
    out
}

fn command_args(usage: &str) -> Overload {
    let mut out: Overload = Vec::new();
    let mut rest = usage;
    while out.len() < 16 {
        let start = match rest.find(|c| c == '<' || c == '[' || c == '(') {
            Some(s) => s,
            None => break,
        };
        let open = rest.as_bytes()[start];
        if open == b'(' {
            break; // une remarque entre parenthèses termine la description
        }
        let close = if open == b'<' { '>' } else { ']' };
        let end = match rest[start + 1..].find(close) {
            Some(e) => start + 1 + e,
            None => break,
        };
        let inner = &rest[start + 1..end];
        rest = &rest[end + 1..];
        let (name, kind) = match inner.split_once(':') {
            Some((n, k)) => (n.trim(), k.trim()),
            None => (inner.trim(), "string"),
        };
        let name: String = name.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == ' ').take(32).collect();
        if name.is_empty() {
            continue;
        }
        let kind = match kind {
            "int" => ArgKind::Int,
            "float" => ArgKind::Float,
            "player" => ArgKind::Player,
            "text" => ArgKind::Text,
            "block" => ArgKind::Block,
            "item" => ArgKind::Item,
            "pos" | "position" => ArgKind::Position,
            k if k.contains('|') => {
                let words: Vec<String> = k
                    .split('|')
                    .map(|w| w.trim().chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == ':' || *c == '.').take(48).collect::<String>())
                    .filter(|w| !w.is_empty())
                    .take(64)
                    .collect();
                if words.is_empty() {
                    ArgKind::String
                } else {
                    ArgKind::Choice(words)
                }
            }
            "string" | "" => ArgKind::String,
            // Un mot imposé (`<mode:set>`)
            word => {
                let word: String = word.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').take(48).collect();
                if word.is_empty() {
                    ArgKind::String
                } else {
                    ArgKind::Choice(vec![word])
                }
            }
        };
        // Après un paramètre facultatif, tous les suivants le sont aussi
        let optional = open == b'[' || out.last().map(|a| a.optional).unwrap_or(false);
        let last = kind == ArgKind::Text;
        out.push(CommandArg { name, kind, optional });
        if last {
            break; // le texte libre prend la fin de la ligne
        }
    }
    out
}

/// Listes de mots annoncées au jeu, sans doublon de valeurs.
#[derive(Default)]
struct Enums {
    values: Vec<String>,
    index: std::collections::HashMap<String, u32>,
    /// (nom, numéros des valeurs)
    lists: Vec<(String, Vec<u32>)>,
}

impl Enums {
    fn list<'a>(&mut self, name: &str, words: impl Iterator<Item = &'a str>) -> u32 {
        if let Some(i) = self.lists.iter().position(|l| l.0 == name) {
            return i as u32;
        }
        let mut ids = Vec::new();
        for w in words {
            let next = self.values.len() as u32;
            let id = *self.index.entry(w.to_string()).or_insert(next);
            if id == next {
                self.values.push(w.to_string());
            }
            ids.push(id);
        }
        self.lists.push((name.to_string(), ids));
        self.lists.len() as u32 - 1
    }
}

/// Liste des commandes annoncée au jeu (format relevé auprès du serveur officiel, protocole
/// 2225) : sans elle, le jeu n'affiche ni n'envoie aucune commande en « / ».
/// Une commande sans forme connue (`None`) reçoit un texte libre facultatif. `blocks` et
/// `items` sont les noms complets (`minecraft:stone`) proposés pour les paramètres de ces types.
pub fn available_commands(commands: &[(String, String, Option<Vec<Overload>>)], blocks: &[String], items: &[String]) -> Vec<u8> {
    let free = vec![vec![CommandArg { name: "args".to_string(), kind: ArgKind::Text, optional: true }]];
    let mut enums = Enums::default();
    // Comme le serveur officiel : chaque nom existe avec et sans « minecraft: »
    fn both(names: &[String]) -> impl Iterator<Item = &str> {
        names.iter().flat_map(|n| {
            let short = n.strip_prefix("minecraft:");
            std::iter::once(n.as_str()).chain(short)
        })
    }
    let mut body = W::default();
    body.varu32(commands.len() as u32);
    for (name, description, forms) in commands {
        body.string(name);
        body.string(description);
        body.u8(0); // drapeaux (u16)
        body.u8(0);
        body.string("any"); // la permission réelle est vérifiée par le serveur
        body.i32(-1); // pas d'alias
        body.varu32(0); // pas de sous-commandes chaînées
        let forms = match forms {
            Some(f) if !f.is_empty() => f,
            _ => &free,
        };
        body.varu32(forms.len() as u32);
        for form in forms {
            body.bool(false);
            body.varu32(form.len() as u32);
            for arg in form {
                // (type, options) : les blocs et les objets portent l'option 2, comme chez le
                // serveur officiel
                let (kind, options) = match &arg.kind {
                    ArgKind::Int => (ARG_INT, 0),
                    ArgKind::Float => (ARG_FLOAT, 0),
                    ArgKind::Player => (ARG_PLAYER, 0),
                    ArgKind::String => (ARG_STRING, 0),
                    ArgKind::Text => (ARG_TEXT, 0),
                    ArgKind::Position => (ARG_POSITION, 0),
                    ArgKind::Block => (ARG_ENUM | enums.list("Block", both(blocks)), 2),
                    ArgKind::Item => (ARG_ENUM | enums.list("Item", both(items)), 2),
                    ArgKind::Choice(words) => {
                        let list = format!("{}_{}_{}", name, arg.name, words.join("-"));
                        (ARG_ENUM | enums.list(&list, words.iter().map(|w| w.as_str())), 0)
                    }
                };
                body.string(&arg.name);
                body.u32(kind);
                body.bool(arg.optional);
                body.u8(options);
            }
        }
    }
    let mut w = W::default();
    w.varu32(enums.values.len() as u32);
    for v in &enums.values {
        w.string(v);
    }
    w.varu32(0); // sous-commandes chaînées
    w.varu32(0); // suffixes
    w.varu32(enums.lists.len() as u32);
    for (name, ids) in &enums.lists {
        w.string(name);
        w.varu32(ids.len() as u32);
        for id in ids {
            w.u32(*id);
        }
    }
    w.varu32(0); // chaînes de sous-commandes
    w.0.extend_from_slice(&body.0);
    w.varu32(0); // énumérations dynamiques
    w.varu32(0); // contraintes
    w.0
}

pub(crate) fn read_string<'a>(d: &'a [u8], pos: &mut usize, max: usize) -> Option<&'a str> {
    let len = read_varu32(d, pos)? as usize;
    if len > max {
        return None;
    }
    let bytes = d.get(*pos..pos.checked_add(len)?)?;
    *pos += len;
    std::str::from_utf8(bytes).ok()
}

/// Lit un message de chat envoyé par un joueur. Seule la variante « auteur et message »
/// de type chat est acceptée ; le nom d'auteur annoncé est ignoré par le serveur.
pub fn read_chat(body: &[u8]) -> Option<String> {
    let mut pos = 1; // « à traduire »
    let variant = read_varu32(body, &mut pos)?;
    let kind = *body.get(pos)?;
    pos += 1;
    if variant != 1 || kind != 1 {
        return None;
    }
    let _author = read_string(body, &mut pos, 64)?;
    let message = read_string(body, &mut pos, 512)?;
    Some(message.to_string())
}

pub fn update_block(x: i32, y: i32, z: i32, block: u32) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(x);
    w.vari32(y);
    w.vari32(z);
    w.varu32(block);
    w.varu32(3); // prévenir les voisins et les joueurs
    w.varu32(0); // couche des blocs
    w.0
}

#[cfg(test)]
mod command_list_tests {
    use super::*;

    fn arg(name: &str, kind: ArgKind, optional: bool) -> CommandArg {
        CommandArg { name: name.to_string(), kind, optional }
    }

    /// Même disposition que la commande « me » relevée auprès du serveur officiel
    /// (`026d65 17… c000 03616e79 ffffffff 00 01 00 01 076d657373616765 44001000 00 00`).
    #[test]
    fn a_command_is_announced_like_the_official_server_does() {
        let p = available_commands(&[("me".to_string(), "d".to_string(), None)], &[], &[]);
        let expected: Vec<u8> = vec![
            0, 0, 0, 0, 0, 1, 2, b'm', b'e', 1, b'd', 0, 0, 3, b'a', b'n', b'y', 0xff, 0xff, 0xff, 0xff, 0, 1, 0, 1, 4, b'a', b'r', b'g', b's', 0x44, 0x00, 0x10, 0x00, 1, 0, 0, 0,
        ];
        assert_eq!(p, expected);
    }

    #[test]
    fn usage_lines_become_parameters() {
        assert_eq!(command_overloads("/wand"), vec![Vec::new()]);
        assert_eq!(command_overloads("/set <bloc>   (ou plusieurs : <a>,<b>)"), vec![vec![arg("bloc", ArgKind::String, false)]]);
        assert_eq!(
            command_overloads("/cyl <bloc:block> <rayon:int> [hauteur:int]"),
            vec![vec![arg("bloc", ArgKind::Block, false), arg("rayon", ArgKind::Int, false), arg("hauteur", ArgKind::Int, true)]]
        );
        assert_eq!(command_overloads("/dire [qui:player] <message:text> <ignoré>"), vec![vec![arg("qui", ArgKind::Player, true), arg("message", ArgKind::Text, true)]]);
        assert_eq!(
            command_overloads("/set <bloc:block> | /set <motif:text>"),
            vec![vec![arg("bloc", ArgKind::Block, false)], vec![arg("motif", ArgKind::Text, false)]]
        );
        assert_eq!(
            command_overloads("/mode <etat:on|off> [qui]"),
            vec![vec![arg("etat", ArgKind::Choice(vec!["on".into(), "off".into()]), false), arg("qui", ArgKind::String, true)]]
        );
    }

    /// Un paramètre « bloc » : la liste « Block » comme chez le serveur officiel
    /// (`tileName 18003000 00 02`), avec chaque nom sous ses deux formes.
    #[test]
    fn block_parameters_come_with_the_list_of_blocks() {
        let forms = command_overloads("/set <bloc:block> [mode:a|b]");
        let p = available_commands(&[("set".to_string(), "d".to_string(), Some(forms))], &["minecraft:stone".to_string()], &[]);
        let mut expected: Vec<u8> = vec![4];
        for w in ["minecraft:stone", "stone", "a", "b"] {
            expected.push(w.len() as u8);
            expected.extend_from_slice(w.as_bytes());
        }
        expected.extend_from_slice(&[0, 0, 2]);
        expected.extend_from_slice(b"\x05Block\x02\x00\x00\x00\x00\x01\x00\x00\x00");
        expected.extend_from_slice(b"\x0cset_mode_a-b\x02\x02\x00\x00\x00\x03\x00\x00\x00");
        expected.extend_from_slice(&[0, 1]);
        expected.extend_from_slice(b"\x03set\x01d\x00\x00\x03any\xff\xff\xff\xff\x00\x01\x00\x02");
        expected.extend_from_slice(b"\x04bloc\x00\x00\x30\x00\x00\x02");
        expected.extend_from_slice(b"\x04mode\x01\x00\x30\x00\x01\x00");
        expected.extend_from_slice(&[0, 0]);
        assert_eq!(p, expected);
    }
}
