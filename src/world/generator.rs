//! Génération du terrain.
//!
//! Deux générateurs :
//! - **plat** : pierre, deux couches de terre, herbe (les mondes créés avant la génération) ;
//! - **normal** : relief, océans, plages, plaines, forêts, déserts, montagnes enneigées,
//!   grottes, minerais, arbres et fleurs, à partir d'une graine.
//!
//! Le terrain est une fonction pure de la graine et de la position : un chunk se régénère
//! à l'identique à tout moment. Il n'est donc jamais enregistré ; seules les modifications
//! des joueurs le sont (voir `World`).
//!
//! Le relief vient de bruits de Perlin superposés (2D pour la hauteur et les biomes, 3D pour
//! les grottes, calculé sur une grille de 4 blocs puis interpolé, comme dans le jeu).

use super::{MAX_Y, MIN_Y};

/// Niveau de la mer (comme dans le jeu).
pub const SEA_LEVEL: i32 = 62;
/// Nombre de sous-chunks de 16 blocs entre `MIN_Y` et `MAX_Y`.
pub const SECTIONS: usize = ((MAX_Y - MIN_Y + 1) / 16) as usize;

// --- Bruit de Perlin -------------------------------------------------------------------

/// Bruit de Perlin à permutation tirée d'une graine.
#[derive(Clone)]
pub(crate) struct Perlin {
    perm: [u8; 512],
}

/// Générateur pseudo-aléatoire rapide et reproductible (SplitMix64).
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    /// Générateur pseudo-aléatoire (splitmix64) : même graine, même suite.
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Entier dans 0..n
    pub fn below(&mut self, n: u32) -> u32 {
        ((self.next() >> 32) * n as u64 >> 32) as u32
    }
    /// Réel dans 0..1
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Mélange d'une graine et de coordonnées, pour un tirage propre à chaque chunk.
pub(crate) fn mix(seed: u64, a: i64, b: i64, salt: u64) -> u64 {
    let mut r = Rng::new(seed ^ (a as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (b as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f) ^ salt.wrapping_mul(0x1656_67b1_9e37_79f9));
    r.next()
}

/// Courbe de lissage du bruit de Perlin (6t^5 - 15t^4 + 10t^3).
#[inline(always)]
pub(crate) fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Interpolation linéaire de a vers b.
#[inline(always)]
pub(crate) fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// Partie entière inférieure, sans passer par la bibliothèque mathématique (le processeur
/// de base visé n'a pas d'instruction d'arrondi : `f64::floor` y est un appel de fonction).
#[inline(always)]
pub(crate) fn ffloor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x {
        t - 1.0
    } else {
        t
    }
}

/// Produit scalaire avec l'un des 12 gradients de Perlin.
#[inline(always)]
fn grad3(hash: u8, x: f64, y: f64, z: f64) -> f64 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

impl Perlin {
    /// Table de permutation mélangée d'après la graine.
    pub(crate) fn new(seed: u64) -> Perlin {
        let mut p: Vec<u8> = (0..=255u8).collect();
        let mut rng = Rng::new(seed);
        for i in (1..256).rev() {
            let j = rng.below(i as u32 + 1) as usize;
            p.swap(i, j);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        Perlin { perm }
    }

    /// Bruit 3D, environ -1..1.
    pub(crate) fn noise3(&self, x: f64, y: f64, z: f64) -> f64 {
        let (xf, yf, zf) = (ffloor(x), ffloor(y), ffloor(z));
        let (xi, yi, zi) = ((xf as i64 & 255) as usize, (yf as i64 & 255) as usize, (zf as i64 & 255) as usize);
        let (x, y, z) = (x - xf, y - yf, z - zf);
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let p = &self.perm;
        // Indices toujours < 512 : les masques le montrent au compilateur (pas de contrôle)
        let a = (p[xi] as usize + yi) & 511;
        let (aa, ab) = ((p[a] as usize + zi) & 511, (p[(a + 1) & 511] as usize + zi) & 511);
        let b = (p[(xi + 1) & 511] as usize + yi) & 511;
        let (ba, bb) = ((p[b] as usize + zi) & 511, (p[(b + 1) & 511] as usize + zi) & 511);
        let (aa1, ab1, ba1, bb1) = ((aa + 1) & 511, (ab + 1) & 511, (ba + 1) & 511, (bb + 1) & 511);
        lerp(
            w,
            lerp(v, lerp(u, grad3(p[aa], x, y, z), grad3(p[ba], x - 1.0, y, z)), lerp(u, grad3(p[ab], x, y - 1.0, z), grad3(p[bb], x - 1.0, y - 1.0, z))),
            lerp(
                v,
                lerp(u, grad3(p[aa1], x, y, z - 1.0), grad3(p[ba1], x - 1.0, y, z - 1.0)),
                lerp(u, grad3(p[ab1], x, y - 1.0, z - 1.0), grad3(p[bb1], x - 1.0, y - 1.0, z - 1.0)),
            ),
        )
    }

    /// Bruit 2D : une tranche du bruit 3D.
    pub(crate) fn noise2(&self, x: f64, z: f64) -> f64 {
        self.noise3(x, 0.5, z)
    }
}

// --- Biomes ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Ocean,
    FrozenOcean,
    Beach,
    Plains,
    Forest,
    BirchForest,
    Desert,
    Mountains,
    SnowyPlains,
}

impl Biome {
    /// Identifiant du biome dans le protocole Bedrock.
    pub fn id(self) -> u32 {
        match self {
            Biome::Ocean => 0,
            Biome::Plains => 1,
            Biome::Desert => 2,
            Biome::Mountains => 3,
            Biome::Forest => 4,
            Biome::FrozenOcean => 10,
            Biome::SnowyPlains => 12,
            Biome::Beach => 16,
            Biome::BirchForest => 27,
        }
    }
}

// --- Blocs employés --------------------------------------------------------------------

/// Identifiants réseau des blocs utilisés par le générateur (état par défaut).
#[derive(Clone, Debug)]
pub struct Palette {
    pub air: u32,
    pub bedrock: u32,
    pub stone: u32,
    pub deepslate: u32,
    pub dirt: u32,
    pub grass: u32,
    pub sand: u32,
    pub sandstone: u32,
    pub gravel: u32,
    pub clay: u32,
    pub water: u32,
    pub ice: u32,
    pub snow_layer: u32,
    pub snow: u32,
    pub granite: u32,
    pub diorite: u32,
    pub andesite: u32,
    pub tuff: u32,
    pub lava: u32,
    pub oak_log: u32,
    pub oak_leaves: u32,
    pub birch_log: u32,
    pub birch_leaves: u32,
    pub spruce_log: u32,
    pub spruce_leaves: u32,
    pub cactus: u32,
    pub short_grass: u32,
    pub dandelion: u32,
    pub poppy: u32,
    /// (minerai dans la pierre, minerai dans la roche profonde)
    pub coal: (u32, u32),
    pub iron: (u32, u32),
    pub copper: (u32, u32),
    pub gold: (u32, u32),
    pub redstone: (u32, u32),
    pub lapis: (u32, u32),
    pub diamond: (u32, u32),
    /// Autres blocs, par nom court (« mycelium », « acacia_log »…), pour le terrain vanilla
    pub named: std::collections::HashMap<String, u32>,
    /// Plantes qui ne vivent que dans l'eau (herbes marines, laminaires) : toujours envoyées
    /// avec de l'eau dans le même bloc (seconde couche)
    pub aquatic: std::collections::HashSet<u32>,
    /// Coraux, gorgones, cornichons de mer : avec de l'eau si un bloc voisin est de l'eau
    pub soaked: std::collections::HashSet<u32>,
    /// Plans officiels des structures, s'ils sont fournis (voir `templates`)
    pub templates: Option<std::sync::Arc<super::templates::Templates>>,
}

