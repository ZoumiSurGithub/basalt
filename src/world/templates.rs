//! Plans officiels des structures (fichiers `.nbt` du serveur Bedrock de Mojang).
//!
//! Le serveur officiel range dans `behavior_packs/vanilla/structures` les plans de ses
//! villages, avant-postes, cités antiques et ruines de piste, au format des structures de
//! Java : une palette de blocs (noms et propriétés de Java), les blocs du plan, et des blocs
//! « jigsaw » qui disent comment les pièces s'emboîtent. Ces fichiers appartiennent à Mojang :
//! ils ne sont pas fournis avec Basalt. S'ils sont présents à côté du serveur (dossier
//! `structures` ou archive `structures.zip`), les structures sont assemblées avec eux, comme
//! dans le jeu ; sinon Basalt garde ses structures faites à la main.
//!
//! Ce module lit les plans, traduit chaque bloc de Java vers Bedrock (dans les 4 rotations),
//! puis assemble une structure pièce par pièce à partir des blocs jigsaw.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use super::generator::Rng;
use crate::nbt::Tag;

/// Familles de plans utilisées (sous-dossiers de `structures`).
const FAMILIES: [&str; 10] = ["village/", "pillageroutpost/", "ancient_city/", "trail_ruins/", "shipwreck/", "ruin/", "igloo/", "ruined_portal/", "fossils/", "mansion/"];

/// Directions des blocs jigsaw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Dir {
    fn parse(s: &str) -> Option<Dir> {
        Some(match s {
            "down" => Dir::Down,
            "up" => Dir::Up,
            "north" => Dir::North,
            "south" => Dir::South,
            "west" => Dir::West,
            "east" => Dir::East,
            _ => return None,
        })
    }
    fn word(self) -> &'static str {
        ["down", "up", "north", "south", "west", "east"][self as usize]
    }
    /// Un quart de tour dans le sens des aiguilles d'une montre, `r` fois (vu du dessus).
    fn turn(self, r: u8) -> Dir {
        let mut d = self;
        for _ in 0..(r & 3) {
            d = match d {
                Dir::North => Dir::East,
                Dir::East => Dir::South,
                Dir::South => Dir::West,
                Dir::West => Dir::North,
                v => v,
            };
        }
        d
    }
    fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }
    fn step(self) -> (i32, i32, i32) {
        match self {
            Dir::Down => (0, -1, 0),
            Dir::Up => (0, 1, 0),
            Dir::North => (0, 0, -1),
            Dir::South => (0, 0, 1),
            Dir::West => (-1, 0, 0),
            Dir::East => (1, 0, 0),
        }
    }
    fn horizontal(self) -> bool {
        !matches!(self, Dir::Up | Dir::Down)
    }
}

/// Bloc jigsaw d'un plan (position dans le plan non tourné).
#[derive(Clone, Debug)]
pub struct Jigsaw {
    pos: (i32, i32, i32),
    front: Dir,
    top: Dir,
    /// Réserve où piocher la pièce suivante (« minecraft:empty » : aucune)
    pub pool: String,
    /// Nom de ce point d'attache, et nom de celui qu'il cherche chez la pièce suivante
    name: String,
    target: String,
    /// Peut tourner librement autour de sa face (sinon le dessus doit correspondre)
    rollable: bool,
    /// Bloc qui le remplace une fois la structure posée, dans les 4 rotations
    finals: [Option<u32>; 4],
}

/// Un plan : ses blocs (déjà traduits pour Bedrock) et ses points d'attache.
pub struct Template {
    pub path: String,
    size: (i32, i32, i32),
    /// État Bedrock de chaque entrée de la palette, pour chacune des 4 rotations
    /// (indice : miroir × 4 + rotation ; miroir 0 aucun, 1 gauche-droite, 2 avant-arrière)
    states: Vec<Vec<Option<u32>>>,
    blocks: Vec<(i16, i16, i16, u16)>,
    /// Coffres et leur table de butin (nom du jeu)
    chests: Vec<((i32, i32, i32), String)>,
    /// Lits et leur couleur (la couleur d'un lit est une donnée à part dans Bedrock)
    beds: Vec<((i32, i32, i32), u8)>,
    /// Repères laissés par les auteurs (« chest », « supplyChest »…) : emplacements de coffres
    markers: Vec<((i32, i32, i32), String)>,
    pub jigsaws: Vec<Jigsaw>,
}

impl Template {
    /// Taille une fois tourné.
    fn size_rot(&self, r: u8) -> (i32, i32, i32) {
        if r & 1 == 1 {
            (self.size.2, self.size.1, self.size.0)
        } else {
            self.size
        }
    }
    /// Position dans le plan tourné d'un point du plan d'origine.
    fn rotate(&self, (x, y, z): (i32, i32, i32), r: u8) -> (i32, i32, i32) {
        let (sx, _, sz) = self.size;
        match r & 3 {
            0 => (x, y, z),
            1 => (sz - 1 - z, y, x),
            2 => (sx - 1 - x, y, sz - 1 - z),
            _ => (z, y, sx - 1 - x),
        }
    }
    /// Blocs du plan tourné : (position relative, état Bedrock).
    pub fn blocks(&self, r: u8) -> impl Iterator<Item = ((i32, i32, i32), u32)> + '_ {
        let r = r & 3;
        self.blocks.iter().filter_map(move |&(x, y, z, i)| {
            let s = self.states[r as usize].get(i as usize).copied().flatten()?;
            Some((self.rotate((x as i32, y as i32, z as i32), r), s))
        })
    }
    /// Blocs du plan non tourné, avec leur état pour la rotation `r` (pour tourner plusieurs
    /// plans ensemble autour d'un même point).
    pub fn blocks_unturned(&self, r: u8) -> impl Iterator<Item = ((i32, i32, i32), u32)> + '_ {
        let r = (r & 3) as usize;
        self.blocks.iter().filter_map(move |&(x, y, z, i)| Some(((x as i32, y as i32, z as i32), self.states[r].get(i as usize).copied().flatten()?)))
    }
    /// Blocs du plan non tourné, avec leur état après le miroir puis la rotation donnés
    /// (pièces du manoir : le jeu les retourne parfois).
    pub fn blocks_mirrored(&self, mirror: u8, r: u8) -> impl Iterator<Item = ((i32, i32, i32), u32)> + '_ {
        let k = (mirror as usize % 3) * 4 + (r & 3) as usize;
        self.blocks.iter().filter_map(move |&(x, y, z, i)| Some(((x as i32, y as i32, z as i32), self.states[k].get(i as usize).copied().flatten()?)))
    }
    /// Repères du plan tourné : (position relative, texte).
    pub fn markers(&self, r: u8) -> impl Iterator<Item = ((i32, i32, i32), &str)> + '_ {
        self.markers.iter().map(move |(p, m)| (self.rotate(*p, r), m.as_str()))
    }
    /// Repères du plan non tourné.
    pub fn markers_unturned(&self) -> impl Iterator<Item = ((i32, i32, i32), &str)> + '_ {
        self.markers.iter().map(|(p, m)| (*p, m.as_str()))
    }
    /// Taille du plan (non tourné).
    pub fn size(&self) -> (i32, i32, i32) {
        self.size
    }
    /// Lits du plan tourné : (position relative, couleur).
    pub fn beds(&self, r: u8) -> impl Iterator<Item = ((i32, i32, i32), u8)> + '_ {
        self.beds.iter().map(move |(p, c)| (self.rotate(*p, r), *c))
    }
    /// Coffres du plan tourné : (position relative, table de butin).
    pub fn chests(&self, r: u8) -> impl Iterator<Item = ((i32, i32, i32), &str)> + '_ {
        self.chests.iter().map(move |(p, l)| (self.rotate(*p, r), l.as_str()))
    }
    /// Hauteur (dans le plan) du jigsaw qui porte ce nom.
    pub fn anchor_y(&self, name: &str) -> Option<i32> {
        self.jigsaws.iter().find(|j| j.name == name).map(|j| j.pos.1)
    }
    /// Nombre de blocs (non vides) du plan.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

