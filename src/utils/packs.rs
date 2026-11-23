//! Packs de ressources : les fichiers `.mcpack` ou `.zip` du dossier `resource_packs/`,
//! envoyés aux joueurs à leur connexion (images, sons, textes des contenus créés par les
//! plugins). Comme sur PocketMine, `resource_packs/resource_packs.yml` dit lesquels envoyer,
//! dans quel ordre, et s'ils sont obligatoires.
//!
//! Le serveur n'ouvre un pack que pour y lire `manifest.json` (son identifiant et sa
//! version) ; le fichier part ensuite tel quel, par morceaux, à la demande du jeu.
//!
//! **Pack chiffré.** Un pack chiffré reste un fichier zip dont `manifest.json` est lisible ;
//! seul son contenu est chiffré. Le serveur ne le déchiffre pas : il transmet au jeu la clé
//! (32 caractères), que l'on range à côté du pack dans un fichier `.key` — `monpack.mcpack`
//! et `monpack.mcpack.key`, comme sur le serveur officiel.

use std::path::Path;
use std::sync::Arc;

use crate::json::Value;

/// Taille d'un morceau envoyé au jeu.
pub const CHUNK_SIZE: usize = 128 * 1024;
/// Taille maximale d'un pack.
const MAX_PACK: u64 = 256 * 1024 * 1024;

/// Un pack prêt à être proposé aux joueurs.
#[derive(Debug, Clone)]
pub struct ResourcePack {
    /// Identifiant du pack (`header.uuid` de son manifeste)
    pub uuid: String,
    /// Version, sous la forme `1.0.0`
    pub version: String,
    /// Nom affiché dans la console
    pub name: String,
    /// Clé de déchiffrement, vide si le pack n'est pas chiffré
    pub key: String,
    /// Le fichier, tel qu'il sera envoyé
    pub data: Arc<Vec<u8>>,
    /// Empreinte SHA-256 du fichier
    pub hash: [u8; 32],
}

impl ResourcePack {
    /// Nom du pack dans les échanges avec le jeu : `identifiant_version`.
    pub fn wire_id(&self) -> String {
        format!("{}_{}", self.uuid, self.version)
    }

    /// Nombre de morceaux du fichier.
    pub fn chunk_count(&self) -> u32 {
        self.data.len().div_ceil(CHUNK_SIZE) as u32
    }

    /// Un morceau du fichier : (position de son premier octet, octets).
    pub fn chunk(&self, index: u32) -> Option<(u64, &[u8])> {
        let start = (index as usize).checked_mul(CHUNK_SIZE)?;
        if start >= self.data.len() {
            return None;
        }
        Some((start as u64, &self.data[start..(start + CHUNK_SIZE).min(self.data.len())]))
    }

    /// Les 16 octets de l'identifiant ; None s'il n'a pas la forme d'un UUID.
    pub fn uuid_bytes(&self) -> Option<[u8; 16]> {
        uuid_bytes(&self.uuid)
    }
}

/// Les 16 octets d'un UUID écrit en hexadécimal, avec ou sans tirets.
pub fn uuid_bytes(text: &str) -> Option<[u8; 16]> {
    let hex: Vec<u8> = text.bytes().filter(|b| *b != b'-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, pair) in hex.chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

fn u16_at(d: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes([*d.get(at)?, *d.get(at + 1)?]) as usize)
}

fn u32_at(d: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes([*d.get(at)?, *d.get(at + 1)?, *d.get(at + 2)?, *d.get(at + 3)?]) as usize)
}