/// Blocs supplémentaires employés par le terrain vanilla.
pub const EXTRA_BLOCKS: &[&str] = &[
    "netherrack", "soul_sand", "soul_soil", "glowstone", "quartz_ore", "nether_gold_ore", "ancient_debris", "basalt", "blackstone",
    "nether_brick", "nether_brick_fence", "nether_wart", "nether_wart[age=3]", "crimson_nylium", "warped_nylium", "crimson_stem", "warped_stem", "nether_wart_block", "warped_wart_block",
    "shroomlight", "crimson_roots", "warped_roots", "crimson_fungus", "warped_fungus", "nether_sprouts", "end_stone", "obsidian",
    "chorus_plant", "chorus_flower", "mycelium", "red_sand", "hardened_clay", "orange_terracotta", "yellow_terracotta", "white_terracotta", "brown_terracotta",
    "red_terracotta", "light_gray_terracotta", "mud", "packed_ice", "blue_ice", "powder_snow", "calcite", "moss_block", "podzol",
    "coarse_dirt", "acacia_log", "acacia_leaves", "jungle_log", "jungle_leaves", "dark_oak_log", "dark_oak_leaves", "cherry_log",
    "cherry_leaves", "mangrove_log", "mangrove_leaves", "mangrove_roots", "pale_oak_log", "pale_oak_leaves", "fern", "large_fern",
    "deadbush", "reeds", "sweet_berry_bush", "pumpkin", "blue_orchid", "allium", "azure_bluet", "red_tulip", "oxeye_daisy",
    "cornflower", "lily_of_the_valley", "waterlily", "seagrass", "emerald_ore", "deepslate_emerald_ore", "brown_mushroom",
    "red_mushroom", "bamboo", "moss_carpet", "azalea", "glow_lichen", "smooth_basalt", "dripstone_block", "sculk",
    "bush", "firefly_bush", "short_dry_grass", "tall_dry_grass", "cactus_flower", "kelp", "sea_pickle[cluster_count=1]",
    "sea_pickle[cluster_count=3]", "seagrass[sea_grass_type=double_bot]", "seagrass[sea_grass_type=double_top]",
    "tube_coral_block", "brain_coral_block", "bubble_coral_block", "fire_coral_block", "horn_coral_block", "tube_coral",
    "brain_coral", "bubble_coral", "fire_coral", "horn_coral", "tube_coral_fan", "brain_coral_fan", "bubble_coral_fan",
    "fire_coral_fan", "horn_coral_fan", "magma", "bee_nest", "vine", "vine[vine_direction_bits=1]", "vine[vine_direction_bits=2]",
    "vine[vine_direction_bits=4]", "vine[vine_direction_bits=8]",
    "pink_petals[growth=1,minecraft:cardinal_direction=north]", "pink_petals[growth=2,minecraft:cardinal_direction=east]",
    "pink_petals[growth=3,minecraft:cardinal_direction=south]", "pink_petals[growth=3,minecraft:cardinal_direction=west]",
    "leaf_litter[growth=1,minecraft:cardinal_direction=north]", "leaf_litter[growth=2,minecraft:cardinal_direction=east]",
    "leaf_litter[growth=3,minecraft:cardinal_direction=south]", "leaf_litter[growth=3,minecraft:cardinal_direction=west]",
    "wildflowers[growth=1,minecraft:cardinal_direction=north]", "wildflowers[growth=3,minecraft:cardinal_direction=south]",
];

impl Palette {
    /// `state` : état par défaut d'un bloc d'après son nom (données du jeu). Sans données, les
    /// blocs sans propriétés gardent leur identifiant calculé ; les autres sont remplacés.
    pub fn new(state: &dyn Fn(&str) -> Option<u32>) -> Palette {
        let hash = |n: &str| crate::bedrock::block_hash(&format!("minecraft:{}", n));
        let get = |n: &str| state(&format!("minecraft:{}", n)).unwrap_or_else(|| hash(n));
        // Bloc à propriétés : sans les données du jeu, un bloc simple le remplace
        let or = |n: &str, fallback: &str| state(&format!("minecraft:{}", n)).unwrap_or_else(|| get(fallback));
        let ore = |n: &str| (or(&format!("{}_ore", n), "stone"), or(&format!("deepslate_{}_ore", n), "stone"));
        Palette {
            air: get("air"),
            bedrock: or("bedrock", "stone"),
            stone: get("stone"),
            deepslate: or("deepslate", "stone"),
            dirt: get("dirt"),
            grass: get("grass_block"),
            sand: get("sand"),
            sandstone: or("sandstone", "sand"),
            gravel: get("gravel"),
            clay: get("clay"),
            water: or("water", "air"),
            ice: get("ice"),
            snow_layer: or("snow_layer", "air"),
            snow: get("snow"),
            granite: get("granite"),
            diorite: get("diorite"),
            andesite: get("andesite"),
            tuff: get("tuff"),
            lava: or("lava", "air"),
            oak_log: or("oak_log", "dirt"),
            oak_leaves: or("oak_leaves", "air"),
            birch_log: or("birch_log", "dirt"),
            birch_leaves: or("birch_leaves", "air"),
            spruce_log: or("spruce_log", "dirt"),
            spruce_leaves: or("spruce_leaves", "air"),
            cactus: or("cactus", "air"),
            short_grass: or("short_grass", "air"),
            dandelion: or("dandelion", "air"),
            poppy: or("poppy", "air"),
            coal: ore("coal"),
            iron: ore("iron"),
            copper: ore("copper"),
            gold: ore("gold"),
            redstone: ore("redstone"),
            lapis: ore("lapis"),
            diamond: ore("diamond"),
            named: EXTRA_BLOCKS
                .iter()
                .map(|n| n.to_string())
                .chain(super::structures::block_specs())
                .map(|n| {
                    let state = or(&n, n.split('[').next().unwrap_or(&n));
                    (n, state)
                })
                .collect(),
            aquatic: Default::default(),
            soaked: Default::default(),
            templates: None,
        }
        .with_water_sets()
    }

    /// Même palette, avec les plans officiels des structures.
    pub fn with_templates(mut self, templates: Option<std::sync::Arc<super::templates::Templates>>) -> Palette {
        self.templates = templates;
        self
    }

