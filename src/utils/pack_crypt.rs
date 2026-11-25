//! Chiffrement des packs de ressources, tel que le jeu sait les lire.
//!
//! Un pack chiffré est une archive ordinaire dont chaque fichier (sauf le manifeste et
//! l'icône) est chiffré avec sa propre clé ; ces clés sont rangées dans `contents.json`,
//! lui-même chiffré avec la **clé du pack** (32 caractères). Le serveur donne cette clé au
//! jeu à la connexion ; un joueur qui ouvre l'archive sans elle n'y lit rien.
//!
//! - Chiffre : AES-256 en mode CFB8 ; le vecteur initial est fait des 16 premiers octets
//!   de la clé.
//! - `contents.json` : un en-tête de 256 octets en clair (version, nombre magique,
//!   identifiant du pack), suivi du JSON chiffré `{"version":1,"content":[{"path","key"}…]}`.
//!
//! Cela protège un pack contre la copie facile, pas contre quelqu'un de décidé : le jeu
//! reçoit forcément la clé pour afficher le pack.

use super::packs::{read_manifest, u16_at, u32_at, zip_store};
use crate::security::crypto::{os_random, Aes256};

/// Nombre magique d'un `contents.json` chiffré
const MAGIC: [u8; 4] = [0xFC, 0xB9, 0xCF, 0x9B];
/// Taille de l'en-tête en clair
const HEADER: usize = 256;
/// Fichiers que le jeu lit avant d'avoir la clé : ils restent en clair
const PLAIN: [&str; 3] = ["manifest.json", "pack_icon.png", "bug_pack_icon.png"];
/// Taille maximale d'un pack une fois décompressé
const MAX_UNPACKED: usize = 512 * 1024 * 1024;

/// AES-256 en mode CFB8 : octet par octet, chaque octet chiffré nourrit le suivant.
fn cfb8(key: &[u8; 32], iv: &[u8; 16], data: &mut [u8], decrypt: bool) {
    let aes = Aes256::new(key);
    let mut shift = *iv;
    for byte in data.iter_mut() {
        let before = *byte;
        *byte ^= aes.encrypt_block(&shift)[0];
        shift.copy_within(1.., 0);
        shift[15] = if decrypt { before } else { *byte };
    }
}

/// Le vecteur initial d'un pack : les 16 premiers octets de la clé.
fn iv_of(key: &[u8; 32]) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&key[..16]);
    iv
}

/// Chiffre comme le jeu l'attend (AES-256-CFB8, vecteur initial tiré de la clé).
pub fn cfb8_encrypt(key: &[u8; 32], data: &mut [u8]) {
    cfb8(key, &iv_of(key), data, false)
}

/// Déchiffre ce que `cfb8_encrypt` a chiffré.
pub fn cfb8_decrypt(key: &[u8; 32], data: &mut [u8]) {
    cfb8(key, &iv_of(key), data, true)
}

/// Une clé de 32 lettres et chiffres tirés au hasard.
pub fn random_key() -> Option<String> {
    const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut key = String::with_capacity(32);
    let mut pool = [0u8; 64];
    while key.len() < 32 {
        if !os_random(&mut pool) {
            return None;
        }
        // (les octets au-delà de 247 sont écartés : chaque caractère reste aussi probable qu'un autre)
        key.extend(pool.iter().filter(|b| **b < 248).take(32 - key.len()).map(|b| ALPHABET[*b as usize % 62] as char));
    }
    Some(key)
}

fn key_bytes(key: &str) -> Result<[u8; 32], String> {
    key.as_bytes().try_into().map_err(|_| "la clé d'un pack fait 32 caractères".to_string())
}