/// Lit un fichier d'une archive zip : celui qui porte ce nom au plus près de la racine
/// (un pack est parfois zippé avec son dossier). Fichiers stockés ou compressés (deflate).
pub fn zip_read(zip: &[u8], wanted: &str, limit: usize) -> Result<Vec<u8>, String> {
    // Fin du répertoire central : cherchée depuis la fin (un commentaire peut la suivre)
    let floor = zip.len().saturating_sub(66_000);
    let end = (floor..zip.len().saturating_sub(21)).rev().find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or("ce n'est pas une archive zip")?;
    let count = u16_at(zip, end + 10).ok_or("archive zip tronquée")?;
    let mut at = u32_at(zip, end + 16).ok_or("archive zip tronquée")?;
    let mut best: Option<(usize, usize, usize, usize)> = None; // (profondeur, méthode, taille compressée, en-tête local)
    for _ in 0..count {
        if zip.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err("répertoire de l'archive zip illisible".into());
        }
        let broken = || "répertoire de l'archive zip tronqué".to_string();
        let (method, packed, name_len) = (u16_at(zip, at + 10).ok_or_else(broken)?, u32_at(zip, at + 20).ok_or_else(broken)?, u16_at(zip, at + 28).ok_or_else(broken)?);
        let (extra, comment, local) = (u16_at(zip, at + 30).ok_or_else(broken)?, u16_at(zip, at + 32).ok_or_else(broken)?, u32_at(zip, at + 42).ok_or_else(broken)?);
        let name = zip.get(at + 46..at + 46 + name_len).ok_or_else(broken)?;
        let name = String::from_utf8_lossy(name).replace('\\', "/");
        if name == wanted || name.ends_with(&format!("/{}", wanted)) {
            let depth = name.matches('/').count();
            if best.map(|b| depth < b.0).unwrap_or(true) {
                best = Some((depth, method, packed, local));
            }
        }
        at += 46 + name_len + extra + comment;
    }
    let (_, method, packed, local) = best.ok_or_else(|| format!("{} introuvable dans l'archive", wanted))?;
    if zip.get(local..local + 4) != Some(&[0x50, 0x4b, 0x03, 0x04]) {
        return Err("archive zip abîmée".into());
    }
    let start = local + 30 + u16_at(zip, local + 26).ok_or("archive zip tronquée")? + u16_at(zip, local + 28).ok_or("archive zip tronquée")?;
    let raw = zip.get(start..start + packed).ok_or("archive zip tronquée")?;
    match method {
        0 if raw.len() <= limit => Ok(raw.to_vec()),
        0 => Err(format!("{} est trop gros", wanted)),
        8 => crate::utils::inflate::inflate(raw, limit).map_err(|e| format!("{} illisible : {}", wanted, e)),
        other => Err(format!("{} est compressé d'une façon non gérée (méthode {})", wanted, other)),
    }
}

