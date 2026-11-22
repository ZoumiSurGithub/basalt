//! Terrain : identifiants de blocs, contenu d'un chunk plat, paquet LevelChunk.

#[allow(unused_imports)]
use super::*;

/// Hauteur du sol du monde plat : le joueur se tient à y = -60.
pub const GROUND_Y: i32 = -60;

/// Identifiant réseau d'un bloc sans propriétés : hachage FNV-1a (32 bits)
/// de son état décrit en NBT petit-boutiste.
pub fn block_hash(name: &str) -> u32 {
    let mut nbt = vec![10, 0, 0, 8, 4, 0];
    nbt.extend_from_slice(b"name");
    nbt.extend_from_slice(&(name.len() as u16).to_le_bytes());
    nbt.extend_from_slice(name.as_bytes());
    nbt.extend_from_slice(&[10, 6, 0]);
    nbt.extend_from_slice(b"states");
    nbt.extend_from_slice(&[0, 0]);
    let mut h = 0x811c_9dc5u32;
    for b in nbt {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Contenu d'un chunk du monde plat : pierre, deux couches de terre, herbe.
pub fn flat_chunk_payload() -> Vec<u8> {
    flat_chunk_payload_with(true)
}

/// Chunk plat minimal (une seule section), pour les tests de compatibilité.
pub fn flat_chunk_payload_with(biomes: bool) -> Vec<u8> {
    let mut w = W::default();
    // Un seul sous-chunk (y = -64 à -49), format 9, une couche de blocs
    w.u8(9);
    w.u8(1);
    w.u8((-4i8) as u8);
    w.u8((2 << 1) | 1); // 2 bits par bloc, identifiants réseau
    // Chaque colonne de 16 blocs tient dans un mot : pierre, terre, terre, herbe, air…
    let column: u32 = 1 | (2 << 2) | (2 << 4) | (3 << 6);
    for _ in 0..256 {
        w.u32(column);
    }
    w.vari32(4);
    for name in ["minecraft:air", "minecraft:stone", "minecraft:dirt", "minecraft:grass_block"].iter() {
        w.vari32(block_hash(name) as i32);
    }
    // Biomes : 24 sous-chunks de plaines (identifiant 1)
    for _ in 0..(if biomes { 24 } else { 0 }) {
        w.u8(1);
        w.vari32(1);
    }
    w.u8(0); // blocs de bordure
    w.0
}

/// Nombre de variantes de LevelChunk essayées tour à tour tant que le jeu les refuse.
pub const CHUNK_VARIANTS: u8 = 7;
/// Variante sans données de biomes dans le contenu.
pub const CHUNK_VARIANT_NO_BIOMES: u8 = 6;

/// Description des variantes du paquet LevelChunk essayées selon les versions.
pub fn chunk_variant_name(variant: u8) -> &'static str {
    match variant {
        0 => "comme avant 1.26.60, plus l'indicateur de biomes",
        1 => "limite absente + liste de cache vide + indicateur",
        2 => "liste de cache vide + indicateur",
        3 => "limite absente + indicateur",
        4 => "indicateur de biomes à vrai",
        5 => "sans indicateur de biomes",
        _ => "sans données de biomes",
    }
}

/// LevelChunk dans la forme par défaut.
pub fn level_chunk(x: i32, z: i32, payload: &[u8], f: crate::versions::Features) -> Vec<u8> {
    level_chunk_variant(x, z, payload, f, 0)
}

/// LevelChunk dans l'une des écritures possibles pour les jeux 1.26.60 et plus. La
/// documentation ne suffit pas à trancher : le serveur change de variante quand le jeu
/// signale qu'il n'a pas pu lire le paquet (voir `Server::chunk_variant`).
pub fn level_chunk_variant(x: i32, z: i32, payload: &[u8], f: crate::versions::Features, variant: u8) -> Vec<u8> {
    if !f.chunk_biome_update_flag {
        return level_chunk_old(x, z, payload);
    }
    let mut w = W::default();
    w.vari32(x);
    w.vari32(z);
    w.vari32(0); // dimension
    w.varu32(1); // nombre de sous-chunks
    if variant == 1 || variant == 3 {
        w.bool(false); // limite de demande de sous-chunks : absente
    }
    w.bool(false); // cache désactivé
    if variant == 1 || variant == 2 {
        w.varu32(0); // liste de cache, vide
    }
    w.varu32(payload.len() as u32);
    w.0.extend_from_slice(payload);
    match variant {
        4 => w.bool(true),
        5 => {}
        _ => w.bool(false),
    }
    w.0
}

/// Formes plus anciennes du paquet, pour les versions qui les attendent.
pub(crate) fn level_chunk_old(x: i32, z: i32, payload: &[u8]) -> Vec<u8> {
    let f = crate::versions::Features { chunk_biome_update_flag: false, ..crate::versions::features(0) };
    level_chunk_legacy(x, z, payload, f)
}

pub(crate) fn level_chunk_legacy(x: i32, z: i32, payload: &[u8], f: crate::versions::Features) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(x);
    w.vari32(z);
    w.vari32(0); // dimension
    w.varu32(1); // nombre de sous-chunks
    w.bool(false); // cache désactivé
    w.varu32(payload.len() as u32);
    w.0.extend_from_slice(payload);
    if f.chunk_biome_update_flag {
        w.bool(false); // pas une simple mise à jour de biomes
    }
    w.0
}

/// LevelChunk des versions d'avant 1.26.60, avec le nombre de sous-chunks du contenu.
pub fn level_chunk_counted(x: i32, z: i32, count: u32, payload: &[u8], f: crate::versions::Features) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(x);
    w.vari32(z);
    w.vari32(0); // dimension
    w.varu32(count); // nombre de sous-chunks
    // Même en-tête que celui accepté par le jeu 1.26.65 (voir `level_chunk_probe`) : la
    // liste du cache est toujours écrite, même vide. Sans elle, le jeu 1.26.40 lisait la
    // taille du contenu comme le nombre d'éléments du cache (« too many input elements »).
    w.bool(false);
    w.bool(false); // cache désactivé
    w.varu32(0); // liste du cache, vide
    w.varu32(payload.len() as u32);
    w.0.extend_from_slice(payload);
    if f.chunk_biome_update_flag {
        w.bool(false); // ajouté en 1.26.60
    }
    w.0
}

#[cfg(test)]
mod counted_tests {
    #[test]
    fn old_versions_get_the_same_header_without_the_biome_flag() {
        let payload = [7u8; 100];
        let old = super::level_chunk_counted(1, 2, 3, &payload, crate::versions::features(2168));
        let new = crate::network::protocol::bedrock::level_chunk_probe(1, 2, 3, &payload, false);
        assert_eq!(&new[..new.len() - 1], &old[..]);
    }
}