/// Tous les plans chargés, par chemin (« village/plains/houses/plains_small_house_1 »).
pub struct Templates {
    by_path: HashMap<String, Arc<Template>>,
    /// Blocs du jeu Java inconnus de Bedrock, ignorés (pour les tests)
    pub unknown: Vec<String>,
}

impl std::fmt::Debug for Templates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Templates({} plans)", self.by_path.len())
    }
}

impl Templates {
    /// Charge les plans depuis un dossier `structures` ou une archive `structures.zip`.
    /// `resolve` donne l'état Bedrock d'un bloc décrit par un texte (`nom[clé=valeur,…]`).
    pub fn load(path: &Path, resolve: &dyn Fn(&str) -> Option<u32>) -> Result<Templates, String> {
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        if path.is_dir() {
            walk(path, path, &mut files)?;
        } else {
            let data = std::fs::read(path).map_err(|e| format!("{} : {}", path.display(), e))?;
            files = unzip(&data)?;
        }
        let mut t = Templates { by_path: HashMap::new(), unknown: Vec::new() };
        let mut unknown = std::collections::BTreeSet::new();
        for (name, bytes) in files {
            // Chemin relatif au dossier « structures », sans l'extension
            let name = name.replace('\\', "/");
            // (« structures/village/… » dans une archive, « village/… » dans un dossier)
            let start = FAMILIES.iter().filter_map(|f| if name.starts_with(f) { Some(0) } else { name.find(&format!("/{}", f)).map(|i| i + 1) }).min();
            let Some(start) = start else { continue };
            let rel = &name[start..];
            if !rel.ends_with(".nbt") {
                continue;
            }
            let key = rel.trim_end_matches(".nbt").to_string();
            let raw = if bytes.starts_with(&[0x1f, 0x8b]) { crate::data::gunzip(&bytes, 1 << 26).map_err(|e| format!("{} : {}", key, e))? } else { bytes };
            let tag = crate::nbt::read_be(&raw).ok_or_else(|| format!("{} : NBT illisible", key))?;
            if let Some(tpl) = read_template(&key, &tag, resolve, &mut unknown) {
                t.by_path.insert(key, Arc::new(tpl));
            }
        }
        if t.by_path.is_empty() {
            return Err("aucun plan de village, d'avant-poste, de cité antique ou de ruine trouvé".into());
        }
        t.unknown = unknown.into_iter().collect();
        Ok(t)
    }

    /// Nombre de plans chargés.
    pub fn count(&self) -> usize {
        self.by_path.len()
    }

    /// Un plan par son chemin.
    pub fn get(&self, path: &str) -> Option<Arc<Template>> {
        self.by_path.get(path).cloned()
    }

    /// Plans d'une réserve (« minecraft:village/plains/houses »), triés par chemin.
    ///
    /// Les réserves du jeu ne sont pas dans ces fichiers : on les reconstitue d'après les
    /// dossiers. La plupart correspondent à un dossier ; quelques-unes sont des parties d'un
    /// dossier (`no_corners`, `grouped`, `tower_top`…).
    pub fn pool(&self, pool: &str) -> Vec<Arc<Template>> {
        let pool = pool.trim_start_matches("minecraft:");
        let in_dir = |dir: &str| -> Vec<Arc<Template>> {
            let mut v: Vec<(&String, &Arc<Template>)> = self
                .by_path
                .iter()
                .filter(|(k, _)| k.strip_prefix(dir).map(|rest| rest.starts_with('/') && !rest[1..].contains('/')).unwrap_or(false))
                .collect();
            v.sort_by(|a, b| a.0.cmp(b.0));
            v.into_iter().map(|(_, t)| t.clone()).collect()
        };
        let file = |path: &str| -> Vec<Arc<Template>> { self.get(path).into_iter().collect() };
        let named = |dir: &str, keep: &dyn Fn(&str) -> bool| -> Vec<Arc<Template>> { in_dir(dir).into_iter().filter(|t| keep(t.path.rsplit('/').next().unwrap_or(""))).collect() };
        match pool {
            "empty" => Vec::new(),
            // Décor des villages : les lanternes rangées à la racine du dossier du biome
            p if p.starts_with("village/") && p.ends_with("/decor") => in_dir(p.trim_end_matches("/decor")),
            "village/common/well_bottoms" => file("village/common/well_bottom"),
            // Bouts de rue : ceux des plaines pour les villages qui n'en ont pas
            p if p.starts_with("village/") && p.ends_with("/terminators") => {
                let v = in_dir(p);
                if v.is_empty() {
                    in_dir("village/plains/terminators")
                } else {
                    v
                }
            }
            "ancient_city/walls/no_corners" => named("ancient_city/walls", &|n| !n.contains("corner")),
            "trail_ruins/buildings/grouped" => named("trail_ruins/buildings", &|n| n.starts_with("group_")),
            "trail_ruins/buildings" => named("trail_ruins/buildings", &|n| !n.starts_with("group_")),
            "trail_ruins/tower" => named("trail_ruins/tower", &|n| n.starts_with("tower_") && !n.starts_with("tower_top")),
            "trail_ruins/tower/tower_top" => named("trail_ruins/tower", &|n| n.starts_with("tower_top")),
            "trail_ruins/tower/additions" => named("trail_ruins/tower", &|n| !n.starts_with("tower_")),
            "pillager_outpost/features" => named("pillageroutpost", &|n| n.starts_with("feature_")),
            p => in_dir(p),
        }
    }
}

/// Fichiers d'un dossier (récursif) : (chemin relatif, contenu).
fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{} : {}", dir.display(), e))?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out)?;
        } else if p.extension().map(|x| x == "nbt").unwrap_or(false) {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            let data = std::fs::read(&p).map_err(|e| format!("{} : {}", p.display(), e))?;
            out.push((rel, data));
        }
    }
    Ok(())
}

/// Fichiers d'une archive zip (stockés ou compressés en « deflate »).
fn unzip(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16le = |p: usize| -> Result<usize, String> { data.get(p..p + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or_else(|| "archive zip tronquée".to_string()) };
    let u32le = |p: usize| -> Result<usize, String> { data.get(p..p + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).ok_or_else(|| "archive zip tronquée".to_string()) };
    // Fin du répertoire central : cherchée en partant de la fin (après un commentaire éventuel)
    let end = (0..data.len().saturating_sub(21)).rev().take(70_000).find(|&i| data[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06])).ok_or("pas une archive zip")?;
    let count = u16le(end + 10)?;
    let mut pos = u32le(end + 16)?;
    let mut out = Vec::new();
    for _ in 0..count {
        if u32le(pos)? != 0x0201_4b50 {
            return Err("répertoire de l'archive zip abîmé".into());
        }
        let method = u16le(pos + 10)?;
        let packed = u32le(pos + 20)?;
        let (name_len, extra, comment) = (u16le(pos + 28)?, u16le(pos + 30)?, u16le(pos + 32)?);
        let local = u32le(pos + 42)?;
        let name = String::from_utf8_lossy(data.get(pos + 46..pos + 46 + name_len).ok_or("archive zip tronquée")?).to_string();
        pos += 46 + name_len + extra + comment;
        if name.ends_with('/') || !name.ends_with(".nbt") {
            continue;
        }
        let start = local + 30 + u16le(local + 26)? + u16le(local + 28)?;
        let raw = data.get(start..start + packed).ok_or("archive zip tronquée")?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => crate::utils::inflate::inflate(raw, 1 << 26).map_err(|e| format!("{} : {}", name, e))?,
            m => return Err(format!("{} : compression {} non prise en charge", name, m)),
        };
        out.push((name, bytes));
    }
    Ok(out)
}

