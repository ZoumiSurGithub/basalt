//! Fiche d'identité d'un plugin : le fichier `plugin.yml` à la racine de son dossier,
//! sur le modèle de PocketMine et d'Altay.
//!
//! ```yaml
//! name: Gardien
//! version: 1.2.0
//! main: main.lua
//! api: 1
//! description: Liste noire et expulsions
//! author: Enzo
//! depend: [Economie]
//! commands:
//!   expulser:
//!     description: Expulse un joueur
//!     usage: "/expulser <joueur>"
//!     permission: gardien.expulser
//! permissions:
//!   gardien.expulser:
//!     description: Autorise /expulser
//!     default: op
//! ```

use crate::json::Value;
use crate::yaml;

/// Version de l'interface des plugins proposée par ce serveur.
pub const API_VERSION: i64 = 1;

pub use crate::permission::{PermDefault, Permission as PermDecl};

#[derive(Debug, Clone, PartialEq)]
pub struct CommandDecl {
    pub name: String,
    pub description: String,
    pub usage: String,
    pub permission: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    /// Fichier principal, relatif au dossier du plugin
    pub main: String,
    pub api: i64,
    pub description: String,
    pub authors: Vec<String>,
    pub website: String,
    /// Plugins indispensables : chargés avant, et leur absence empêche le chargement
    pub depend: Vec<String>,
    /// Plugins facultatifs : chargés avant s'ils sont présents
    pub softdepend: Vec<String>,
    pub commands: Vec<CommandDecl>,
    pub permissions: Vec<PermDecl>,
}

fn text(v: Option<&Value>, max: usize) -> Option<String> {
    let s = match v? {
        Value::Str(s) => s.clone(),
        Value::Num(n) => yaml::format_number(*n),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    Some(s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string())
}

/// Liste de textes : `[a, b]`, une liste à tirets, ou un seul texte.
fn list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Arr(items)) => items.iter().filter_map(|i| text(Some(i), 64)).filter(|s| !s.is_empty()).take(32).collect(),
        Some(other) => text(Some(other), 64).filter(|s| !s.is_empty()).into_iter().collect(),
        None => Vec::new(),
    }
}