    /// Note les plantes aquatiques et les blocs gorgés d'eau de la palette.
    fn with_water_sets(mut self) -> Palette {
        for (name, state) in &self.named {
            let base = name.split('[').next().unwrap_or(name);
            if base == "seagrass" || base == "kelp" {
                self.aquatic.insert(*state);
            } else if base == "sea_pickle" || base.ends_with("_coral") || base.ends_with("_coral_fan") {
                self.soaked.insert(*state);
            }
        }
        self.aquatic.remove(&self.air);
        self.soaked.remove(&self.air);
        self
    }
}

// --- Colonne de chunk ------------------------------------------------------------------

/// Un sous-chunk de 16×16×16 blocs.
#[derive(Clone, Debug)]
pub enum Section {
    /// Le même bloc partout (souvent de l'air)
    Uniform(u32),
    /// Palette (256 blocs au plus) et indices (x << 8 | z << 4 | y)
    Mixed { palette: Vec<u32>, index: Box<[u8; 4096]> },
}

impl Section {
    /// État d'un bloc de la section.
    pub fn get(&self, x: usize, y: usize, z: usize) -> u32 {
        match self {
            Section::Uniform(b) => *b,
            Section::Mixed { palette, index } => palette[index[(x << 8) | (z << 4) | y] as usize],
        }
    }
}

/// Un chunk entier (16×16, de `MIN_Y` à `MAX_Y`), tel que généré.
#[derive(Clone, Debug)]
pub struct Column {
    pub sections: Vec<Section>,
    pub biome: Biome,
    /// Biomes du terrain vanilla : identifiant Bedrock par case de 4×4×4 blocs, sous-chunk par
    /// sous-chunk (64 cases chacun, x * 16 + z * 4 + y). Absent : `biome` partout.
    pub biome_ids: Option<Vec<u8>>,
    /// Coffres des structures générées (x local, y, z local, butin) : remplis au premier clic
    pub chests: Vec<(u8, i16, u8, super::structures::Loot)>,
    /// Lits des structures et leur couleur (x local, y, z local, couleur 0 blanc … 15 noir)
    pub beds: Vec<(u8, i16, u8, u8)>,
    /// Cloches des villages (x local, y, z local)
    pub bells: Vec<(u8, i16, u8)>,
    /// Blocs de structure posés dans l'eau (escaliers d'une épave…) : ils gardent cette eau
    pub wet: Vec<(u8, i16, u8)>,
}

impl Column {
    /// Bloc à une position locale (`x`, `z` de 0 à 15, `y` absolu).
    pub fn get(&self, x: usize, y: i32, z: usize) -> u32 {
        let dy = (y - MIN_Y) as usize;
        match self.sections.get(dy >> 4) {
            Some(s) => s.get(x, dy & 15, z),
            None => 0,
        }
    }

    /// Construit la colonne à partir d'un tableau plein (x, z, y).
    pub fn from_blocks(blocks: &[u32], air: u32, biome: Biome) -> Column {
        let height = (MAX_Y - MIN_Y + 1) as usize;
        let mut sections = Vec::with_capacity(SECTIONS);
        for s in 0..SECTIONS {
            let mut palette: Vec<u32> = Vec::new();
            let mut index = Box::new([0u8; 4096]);
            for x in 0..16 {
                for z in 0..16 {
                    for y in 0..16 {
                        let b = blocks[(x * 16 + z) * height + s * 16 + y];
                        let i = match palette.iter().position(|p| *p == b) {
                            Some(i) => i,
                            None => {
                                palette.push(b);
                                palette.len() - 1
                            }
                        };
                        index[(x << 8) | (z << 4) | y] = i.min(255) as u8;
                    }
                }
            }
            sections.push(if palette.len() == 1 { Section::Uniform(palette[0]) } else { Section::Mixed { palette, index } });
        }
        let _ = air;
        Column { sections, biome, biome_ids: None, chests: Vec::new(), beds: Vec::new(), bells: Vec::new(), wet: Vec::new() }
    }
}

// --- Générateur ------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Flat,
    /// Terrain à la manière du jeu depuis la 1.18 (bruits de climat, biomes, grottes)
    Vanilla,
    /// Monde vide : rien que de l'air (cartes construites à la main, mini-jeux)
    Void,
    /// Le Nether : une caverne de netherrack entre deux couches de roche mère, sur une mer de lave
    Nether,
    /// L'End : une île de pierre de l'End dans le vide, et des îles lointaines
    End,
    /// Générateur fourni par un plugin (`level-type=plugin:<nom>`)
    Plugin,
}

impl Kind {
    /// Nom enregistré dans `level.nbt`, ou réglage `level-type` de `server.properties`.
    /// Tout ce qui n'est ni plat ni un plugin est le terrain du jeu (`vanilla`, `normal`,
    /// `default`… ; l'ancien terrain propre à Basalt n'existe plus).
    pub fn parse(text: &str) -> Kind {
        match text.trim().to_ascii_lowercase().as_str() {
            "flat" | "plat" => Kind::Flat,
            "void" | "vide" => Kind::Void,
            "nether" => Kind::Nether,
            "end" | "the_end" => Kind::End,
            t if t.starts_with("plugin:") => Kind::Plugin,
            _ => Kind::Vanilla,
        }
    }
    /// Réglage `level-type` de `server.properties`, pour un nouveau monde.
    pub fn from_config(text: &str) -> Kind {
        Kind::parse(text)
    }
    /// Vrai si ce nom, lu dans un monde enregistré, désigne l'ancien terrain de Basalt,
    /// retiré : le monde continue avec le terrain du jeu.
    pub fn was_legacy(text: &str) -> bool {
        text.trim().eq_ignore_ascii_case("normal")
    }
    /// Dimension annoncée au jeu : 0 la surface, 1 le Nether, 2 l'End. Elle décide du ciel,
    /// du brouillard et de la hauteur du monde chez le joueur.
    pub fn dimension(self) -> i32 {
        match self {
            Kind::Nether => 1,
            Kind::End => 2,
            _ => 0,
        }
    }
    /// Tranches de 16 blocs que le jeu attend dans cette dimension, parmi les 24 de Basalt
    /// (de -64 à 319) : le Nether va de 0 à 127, l'End de 0 à 255 (relevé auprès du serveur
    /// officiel : 8 et 16 tranches de biomes).
    pub fn sections(self) -> std::ops::Range<usize> {
        match self {
            Kind::Nether => 4..12,
            Kind::End => 4..20,
            _ => 0..SECTIONS,
        }
    }
    /// Nom de la sorte de terrain dans server.properties.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Flat => "flat",
            Kind::Vanilla => "vanilla",
            Kind::Void => "void",
            Kind::Nether => "nether",
            Kind::End => "end",
            Kind::Plugin => "plugin",
        }
    }
}

pub struct Generator {
    pub kind: Kind,
    pub seed: u64,
    pub blocks: Palette,
    vanilla: Option<super::vanilla::Vanilla>,
    /// Générateur d'un plugin, et son nom (gardé même s'il n'a pas pu être chargé)
    plugin: Option<std::sync::Arc<crate::plugin::generator::LuaGenerator>>,
    plugin_name: String,
}