/// Tous les fichiers d'une archive zip : (nom, contenu). Les dossiers sont ignorés.
pub fn zip_entries(zip: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let floor = zip.len().saturating_sub(66_000);
    let end = (floor..zip.len().saturating_sub(21)).rev().find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or("ce n'est pas une archive zip")?;
    let count = u16_at(zip, end + 10).ok_or("archive zip tronquée")?;
    let mut at = u32_at(zip, end + 16).ok_or("archive zip tronquée")?;
    let broken = || "répertoire de l'archive zip tronqué".to_string();
    let (mut files, mut total) = (Vec::new(), 0usize);
    for _ in 0..count {
        if zip.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err("répertoire de l'archive zip illisible".into());
        }
        let (flags, method, packed, name_len) = (u16_at(zip, at + 8).ok_or_else(broken)?, u16_at(zip, at + 10).ok_or_else(broken)?, u32_at(zip, at + 20).ok_or_else(broken)?, u16_at(zip, at + 28).ok_or_else(broken)?);
        let (extra, comment, local) = (u16_at(zip, at + 30).ok_or_else(broken)?, u16_at(zip, at + 32).ok_or_else(broken)?, u32_at(zip, at + 42).ok_or_else(broken)?);
        let name = String::from_utf8_lossy(zip.get(at + 46..at + 46 + name_len).ok_or_else(broken)?).replace('\\', "/");
        at += 46 + name_len + extra + comment;
        if name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err("archive zip protégée par mot de passe : non gérée".into());
        }
        // (un nom qui sortirait du dossier n'a rien à faire dans un pack)
        if name.starts_with('/') || name.split('/').any(|part| part == ".." || part.is_empty()) {
            return Err(format!("nom de fichier refusé dans l'archive : {}", name));
        }
        if zip.get(local..local + 4) != Some(&[0x50, 0x4b, 0x03, 0x04]) {
            return Err("archive zip abîmée".into());
        }
        let start = local + 30 + u16_at(zip, local + 26).ok_or("archive zip tronquée")? + u16_at(zip, local + 28).ok_or("archive zip tronquée")?;
        let raw = zip.get(start..start + packed).ok_or("archive zip tronquée")?;
        let left = MAX_UNPACKED - total;
        let data = match method {
            0 if raw.len() <= left => raw.to_vec(),
            8 => crate::utils::inflate::inflate(raw, left).map_err(|e| format!("{} illisible : {}", name, e))?,
            0 => return Err("pack trop gros une fois décompressé".into()),
            other => return Err(format!("{} est compressé d'une façon non gérée (méthode {})", name, other)),
        };
        total += data.len();
        files.push((name, data));
    }
    Ok(files)
}

/// Les fichiers d'un pack, ramenés à sa racine (un pack est parfois zippé avec son dossier).
fn pack_files(zip: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let files = zip_entries(zip)?;
    let manifest = files.iter().map(|(n, _)| n.as_str()).filter(|n| *n == "manifest.json" || n.ends_with("/manifest.json")).min_by_key(|n| n.matches('/').count()).ok_or("manifest.json introuvable : ce n'est pas un pack")?;
    let root = manifest.trim_end_matches("manifest.json").to_string();
    Ok(files.into_iter().filter_map(|(n, d)| n.strip_prefix(&root).map(|short| (short.to_string(), d))).collect())
}