/// Lit un plan et traduit ses blocs.
fn read_template(path: &str, tag: &Tag, resolve: &dyn Fn(&str) -> Option<u32>, unknown: &mut std::collections::BTreeSet<String>) -> Option<Template> {
    let size: Vec<i32> = tag.get("size")?.as_list()?.iter().filter_map(|t| t.as_int()).collect();
    if size.len() != 3 {
        return None;
    }
    let mut palette: Vec<(String, Vec<(String, String)>)> = tag
        .get("palette")?
        .as_list()?
        .iter()
        .map(|e| {
            let name = e.get("Name").and_then(|n| n.as_str()).unwrap_or("minecraft:air").trim().to_string();
            (name, props_of(e.get("Properties")))
        })
        .collect();
    let mut t = Template { path: path.to_string(), size: (size[0], size[1], size[2]), states: vec![Vec::new(); 12], blocks: Vec::new(), chests: Vec::new(), beds: Vec::new(), markers: Vec::new(), jigsaws: Vec::new() };
    for b in tag.get("blocks")?.as_list()? {
        let pos: Vec<i32> = b.get("pos").and_then(|p| p.as_list()).map(|l| l.iter().filter_map(|t| t.as_int()).collect()).unwrap_or_default();
        let state = b.get("state").and_then(|s| s.as_int()).unwrap_or(0) as usize;
        if pos.len() != 3 || state >= palette.len() {
            continue;
        }
        let at = (pos[0], pos[1], pos[2]);
        let (name, props) = &palette[state];
        let nbt = b.get("nbt");
        if name == "minecraft:jigsaw" || nbt.and_then(|n| n.get("final_state")).is_some() {
            if let Some(j) = read_jigsaw(at, props, nbt?, resolve, unknown) {
                t.jigsaws.push(j);
            }
            continue;
        }
        if name == "minecraft:structure_block" {
            if let Some(m) = nbt.and_then(|n| n.get("metadata")).and_then(|m| m.as_str()) {
                t.markers.push((at, m.to_string()));
            }
        }
        if let Some(color) = name.strip_prefix("minecraft:").and_then(|n| n.strip_suffix("_bed")).and_then(dye_index) {
            t.beds.push((at, color));
        }
        if name.ends_with("chest") {
            if let Some(loot) = nbt.and_then(|n| n.get("LootTable")).and_then(|l| l.as_str()) {
                t.chests.push((at, loot.to_string()));
            }
        }
        t.blocks.push((at.0 as i16, at.1 as i16, at.2 as i16, state as u16));
    }
    // Formes qui dépendent des voisins (coins d'escaliers, barrières, vitres, murets) :
    // recalculées comme le jeu le fait en posant un plan (les plus anciens les notent mal)
    fix_shapes(&mut palette, &mut t.blocks);
    for (k, list) in t.states.iter_mut().enumerate() {
        let (m, r) = ((k / 4) as u8, (k % 4) as u8);
        // Le miroir ne sert qu'au manoir : inutile de le calculer pour les autres plans
        if m > 0 && !path.starts_with("mansion/") {
            continue;
        }
        for (name, props) in &palette {
            let s = java_to_bedrock(name, &rotate_props(&mirror_props(props, m), r), resolve);
            if s.is_none() && name != "minecraft:jigsaw" {
                #[cfg(test)]
                eprintln!("inconnu {} {:?} r{} dans {}", name, props, r, path);
                unknown.insert(name.clone());
            }
            list.push(s);
        }
    }
    Some(t)
}

/// Nom actuel d'un bloc (les anciens noms traduits), sans « minecraft: ».
fn modern_name(name: &str, props: &[(String, String)]) -> String {
    let short = name.trim().trim_start_matches("minecraft:");
    match legacy_name(short, props) {
        Some((n, _)) => n,
        None => short.to_string(),
    }
}

/// Bloc plein (au sens des raccords : une barrière ou une vitre s'y accroche), d'après son nom.
fn java_full(short: &str) -> bool {
    const PARTIAL: [&str; 62] = [
        "air", "slab", "stairs", "fence", "_wall", "pane", "iron_bars", "door", "torch", "lantern", "carpet", "flower", "grass", "fern", "leaves", "sign",
        "banner", "button", "lever", "pressure_plate", "ladder", "vine", "chest", "_bed", "candle", "rail", "skull", "head", "pot", "chain", "campfire",
        "bell", "snow", "water", "lava", "cake", "web", "cobweb", "sapling", "mushroom", "wheat", "carrots", "potatoes", "beetroots", "stem", "cane", "cactus",
        "kelp", "seagrass", "coral", "pickle", "scaffolding", "anvil", "grindstone", "lectern", "brewing", "cauldron", "hopper", "structure", "jigsaw",
        "sculk_vein", "path",
    ];
    !PARTIAL.iter().any(|p| short.contains(p)) || short == "snow_block" || short.ends_with("_wood") && !short.contains("door")
}