/// Un bloc de forteresse du Nether.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Piece {
    Brick,
    Air,
    Fence,
    /// Sable des âmes du jardin de verrues, et les verrues plantées dessus
    Soul,
    Wart,
    /// Toit d'un pont, posé seulement là où le pont traverse la roche
    Roof,
    /// Coffre (tourné vers l'est, sinon vers le sud)
    Chest(bool),
}

/// Salles au bout des ponts : (centre x, centre z) par rapport au centre de la forteresse.
const WART_ROOM: (i32, i32) = (FORTRESS_ARM - 4, 0);
const CHEST_ROOMS: [(i32, i32); 2] = [(-(FORTRESS_ARM - 4), 0), (0, -(FORTRESS_ARM - 4))];
const TERRACE: (i32, i32) = (0, FORTRESS_ARM - 4);

/// Le bloc de la forteresse à cet endroit (coordonnées par rapport à son centre), s'il y en a un.
pub fn fortress_piece(dx: i32, y: i32, dz: i32) -> Option<Piece> {
    let ry = y - FORTRESS_Y;
    if !(0..=6).contains(&ry) {
        return None;
    }
    // La salle centrale : 13 × 13, quatre portes vers les ponts
    if dx.abs() <= 6 && dz.abs() <= 6 {
        let wall = dx.abs() == 6 || dz.abs() == 6;
        return Some(match ry {
            0 | 6 => Piece::Brick,
            1..=3 if wall && (dx.abs() <= 1 || dz.abs() <= 1) => Piece::Air,
            _ if wall => Piece::Brick,
            _ => Piece::Air,
        });
    }
    // Les salles fermées au bout de trois ponts : 9 × 9, une porte vers le centre
    for (i, (ex, ez)) in std::iter::once(WART_ROOM).chain(CHEST_ROOMS).enumerate() {
        let (lx, lz) = (dx - ex, dz - ez);
        if lx.abs() > 4 || lz.abs() > 4 || ry > 5 {
            continue;
        }
        let wall = lx.abs() == 4 || lz.abs() == 4;
        // (la porte : sur le mur tourné vers le centre)
        let door = if ex != 0 { lx == -4 * ex.signum() && lz.abs() <= 1 } else { lz == -4 * ez.signum() && lx.abs() <= 1 };
        let garden = i == 0 && lx.abs() <= 2 && lz.abs() <= 2;
        // (le coffre : contre le mur du fond)
        let chest = i > 0 && if ex != 0 { lx == 3 * ex.signum() && lz == 0 } else { lz == 3 * ez.signum() && lx == 0 };
        return Some(match ry {
            0 if garden => Piece::Soul,
            0 | 5 => Piece::Brick,
            1..=3 if wall && door => Piece::Air,
            _ if wall => Piece::Brick,
            1 if garden => Piece::Wart,
            1 if chest => Piece::Chest(ex != 0),
            _ => Piece::Air,
        });
    }
    // La terrasse des blazes : à ciel ouvert, bordée de barrières
    let (lx, lz) = (dx - TERRACE.0, dz - TERRACE.1);
    if lx.abs() <= 4 && lz.abs() <= 4 && ry <= 4 {
        let edge = lx.abs() == 4 || lz.abs() == 4;
        return Some(match ry {
            0 => Piece::Brick,
            1 if edge && !(lz == -4 && lx.abs() <= 1) => Piece::Fence,
            _ => Piece::Air,
        });
    }
    // Les ponts : cinq blocs de large, une barrière de chaque côté
    let off = match (dz.abs() <= 2 && dx.abs() <= FORTRESS_ARM, dx.abs() <= 2 && dz.abs() <= FORTRESS_ARM) {
        (true, _) => dz.abs(),
        (_, true) => dx.abs(),
        _ => return None,
    };
    match ry {
        0 => Some(Piece::Brick),
        1 if off == 2 => Some(Piece::Fence),
        1..=4 => Some(Piece::Air),
        5 => Some(Piece::Roof),
        _ => None,
    }
}

/// Un pilier de la forteresse descend-il d'ici vers la lave ? (Sous les ponts tous les douze
/// blocs, et aux coins des salles.)
fn fortress_pillar(dx: i32, dz: i32) -> bool {
    let bridge = (dz.abs() <= 1 && dx.abs() <= FORTRESS_ARM && dx.rem_euclid(12) == 0) || (dx.abs() <= 1 && dz.abs() <= FORTRESS_ARM && dz.rem_euclid(12) == 0);
    let corner = |cx: i32, cz: i32, half: i32| (dx - cx).abs() == half && (dz - cz).abs() == half;
    bridge || corner(0, 0, 6) || corner(WART_ROOM.0, WART_ROOM.1, 4) || CHEST_ROOMS.iter().any(|c| corner(c.0, c.1, 4)) || corner(TERRACE.0, TERRACE.1, 4)
}

/// Biomes du Nether et de l'End, par leur numéro dans le jeu (relevés : 8 et 179 dans les
/// chunks du Nether du serveur officiel, 9 dans ceux de l'End).
pub const NETHER_WASTES: u8 = 8;
pub const NETHER_SOUL_VALLEY: u8 = 178;
pub const NETHER_CRIMSON: u8 = 179;
pub const NETHER_WARPED: u8 = 180;
pub const NETHER_DELTAS: u8 = 181;
pub const END_BIOME: u8 = 9;
/// Forteresse du Nether : hauteur de son sol, demi-longueur de ses ponts, pas de la grille
pub const FORTRESS_Y: i32 = 64;
pub const FORTRESS_ARM: i32 = 72;
const FORTRESS_GRID: i32 = 400;

impl Generator {
    /// Le générateur vanilla, pour un monde de ce type.
    pub fn vanilla(&self) -> Option<&super::vanilla::Vanilla> {
        self.vanilla.as_ref()
    }

    /// Générateur de cette sorte, avec une graine.
    pub fn new(kind: Kind, seed: u64, blocks: Palette) -> Generator {
        Generator {
            kind,
            seed,
            blocks,
            vanilla: if kind == Kind::Vanilla { Some(super::vanilla::Vanilla::new(seed)) } else { None },
            plugin: None,
            plugin_name: String::new(),
        }
    }

    /// Générateur d'un plugin. `plugin` est absent s'il n'a pas pu être chargé : le monde
    /// garde alors son type (enregistré tel quel) mais reste vide.
    pub fn with_plugin(name: &str, seed: u64, blocks: Palette, plugin: Option<std::sync::Arc<crate::plugin::generator::LuaGenerator>>) -> Generator {
        Generator { plugin, plugin_name: name.to_string(), ..Generator::new(Kind::Plugin, seed, blocks) }
    }

    /// Type de terrain tel qu'enregistré dans `level.nbt` (`plugin:<nom>` pour un plugin).
    pub fn stored_name(&self) -> String {
        match self.kind {
            Kind::Plugin => format!("plugin:{}", self.plugin_name),
            k => k.name().to_string(),
        }
    }