/// Retire les commentaires `//` et `/* */` d'un JSON (les manifestes en contiennent parfois).
fn strip_comments(text: &str) -> String {
    let (mut out, mut chars, mut in_string) = (String::with_capacity(text.len()), text.chars().peekable(), false);
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut last = ' ';
            for n in chars.by_ref() {
                if last == '*' && n == '/' {
                    break;
                }
                last = n;
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Identifiant, version et nom d'un pack, d'après son manifeste.
pub fn read_manifest(text: &str) -> Result<(String, String, String), String> {
    let text = strip_comments(text.trim_start_matches('\u{feff}'));
    let doc = crate::json::parse(&text).map_err(|e| format!("manifest.json illisible : {}", e))?;
    let header = doc.get("header").ok_or("manifest.json : section « header » absente")?;
    let uuid = header.get("uuid").and_then(|v| v.as_str()).ok_or("manifest.json : « header.uuid » absent")?.to_lowercase();
    if uuid_bytes(&uuid).is_none() {
        return Err(format!("manifest.json : « {} » n'est pas un identifiant valide", uuid));
    }
    let version = match header.get("version") {
        Some(Value::Arr(parts)) if !parts.is_empty() => parts.iter().map(|p| format!("{}", p.as_f64().unwrap_or(0.0) as u32)).collect::<Vec<_>>().join("."),
        Some(Value::Str(s)) => s.clone(),
        _ => return Err("manifest.json : « header.version » absente".into()),
    };
    let name = header.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    Ok((uuid, version, name))
}

/// Charge un pack depuis ses octets ; `key` : sa clé s'il est chiffré.
pub fn from_bytes(data: Vec<u8>, key: &str, file: &str) -> Result<ResourcePack, String> {
    let manifest = zip_read(&data, "manifest.json", 1 << 20)?;
    let (uuid, version, name) = read_manifest(&String::from_utf8_lossy(&manifest))?;
    let key = key.trim();
    if !key.is_empty() && key.len() != 32 {
        return Err(format!("la clé doit faire 32 caractères, celle-ci en fait {}", key.len()));
    }
    let hash = crate::security::crypto::sha256(&data);
    // (un nom traduit — « pack.name » — ne dit rien dans la console : on montre le fichier)
    let name = if name.is_empty() || name.contains("pack.") { file.to_string() } else { name };
    Ok(ResourcePack { uuid, version, name, key: key.to_string(), data: Arc::new(data), hash })
}

/// Nom du fichier de réglages, dans le dossier des packs (le même que PocketMine).
pub const SETTINGS: &str = "resource_packs.yml";

/// Fichier de réglages écrit à la première exécution.
const DEFAULT_SETTINGS: &str = r#"# Packs de ressources envoyés aux joueurs à leur connexion.
# Mode d'emploi : docs/PACKS.md

# true : un joueur qui refuse les packs ne peut pas entrer.
# false : il peut jouer sans (les images propres au serveur lui manqueront).
force_resources: false

# Les packs à envoyer : le nom de chaque fichier de ce dossier (.mcpack ou .zip).
# L'ordre compte : en cas de conflit entre deux packs, le premier de la liste l'emporte.
# Un fichier présent dans le dossier mais absent de cette liste n'est pas envoyé.
#
# Pack chiffré : mettez sa clé (32 caractères) dans un fichier du même nom suivi de
# « .key », à côté du pack — monpack.mcpack et monpack.mcpack.key.
resource_stack:
  # - monpack.mcpack
"#;

/// Lit `resource_packs.yml` : (packs obligatoires, fichiers à charger dans l'ordre).
pub fn read_settings(text: &str) -> Result<(bool, Vec<String>), String> {
    let doc = crate::yaml::parse(text)?;
    let force = matches!(doc.get("force_resources"), Some(Value::Bool(true)));
    let files = match doc.get("resource_stack") {
        Some(Value::Arr(items)) => items.iter().filter_map(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        _ => Vec::new(),
    };
    Ok((force, files))
}

/// Charge un pack d'après son nom de fichier dans le dossier.
fn load_file(dir: &Path, file: &str) -> Result<ResourcePack, String> {
    if file.contains("..") || file.contains('/') || file.contains('\\') {
        return Err("le pack doit se trouver dans le dossier des packs".into());
    }
    let path = dir.join(file);
    if path.is_dir() {
        return Err("c'est un dossier ; compressez son contenu en .zip ou .mcpack".into());
    }
    let size = std::fs::metadata(&path).map_err(|_| "fichier introuvable dans le dossier des packs".to_string())?.len();
    if size > MAX_PACK {
        return Err(format!("plus de {} Mo", MAX_PACK >> 20));
    }
    let data = std::fs::read(&path).map_err(|e| e.to_string())?;
    // Clé : « pack.mcpack.key », ou « pack.key »
    let beside = [path.with_file_name(format!("{}.key", file)), path.with_extension("key")];
    let key = beside.iter().find_map(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    from_bytes(data, &key, file)
}

/// Charge les packs du dossier d'après son `resource_packs.yml`, créé avec le dossier s'il
/// manque. Renvoie les packs dans l'ordre de la liste, et s'ils sont obligatoires. Un pack
/// illisible est signalé et laissé de côté.
pub fn load_dir(dir: &Path) -> (Vec<ResourcePack>, bool) {
    let settings = dir.join(SETTINGS);
    if !settings.exists() {
        let _ = std::fs::create_dir_all(dir);
        if let Err(e) = std::fs::write(&settings, DEFAULT_SETTINGS) {
            warn!("Packs de ressources : {} n'a pas pu être créé ({})", settings.display(), e);
        }
        return (Vec::new(), false);
    }
    let (force, files) = match std::fs::read_to_string(&settings).map_err(|e| e.to_string()).and_then(|t| read_settings(&t)) {
        Ok(found) => found,
        Err(why) => {
            warn!("Packs de ressources : {} illisible ({}) ; aucun pack n'est envoyé", settings.display(), why);
            return (Vec::new(), false);
        }
    };
    let mut packs: Vec<ResourcePack> = Vec::new();
    for file in &files {
        match load_file(dir, file) {
            Ok(pack) if packs.iter().any(|p| p.uuid == pack.uuid) => warn!("Pack de ressources {} : même identifiant qu'un pack déjà chargé, ignoré", file),
            Ok(pack) => {
                info!("Pack de ressources : {} {} ({} Ko{})", pack.name, pack.version, pack.data.len() / 1024, if pack.key.is_empty() { "" } else { ", chiffré" });
                packs.push(pack);
            }
            Err(why) => warn!("Pack de ressources {} ignoré : {}", file, why),
        }
    }
    // Un pack déposé mais oublié dans la liste : on le dit, c'est l'erreur la plus courante
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut forgotten: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| (n.to_lowercase().ends_with(".mcpack") || n.to_lowercase().ends_with(".zip")) && !files.contains(n))
            .collect();
        forgotten.sort();
        for file in forgotten {
            info!("Pack de ressources {} présent mais pas envoyé : ajoutez-le à « resource_stack » dans {}", file, settings.display());
        }
    }
    if force && !packs.is_empty() {
        info!("Packs de ressources obligatoires pour entrer (force_resources)");
    }
    (packs, force)
}

/// Archive zip minimale (fichiers stockés, sans compression) : pour les tests.
pub fn zip_store(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for b in data {
            crc ^= *b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }
    let (mut out, mut directory) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let offset = out.len() as u32;
        let mut common = Vec::new();
        common.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, drapeaux, méthode 0, heure, date
        common.extend_from_slice(&crc32(data).to_le_bytes());
        common.extend_from_slice(&(data.len() as u32).to_le_bytes());
        common.extend_from_slice(&(data.len() as u32).to_le_bytes());
        common.extend_from_slice(&(name.len() as u16).to_le_bytes());
        common.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
        out.extend_from_slice(&common);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        directory.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0]);
        directory.extend_from_slice(&common);
        directory.extend_from_slice(&[0; 10]); // commentaire, disque, attributs
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        // un commentaire, comme en laissent les outils
        "format_version": 2,
        "header": { "name": "Mon pack", "uuid": "0FBA4063-DBA1-4281-9B89-FF9390653530", "version": [1, 2, 0], "min_engine_version": [1, 20, 0] },
        "modules": [ { "type": "resources", "uuid": "7f4bb4f1-0000-4000-8000-000000000001", "version": [1, 0, 0] } ]
    }"#;

    #[test]
    fn packs_are_read_from_their_archive() {
        let zip = zip_store(&[("textures/items/rubis.png", &[1, 2, 3]), ("manifest.json", MANIFEST.as_bytes())]);
        let pack = from_bytes(zip.clone(), "", "monpack.mcpack").unwrap();
        assert_eq!((pack.uuid.as_str(), pack.version.as_str(), pack.name.as_str()), ("0fba4063-dba1-4281-9b89-ff9390653530", "1.2.0", "Mon pack"));
        assert_eq!(pack.wire_id(), "0fba4063-dba1-4281-9b89-ff9390653530_1.2.0");
        assert_eq!(pack.uuid_bytes().map(|b| (b[0], b[15])), Some((0x0f, 0x30)));
        assert_eq!(pack.hash, crate::security::crypto::sha256(&zip));
        // Un pack zippé avec son dossier : le manifeste est un cran plus bas
        let nested = zip_store(&[("MonPack/manifest.json", MANIFEST.as_bytes()), ("MonPack/sub/manifest.json", b"{}")]);
        assert_eq!(from_bytes(nested, "", "x.zip").unwrap().version, "1.2.0");
        // Clé d'un pack chiffré : 32 caractères, transmise telle quelle
        let key = "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345";
        assert_eq!(from_bytes(zip.clone(), &format!("{}\n", key), "x.zip").unwrap().key, key);
        assert!(from_bytes(zip, "trop courte", "x.zip").is_err());
        // Ce qui n'est pas un pack
        assert!(from_bytes(b"pas un zip du tout, mais assez long pour chercher".to_vec(), "", "x.zip").is_err());
        assert!(from_bytes(zip_store(&[("autre.txt", b"rien")]), "", "x.zip").is_err());
        assert!(read_manifest(r#"{"header":{"uuid":"pas-un-uuid","version":[1,0,0]}}"#).is_err());
    }

    #[test]
    fn the_settings_file_lists_the_packs_to_send() {
        // Le fichier créé par défaut : rien d'obligatoire, aucun pack
        assert_eq!(read_settings(DEFAULT_SETTINGS), Ok((false, vec![])));
        let text = "force_resources: true\nresource_stack:\n  - premier.mcpack\n  - second.zip\n";
        assert_eq!(read_settings(text), Ok((true, vec!["premier.mcpack".to_string(), "second.zip".to_string()])));
        // Un dossier complet : créé au premier passage, puis lu dans l'ordre de la liste
        let dir = std::env::temp_dir().join(format!("basalt-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_dir(&dir).0.is_empty() && dir.join(SETTINGS).exists());
        let manifest = |uuid: &str| format!(r#"{{"header":{{"name":"P","uuid":"{}","version":[1,0,0]}}}}"#, uuid);
        let (a, b) = (manifest("11111111-2222-4333-8444-555555555555"), manifest("99999999-2222-4333-8444-555555555555"));
        std::fs::write(dir.join("a.mcpack"), zip_store(&[("manifest.json", a.as_bytes())])).unwrap();
        std::fs::write(dir.join("b.zip"), zip_store(&[("manifest.json", b.as_bytes())])).unwrap();
        std::fs::write(dir.join("b.zip.key"), "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345").unwrap();
        std::fs::write(dir.join("oublie.zip"), zip_store(&[("manifest.json", a.as_bytes())])).unwrap();
        std::fs::write(dir.join(SETTINGS), "force_resources: true\nresource_stack:\n  - b.zip\n  - a.mcpack\n  - absent.zip\n  - ../ailleurs.zip\n").unwrap();
        let (packs, force) = load_dir(&dir);
        assert!(force);
        let seen: Vec<(&str, bool)> = packs.iter().map(|p| (&p.uuid[..8], !p.key.is_empty())).collect();
        assert_eq!(seen, vec![("99999999", true), ("11111111", false)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn packs_are_cut_into_chunks() {
        let mut files: Vec<(&str, &[u8])> = vec![("manifest.json", MANIFEST.as_bytes())];
        let big = vec![7u8; CHUNK_SIZE * 2 + 10];
        files.push(("gros.bin", &big));
        let pack = from_bytes(zip_store(&files), "", "x.zip").unwrap();
        assert_eq!(pack.chunk_count(), 3);
        let total: usize = (0..pack.chunk_count()).map(|i| pack.chunk(i).unwrap().1.len()).sum();
        assert_eq!(total, pack.data.len());
        assert_eq!(pack.chunk(1).unwrap().0, CHUNK_SIZE as u64);
        assert!(pack.chunk(3).is_none());
    }

    #[test]
    fn compressed_entries_are_inflated() {
        // Un manifeste compressé (deflate), comme le font les outils d'archive
        let packed = crate::utils::deflate::deflate(MANIFEST.as_bytes(), crate::utils::deflate::Level::Best);
        let mut zip = zip_store(&[("manifest.json", &packed)]);
        // (méthode 8 dans l'en-tête local et dans le répertoire)
        zip[8] = 8;
        let directory = zip.windows(4).position(|w| w == [0x50, 0x4b, 0x01, 0x02]).unwrap();
        zip[directory + 10] = 8;
        assert_eq!(String::from_utf8(zip_read(&zip, "manifest.json", 1 << 20).unwrap()).unwrap(), MANIFEST);
    }
}