/// Recalcule, dans le plan non tourné, les coins des escaliers et les raccords des
/// barrières, vitres, barreaux et murets, d'après leurs voisins dans le plan (règles du jeu).
fn fix_shapes(palette: &mut Vec<(String, Vec<(String, String)>)>, blocks: &mut [(i16, i16, i16, u16)]) {
    let at: HashMap<(i32, i32, i32), usize> = blocks.iter().map(|&(x, y, z, i)| ((x as i32, y as i32, z as i32), i as usize)).collect();
    let names: Vec<String> = palette.iter().map(|(n, p)| modern_name(n, p)).collect();
    let prop = |i: usize, k: &str| palette[i].1.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
    let stairs = |i: usize| -> Option<(Dir, String)> {
        if !names[i].ends_with("_stairs") {
            return None;
        }
        Some((Dir::parse(&prop(i, "facing")?)?, prop(i, "half").unwrap_or_else(|| "bottom".into())))
    };
    let mut changes: Vec<(usize, Vec<(String, String)>)> = Vec::new();
    for (k, &(x, y, z, i)) in blocks.iter().enumerate() {
        let i = i as usize;
        let (x, y, z) = (x as i32, y as i32, z as i32);
        let near = |d: Dir| {
            let (dx, dy, dz) = d.step();
            at.get(&(x + dx, y + dy, z + dz)).copied()
        };
        let name = &names[i];
        let mut props = palette[i].1.clone();
        let set = |props: &mut Vec<(String, String)>, key: &str, v: String| match props.iter_mut().find(|(a, _)| a == key) {
            Some(e) => e.1 = v,
            None => props.push((key.to_string(), v)),
        };
        if let Some((facing, half)) = stairs(i) {
            // StairBlock.getStairsShape du jeu
            let counter = facing.turn(3);
            let can_take = |face: Dir| match near(face).and_then(stairs) {
                Some((f, h)) => f != facing || h != half,
                None => true,
            };
            let axis = |d: Dir| matches!(d, Dir::North | Dir::South);
            let mut shape = "straight";
            if let Some((f, h)) = near(facing).and_then(stairs) {
                if h == half && axis(f) != axis(facing) && can_take(f.opposite()) {
                    shape = if f == counter { "outer_left" } else { "outer_right" };
                }
            }
            if shape == "straight" {
                if let Some((f, h)) = near(facing.opposite()).and_then(stairs) {
                    if h == half && axis(f) != axis(facing) && can_take(f) {
                        shape = if f == counter { "inner_left" } else { "inner_right" };
                    }
                }
            }
            set(&mut props, "shape", shape.into());
        } else if name.ends_with("_fence") || name.ends_with("_pane") || name == "iron_bars" || (name.ends_with("_wall") && !name.contains("torch") && !name.contains("sign") && !name.contains("banner")) {
            let wall = name.ends_with("_wall");
            let fence = name.ends_with("_fence");
            let mut sides = Vec::new();
            for d in [Dir::North, Dir::East, Dir::South, Dir::West] {
                let joins = near(d).map(|n| {
                    let o = &names[n];
                    java_full(o)
                        || (fence && (o.ends_with("_fence") && (o == "nether_brick_fence") == (name == "nether_brick_fence") || o.ends_with("fence_gate")))
                        || (!fence && !wall && (o.ends_with("_pane") || o == "iron_bars" || o.ends_with("_wall")))
                        || (wall && (o.ends_with("_wall") || o.ends_with("fence_gate") || o.ends_with("_pane") || o == "iron_bars"))
                });
                sides.push((d, joins.unwrap_or(false)));
            }
            for (d, j) in &sides {
                let v = if wall {
                    if *j { "low" } else { "none" }.to_string()
                } else {
                    j.to_string()
                };
                set(&mut props, d.word(), v);
            }
            if wall {
                let on = |d: Dir| sides.iter().any(|(s, j)| *s == d && *j);
                let straight = (on(Dir::North) && on(Dir::South) && !on(Dir::East) && !on(Dir::West)) || (on(Dir::East) && on(Dir::West) && !on(Dir::North) && !on(Dir::South));
                let above = near(Dir::Up).map(|n| names[n].ends_with("_wall")).unwrap_or(false);
                set(&mut props, "up", (!straight || above).to_string());
            }
        } else {
            continue;
        }
        if props != palette[i].1 {
            changes.push((k, props));
        }
    }
    // Nouvelles entrées de palette (une par combinaison)
    let mut index: HashMap<(String, Vec<(String, String)>), u16> = HashMap::new();
    for (k, props) in changes {
        let old = blocks[k].3 as usize;
        let key = (palette[old].0.clone(), props);
        let id = *index.entry(key.clone()).or_insert_with(|| {
            palette.push(key);
            (palette.len() - 1) as u16
        });
        blocks[k].3 = id;
    }
}

/// Numéro d'une couleur de teinture (ordre du jeu : blanc 0 … noir 15).
pub fn dye_index(color: &str) -> Option<u8> {
    const DYES: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
    DYES.iter().position(|d| *d == color).map(|i| i as u8)
}

/// Propriétés d'une entrée de palette : (clé, valeur) en texte.
fn props_of(tag: Option<&Tag>) -> Vec<(String, String)> {
    match tag {
        Some(Tag::Compound(list)) => list.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect(),
        _ => Vec::new(),
    }
}

/// Bloc jigsaw : ancien format (« target_pool », « attachement_type », « facing ») ou
/// nouveau (« pool », « name », « target », « joint », « orientation »).
fn read_jigsaw(pos: (i32, i32, i32), props: &[(String, String)], nbt: &Tag, resolve: &dyn Fn(&str) -> Option<u32>, unknown: &mut std::collections::BTreeSet<String>) -> Option<Jigsaw> {
    let get = |k: &str| nbt.get(k).and_then(|t| t.as_str()).map(|s| s.to_string());
    let prop = |k: &str| props.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    let (front, top) = match prop("orientation") {
        Some(o) => {
            let (f, t) = o.split_once('_')?;
            (Dir::parse(f)?, Dir::parse(t)?)
        }
        None => {
            let f = Dir::parse(prop("facing").unwrap_or("up"))?;
            (f, if f.horizontal() { Dir::Up } else { Dir::North })
        }
    };
    let (pool, name, target) = match get("pool") {
        Some(pool) => (pool, get("name").unwrap_or_default(), get("target").unwrap_or_default()),
        None => {
            let a = get("attachement_type").unwrap_or_default();
            (get("target_pool").unwrap_or_default(), a.clone(), a)
        }
    };
    let rollable = get("joint").map(|j| j == "rollable").unwrap_or(true);
    let (fname, fprops) = parse_state(get("final_state").as_deref().unwrap_or("minecraft:air"));
    let mut finals = [None; 4];
    for (r, f) in finals.iter_mut().enumerate() {
        *f = java_to_bedrock(&fname, &rotate_props(&fprops, r as u8), resolve);
    }
    if finals[0].is_none() {
        unknown.insert(fname);
    }
    Some(Jigsaw { pos, front, top, pool, name, target, rollable, finals })
}

/// « minecraft:oak_stairs[facing=east,half=bottom] » -> (nom, propriétés).
fn parse_state(s: &str) -> (String, Vec<(String, String)>) {
    let s = s.trim();
    match s.find('[') {
        Some(i) => {
            let props = s[i + 1..]
                .trim_end_matches(']')
                .split(',')
                .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                .collect();
            (s[..i].trim().to_string(), props)
        }
        None => (s.to_string(), Vec::new()),
    }
}

/// Propriétés d'un bloc Java retourné : 1 gauche-droite (nord <-> sud), 2 avant-arrière
/// (est <-> ouest). Les coins d'escaliers et les charnières changent de côté.
fn mirror_props(props: &[(String, String)], m: u8) -> Vec<(String, String)> {
    if m == 0 {
        return props.to_vec();
    }
    let flip = |w: &str| -> String {
        match (m, w) {
            (1, "north") => "south".into(),
            (1, "south") => "north".into(),
            (2, "east") => "west".into(),
            (2, "west") => "east".into(),
            _ => w.into(),
        }
    };
    props
        .iter()
        .map(|(k, v)| match k.as_str() {
            "facing" => (k.clone(), flip(v)),
            "north" | "south" | "east" | "west" => (flip(k), v.clone()),
            "shape" => (k.clone(), if v.contains("left") { v.replace("left", "right") } else { v.replace("right", "left") }),
            "hinge" => (k.clone(), if v == "left" { "right".into() } else { "left".into() }),
            "rotation" => {
                let r = v.parse::<i32>().unwrap_or(0);
                (k.clone(), (if m == 1 { 8 - r } else { 16 - r }).rem_euclid(16).to_string())
            }
            _ => (k.clone(), v.clone()),
        })
        .collect()
}