    /// Hauteur du sol (dernier bloc plein) d'une colonne, avant grottes et arbres. Pour le
    /// terrain d'un plugin, que le serveur ne sait pas prévoir : le niveau de la mer.
    pub fn surface(&self, x: i32, z: i32) -> i32 {
        match (&self.vanilla, self.kind) {
            (Some(v), _) => v.surface(x, z).0,
            (None, Kind::Flat) => -61,
            (None, _) => SEA_LEVEL,
        }
    }

    /// Biome du Nether à cet endroit : de grandes régions, tirées de deux bruits lents.
    pub fn nether_biome(&self, x: i32, z: i32) -> u8 {
        let n = Perlin::new(self.seed ^ 0x4249);
        let (t, h) = (n.noise2(x as f64 / 230.0, z as f64 / 230.0), n.noise2(x as f64 / 230.0 + 500.0, z as f64 / 230.0 - 300.0));
        match (t, h) {
            (t, h) if t > 0.18 && h > 0.0 => NETHER_CRIMSON,
            (t, _) if t > 0.18 => NETHER_DELTAS,
            (t, h) if t < -0.18 && h > 0.0 => NETHER_WARPED,
            (t, _) if t < -0.18 => NETHER_SOUL_VALLEY,
            _ => NETHER_WASTES,
        }
    }

    /// Centre de la forteresse du Nether dont les ponts peuvent passer ici, s'il y en a une
    /// dans cette case de la grille (deux cases sur trois en ont une).
    pub fn fortress_near(&self, x: i32, z: i32) -> Option<(i32, i32)> {
        if self.kind != Kind::Nether {
            return None;
        }
        let (rx, rz) = (x.div_euclid(FORTRESS_GRID), z.div_euclid(FORTRESS_GRID));
        let roll = mix(self.seed, rx as i64, rz as i64, 0x464f);
        if roll % 3 == 0 {
            return None;
        }
        let span = (FORTRESS_GRID - 2 * FORTRESS_ARM - 8) as u64;
        let fx = rx * FORTRESS_GRID + FORTRESS_ARM + 4 + ((roll >> 8) % span) as i32;
        let fz = rz * FORTRESS_GRID + FORTRESS_ARM + 4 + ((roll >> 24) % span) as i32;
        Some((fx, fz))
    }

    /// Ce point est-il un endroit dégagé d'une forteresse (pont, salle, terrasse) ? C'est là
    /// qu'apparaissent blazes et wither squelettes.
    pub fn in_fortress(&self, x: i32, y: i32, z: i32) -> bool {
        match self.fortress_near(x, z) {
            Some((fx, fz)) => matches!(fortress_piece(x - fx, y, z - fz), Some(Piece::Air)),
            None => false,
        }
    }

    /// Biome d'une colonne des terrains plat et de plugin : des plaines. (Le terrain du jeu
    /// a ses propres biomes, voir `vanilla.rs`.)
    pub fn biome(&self, _x: i32, _z: i32) -> Biome {
        Biome::Plains
    }

    /// Point d'apparition : (x, hauteur du sol, z).
    pub fn spawn(&self) -> (i32, i32, i32) {
        if self.kind == Kind::Plugin {
            return self.plugin.as_ref().and_then(|p| p.spawn()).unwrap_or((8, 64, 8));
        }
        match &self.vanilla {
            Some(v) => v.spawn(),
            None => (0, self.surface(0, 0), 0),
        }
    }