/// Nom de plugin : lettres, chiffres, `_` et `-`, 32 caractères au plus.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Nom de permission ou de commande : minuscules, chiffres, `.`, `_`, `-`.
fn valid_node(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

impl Manifest {
    /// Fiche d'un plugin sans `plugin.yml` (un simple fichier .lua, ou un dossier avec main.lua).
    pub fn implicit(name: &str, main: &str) -> Manifest {
        Manifest {
            name: name.to_string(),
            version: "0".into(),
            main: main.to_string(),
            api: API_VERSION,
            description: String::new(),
            authors: Vec::new(),
            website: String::new(),
            depend: Vec::new(),
            softdepend: Vec::new(),
            commands: Vec::new(),
            permissions: Vec::new(),
        }
    }

    pub fn parse(source: &str) -> Result<Manifest, String> {
        let doc = yaml::parse(source)?;
        if !matches!(doc, Value::Obj(_)) {
            return Err("le fichier doit contenir des lignes « clé: valeur »".into());
        }
        let name = text(doc.get("name"), 64).ok_or("« name » manquant")?;
        if !valid_name(&name) {
            return Err(format!("« name: {} » invalide (lettres, chiffres, _ et - ; 32 caractères au plus)", name));
        }
        let version = text(doc.get("version"), 32).filter(|v| !v.is_empty()).ok_or("« version » manquant")?;
        let main = text(doc.get("main"), 128).unwrap_or_else(|| "main.lua".to_string()).replace('\\', "/");
        if main.is_empty() || main.starts_with('/') || main.contains("..") || main.contains(':') || !main.ends_with(".lua") {
            return Err(format!("« main: {} » invalide : un fichier .lua du dossier du plugin est attendu", main));
        }
        let api = match doc.get("api") {
            None => API_VERSION,
            Some(Value::Num(n)) if n.fract() == 0.0 => *n as i64,
            Some(Value::Arr(items)) => {
                // Liste de versions acceptées : il suffit que la nôtre y soit
                let wanted: Vec<i64> = items.iter().filter_map(|i| if let Value::Num(n) = i { Some(*n as i64) } else { None }).collect();
                if wanted.contains(&API_VERSION) {
                    API_VERSION
                } else {
                    wanted.first().copied().unwrap_or(-1)
                }
            }
            Some(_) => return Err("« api » doit être un nombre entier".into()),
        };
        if api != API_VERSION {
            return Err(format!("écrit pour l'interface {} alors que ce serveur propose la {}", api, API_VERSION));
        }
        let mut authors = list(doc.get("authors"));
        if let Some(a) = text(doc.get("author"), 64).filter(|a| !a.is_empty()) {
            authors.insert(0, a);
        }

        let mut commands = Vec::new();
        if let Some(Value::Obj(items)) = doc.get("commands") {
            for (raw, body) in items.iter().take(64) {
                let name = raw.trim().trim_start_matches('/').to_lowercase();
                if !valid_node(&name) || name.contains('.') {
                    return Err(format!("commande « {} » : nom invalide", raw));
                }
                let permission = text(body.get("permission"), 64).filter(|p| !p.is_empty());
                if let Some(p) = &permission {
                    if !valid_node(p) {
                        return Err(format!("commande « {} » : permission « {} » invalide", name, p));
                    }
                }
                commands.push(CommandDecl {
                    name,
                    description: text(body.get("description"), 128).unwrap_or_default(),
                    usage: text(body.get("usage"), 512).unwrap_or_default(),
                    permission,
                });
            }
        }
        let mut permissions = Vec::new();
        if let Some(Value::Obj(items)) = doc.get("permissions") {
            for (raw, body) in items.iter().take(256) {
                let name = raw.trim().to_lowercase();
                if !valid_node(&name) {
                    return Err(format!("permission « {} » : nom invalide (minuscules, chiffres et points)", raw));
                }
                let default = match body.get("default") {
                    None => PermDefault::Op,
                    Some(Value::Bool(true)) => PermDefault::Everyone,
                    Some(Value::Bool(false)) => PermDefault::Nobody,
                    Some(Value::Str(s)) => match s.to_lowercase().as_str() {
                        "op" | "isop" | "operator" | "admin" => PermDefault::Op,
                        "notop" | "!op" | "notoperator" | "notadmin" => PermDefault::NotOp,
                        "true" | "all" | "everyone" => PermDefault::Everyone,
                        "false" | "none" | "nobody" => PermDefault::Nobody,
                        other => return Err(format!("permission « {} » : « default: {} » inconnu (op, notop, true ou false)", name, other)),
                    },
                    Some(_) => return Err(format!("permission « {} » : « default » invalide", name)),
                };
                permissions.push(PermDecl { name, description: text(body.get("description"), 128).unwrap_or_default(), default });
            }
        }
        // Une commande peut citer une permission non déclarée : elle est alors réservée aux opérateurs
        for c in &commands {
            if let Some(p) = &c.permission {
                if !permissions.iter().any(|d| d.name == *p) {
                    permissions.push(PermDecl { name: p.clone(), description: String::new(), default: PermDefault::Op });
                }
            }
        }
        Ok(Manifest {
            name,
            version,
            main,
            api,
            description: text(doc.get("description"), 256).unwrap_or_default(),
            authors,
            website: text(doc.get("website"), 128).unwrap_or_default(),
            depend: list(doc.get("depend")),
            softdepend: list(doc.get("softdepend")),
            commands,
            permissions,
        })
    }
}

/// Ordre de chargement : chaque plugin après ceux dont il dépend. Renvoie les indices dans
/// l'ordre, et pour chaque plugin écarté la raison (dépendance absente, boucle).
pub fn load_order(manifests: &[Manifest]) -> (Vec<usize>, Vec<(usize, String)>) {
    let index = |name: &str| manifests.iter().position(|m| m.name.eq_ignore_ascii_case(name));
    let mut rejected: Vec<(usize, String)> = Vec::new();
    let mut alive: Vec<bool> = vec![true; manifests.len()];
    // Les dépendances absentes écartent le plugin, et par ricochet ceux qui en dépendent
    loop {
        let mut changed = false;
        for (i, m) in manifests.iter().enumerate() {
            if !alive[i] {
                continue;
            }
            for d in &m.depend {
                let ok = index(d).map(|j| alive[j]).unwrap_or(false);
                if !ok {
                    alive[i] = false;
                    rejected.push((i, format!("dépendance « {} » absente ou non chargeable", d)));
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut order = Vec::new();
    let mut done: Vec<bool> = vec![false; manifests.len()];
    loop {
        let mut progressed = false;
        for (i, m) in manifests.iter().enumerate() {
            if !alive[i] || done[i] {
                continue;
            }
            let ready = m.depend.iter().chain(m.softdepend.iter()).all(|d| match index(d) {
                Some(j) => !alive[j] || done[j] || j == i,
                None => true,
            });
            if ready {
                done[i] = true;
                order.push(i);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    for i in 0..manifests.len() {
        if alive[i] && !done[i] {
            rejected.push((i, "dépendances en boucle".into()));
        }
    }
    (order, rejected)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "name: Gardien\nversion: 1.2.0\nmain: src/main.lua\napi: 1\ndescription: Liste noire\nauthor: Enzo\nauthors: [Lea]\ndepend: [Economie]\nsoftdepend: Monde\ncommands:\n  expulser:\n    description: Expulse un joueur\n    usage: \"/expulser <joueur>\"\n    permission: gardien.expulser\n  aide:\n    description: Aide\npermissions:\n  gardien.voir:\n    description: Voir la liste\n    default: true\n";

    #[test]
    fn a_full_manifest_is_understood() {
        let m = Manifest::parse(SAMPLE).unwrap();
        assert_eq!((m.name.as_str(), m.version.as_str(), m.main.as_str(), m.api), ("Gardien", "1.2.0", "src/main.lua", 1));
        assert_eq!(m.authors, vec!["Enzo".to_string(), "Lea".to_string()]);
        assert_eq!((m.depend.clone(), m.softdepend.clone()), (vec!["Economie".to_string()], vec!["Monde".to_string()]));
        assert_eq!(m.commands[0].permission.as_deref(), Some("gardien.expulser"));
        assert_eq!(m.commands[0].usage, "/expulser <joueur>");
        // permission déclarée, et permission seulement citée par une commande (réservée aux opérateurs)
        assert_eq!(m.permissions[0].default, PermDefault::Everyone);
        assert_eq!((m.permissions[1].name.as_str(), m.permissions[1].default), ("gardien.expulser", PermDefault::Op));
    }

    #[test]
    fn bad_manifests_are_refused_with_a_reason() {
        assert!(Manifest::parse("version: 1").unwrap_err().contains("name"));
        assert!(Manifest::parse("name: Mon Plugin\nversion: 1").unwrap_err().contains("invalide"));
        assert!(Manifest::parse("name: A").unwrap_err().contains("version"));
        assert!(Manifest::parse("name: A\nversion: 1\nmain: ../../autre.lua").unwrap_err().contains("main"));
        assert!(Manifest::parse("name: A\nversion: 1\nmain: C:/x.lua").unwrap_err().contains("main"));
        assert!(Manifest::parse("name: A\nversion: 1\napi: 99").unwrap_err().contains("interface"));
        assert!(Manifest::parse("name: A\nversion: 1\npermissions:\n  a.b:\n    default: parfois").unwrap_err().contains("default"));
        assert!(Manifest::parse("name: A\nversion: 1\napi: [1, 2]").is_ok());
    }

    #[test]
    fn plugins_load_after_what_they_need() {
        let mk = |name: &str, depend: &[&str], soft: &[&str]| {
            let mut m = Manifest::implicit(name, "main.lua");
            m.depend = depend.iter().map(|s| s.to_string()).collect();
            m.softdepend = soft.iter().map(|s| s.to_string()).collect();
            m
        };
        let all = vec![mk("A", &["B"], &[]), mk("B", &[], &["C"]), mk("C", &[], &[]), mk("D", &["Absent"], &[]), mk("E", &["D"], &[]), mk("X", &["Y"], &[]), mk("Y", &["X"], &[])];
        let (order, rejected) = load_order(&all);
        let names: Vec<&str> = order.iter().map(|i| all[*i].name.as_str()).collect();
        assert_eq!(names, vec!["C", "B", "A"]);
        let why = |n: &str| rejected.iter().find(|(i, _)| all[*i].name == n).map(|(_, w)| w.clone()).unwrap();
        assert!(why("D").contains("Absent"));
        assert!(why("E").contains("D"));
        assert!(why("X").contains("boucle") && why("Y").contains("boucle"));
    }
}