/// Propriétés d'un bloc Java après `r` quarts de tour (sens des aiguilles d'une montre).
fn rotate_props(props: &[(String, String)], r: u8) -> Vec<(String, String)> {
    let r = r & 3;
    props
        .iter()
        .map(|(k, v)| {
            let side = |w: &str| Dir::parse(w).filter(|d| d.horizontal()).map(|d| d.turn(r).word().to_string());
            match k.as_str() {
                "facing" => (k.clone(), side(v).unwrap_or_else(|| v.clone())),
                "axis" if r & 1 == 1 => (k.clone(), if v == "x" { "z".into() } else if v == "z" { "x".into() } else { v.clone() }),
                "north" | "south" | "east" | "west" => (side(k).unwrap_or_else(|| k.clone()), v.clone()),
                "rotation" => (k.clone(), ((v.parse::<u32>().unwrap_or(0) + 4 * r as u32) % 16).to_string()),
                _ => (k.clone(), v.clone()),
            }
        })
        .collect()
}

// --- Traduction des blocs Java vers Bedrock ---------------------------------------------

/// `facing_direction` de Bedrock : 0 bas, 1 haut, 2 nord, 3 sud, 4 ouest, 5 est.
fn face6(d: Dir) -> u8 {
    d as u8
}
/// `direction` des lits, cloches, métiers à tisser… : 0 sud, 1 ouest, 2 nord, 3 est.
fn swne(d: Dir) -> u8 {
    match d {
        Dir::South => 0,
        Dir::West => 1,
        Dir::North => 2,
        _ => 3,
    }
}
/// `weirdo_direction` des escaliers et `direction` des trappes : 0 est, 1 ouest, 2 sud, 3 nord.
fn ewsn(d: Dir) -> u8 {
    match d {
        Dir::East => 0,
        Dir::West => 1,
        Dir::South => 2,
        _ => 3,
    }
}
/// Un quart de tour à gauche (vu du dessus).
fn left_of(d: Dir) -> Dir {
    d.turn(3)
}

/// Nom actuel d'un bloc noté à l'ancienne (« planks » + « variant=birch » -> « birch_planks »),
/// avec les propriétés qui restent. None : nom déjà actuel.
fn legacy_name(short: &str, props: &[(String, String)]) -> Option<(String, Vec<(String, String)>)> {
    let get = |k: &str| props.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    let keep = |drop: &[&str]| -> Vec<(String, String)> { props.iter().filter(|(k, _)| !drop.contains(&k.as_str())).cloned().collect() };
    let color = || match get("color").unwrap_or("white") {
        "silver" => "light_gray".to_string(),
        c => c.to_string(),
    };
    let slab_kind = |v: &str| -> String {
        match v {
            "smoothstone" | "stone" => "smooth_stone".into(),
            "stone_brick" => "stone_brick".into(),
            "wood_old" => "oak".into(),
            other => other.into(),
        }
    };
    Some(match short {
        "carpet" => (format!("{}_carpet", color()), Vec::new()),
        "wool" => (format!("{}_wool", color()), Vec::new()),
        "planks" => (format!("{}_planks", get("variant").unwrap_or("oak")), Vec::new()),
        "log" | "log2" => (format!("{}_log", get("variant").unwrap_or("oak")), keep(&["variant"])),
        "leaves" | "leaves2" => (format!("{}_leaves", get("variant").unwrap_or("oak")), Vec::new()),
        "sapling" => (format!("{}_sapling", get("type").unwrap_or("oak")), keep(&["type"])),
        "fence" => ("oak_fence".into(), keep(&[])),
        // Blocs infestés : posés comme le bloc qu'ils imitent (pas encore de poissons d'argent)
        "monster_egg" => {
            let n = match get("variant").unwrap_or("stone") {
                "stone_brick" => "stone_bricks",
                "mossy_brick" => "mossy_stone_bricks",
                "cracked_brick" => "cracked_stone_bricks",
                "chiseled_brick" => "chiseled_stone_bricks",
                "cobblestone" => "cobblestone",
                _ => "stone",
            };
            (n.into(), Vec::new())
        }
        "stonebrick" => {
            let n = match get("variant").unwrap_or("stonebrick") {
                "mossy_stonebrick" => "mossy_stone_bricks",
                "cracked_stonebrick" => "cracked_stone_bricks",
                "chiseled_stonebrick" => "chiseled_stone_bricks",
                _ => "stone_bricks",
            };
            (n.into(), Vec::new())
        }
        "stone_slab" | "wooden_slab" => (format!("{}_slab", slab_kind(get("variant").unwrap_or("oak"))), vec![("type".into(), get("half").unwrap_or("bottom").into())]),
        "double_stone_slab" | "double_wooden_slab" => (format!("{}_slab", slab_kind(get("variant").unwrap_or("oak"))), vec![("type".into(), "double".into())]),
        _ => return None,
    })
}