    /// Génère un chunk entier.
    pub fn generate(&self, cx: i32, cz: i32) -> Column {
        if self.kind == Kind::Plugin {
            let height = (MAX_Y - MIN_Y + 1) as usize;
            return match &self.plugin {
                Some(p) => p.generate(&self.blocks, cx, cz),
                None => Column::from_blocks(&vec![self.blocks.air; 256 * height], self.blocks.air, Biome::Plains),
            };
        }
        if let Some(v) = &self.vanilla {
            return v.generate(&self.blocks, cx, cz);
        }
        // Terrain plat : roche, deux couches de terre, herbe
        let b = &self.blocks;
        let height = (MAX_Y - MIN_Y + 1) as usize;
        let mut blocks = vec![b.air; 16 * 16 * height];
        if self.kind == Kind::Void {
            return Column::from_blocks(&blocks, b.air, Biome::Plains);
        }
        let at = |x: usize, z: usize, y: i32| (x * 16 + z) * height + (y - MIN_Y) as usize;
        let named = |n: &str, or: u32| b.named.get(n).copied().unwrap_or(or);
        if self.kind == Kind::Nether {
            let n = |name: &str| named(name, b.stone);
            let (rack, glow, quartz, gold, debris, magma, gravel) = (n("netherrack"), n("glowstone"), n("quartz_ore"), n("nether_gold_ore"), n("ancient_debris"), n("magma"), b.gravel);
            let (soul_sand, soul_soil, basalt, blackstone, brick) = (n("soul_sand"), n("soul_soil"), n("basalt"), n("blackstone"), n("nether_brick"));
            let shape = Perlin::new(self.seed ^ 0x4e45);
            let patch = Perlin::new(self.seed ^ 0x534f);
            // La forme de la caverne, avec deux colonnes de marge : les grands champignons
            // débordent sur les colonnes voisines, il faut connaître leur sol
            const M: i32 = 2;
            const W: usize = 20;
            let mut solid = vec![false; W * W * 128];
            let cell = |x: i32, z: i32, y: usize| ((x + M) as usize * W + (z + M) as usize) * 128 + y;
            for x in -M..16 + M {
                for z in -M..16 + M {
                    let (wx, wz) = ((cx * 16 + x) as f64, (cz * 16 + z) as f64);
                    for y in 0..128usize {
                        let yf = y as f64;
                        // Plein près du sol et du plafond, creusé de grandes salles entre les deux
                        let edge = ((20.0 - yf) / 20.0).max(0.0) + ((yf - 104.0) / 20.0).max(0.0);
                        let density = shape.noise3(wx / 56.0, yf / 30.0, wz / 56.0) + 0.45 * shape.noise3(wx / 21.0, yf / 14.0, wz / 21.0) + edge * 1.6 - 0.05;
                        solid[cell(x, z, y)] = density > 0.0 || y == 0 || y == 127;
                    }
                }
            }
            // Le premier sol au-dessus de la lave, avec au moins trois blocs d'air dessus
            let floor_of = |x: i32, z: i32| (32..110usize).find(|y| solid[cell(x, z, *y)] && !solid[cell(x, z, *y + 1)] && !solid[cell(x, z, *y + 2)] && !solid[cell(x, z, *y + 3)]);
            let roll_at = |wx: i32, wz: i32, salt: u64| mix(self.seed, wx as i64, wz as i64, salt);
            let mut chests = Vec::new();
            for x in 0..16usize {
                for z in 0..16usize {
                    let (bx, bz) = (cx * 16 + x as i32, cz * 16 + z as i32);
                    let (wx, wz) = (bx as f64, bz as f64);
                    let biome = self.nether_biome(bx, bz);
                    let sandy = patch.noise2(wx / 40.0, wz / 40.0) > 0.35;
                    let gravelly = patch.noise2(wx / 30.0 - 90.0, wz / 30.0) > 0.45;
                    let lit = patch.noise2(wx / 9.0 + 300.0, wz / 9.0) > 0.55;
                    let here = |y: usize| solid[cell(x as i32, z as i32, y)];
                    for y in 0..128usize {
                        let roll = mix(self.seed, bx as i64, bz as i64, y as u64) % 1000;
                        let floor = here(y) && y < 127 && !here(y + 1);
                        let under = here(y) && y < 126 && !floor && !here(y + 2);
                        let block = if y == 0 || y == 127 || ((y < 4 || y > 123) && roll % 3 == 0) {
                            b.bedrock
                        } else if !here(y) {
                            // (mer de lave au fond ; grappes de pierre lumineuse sous les plafonds)
                            if y <= 31 {
                                b.lava
                            } else if lit && y > 45 && here(y + 1) {
                                glow
                            } else {
                                b.air
                            }
                        } else if floor && (27..=36).contains(&y) && roll < 300 {
                            magma
                        } else if (floor || under) && y > 31 {
                            // Le sol de chaque biome
                            match biome {
                                NETHER_CRIMSON if floor => n("crimson_nylium"),
                                NETHER_WARPED if floor => n("warped_nylium"),
                                NETHER_SOUL_VALLEY => {
                                    if roll % 2 == 0 {
                                        soul_sand
                                    } else {
                                        soul_soil
                                    }
                                }
                                NETHER_DELTAS => match roll % 10 {
                                    0 => magma,
                                    1..=4 => blackstone,
                                    _ => basalt,
                                },
                                NETHER_WASTES if sandy && y < 45 => soul_sand,
                                NETHER_WASTES if gravelly && y < 70 => gravel,
                                _ => rack,
                            }
                        } else if (8..=22).contains(&y) && roll == 999 {
                            debris
                        } else if roll < 9 {
                            quartz
                        } else if roll < 12 {
                            gold
                        } else if biome == NETHER_DELTAS && roll % 7 == 0 {
                            blackstone
                        } else {
                            rack
                        };
                        blocks[at(x, z, y as i32)] = block;
                    }
                    // Ce qui pousse sur le sol
                    if let Some(f) = floor_of(x as i32, z as i32) {
                        let roll = roll_at(bx, bz, 7) % 100;
                        let plant = match biome {
                            NETHER_CRIMSON => match roll {
                                0..=11 => Some(n("crimson_roots")),
                                12..=14 => Some(n("crimson_fungus")),
                                _ => None,
                            },
                            NETHER_WARPED => match roll {
                                0..=9 => Some(n("warped_roots")),
                                10..=15 => Some(n("nether_sprouts")),
                                16..=18 => Some(n("warped_fungus")),
                                _ => None,
                            },
                            NETHER_WASTES => match roll {
                                0 => Some(n("brown_mushroom")),
                                1 => Some(n("red_mushroom")),
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(plant) = plant {
                            blocks[at(x, z, f as i32 + 1)] = plant;
                        }
                        // Vallée des âmes : des colonnes de basalte, du sol au plafond
                        if biome == NETHER_SOUL_VALLEY && roll_at(bx >> 1, bz >> 1, 11) % 70 == 0 {
                            let mut y = f + 1;
                            while y < 126 && !here(y) {
                                blocks[at(x, z, y as i32)] = basalt;
                                y += 1;
                            }
                        }
                    }
                    // Les grands champignons des forêts : un pied, un chapeau de trois couches
                    for tx in x as i32 - M..=x as i32 + M {
                        for tz in z as i32 - M..=z as i32 + M {
                            let (wtx, wtz) = (cx * 16 + tx, cz * 16 + tz);
                            let kind = self.nether_biome(wtx, wtz);
                            if !matches!(kind, NETHER_CRIMSON | NETHER_WARPED) || roll_at(wtx, wtz, 21) % 42 != 0 {
                                continue;
                            }
                            let Some(f) = floor_of(tx, tz) else { continue };
                            let room = (f + 1..126).take_while(|y| !solid[cell(tx, tz, *y)]).count();
                            let tall = (4 + roll_at(wtx, wtz, 22) % 6) as usize;
                            if room < tall + 4 {
                                continue;
                            }
                            let (stem, cap) = if kind == NETHER_CRIMSON { (n("crimson_stem"), n("nether_wart_block")) } else { (n("warped_stem"), n("warped_wart_block")) };
                            let (dx, dz) = ((tx - x as i32).abs(), (tz - z as i32).abs());
                            let top = f + tall;
                            if dx == 0 && dz == 0 {
                                for y in f + 1..=top {
                                    blocks[at(x, z, y as i32)] = stem;
                                }
                                blocks[at(x, z, top as i32 + 1)] = cap;
                                continue;
                            }
                            // (chapeau : large de 5 en bas, de 3 au sommet, coins arrondis)
                            for (layer, radius) in [(top - 1, 2), (top, 2), (top + 1, 1)] {
                                if dx.max(dz) <= radius && !(dx == radius && dz == radius && radius == 2) && (layer != top - 1 || dx.max(dz) == 2) {
                                    let slot = at(x, z, layer as i32);
                                    if blocks[slot] == b.air {
                                        blocks[slot] = if roll_at(bx, bz, layer as u64 + 40) % 9 == 0 { n("shroomlight") } else { cap };
                                    }
                                }
                            }
                        }
                    }
                    // La forteresse : une salle centrale, quatre ponts, et au bout de chacun une
                    // salle (verrues du Nether, deux salles au coffre) ou la terrasse des blazes
                    if let Some((fx, fz)) = self.fortress_near(bx, bz) {
                        let (dx, dz) = (bx - fx, bz - fz);
                        if fortress_pillar(dx, dz) {
                            for y in 30..FORTRESS_Y {
                                blocks[at(x, z, y)] = brick;
                            }
                        }
                        for y in FORTRESS_Y..=FORTRESS_Y + 6 {
                            let Some(piece) = fortress_piece(dx, y, dz) else { continue };
                            let slot = at(x, z, y);
                            match piece {
                                Piece::Brick => blocks[slot] = brick,
                                Piece::Air => blocks[slot] = b.air,
                                Piece::Fence => blocks[slot] = n("nether_brick_fence"),
                                Piece::Soul => blocks[slot] = soul_sand,
                                Piece::Wart => blocks[slot] = if roll_at(bx, bz, 51) % 3 == 0 { n("nether_wart[age=3]") } else { n("nether_wart") },
                                // (le toit d'un pont : seulement là où il traverse la roche)
                                Piece::Roof => {
                                    if here(y as usize) {
                                        blocks[slot] = brick;
                                    }
                                }
                                Piece::Chest(east) => {
                                    blocks[slot] = n(if east { "chest[minecraft:cardinal_direction=east]" } else { "chest[minecraft:cardinal_direction=south]" });
                                    chests.push((x as u8, y as i16, z as u8, super::structures::Loot::NetherFortress));
                                }
                            }
                        }
                    }
                }
            }
            let mut column = Column::from_blocks(&blocks, b.air, Biome::Desert);
            column.chests = chests;
            column.biome_ids = Some(vec![self.nether_biome(cx * 16 + 8, cz * 16 + 8); SECTIONS * 64]);
            return column;
        }
        if self.kind == Kind::End {
            let (stone, obsidian) = (named("end_stone", b.stone), named("obsidian", b.stone));
            let shape = Perlin::new(self.seed ^ 0x454e);
            for x in 0..16 {
                for z in 0..16 {
                    let (bx, bz) = (cx * 16 + x as i32, cz * 16 + z as i32);
                    let (wx, wz) = (bx as f64, bz as f64);
                    let far = (wx * wx + wz * wz).sqrt();
                    let bump = shape.noise2(wx / 36.0, wz / 36.0);
                    // L'île centrale : un disque bombé, plus épais au milieu
                    let (top, thick) = if far < 130.0 {
                        (58.0 + bump * 3.0, (1.0 - (far / 130.0).powi(2)) * 42.0 + bump * 5.0)
                    } else if far > 700.0 {
                        // Les îles lointaines
                        let n = shape.noise2(wx / 95.0 + 50.0, wz / 95.0) + 0.3 * bump;
                        (62.0 + bump * 4.0, (n - 0.38) * 110.0)
                    } else {
                        (0.0, 0.0)
                    };
                    if thick > 0.5 {
                        for y in (top - thick).floor().max(1.0) as i32..=top.floor() as i32 {
                            blocks[at(x, z, y)] = stone;
                        }
                    }
                    // Sur les îles lointaines : des plantes de chorus
                    if far > 700.0 && thick > 6.0 && mix(self.seed, bx as i64, bz as i64, 31) % 55 == 0 {
                        let tall = 2 + (mix(self.seed, bx as i64, bz as i64, 32) % 4) as i32;
                        let base = top.floor() as i32 + 1;
                        for y in base..base + tall {
                            blocks[at(x, z, y)] = named("chorus_plant", stone);
                        }
                        blocks[at(x, z, base + tall)] = named("chorus_flower", stone);
                    }
                    // Au centre : la fontaine de roche mère (le portail de retour, éteint)
                    if far < 3.6 {
                        let base = (58.0 + shape.noise2(0.0, 0.0) * 3.0).floor() as i32 + 1;
                        let ring = far > 2.5;
                        for y in (top.floor() as i32 + 1).min(base)..base {
                            blocks[at(x, z, y)] = b.bedrock;
                        }
                        blocks[at(x, z, base - 1)] = b.bedrock;
                        if ring {
                            blocks[at(x, z, base)] = b.bedrock;
                        }
                        if bx == 0 && bz == 0 {
                            for y in base..base + 4 {
                                blocks[at(x, z, y)] = b.bedrock;
                            }
                        }
                    }
                    // Les dix piliers d'obsidienne, en cercle autour du centre
                    for i in 0..10 {
                        let angle = i as f64 * std::f64::consts::TAU / 10.0;
                        let (px, pz) = ((42.0 * angle.cos()).round(), (42.0 * angle.sin()).round());
                        let radius = 2.0 + (i % 3) as f64;
                        if (wx - px).powi(2) + (wz - pz).powi(2) <= radius * radius + 0.5 {
                            for y in 40..=(76 + 3 * i) {
                                blocks[at(x, z, y)] = obsidian;
                            }
                        }
                    }
                }
            }
            let mut column = Column::from_blocks(&blocks, b.air, Biome::Desert);
            column.biome_ids = Some(vec![END_BIOME; SECTIONS * 64]);
            return column;
        }
        for x in 0..16 {
            for z in 0..16 {
                blocks[at(x, z, -64)] = b.stone;
                blocks[at(x, z, -63)] = b.dirt;
                blocks[at(x, z, -62)] = b.dirt;
                blocks[at(x, z, -61)] = b.grass;
            }
        }
        Column::from_blocks(&blocks, b.air, Biome::Plains)
    }
}

/// Graine écrite dans `server.properties` : un nombre, ou un texte (comme dans le jeu, le
/// texte donne toujours la même graine). Vide : graine au hasard.
pub fn parse_seed(text: &str) -> u64 {
    let text = text.trim();
    if text.is_empty() {
        let mut buf = [0u8; 8];
        if crate::crypto::os_random(&mut buf) {
            return u64::from_le_bytes(buf);
        }
        return std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    }
    if let Ok(n) = text.parse::<i64>() {
        return n as u64;
    }
    // Texte : même calcul que Java (hashCode), pour retrouver les graines connues
    let mut h: i32 = 0;
    for c in text.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as i32);
    }
    h as i64 as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generator(kind: Kind, seed: u64) -> Generator {
        Generator::new(kind, seed, Palette::new(&|_| None))
    }

