//! Permissions, comme dans PocketMine et Altay : chaque permission a une règle par défaut,
//! peut être réglée joueur par joueur, et certains joueurs sont opérateurs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// À qui une permission est accordée quand rien d'autre n'est précisé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermDefault {
    /// Aux opérateurs seulement
    Op,
    /// À tous sauf aux opérateurs
    NotOp,
    Everyone,
    Nobody,
}

/// Une permission enregistrée (par le `plugin.yml` d'un plugin).
#[derive(Debug, Clone, PartialEq)]
pub struct Permission {
    pub name: String,
    pub description: String,
    pub default: PermDefault,
}

/// Nom de permission valide : minuscules, chiffres, `.`, `_`, `-`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// Un joueur a-t-il cette permission ? Son réglage propre d'abord, puis la règle par défaut ;
/// une permission que personne n'a déclarée est réservée aux opérateurs.
pub fn check(op: bool, own: &[(String, bool)], declared: &[(String, PermDefault)], node: &str) -> bool {
    if let Some((_, granted)) = own.iter().find(|(n, _)| n == node) {
        return *granted;
    }
    match declared.iter().find(|(n, _)| n == node).map(|(_, d)| *d).unwrap_or(PermDefault::Op) {
        PermDefault::Op => op,
        PermDefault::NotOp => !op,
        PermDefault::Everyone => true,
        PermDefault::Nobody => false,
    }
}

/// Liste de pseudos enregistrée dans un fichier, un par ligne : les opérateurs (`ops.txt`)
/// et la liste blanche (`white-list.txt`). Elle vaut pour tout le serveur, quel que soit le monde.
#[derive(Debug, Default)]
pub struct NameList {
    names: HashSet<String>,
    path: PathBuf,
}

/// Liste des opérateurs.
pub type OpList = NameList;

impl NameList {
    /// Lit une liste de pseudos (ops.txt, white-list.txt) ; fichier absent : liste vide.
    pub fn load(path: &Path) -> NameList {
        let names = std::fs::read_to_string(path)
            .map(|text| text.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty() && !l.starts_with('#')).take(10_000).collect())
            .unwrap_or_default();
        NameList { names, path: path.to_path_buf() }
    }

    /// Relit le fichier (après une modification à la main).
    pub fn reload(&mut self) {
        *self = NameList::load(&self.path.clone());
    }

    /// Pseudo présent (sans tenir compte des majuscules) ?
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(&name.trim().to_lowercase())
    }

    pub fn is_op(&self, name: &str) -> bool {
        self.contains(name)
    }

    /// Nombre de pseudos.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Pseudos des opérateurs, triés.
    pub fn list(&self) -> Vec<String> {
        let mut list: Vec<String> = self.names.iter().cloned().collect();
        list.sort();
        list
    }

    /// Ajoute ou retire un pseudo et enregistre la liste. Renvoie `Ok(true)` si
    /// quelque chose a changé.
    pub fn set(&mut self, name: &str, op: bool) -> std::io::Result<bool> {
        let key = name.trim().to_lowercase();
        if key.is_empty() || key.len() > 32 {
            return Ok(false);
        }
        let changed = if op { self.names.insert(key) } else { self.names.remove(&key) };
        if !changed {
            return Ok(false);
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text: String = self.list().iter().map(|n| format!("{}\n", n)).collect();
        std::fs::write(&self.path, text)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_overrides_and_operators() {
        let declared = vec![("a.tous".to_string(), PermDefault::Everyone), ("a.op".to_string(), PermDefault::Op), ("a.simple".to_string(), PermDefault::NotOp), ("a.personne".to_string(), PermDefault::Nobody)];
        let none: Vec<(String, bool)> = Vec::new();
        assert!(check(false, &none, &declared, "a.tous"));
        assert!(!check(false, &none, &declared, "a.op") && check(true, &none, &declared, "a.op"));
        assert!(check(false, &none, &declared, "a.simple") && !check(true, &none, &declared, "a.simple"));
        assert!(!check(true, &none, &declared, "a.personne"));
        assert!(!check(false, &none, &declared, "inconnue") && check(true, &none, &declared, "inconnue"));
        let own = vec![("a.op".to_string(), true), ("a.tous".to_string(), false)];
        assert!(check(false, &own, &declared, "a.op") && !check(true, &own, &declared, "a.tous"));

        let path = std::env::temp_dir().join(format!("basalt-ops-test-{}", std::process::id())).join("ops.txt");
        let mut ops = OpList::load(&path);
        assert!(!ops.is_op("Enzo"));
        assert!(ops.set("Enzo", true).unwrap());
        assert!(!ops.set("enzo", true).unwrap());
        assert!(OpList::load(&path).is_op("ENZO"));
        assert!(ops.set("Enzo", false).unwrap());
        assert!(OpList::load(&path).list().is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
