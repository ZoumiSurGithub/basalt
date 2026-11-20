//! Le monde : un terrain plat, plus les modifications faites par les joueurs.
//!
//! Le terrain de base est calculé (pierre, terre, terre, herbe, de y = -64 à -61).
//! Seuls les blocs modifiés sont gardés en mémoire, rangés par chunk. Un chunk
//! sans modification est envoyé avec le contenu commun précalculé ; un chunk
//! modifié est encodé à la demande.
//!
//! Les modifications sont enregistrées dans `level.nbt`.
//!
//! Les plugins lisent le monde depuis leur propre fil, par une « vue » (`WorldView`) : une
//! copie instantanée qui ne coûte rien à prendre. Le stockage est partagé, et n'est recopié
//! (chunk par chunk) que si le serveur modifie un bloc pendant qu'une vue est encore utilisée.

use std::collections::HashMap;
use std::sync::Arc;

use crate::bedrock::{block_hash, W};

/// Hauteur minimale du monde (surface).
pub const MIN_Y: i32 = -64;
/// Hauteur maximale où l'on peut construire.
pub const MAX_Y: i32 = 319;
/// Blocs modifiés tolérés au total : au-delà, les nouvelles modifications sont refusées.
const MAX_EDITS: usize = 2_000_000;

/// Blocs modifiés d'un chunk : (x local, y, z local) -> identifiant réseau du bloc.
type ChunkEdits = HashMap<(u8, i16, u8), u32>;

/// Vue du monde pour un autre fil (les plugins) : lecture seule, figée au moment où elle a
/// été prise.
#[derive(Clone)]
pub struct WorldView {
    edits: Arc<HashMap<(i32, i32), Arc<ChunkEdits>>>,
    air: u32,
    stone: u32,
    dirt: u32,
    grass: u32,
}

impl WorldView {
    /// Bloc présent à une position (identifiant réseau du bloc et de son état).
    pub fn get(&self, x: i32, y: i32, z: i32) -> u32 {
        if !World::in_bounds(y) {
            return self.air;
        }
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        match self.edits.get(&(x >> 4, z >> 4)).and_then(|c| c.get(&key)) {
            Some(b) => *b,
            None => match y {
                -64 => self.stone,
                -63 | -62 => self.dirt,
                -61 => self.grass,
                _ => self.air,
            },
        }
    }
}

/// Le monde (comme `World` dans PocketMine).
pub struct World {
    /// (chunk x, chunk z) -> (x local, y, z local) -> identifiant réseau du bloc
    edits: Arc<HashMap<(i32, i32), Arc<ChunkEdits>>>,
    count: usize,
    /// Modifié depuis le dernier enregistrement
    pub dirty: bool,
    /// Données attachées aux blocs : contenu des coffres, état des fours, texte des panneaux
    pub tiles: crate::tile::TileMap,
    pub air: u32,
    stone: u32,
    dirt: u32,
    grass: u32,
}

impl Default for World {
    fn default() -> World {
        World::new()
    }
}

impl World {
    pub fn new() -> World {
        World {
            edits: Arc::new(HashMap::new()),
            count: 0,
            dirty: false,
            tiles: crate::tile::TileMap::new(),
            air: block_hash("minecraft:air"),
            stone: block_hash("minecraft:stone"),
            dirt: block_hash("minecraft:dirt"),
            grass: block_hash("minecraft:grass_block"),
        }
    }

    /// Bloc du terrain d'origine à cette hauteur.
    fn base(&self, y: i32) -> u32 {
        match y {
            -64 => self.stone,
            -63 | -62 => self.dirt,
            -61 => self.grass,
            _ => self.air,
        }
    }

    pub fn in_bounds(y: i32) -> bool {
        (MIN_Y..=MAX_Y).contains(&y)
    }

