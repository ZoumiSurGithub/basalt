//! Outils de mise au point avec le vrai jeu : variantes d'envoi du terrain.

use super::*;

// ---------------------------------------------------------------- Sondes ----
// Mise au point avec le vrai jeu : quand un joueur reste bloqué sur l'écran de
// chargement, le serveur lui renvoie le terrain sous d'autres formes, une par une,
// et le journal montre laquelle le débloque.

pub const PROBES: u8 = 6;

/// Description d'une expérience de mise au point des chunks.
pub fn probe_name(probe: u8) -> &'static str {
    match probe {
        1 => "chunks normaux, indicateur de biomes à vrai",
        2 => "chunks sans biomes, indicateur à vrai",
        3 => "biome 0 au lieu du biome 1",
        4 => "25 sections de biomes au lieu de 24",
        5 => "biomes sur 1 bit avec palette de deux entrées",
        _ => "sous-chunk plein de pierre sur 16 blocs de haut",
    }
}

/// Renvoie (nombre de sous-chunks, contenu, indicateur de biomes) pour une sonde.
pub fn probe_chunk(probe: u8) -> Option<(u32, Vec<u8>, bool)> {
    let sub = |w: &mut W, column: u32| {
        w.u8(9);
        w.u8(1);
        w.u8((-4i8) as u8);
        w.u8((2 << 1) | 1);
        for _ in 0..256 {
            w.u32(column);
        }
        w.vari32(4);
        for name in ["minecraft:air", "minecraft:stone", "minecraft:dirt", "minecraft:grass_block"].iter() {
            w.vari32(block_hash(name) as i32);
        }
    };
    let ground: u32 = 1 | (2 << 2) | (2 << 4) | (3 << 6);
    let simple_biomes = |w: &mut W, n: usize, id: i32| {
        for _ in 0..n {
            w.u8(1);
            w.vari32(id);
        }
    };
    let mut w = W::default();
    let flag = match probe {
        1 => {
            sub(&mut w, ground);
            simple_biomes(&mut w, 24, 1);
            true
        }
        2 => {
            sub(&mut w, ground);
            true
        }
        3 => {
            sub(&mut w, ground);
            simple_biomes(&mut w, 24, 0);
            false
        }
        4 => {
            sub(&mut w, ground);
            simple_biomes(&mut w, 25, 1);
            false
        }
        5 => {
            sub(&mut w, ground);
            for _ in 0..24 {
                w.u8((1 << 1) | 1);
                for _ in 0..128 {
                    w.u32(0);
                }
                w.vari32(2);
                w.vari32(1);
                w.vari32(1);
            }
            false
        }
        6 => {
            sub(&mut w, 0x5555_5555);
            simple_biomes(&mut w, 24, 1);
            false
        }
        _ => return None,
    };
    w.u8(0); // blocs de bordure
    Some((1, w.0, flag))
}

/// LevelChunk avec l'en-tête accepté par le jeu 1.26.65.
pub fn level_chunk_probe(x: i32, z: i32, count: u32, payload: &[u8], flag: bool) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(x);
    w.vari32(z);
    w.vari32(0);
    w.varu32(count);
    w.bool(false);
    w.bool(false);
    w.varu32(0);
    w.varu32(payload.len() as u32);
    w.0.extend_from_slice(payload);
    w.bool(flag);
    w.0
}