/// État Bedrock d'un bloc Java (nom et propriétés). None : bloc sans équivalent.
pub fn java_to_bedrock(name: &str, props: &[(String, String)], resolve: &dyn Fn(&str) -> Option<u32>) -> Option<u32> {
    // Anciens noms (plans du manoir, d'avant la 1.13) : d'abord vers les noms actuels
    if let Some((modern, rest)) = legacy_name(name.trim().trim_start_matches("minecraft:"), props) {
        return java_to_bedrock(&modern, &rest, resolve);
    }
    let short = name.trim().trim_start_matches("minecraft:");
    let p = |k: &str| props.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    let flag = |k: &str| (p(k) == Some("true")) as u8;
    let num = |k: &str| p(k).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let facing = p("facing").and_then(Dir::parse).unwrap_or(Dir::North);
    let mut out: Vec<String> = Vec::new();
    let mut set = |k: &str, v: String| out.push(format!("{}={}", k, v));
    let bedrock: String = match short {
        "air" | "cave_air" | "void_air" | "structure_block" | "structure_void" | "piston_head" => "air".into(),
        n if n.ends_with("banner") => "air".into(),
        "grass" => "short_grass".into(),
        "dirt_path" | "grass_path" => "grass_path".into(),
        "bricks" => "brick_block".into(),
        "magma_block" => "magma".into(),
        "terracotta" => "hardened_clay".into(),
        "melon" => "melon_block".into(),
        "note_block" => "noteblock".into(),
        "snow_block" => "snow".into(),
        n if n.starts_with("potted_") => "flower_pot".into(),
        n if n.ends_with("pressure_plate") => {
            set("redstone_signal", "0".into());
            if n == "oak_pressure_plate" {
                "wooden_pressure_plate".into()
            } else {
                n.into()
            }
        }
        "snow" => {
            set("height", (num("layers") - 1).clamp(0, 7).to_string());
            "snow_layer".into()
        }
        n if n.ends_with("_stairs") => {
            let base = if n == "cobblestone_stairs" { "stone_stairs" } else { n };
            // Coins : Bedrock ne note que les coins « droits » ; un coin gauche est le coin
            // droit de l'escalier tourné d'un quart de tour à gauche
            let (dir, corner) = match p("shape").unwrap_or("straight") {
                "outer_left" => (left_of(facing), "outer_right"),
                "inner_left" => (left_of(facing), "inner_right"),
                "outer_right" => (facing, "outer_right"),
                "inner_right" => (facing, "inner_right"),
                _ => (facing, "none"),
            };
            set("weirdo_direction", ewsn(dir).to_string());
            set("upside_down_bit", ((p("half") == Some("top")) as u8).to_string());
            set("minecraft:corner", corner.into());
            base.into()
        }
        n if n.ends_with("_slab") => {
            if p("type") == Some("double") {
                set("minecraft:vertical_half", "bottom".into());
                n.replace("_slab", "_double_slab")
            } else {
                set("minecraft:vertical_half", p("type").unwrap_or("bottom").into());
                n.into()
            }
        }
        n if n.ends_with("_door") => {
            // Porte : direction notée avec un quart de tour d'écart (relevé en jeu)
            set("minecraft:cardinal_direction", facing.turn(1).word().into());
            set("door_hinge_bit", ((p("hinge") == Some("right")) as u8).to_string());
            set("open_bit", flag("open").to_string());
            set("upper_block_bit", ((p("half") == Some("upper")) as u8).to_string());
            if n == "oak_door" {
                "wooden_door".into()
            } else {
                n.into()
            }
        }
        n if n.ends_with("_bed") => {
            set("direction", swne(facing).to_string());
            set("head_piece_bit", ((p("part") == Some("head")) as u8).to_string());
            "bed".into()
        }
        n if n.ends_with("fence_gate") => {
            set("minecraft:cardinal_direction", facing.word().into());
            set("in_wall_bit", flag("in_wall").to_string());
            set("open_bit", flag("open").to_string());
            if n == "oak_fence_gate" {
                "fence_gate".into()
            } else {
                n.into()
            }
        }
        n if n.ends_with("trapdoor") => {
            set("direction", ewsn(facing).to_string());
            set("open_bit", flag("open").to_string());
            set("upside_down_bit", ((p("half") == Some("top")) as u8).to_string());
            if n == "oak_trapdoor" {
                "trapdoor".into()
            } else {
                n.into()
            }
        }
        n if n.ends_with("_fence") || n.ends_with("_pane") || n == "iron_bars" => {
            for side in ["east", "north", "south", "west"] {
                set(&format!("minecraft:connection_{}", side), flag(side).to_string());
            }
            n.into()
        }
        n if n.ends_with("_wall") && !n.ends_with("_wall_torch") && !n.ends_with("_wall_sign") => {
            for side in ["east", "north", "south", "west"] {
                let v = match p(side).unwrap_or("none") {
                    "low" | "true" => "short",
                    "tall" => "tall",
                    _ => "none",
                };
                set(&format!("wall_connection_type_{}", side), v.into());
            }
            set("wall_post_bit", flag("up").to_string());
            n.into()
        }
        "wall_torch" => {
            // Le mot désigne le côté du bloc qui porte la torche
            set("torch_facing_direction", facing.opposite().word().into());
            "torch".into()
        }
        "torch" => {
            set("torch_facing_direction", "top".into());
            "torch".into()
        }
        "redstone_wall_torch" | "redstone_torch" => {
            let side = if short == "redstone_torch" { "top" } else { facing.opposite().word() };
            set("torch_facing_direction", side.into());
            if p("lit") == Some("false") {
                "unlit_redstone_torch".into()
            } else {
                "redstone_torch".into()
            }
        }
        n if n == "ladder" || n.ends_with("_wall_sign") || n == "skeleton_skull" => {
            set("facing_direction", if n == "skeleton_skull" { 1 } else { face6(facing) }.to_string());
            n.into()
        }
        n if n.ends_with("_button") => {
            let f = match p("face") {
                Some("floor") => 1,
                Some("ceiling") => 0,
                _ => face6(facing),
            };
            set("facing_direction", f.to_string());
            n.into()
        }
        n if n.ends_with("glazed_terracotta") => {
            set("facing_direction", face6(facing).to_string());
            if n == "light_gray_glazed_terracotta" {
                "silver_glazed_terracotta".into()
            } else {
                n.into()
            }
        }
        "barrel" | "sticky_piston" | "piston" => {
            set("facing_direction", face6(facing).to_string());
            if short == "barrel" {
                set("open_bit", flag("open").to_string());
            }
            short.into()
        }
        "chest" | "trapped_chest" | "furnace" | "blast_furnace" | "smoker" | "lectern" | "carved_pumpkin" | "jack_o_lantern" | "anvil" | "chipped_anvil" | "damaged_anvil" => {
            set("minecraft:cardinal_direction", facing.word().into());
            short.into()
        }
        "stonecutter" => {
            set("minecraft:cardinal_direction", facing.word().into());
            "stonecutter_block".into()
        }
        "campfire" | "soul_campfire" => {
            set("minecraft:cardinal_direction", facing.word().into());
            set("extinguished", ((p("lit") == Some("false")) as u8).to_string());
            short.into()
        }
        "repeater" => {
            set("minecraft:cardinal_direction", facing.word().into());
            set("repeater_delay", (num("delay") - 1).clamp(0, 3).to_string());
            "unpowered_repeater".into()
        }
        "comparator" => {
            set("minecraft:cardinal_direction", facing.word().into());
            set("output_subtract_bit", ((p("mode") == Some("subtract")) as u8).to_string());
            "unpowered_comparator".into()
        }
        "bell" => {
            let a = match p("attachment") {
                Some("ceiling") => "hanging",
                Some("single_wall") => "side",
                Some("double_wall") => "multiple",
                _ => "standing",
            };
            set("attachment", a.into());
            set("direction", swne(facing).to_string());
            "bell".into()
        }
        "grindstone" => {
            let a = match p("face") {
                Some("ceiling") => "hanging",
                Some("wall") => "side",
                _ => "standing",
            };
            set("attachment", a.into());
            set("direction", swne(facing).to_string());
            "grindstone".into()
        }
        "loom" => {
            set("direction", swne(facing).to_string());
            "loom".into()
        }
        "lever" => {
            let along = if matches!(facing, Dir::North | Dir::South) { "north_south" } else { "east_west" };
            let v = match p("face") {
                Some("floor") => format!("up_{}", along),
                Some("ceiling") => format!("down_{}", along),
                _ => facing.word().to_string(),
            };
            set("lever_direction", v);
            set("open_bit", flag("powered").to_string());
            "lever".into()
        }
        "lantern" | "soul_lantern" => {
            set("hanging", flag("hanging").to_string());
            short.into()
        }
        n if n.ends_with("candle") => {
            set("candles", (num("candles") - 1).clamp(0, 3).to_string());
            set("lit", flag("lit").to_string());
            n.into()
        }
        "wheat" | "carrots" | "potatoes" | "beetroots" => {
            set("growth", num("age").clamp(0, 7).to_string());
            short.into()
        }
        "melon_stem" | "pumpkin_stem" => {
            set("growth", num("age").clamp(0, 7).to_string());
            short.into()
        }
        "attached_melon_stem" | "attached_pumpkin_stem" => {
            set("growth", "7".into());
            set("facing_direction", face6(facing).to_string());
            short.trim_start_matches("attached_").into()
        }
        "farmland" => {
            set("moisturized_amount", num("moisture").clamp(0, 7).to_string());
            "farmland".into()
        }
        "tall_grass" | "large_fern" | "sunflower" | "lilac" | "rose_bush" | "peony" => {
            set("upper_block_bit", ((p("half") == Some("upper")) as u8).to_string());
            short.into()
        }
        n if n.ends_with("_leaves") => {
            set("persistent_bit", "1".into());
            n.into()
        }
        "water" | "lava" => {
            set("liquid_depth", num("level").clamp(0, 15).to_string());
            short.into()
        }
        "cauldron" | "water_cauldron" => {
            let level = num("level");
            if level > 0 {
                set("fill_level", (level * 2).clamp(0, 6).to_string());
                set("cauldron_liquid", "water".into());
            }
            "cauldron".into()
        }
        "composter" => {
            set("composter_fill_level", num("level").clamp(0, 8).to_string());
            "composter".into()
        }
        "vine" => {
            let bits = flag("south") | flag("west") << 1 | flag("north") << 2 | flag("east") << 3;
            set("vine_direction_bits", bits.to_string());
            "vine".into()
        }
        "redstone_wire" => {
            set("redstone_signal", num("power").clamp(0, 15).to_string());
            "redstone_wire".into()
        }
        "redstone_lamp" => {
            if p("lit") == Some("true") {
                "lit_redstone_lamp".into()
            } else {
                "redstone_lamp".into()
            }
        }
        "sea_pickle" => {
            set("cluster_count", (num("pickles") - 1).clamp(0, 3).to_string());
            set("dead_bit", ((p("waterlogged") != Some("true")) as u8).to_string());
            "sea_pickle".into()
        }
        n if n.ends_with("_sapling") => {
            set("age_bit", num("stage").clamp(0, 1).to_string());
            n.into()
        }
        n => {
            // Bûches, piliers, bottes de foin… : seul l'axe compte
            if let Some(axis) = p("axis") {
                set("pillar_axis", axis.into());
            }
            n.into()
        }
    };
    let full = format!("minecraft:{}", bedrock);
    if !out.is_empty() {
        if let Some(s) = resolve(&format!("{}[{}]", full, out.join(","))) {
            return Some(s);
        }
        #[cfg(test)]
        eprintln!("propriétés refusées : {}[{}]", full, out.join(","));
    }
    resolve(&full)
}