    /// Bloc présent à une position.
    pub fn get(&self, x: i32, y: i32, z: i32) -> u32 {
        if !Self::in_bounds(y) {
            return self.air;
        }
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        match self.edits.get(&(x >> 4, z >> 4)).and_then(|c| c.get(&key)) {
            Some(b) => *b,
            None => self.base(y),
        }
    }

    /// Change un bloc. Renvoie faux si la position est hors du monde ou si la limite est atteinte.
    pub fn set(&mut self, x: i32, y: i32, z: i32, block: u32) -> bool {
        if !Self::in_bounds(y) {
            return false;
        }
        let chunk = (x >> 4, z >> 4);
        let key = ((x & 15) as u8, y as i16, (z & 15) as u8);
        if block == self.base(y) {
            // retour à l'état d'origine : rien à garder
            if self.edits.get(&chunk).map(|c| c.contains_key(&key)).unwrap_or(false) {
                let all = Arc::make_mut(&mut self.edits);
                if let Some(c) = all.get_mut(&chunk) {
                    let c = Arc::make_mut(c);
                    c.remove(&key);
                    self.count -= 1;
                    self.dirty = true;
                    if c.is_empty() {
                        all.remove(&chunk);
                    }
                }
            }
            return true;
        }
        let known = self.edits.get(&chunk).map(|c| c.contains_key(&key)).unwrap_or(false);
        if !known {
            if self.count >= MAX_EDITS {
                return false;
            }
            self.count += 1;
        }
        if self.edits.get(&chunk).and_then(|c| c.get(&key)) == Some(&block) {
            return true;
        }
        let all = Arc::make_mut(&mut self.edits);
        Arc::make_mut(all.entry(chunk).or_default()).insert(key, block);
        self.dirty = true;
        true
    }

    /// Vue du monde pour un autre fil (plugins) : copie instantanée, sans recopie des blocs.
    pub fn view(&self) -> WorldView {
        WorldView { edits: self.edits.clone(), air: self.air, stone: self.stone, dirt: self.dirt, grass: self.grass }
    }

    /// Nombre de blocs qui diffèrent du terrain d'origine.
    pub fn edit_count(&self) -> usize {
        self.count
    }

    /// Enregistre les blocs modifiés dans `level.nbt`. Chaque bloc occupe quatre entiers
    /// (x, y, z, identifiant) ; l'identifiant est stable (dérivé du nom et de l'état), donc
    /// valable d'une version du jeu à l'autre.
    pub fn save(&mut self, dir: &std::path::Path) -> std::io::Result<()> {
        let doc = self.snapshot();
        Self::write_snapshot(dir, &doc)
    }

    /// Document à enregistrer (la partie rapide de l'enregistrement, faite dans la boucle
    /// principale) ; le monde est ensuite considéré comme enregistré.
    pub fn snapshot(&mut self) -> crate::nbt::Tag {
        use crate::nbt::Tag;
        self.dirty = false;
        let mut blocks = Vec::with_capacity(self.count * 4);
        for ((cx, cz), edits) in self.edits.iter() {
            for ((x, y, z), block) in edits.iter() {
                blocks.extend_from_slice(&[cx * 16 + *x as i32, *y as i32, cz * 16 + *z as i32, *block as i32]);
            }
        }
        Tag::compound(vec![("Version", Tag::Int(1)), ("Blocks", Tag::IntArray(blocks))])
    }

    /// Écrit un document préparé par `snapshot` (compression et disque : la partie lente,
    /// qui peut se faire sur un autre fil).
    pub fn write_snapshot(dir: &std::path::Path, doc: &crate::nbt::Tag) -> std::io::Result<()> {
        crate::nbt::save_file(&dir.join("level.nbt"), doc)?;
        // L'ancien format n'a plus lieu d'être une fois le nouveau fichier écrit
        let _ = std::fs::remove_file(dir.join("blocs.bin"));
        Ok(())
    }