/// Chiffre un pack. Renvoie l'archive chiffrée, la clé du pack, et le nombre de fichiers chiffrés.
pub fn encrypt_pack(zip: &[u8]) -> Result<(Vec<u8>, String, usize), String> {
    let mut files = pack_files(zip)?;
    let manifest = files.iter().find(|(n, _)| n == "manifest.json").map(|(_, d)| String::from_utf8_lossy(d).into_owned()).ok_or("manifest.json introuvable")?;
    let (uuid, _, _) = read_manifest(&manifest)?;
    if uuid.len() != 36 {
        return Err(format!("identifiant du pack inattendu dans manifest.json : {}", uuid));
    }
    if files.iter().any(|(n, d)| n == "contents.json" && d.get(4..8) == Some(&MAGIC[..])) {
        return Err("ce pack est déjà chiffré".into());
    }
    // (un contents.json en clair, laissé par un outil, est remplacé)
    files.retain(|(n, _)| n != "contents.json");
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let no_random = || "pas de source de hasard pour créer les clés".to_string();
    let master = random_key().ok_or_else(no_random)?;
    let mut listing = String::from("{\"version\":1,\"content\":[");
    let mut sealed = 0;
    for (i, (name, data)) in files.iter_mut().enumerate() {
        if i > 0 {
            listing.push(',');
        }
        let path = crate::json::Value::Str(name.clone()).to_json();
        if PLAIN.contains(&name.as_str()) {
            listing.push_str(&format!("{{\"path\":{}}}", path));
        } else {
            let key = random_key().ok_or_else(no_random)?;
            cfb8_encrypt(&key_bytes(&key)?, data);
            listing.push_str(&format!("{{\"path\":{},\"key\":\"{}\"}}", path, key));
            sealed += 1;
        }
    }
    listing.push_str("]}");
    let mut contents = vec![0u8; HEADER];
    contents[4..8].copy_from_slice(&MAGIC);
    contents[16] = uuid.len() as u8;
    contents[17..17 + uuid.len()].copy_from_slice(uuid.as_bytes());
    let mut body = listing.into_bytes();
    cfb8_encrypt(&key_bytes(&master)?, &mut body);
    contents.extend_from_slice(&body);
    files.push(("contents.json".to_string(), contents));
    let borrowed: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    Ok((zip_store(&borrowed), master, sealed))
}

/// Déchiffre un pack avec sa clé : les fichiers en clair, sans `contents.json`.
pub fn decrypt_pack(zip: &[u8], key: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files = pack_files(zip)?;
    let at = files.iter().position(|(n, _)| n == "contents.json").ok_or("contents.json introuvable : ce pack n'est pas chiffré")?;
    let (_, contents) = files.remove(at);
    if contents.len() < HEADER || contents[4..8] != MAGIC {
        return Err("contents.json n'est pas celui d'un pack chiffré".into());
    }
    let mut body = contents[HEADER..].to_vec();
    cfb8_decrypt(&key_bytes(key.trim())?, &mut body);
    let listing = String::from_utf8(body).ok().and_then(|t| crate::json::parse(&t).ok()).ok_or("clé du pack incorrecte")?;
    let entries = listing.get("content").and_then(|c| c.as_arr()).ok_or("clé du pack incorrecte")?;
    for entry in entries {
        let (Some(path), Some(file_key)) = (entry.get("path").and_then(|p| p.as_str()), entry.get("key").and_then(|k| k.as_str())) else { continue };
        if let Some((_, data)) = files.iter_mut().find(|(n, _)| n == path) {
            cfb8_decrypt(&key_bytes(file_key)?, data);
        }
    }
    Ok(files)
}

