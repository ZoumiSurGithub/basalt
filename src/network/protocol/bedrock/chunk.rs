//! Terrain : identifiants de blocs, contenu d'un chunk plat, paquet LevelChunk.

use super::*;

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