    /// Recharge les blocs enregistrés. Renvoie leur nombre (0 si aucun fichier).
    pub fn load(&mut self, dir: &std::path::Path) -> Result<usize, String> {
        match crate::nbt::load_file(&dir.join("level.nbt"))? {
            Some(doc) => {
                let blocks = doc.get("Blocks").and_then(|t| t.as_int_array()).ok_or("level.nbt : liste « Blocks » absente")?;
                if blocks.len() % 4 != 0 {
                    return Err("level.nbt : liste « Blocks » tronquée".into());
                }
                for b in blocks.chunks_exact(4) {
                    self.set(b[0], b[1], b[2], b[3] as u32);
                }
                self.dirty = false;
                Ok(self.count)
            }
            None => self.load_legacy(dir),
        }
    }

    /// Ancien format (`blocs.bin`) : relu une dernière fois, puis remplacé par `level.nbt`
    /// au prochain enregistrement.
    fn load_legacy(&mut self, dir: &std::path::Path) -> Result<usize, String> {
        let data = match std::fs::read(dir.join("blocs.bin")) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.to_string()),
        };
        if data.len() < 16 || &data[..8] != b"BASALTW1" {
            return Err("fichier du monde non reconnu".into());
        }
        let count = u64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8])) as usize;
        if data.len() != 16 + count.saturating_mul(16) {
            return Err("fichier du monde tronqué".into());
        }
        for e in data[16..].chunks_exact(16) {
            let v = |i: usize| i32::from_le_bytes([e[i], e[i + 1], e[i + 2], e[i + 3]]);
            self.set(v(0), v(4), v(8), v(12) as u32);
        }
        // Reste « modifié » : le prochain enregistrement écrit level.nbt
        self.dirty = self.count > 0;
        Ok(self.count)
    }

    pub fn is_edited(&self, cx: i32, cz: i32) -> bool {
        self.edits.contains_key(&(cx, cz))
    }

    /// Contenu d'un chunk modifié : (nombre de sous-chunks, données). None si le chunk est d'origine.
    pub fn chunk_payload(&self, cx: i32, cz: i32) -> Option<(u32, Vec<u8>)> {
        let edits = self.edits.get(&(cx, cz))?;
        // Sous-chunk le plus haut à envoyer : celui du terrain d'origine, ou plus haut si l'on a construit.
        let mut top = MIN_Y + 15;
        for ((_, y, _), block) in edits.iter() {
            if *block != self.air && (*y as i32) > top {
                top = *y as i32;
            }
        }
        let sections = ((top - MIN_Y) / 16 + 1) as u32;
        let mut w = W::default();
        for s in 0..sections as i32 {
            let base_y = MIN_Y + s * 16;
            let mut palette: Vec<u32> = vec![self.air];
            let mut indices = vec![0u16; 4096];
            for x in 0..16usize {
                for z in 0..16usize {
                    for dy in 0..16usize {
                        let y = base_y + dy as i32;
                        let block = match edits.get(&(x as u8, y as i16, z as u8)) {
                            Some(b) => *b,
                            None => self.base(y),
                        };
                        let idx = match palette.iter().position(|p| *p == block) {
                            Some(i) => i,
                            None => {
                                palette.push(block);
                                palette.len() - 1
                            }
                        };
                        indices[(x << 8) | (z << 4) | dy] = idx as u16;
                    }
                }
            }
            encode_sub_chunk(&mut w, (base_y >> 4) as i8, &palette, &indices);
        }
        for _ in 0..24 {
            w.u8(1); // biome unique pour la section
            w.vari32(1); // plaines
        }
        w.u8(0); // blocs de bordure
        Some((sections, w.0))
    }
}