// --- Assemblage ---------------------------------------------------------------------------

/// Boîte (bornes incluses).
#[derive(Clone, Copy, Debug)]
pub struct Box3 {
    pub min: (i32, i32, i32),
    pub max: (i32, i32, i32),
}

impl Box3 {
    fn intersects(&self, o: &Box3) -> bool {
        self.min.0 <= o.max.0 && self.max.0 >= o.min.0 && self.min.1 <= o.max.1 && self.max.1 >= o.min.1 && self.min.2 <= o.max.2 && self.max.2 >= o.min.2
    }
    fn contains(&self, p: (i32, i32, i32)) -> bool {
        (self.min.0..=self.max.0).contains(&p.0) && (self.min.1..=self.max.1).contains(&p.1) && (self.min.2..=self.max.2).contains(&p.2)
    }
    fn inside(&self, o: &Box3) -> bool {
        self.min.0 >= o.min.0 && self.max.0 <= o.max.0 && self.min.1 >= o.min.1 && self.max.1 <= o.max.1 && self.min.2 >= o.min.2 && self.max.2 <= o.max.2
    }
}

/// Une pièce posée : plan, rotation, coin (x, y, z minimaux).
pub struct Piece {
    pub template: Arc<Template>,
    pub rot: u8,
    pub at: (i32, i32, i32),
    /// Pièce qui suit le relief (rues des villages) : chaque bloc est posé à la hauteur du
    /// sol de sa colonne, plus sa hauteur dans le plan
    pub follows_ground: bool,
    pub bbox: Box3,
}

/// Réglages d'un assemblage.
pub struct Rules<'a> {
    /// Profondeur maximale (nombre de pièces entre le départ et la plus lointaine)
    pub depth: u32,
    /// Distance horizontale maximale au départ
    pub reach: i32,
    /// Vrai pour les réserves dont les pièces suivent le relief
    pub follows_ground: &'a dyn Fn(&str) -> bool,
    /// Réserve de secours quand rien ne convient (bouts de rue), et chance de ne rien poser
    pub fallback: &'a dyn Fn(&str) -> Option<String>,
    pub empty_chance: &'a dyn Fn(&str) -> f64,
}

/// Résultat : les pièces, et les blocs jigsaw à remplacer (position, état, suit le relief).
#[derive(Default)]
pub struct Layout {
    pub pieces: Vec<Piece>,
    /// (position, état, suit le relief) ; pour une pièce qui suit le relief, y est la
    /// hauteur dans le plan
    pub finals: Vec<((i32, i32, i32), u32, bool)>,
}

impl Templates {
    /// Assemble une structure à partir d'une pièce de départ, en suivant les jigsaws comme
    /// le jeu : chaque point d'attache libre pioche une pièce dans sa réserve, la tourne et
    /// la place contre lui si elle ne chevauche rien ; sinon il essaie la suivante.
    ///
    /// `start_y(t, r)` donne le bas de la pièce de départ, `ground(x, z)` la hauteur du sol.
    #[allow(clippy::too_many_arguments)]
    pub fn assemble(&self, start: Arc<Template>, rot: u8, (x, z): (i32, i32), start_y: i32, rules: &Rules, rng: &mut Rng, ground: &mut dyn FnMut(i32, i32) -> i32) -> Layout {
        let mut out = Layout::default();
        let (sx, sy, sz) = start.size_rot(rot);
        let at = (x - sx / 2, start_y, z - sz / 2);
        let follows = false;
        let bbox = Box3 { min: at, max: (at.0 + sx - 1, at.1 + sy - 1, at.2 + sz - 1) };
        let limit = Box3 { min: (x - rules.reach, -30_000, z - rules.reach), max: (x + rules.reach, 30_000, z + rules.reach) };
        out.pieces.push(Piece { template: start, rot, at, follows_ground: follows, bbox });
        // File des pièces dont les jigsaws restent à traiter : (indice, profondeur)
        let mut queue = std::collections::VecDeque::from([(0usize, 0u32)]);
        while let Some((index, depth)) = queue.pop_front() {
            let (parent_t, parent_rot, parent_at, parent_box, parent_follows) = {
                let p = &out.pieces[index];
                (p.template.clone(), p.rot, p.at, p.bbox, p.follows_ground)
            };
            for j in &parent_t.jigsaws {
                let rel = parent_t.rotate(j.pos, parent_rot);
                let jpos = (parent_at.0 + rel.0, parent_at.1 + rel.1, parent_at.2 + rel.2);
                let front = j.front.turn(parent_rot);
                let top = j.top.turn(parent_rot);
                let step = front.step();
                let target = (jpos.0 + step.0, jpos.1 + step.1, jpos.2 + step.2);
                let pool = j.pool.trim_start_matches("minecraft:").to_string();
                if pool == "empty" {
                    continue;
                }
                // Pièces candidates : la réserve (sauf au bout de la profondeur), puis le secours
                let mut candidates: Vec<(Arc<Template>, String)> = Vec::new();
                if depth < rules.depth {
                    if (rng.unit()) < (rules.empty_chance)(&pool) {
                        continue;
                    }
                    let mut list = self.pool(&pool);
                    shuffle(&mut list, rng);
                    candidates.extend(list.into_iter().map(|t| (t, pool.clone())));
                }
                if let Some(fb) = (rules.fallback)(&pool) {
                    let mut list = self.pool(&fb);
                    shuffle(&mut list, rng);
                    candidates.extend(list.into_iter().map(|t| (t, fb.clone())));
                }
                let inside = parent_box.contains(target);

                'search: for (cand, cand_pool) in candidates {
                    let mut rots = [0u8, 1, 2, 3];
                    shuffle(&mut rots, rng);
                    for r in rots {
                        let mut ks: Vec<&Jigsaw> = cand.jigsaws.iter().collect();
                        shuffle(&mut ks, rng);
                        for k in ks {
                            if k.name != j.target {
                                continue;
                            }
                            let kf = k.front.turn(r);
                            if kf != front.opposite() {
                                continue;
                            }
                            // Jointure alignée (verticale) : les dessus doivent se correspondre
                            if !front.horizontal() && !j.rollable && k.top.turn(r) != top {
                                continue;
                            }
                            let krel = cand.rotate(k.pos, r);
                            let child_follows = (rules.follows_ground)(&cand_pool);
                            // Hauteur : rigide sur rigide, au bloc près ; sinon d'après le sol
                            let min_y = if !parent_follows && !child_follows { target.1 - krel.1 } else { ground(target.0, target.2) + 1 - krel.1 };
                            let cat = (target.0 - krel.0, min_y, target.2 - krel.2);
                            let (cx, cy, cz) = cand.size_rot(r);
                            let cbox = if child_follows {
                                Box3 { min: (cat.0, -20_000, cat.2), max: (cat.0 + cx - 1, 20_000, cat.2 + cz - 1) }
                            } else {
                                Box3 { min: cat, max: (cat.0 + cx - 1, cat.1 + cy - 1, cat.2 + cz - 1) }
                            };
                            let fits = if inside {
                                cbox.inside(&parent_box) && out.pieces.iter().enumerate().all(|(i, p)| i == index || !p.bbox.intersects(&cbox))
                            } else {
                                cbox.inside(&limit) && out.pieces.iter().all(|p| !p.bbox.intersects(&cbox))
                            };
                            if !fits {
                                continue;
                            }
                            out.pieces.push(Piece { template: cand.clone(), rot: r, at: cat, follows_ground: child_follows, bbox: cbox });
                            queue.push_back((out.pieces.len() - 1, depth + 1));
                            break 'search;
                        }
                    }
                }
            }
        }
        // Les jigsaws deviennent leur bloc final
        for p in &out.pieces {
            for j in &p.template.jigsaws {
                if let Some(s) = j.finals[p.rot as usize] {
                    // Pièce qui suit le relief : hauteur dans le plan (le sol s'y ajoute)
                    let rel = p.template.rotate(j.pos, p.rot);
                    let y = if p.follows_ground { rel.1 } else { p.at.1 + rel.1 };
                    out.finals.push(((p.at.0 + rel.0, y, p.at.2 + rel.2), s, p.follows_ground));
                }
            }
        }
        out
    }
}