/// Chiffre le pack d'un fichier et écrit, à côté de lui, `<nom>-chiffre.mcpack` et sa clé
/// (`<nom>-chiffre.mcpack.key`). Renvoie (fichier écrit, fichiers chiffrés). L'original
/// n'est pas touché.
pub fn encrypt_file(path: &std::path::Path) -> Result<(std::path::PathBuf, usize), String> {
    let zip = std::fs::read(path).map_err(|e| format!("{} : {}", path.display(), e))?;
    let (sealed_zip, key, count) = encrypt_pack(&zip)?;
    let stem = path.file_stem().and_then(|s| s.to_str()).ok_or("nom de fichier illisible")?;
    let out = path.with_file_name(format!("{}-chiffre.mcpack", stem));
    let key_file = path.with_file_name(format!("{}-chiffre.mcpack.key", stem));
    std::fs::write(&out, sealed_zip).map_err(|e| format!("{} : {}", out.display(), e))?;
    std::fs::write(&key_file, &key).map_err(|e| format!("{} : {}", key_file.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600));
    }
    Ok((out, count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn cfb8_matches_the_reference_vectors() {
        // NIST SP 800-38A, F.3.11 et F.3.12 (CFB8-AES256)
        let key: [u8; 32] = hex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4").try_into().unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let plain = hex("6bc1bee22e409f96e93d7e117393172aae2d");
        let mut data = plain.clone();
        cfb8(&key, &iv, &mut data, false);
        assert_eq!(data, hex("dc1f1a8520a64db55fcc8ac554844e889700"));
        cfb8(&key, &iv, &mut data, true);
        assert_eq!(data, plain);
        // Et tel que le jeu s'en sert : le vecteur initial est le début de la clé
        let mut text = b"Un pack de ressources, ses textures et ses sons.".to_vec();
        cfb8_encrypt(&key, &mut text);
        assert_ne!(&text[..], &b"Un pack de ressources, ses textures et ses sons."[..]);
        cfb8_decrypt(&key, &mut text);
        assert_eq!(&text[..], &b"Un pack de ressources, ses textures et ses sons."[..]);
    }

    #[test]
    fn keys_are_thirty_two_letters_and_digits() {
        let (a, b) = (random_key().unwrap(), random_key().unwrap());
        assert!(a.len() == 32 && a.bytes().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, b);
    }

    const MANIFEST: &str = r#"{"format_version":2,"header":{"name":"Essai","uuid":"11111111-2222-3333-4444-555555555555","version":[1,0,0]},"modules":[]}"#;

    #[test]
    fn a_pack_is_encrypted_and_read_back_with_its_key() {
        let texture = vec![7u8; 5000];
        // Zippé avec son dossier, comme on le fait souvent
        let zip = zip_store(&[("MonPack/manifest.json", MANIFEST.as_bytes()), ("MonPack/pack_icon.png", b"icone"), ("MonPack/textures/items/rubis.png", &texture), ("MonPack/sounds.json", b"{}")]);
        let (sealed, key, count) = encrypt_pack(&zip).unwrap();
        assert_eq!((key.len(), count), (32, 2));
        let files = zip_entries(&sealed).unwrap();
        let get = |name: &str| files.iter().find(|(n, _)| n == name).map(|(_, d)| d.clone()).unwrap_or_else(|| panic!("{} manque", name));
        // Le manifeste et l'icône restent lisibles, à la racine ; le reste ne l'est plus
        assert_eq!(get("manifest.json"), MANIFEST.as_bytes());
        assert_eq!(get("pack_icon.png"), b"icone");
        assert!(get("textures/items/rubis.png") != texture && get("textures/items/rubis.png").len() == texture.len());
        // contents.json : en-tête en clair avec l'identifiant du pack, liste chiffrée ensuite
        let contents = get("contents.json");
        assert_eq!(&contents[..17], &[0, 0, 0, 0, 0xFC, 0xB9, 0xCF, 0x9B, 0, 0, 0, 0, 0, 0, 0, 0, 36]);
        assert_eq!(&contents[17..53], b"11111111-2222-3333-4444-555555555555");
        assert!(contents[53..256].iter().all(|b| *b == 0) && !contents[256..].starts_with(b"{\"version\""));
        // Basalt le reconnaît comme un pack, avec sa clé
        assert!(crate::utils::packs::from_bytes(sealed.clone(), &key, "x.mcpack").is_ok());
        // Avec la clé, tout revient ; avec une autre, rien
        let back = decrypt_pack(&sealed, &key).unwrap();
        let back_get = |name: &str| back.iter().find(|(n, _)| n == name).map(|(_, d)| d.clone()).unwrap();
        assert_eq!(back_get("textures/items/rubis.png"), texture);
        assert_eq!(back_get("sounds.json"), b"{}");
        assert!(decrypt_pack(&sealed, "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345").is_err());
        // Un pack déjà chiffré n'est pas chiffré deux fois
        assert_eq!(encrypt_pack(&sealed).unwrap_err(), "ce pack est déjà chiffré");
    }

    #[test]
    fn what_is_not_a_pack_is_refused() {
        assert!(encrypt_pack(b"pas un zip").is_err());
        assert!(encrypt_pack(&zip_store(&[("textures/a.png", b"x")])).unwrap_err().contains("manifest.json"));
        // Un nom qui sortirait du dossier
        assert!(zip_entries(&zip_store(&[("../dehors.txt", b"x")])).is_err());
    }
}