/// Encode un sous-chunk au format 9 : une couche de blocs avec sa palette.
fn encode_sub_chunk(w: &mut W, y_index: i8, palette: &[u32], indices: &[u16]) {
    w.u8(9);
    w.u8(1);
    w.u8(y_index as u8);
    let bits: usize = [1usize, 2, 3, 4, 5, 6, 8, 16]
        .iter()
        .copied()
        .find(|b| (1usize << b) >= palette.len())
        .unwrap_or(16);
    w.u8(((bits as u8) << 1) | 1);
    let per_word = 32 / bits;
    let mut word: u32 = 0;
    let mut filled = 0;
    for idx in indices {
        word |= (*idx as u32) << (bits * filled);
        filled += 1;
        if filled == per_word {
            w.u32(word);
            word = 0;
            filled = 0;
        }
    }
    if filled > 0 {
        w.u32(word);
    }
    w.vari32(palette.len() as i32);
    for block in palette {
        w.vari32(*block as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_are_tracked_and_reverted() {
        let mut world = World::new();
        assert_eq!(world.get(3, -61, 4), world.grass);
        assert!(world.chunk_payload(0, 0).is_none());
        assert!(world.set(3, -61, 4, world.air));
        assert_eq!(world.get(3, -61, 4), world.air);
        assert!(world.is_edited(0, 0));
        // remettre le bloc d'origine efface la modification
        assert!(world.set(3, -61, 4, world.grass));
        assert!(!world.is_edited(0, 0));
        assert!(!world.set(0, 400, 0, world.stone));
    }

    #[test]
    fn untouched_edit_chunk_matches_flat_payload() {
        // Un chunk dont la seule modification est hors du premier sous-chunk garde
        // un premier sous-chunk identique au terrain commun.
        let mut world = World::new();
        assert!(world.set(-1, -40, -1, world.stone));
        let (sections, payload) = world.chunk_payload(-1, -1).unwrap();
        assert_eq!(sections, 2);
        let flat = crate::bedrock::flat_chunk_payload();
        // même taille de premier sous-chunk : en-tête 4 octets + 256 mots + palette
        assert_eq!(&payload[..4], &flat[..4]);
        assert_eq!(payload[4..4 + 1024], flat[4..4 + 1024]);
    }

    #[test]
    fn negative_coordinates_use_floor_division() {
        let mut world = World::new();
        assert!(world.set(-1, -61, -17, world.air));
        assert!(world.is_edited(-1, -2));
        assert_eq!(world.get(-1, -61, -17), world.air);
        assert_eq!(world.get(-1, -61, -16), world.grass);
    }

    #[test]
    fn world_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("basalt-world-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut w = World::new();
        assert_eq!(w.load(&dir).unwrap(), 0);
        let stone = block_hash("minecraft:stone");
        w.set(-17, 10, 33, stone);
        w.set(5, -61, 5, w.air);
        assert!(w.dirty);
        w.save(&dir).unwrap();
        assert!(!w.dirty);
        let mut again = World::new();
        assert_eq!(again.load(&dir).unwrap(), 2);
        assert_eq!(again.get(-17, 10, 33), stone);
        assert_eq!(again.get(5, -61, 5), again.air);
        assert_eq!(again.get(6, -61, 5), w.get(6, -61, 5));
        // Ancien format : relu, puis remplacé
        let mut old = Vec::from(&b"BASALTW1"[..]);
        old.extend_from_slice(&1u64.to_le_bytes());
        for v in [3i32, 4, 5, stone as i32] {
            old.extend_from_slice(&v.to_le_bytes());
        }
        let legacy = dir.join("ancien");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("blocs.bin"), &old).unwrap();
        let mut migrated = World::new();
        assert_eq!(migrated.load(&legacy).unwrap(), 1);
        assert!(migrated.dirty);
        migrated.save(&legacy).unwrap();
        assert!(legacy.join("level.nbt").exists() && !legacy.join("blocs.bin").exists());
        assert_eq!(World::new().load(&legacy).unwrap(), 1);
        std::fs::write(dir.join("level.nbt"), b"n'importe quoi").unwrap();
        assert!(World::new().load(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