/// Mélange de Fisher-Yates.
fn shuffle<T>(v: &mut [T], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        let j = rng.below(i as u32 + 1) as usize;
        v.swap(i, j);
    }
}

/// Une pièce par son chemin, ou la première d'une réserve (pour les tests et les départs).
pub fn pick(list: &[Arc<Template>], rng: &mut Rng) -> Option<Arc<Template>> {
    if list.is_empty() {
        None
    } else {
        Some(list[rng.below(list.len() as u32) as usize].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plans du jeu pour les tests : `BASALT_STRUCTURES` ou /tmp/vs/structures (absents de
    /// Basalt : sans eux, ces tests ne vérifient rien).
    pub fn test_templates(data: &crate::data::GameData) -> Option<Templates> {
        let path = std::env::var("BASALT_STRUCTURES").unwrap_or_else(|_| "/tmp/vs/structures".into());
        let path = Path::new(&path);
        if !path.exists() {
            eprintln!("plans absents ({}) : test ignoré", path.display());
            return None;
        }
        Some(Templates::load(path, &|s| data.state_from_spec(s)).unwrap())
    }

    fn data() -> crate::data::GameData {
        crate::data::GameData::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("data")).unwrap()
    }

    #[test]
    fn rotations_and_jigsaws() {
        assert_eq!(Dir::North.turn(1), Dir::East);
        assert_eq!(Dir::West.turn(1), Dir::North);
        assert_eq!(Dir::Up.turn(3), Dir::Up);
        let props = vec![("facing".to_string(), "north".to_string()), ("east".to_string(), "true".to_string()), ("axis".to_string(), "x".to_string())];
        let r = rotate_props(&props, 1);
        assert!(r.contains(&("facing".into(), "east".into())));
        assert!(r.contains(&("south".into(), "true".into())));
        assert!(r.contains(&("axis".into(), "z".into())));
    }

    #[test]
    fn java_blocks_become_bedrock() {
        let d = data();
        let resolve = |s: &str| d.state_from_spec(s);
        let conv = |name: &str, props: &[(&str, &str)]| {
            let props: Vec<(String, String)> = props.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
            java_to_bedrock(name, &props, &resolve).and_then(|s| d.block_name(s).map(|n| n.to_string()))
        };
        assert_eq!(conv("minecraft:oak_door", &[("facing", "south"), ("half", "upper")]).as_deref(), Some("minecraft:wooden_door"));
        assert_eq!(conv("minecraft:cobblestone_stairs", &[("facing", "south"), ("half", "top"), ("shape", "outer_left")]).as_deref(), Some("minecraft:stone_stairs"));
        assert_eq!(conv("minecraft:white_bed", &[("facing", "south"), ("part", "head")]).as_deref(), Some("minecraft:bed"));
        assert_eq!(conv("minecraft:spruce_slab", &[("type", "double")]).as_deref(), Some("minecraft:spruce_double_slab"));
        assert_eq!(conv("minecraft:wall_torch", &[("facing", "west")]).as_deref(), Some("minecraft:torch"));
        assert_eq!(conv("minecraft:grass", &[]).as_deref(), Some("minecraft:short_grass"));
        assert_eq!(conv("minecraft:monster_egg", &[("variant", "cobblestone")]).as_deref(), Some("minecraft:cobblestone"));
        assert_eq!(conv("minecraft:planks", &[("variant", "birch")]).as_deref(), Some("minecraft:birch_planks"));
        assert_eq!(conv("minecraft:light_gray_glazed_terracotta", &[("facing", "west")]).as_deref(), Some("minecraft:silver_glazed_terracotta"));
        // Les propriétés passent bien (sinon on retomberait sur l'état par défaut)
        let st = java_to_bedrock("minecraft:oak_stairs", &[("facing".into(), "west".into()), ("half".into(), "top".into())], &resolve).unwrap();
        assert_eq!(d.prop(st, "weirdo_direction"), Some(crate::data::Prop::Int(1)));
        assert_eq!(d.prop(st, "upside_down_bit"), Some(crate::data::Prop::Int(1)));
        let st = java_to_bedrock("minecraft:oak_log", &[("axis".into(), "z".into())], &resolve).unwrap();
        assert_eq!(d.prop(st, "pillar_axis"), Some(crate::data::Prop::Str("z".into())));
        let st = java_to_bedrock("minecraft:cobblestone_wall", &[("east".into(), "true".into()), ("up".into(), "true".into())], &resolve).unwrap();
        assert_eq!(d.prop(st, "wall_connection_type_east"), Some(crate::data::Prop::Str("short".into())));
    }

    #[test]
    fn official_templates_load() {
        let d = data();
        let Some(t) = test_templates(&d) else { return };
        assert!(t.count() > 400, "{} plans", t.count());
        assert!(t.unknown.is_empty(), "blocs sans équivalent : {:?}", t.unknown);
        for pool in ["village/plains/town_centers", "village/plains/streets", "village/plains/houses", "village/plains/decor", "village/snowy/terminators", "ancient_city/city_center", "ancient_city/walls/no_corners", "trail_ruins/tower", "trail_ruins/tower/tower_top", "pillager_outpost/features"] {
            assert!(!t.pool(pool).is_empty(), "réserve vide : {}", pool);
        }
        // Chaque bloc a un état dans les quatre rotations
        let house = t.get("village/plains/houses/plains_small_house_1").unwrap();
        for r in 0..4 {
            assert_eq!(house.blocks(r).count(), house.len());
        }
    }
}