    #[test]
    fn every_other_name_is_the_game_terrain() {
        for name in ["vanilla", "normal", "default", "", "Basalt", "legacy"] {
            assert_eq!(Kind::parse(name), Kind::Vanilla, "{}", name);
            assert_eq!(Kind::from_config(name), Kind::Vanilla, "{}", name);
        }
        assert_eq!((Kind::parse("flat"), Kind::parse("plugin:iles")), (Kind::Flat, Kind::Plugin));
        // (un monde enregistré sous l'ancien nom est reconnu, pour prévenir)
        assert!(Kind::was_legacy("normal") && !Kind::was_legacy("vanilla"));
    }

    #[test]
    fn flat_terrain_is_unchanged() {
        let g = generator(Kind::Flat, 0);
        let c = g.generate(10, -3);
        let b = &g.blocks;
        assert_eq!((c.get(0, -64, 0), c.get(3, -63, 4), c.get(3, -62, 4), c.get(15, -61, 15), c.get(1, -60, 1)), (b.stone, b.dirt, b.dirt, b.grass, b.air));
    }

    #[test]
    fn text_seeds_match_java() {
        // "Hello".hashCode() en Java
        assert_eq!(parse_seed("Hello"), 69609650u64);
        assert_eq!(parse_seed("-5"), (-5i64) as u64);
    }
}

#[cfg(test)]
mod dimension_tests {
    use super::*;

    fn count(c: &Column, state: u32, ys: std::ops::RangeInclusive<i32>) -> usize {
        let mut n = 0;
        for x in 0..16 {
            for z in 0..16 {
                for y in ys.clone() {
                    n += (c.get(x, y, z) == state) as usize;
                }
            }
        }
        n
    }

    #[test]
    fn the_nether_is_a_cave_between_two_bedrock_layers() {
        let g = Generator::new(Kind::Nether, 7, Palette::new(&|_| None));
        let b = &g.blocks;
        let rack = b.named["netherrack"];
        let (mut air, mut solid, mut lava) = (0, 0, 0);
        for cx in -2..2 {
            let c = g.generate(cx, 1);
            assert_eq!(count(&c, b.bedrock, 0..=0), 256, "sol de roche mère");
            assert_eq!(count(&c, b.bedrock, 127..=127), 256, "plafond de roche mère");
            assert_eq!(count(&c, b.air, 128..=200), 256 * 73, "rien au-dessus du plafond");
            air += count(&c, b.air, 32..=110);
            solid += count(&c, rack, 1..=126);
            lava += count(&c, b.lava, 1..=31);
        }
        // de grandes salles, mais pas le vide : entre un cinquième et les quatre cinquièmes d'air
        let total = 4 * 256 * 79;
        assert!(air > total / 5 && air < total * 4 / 5, "{} cases d'air sur {}", air, total);
        assert!(solid > 20_000 && lava > 0, "{} netherrack, {} lave", solid, lava);
        assert_eq!(Kind::parse("nether"), Kind::Nether);
        assert_eq!(Kind::parse(Kind::End.name()), Kind::End);
    }

    #[test]
    fn the_end_is_an_island_in_the_void() {
        let g = Generator::new(Kind::End, 7, Palette::new(&|_| None));
        let b = &g.blocks;
        let (stone, obsidian) = (b.named["end_stone"], b.named["obsidian"]);
        let centre = g.generate(0, 0);
        // sous les pieds au centre : de la pierre de l'End, épaisse, et le vide dessous
        assert!((50..=64).any(|y| centre.get(8, y, 8) == stone));
        assert!(count(&centre, stone, 1..=70) > 256 * 25);
        assert_eq!(count(&centre, b.air, -64..=0), 256 * 65);
        // un pilier d'obsidienne à 42 blocs du centre, qui dépasse l'île
        let pillar = g.generate(2, 0);
        assert_eq!(pillar.get(42 - 32, 75, 0), obsidian);
        // entre l'île et les îles lointaines : rien
        let gap = g.generate(20, 20);
        assert_eq!(count(&gap, b.air, -64..=319), 256 * 384);
    }
}

#[cfg(test)]
mod dimension_block_tests {
    /// Tous les blocs du Nether et de l'End existent sous ce nom dans les données du jeu, ainsi
    /// que les états dont les portails ont besoin.
    #[test]
    fn nether_and_end_blocks_exist_in_the_game_data() {
        let data = crate::data::GameData::load(std::path::Path::new("data")).unwrap();
        for name in [
            "netherrack", "soul_sand", "soul_soil", "glowstone", "quartz_ore", "nether_gold_ore", "ancient_debris", "basalt", "blackstone", "nether_brick",
            "crimson_nylium", "warped_nylium", "crimson_stem", "warped_stem", "nether_wart_block", "warped_wart_block", "shroomlight", "crimson_roots",
            "warped_roots", "crimson_fungus", "warped_fungus", "nether_sprouts", "end_stone", "obsidian", "chorus_plant", "chorus_flower", "magma", "nether_brick_fence", "nether_wart", "nether_wart[age=3]",
            "portal[portal_axis=x]", "portal[portal_axis=z]", "end_portal", "end_portal_frame",
        ] {
            assert!(data.state_from_spec(&format!("minecraft:{}", name)).is_some(), "bloc inconnu des données du jeu : {}", name);
        }
        let frame = data.state_from_spec("minecraft:end_portal_frame").unwrap();
        let eyed = data.with_props(frame, &[("end_portal_eye_bit", crate::data::Prop::Int(1))]).expect("cadre avec son œil");
        assert_ne!(frame, eyed);
        assert_eq!(data.prop(eyed, "end_portal_eye_bit"), Some(crate::data::Prop::Int(1)));
        assert_eq!(data.block_name(data.state_from_spec("minecraft:portal[portal_axis=z]").unwrap()), Some("minecraft:portal"));
    }
}

#[cfg(test)]
mod fortress_tests {
    use super::*;

    /// Une forteresse entière : salle centrale avec ses quatre portes, ponts bordés de
    /// barrières, jardin de verrues, deux coffres avec leur butin, terrasse, piliers.
    #[test]
    fn a_fortress_has_its_rooms_bridges_and_chests() {
        let g = Generator::new(Kind::Nether, 11, Palette::new(&|_| None));
        let b = &g.blocks;
        // (une case de la grille qui a une forteresse)
        let (fx, fz) = (0..40).find_map(|i| g.fortress_near(i * 400 + 200, 200)).expect("aucune forteresse sur quarante cases");
        let block = |x: i32, y: i32, z: i32| g.generate(x.div_euclid(16), z.div_euclid(16)).get(x.rem_euclid(16) as usize, y, z.rem_euclid(16) as usize);
        let (brick, fence) = (b.named["nether_brick"], b.named["nether_brick_fence"]);
        let y = FORTRESS_Y;
        // Salle centrale : sol, toit, mur plein au coin, porte au milieu de chaque mur
        assert_eq!((block(fx, y, fz), block(fx, y + 6, fz), block(fx, y + 1, fz)), (brick, brick, b.air));
        assert_eq!((block(fx + 6, y + 2, fz + 4), block(fx + 6, y + 2, fz), block(fx, y + 2, fz - 6)), (brick, b.air, b.air));
        // Pont vers l'est : sol, barrières, passage libre, pilier jusqu'à la lave
        assert_eq!((block(fx + 30, y, fz), block(fx + 30, y + 1, fz + 2), block(fx + 30, y + 1, fz - 2), block(fx + 30, y + 2, fz)), (brick, fence, fence, b.air));
        assert_eq!((block(fx + 36, y - 20, fz), block(fx + 36, 30, fz)), (brick, brick));
        // Jardin de verrues au bout du pont de l'est
        let (wx, wz) = (fx + WART_ROOM.0, fz + WART_ROOM.1);
        assert_eq!(block(wx, y, wz), b.named["soul_sand"]);
        assert!([b.named["nether_wart"], b.named["nether_wart[age=3]"]].contains(&block(wx, y + 1, wz)));
        assert_eq!((block(wx - 4, y + 1, wz), block(wx + 4, y + 1, wz), block(wx, y + 5, wz)), (b.air, brick, brick));
        // Les deux coffres, déclarés avec le butin des forteresses
        for (i, (cx, cz)) in CHEST_ROOMS.iter().enumerate() {
            let (x, z) = if i == 0 { (fx + cx - 3, fz + cz) } else { (fx + cx, fz + cz - 3) };
            let column = g.generate(x.div_euclid(16), z.div_euclid(16));
            let key = (x.rem_euclid(16) as u8, (y + 1) as i16, z.rem_euclid(16) as u8);
            assert_eq!(column.chests.iter().find(|c| (c.0, c.1, c.2) == key).map(|c| c.3), Some(crate::world::structures::Loot::NetherFortress), "coffre {}", i);
            assert_ne!(column.get(key.0 as usize, y + 1, key.2 as usize), b.air);
        }
        // Terrasse des blazes : à ciel ouvert, bordée de barrières, ouverte sur le pont
        let (tx, tz) = (fx + TERRACE.0, fz + TERRACE.1);
        assert_eq!((block(tx, y, tz), block(tx + 4, y + 1, tz), block(tx, y + 1, tz - 4), block(tx, y + 3, tz)), (brick, fence, b.air, b.air));
        // Les créatures de la forteresse apparaissent là où l'on peut se tenir
        assert!(g.in_fortress(fx + 30, y + 1, fz) && g.in_fortress(tx, y + 1, tz) && g.in_fortress(fx, y + 1, fz));
        assert!(!g.in_fortress(fx + 30, y, fz) && !g.in_fortress(fx + 30, y + 1, fz + 9));
    }
}
