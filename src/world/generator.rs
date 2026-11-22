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

    /// Plusieurs octaves superposées, environ -1..1.
    fn fbm2(&self, x: f64, z: f64, octaves: u32) -> f64 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for _ in 0..octaves {
            sum += self.noise2(x * freq, z * freq) * amp;
            norm += amp;
            amp *= 0.5;
            freq *= 2.0;
        }
        sum / norm
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
    "mycelium", "red_sand", "hardened_clay", "orange_terracotta", "yellow_terracotta", "white_terracotta", "brown_terracotta",
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
    "fire_coral_fan", "horn_coral_fan", "magma", "bee_nest", "vine",
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
    pub(crate) fn from_blocks(blocks: &[u32], air: u32, biome: Biome) -> Column {
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
    /// Premier terrain de Basalt (mondes créés avec lui : gardé pour ne pas les changer)
    Normal,
    /// Terrain à la manière du jeu depuis la 1.18 (bruits de climat, biomes, grottes)
    Vanilla,
    /// Générateur fourni par un plugin (`level-type=plugin:<nom>`)
    Plugin,
}

impl Kind {
    /// Nom enregistré dans `level.nbt`.
    pub fn parse(text: &str) -> Kind {
        match text.trim().to_ascii_lowercase().as_str() {
            "flat" | "plat" => Kind::Flat,
            "normal" => Kind::Normal,
            t if t.starts_with("plugin:") => Kind::Plugin,
            _ => Kind::Vanilla,
        }
    }
    /// Réglage `level-type` de `server.properties`, pour un nouveau monde.
    pub fn from_config(text: &str) -> Kind {
        match text.trim().to_ascii_lowercase().as_str() {
            "flat" | "plat" => Kind::Flat,
            "basalt" | "legacy" => Kind::Normal,
            t if t.starts_with("plugin:") => Kind::Plugin,
            _ => Kind::Vanilla,
        }
    }
    /// Nom de la sorte de terrain dans server.properties.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Flat => "flat",
            Kind::Normal => "normal",
            Kind::Vanilla => "vanilla",
            Kind::Plugin => "plugin",
        }
    }
}

pub struct Generator {
    pub kind: Kind,
    pub seed: u64,
    pub blocks: Palette,
    height: Perlin,
    hills: Perlin,
    ridge: Perlin,
    temperature: Perlin,
    humidity: Perlin,
    caves: Perlin,
    tunnels_a: Perlin,
    tunnels_b: Perlin,
    detail: Perlin,
    vanilla: Option<super::vanilla::Vanilla>,
    /// Générateur d'un plugin, et son nom (gardé même s'il n'a pas pu être chargé)
    plugin: Option<std::sync::Arc<crate::plugin::generator::LuaGenerator>>,
    plugin_name: String,
}

/// Arbre à poser : position du pied du tronc et essence.
#[derive(Clone, Copy)]
enum Tree {
    Oak,
    Birch,
    Spruce,
}

impl Generator {
    /// Le générateur vanilla, pour un monde de ce type.
    pub fn vanilla(&self) -> Option<&super::vanilla::Vanilla> {
        self.vanilla.as_ref()
    }

    /// Générateur de cette sorte, avec une graine.
    pub fn new(kind: Kind, seed: u64, blocks: Palette) -> Generator {
        let p = |salt: u64| Perlin::new(mix(seed, 0, 0, salt));
        Generator {
            kind,
            seed,
            blocks,
            height: p(1),
            hills: p(2),
            ridge: p(3),
            temperature: p(4),
            humidity: p(5),
            caves: p(6),
            tunnels_a: p(7),
            tunnels_b: p(8),
            detail: p(9),
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

    /// Hauteur du sol (dernier bloc plein) d'une colonne, avant grottes et arbres.
    pub fn surface(&self, x: i32, z: i32) -> i32 {
        if self.kind == Kind::Flat {
            return -61;
        }
        if let Some(v) = &self.vanilla {
            return v.surface(x, z).0;
        }
        let (fx, fz) = (x as f64, z as f64);
        let continent = self.height.fbm2(fx / 512.0, fz / 512.0, 5);
        let hills = self.hills.fbm2(fx / 128.0, fz / 128.0, 4);
        let ridge = 1.0 - self.ridge.fbm2(fx / 256.0, fz / 256.0, 4).abs() * 2.0;
        // Les montagnes ne poussent que sur les terres hautes
        let mountain = ((continent - 0.15) * 4.0).clamp(0.0, 1.0) * ridge.max(0.0).powf(1.5);
        let h = SEA_LEVEL as f64 + 2.0 + continent * 28.0 + hills * 7.0 * (0.4 + continent.max(0.0)) + mountain * 70.0;
        (h.round() as i32).clamp(MIN_Y + 8, MAX_Y - 40)
    }

    /// Climat simple (température, humidité) des terrains non vanilla.
    fn climate(&self, x: i32, z: i32) -> (f64, f64) {
        let t = self.temperature.fbm2(x as f64 / 700.0, z as f64 / 700.0, 3);
        let h = self.humidity.fbm2(x as f64 / 500.0, z as f64 / 500.0, 3);
        (t, h)
    }

    /// Biome d'une colonne.
    pub fn biome(&self, x: i32, z: i32) -> Biome {
        if self.kind == Kind::Flat {
            return Biome::Plains;
        }
        let height = self.surface(x, z);
        let (t, h) = self.climate(x, z);
        if height < SEA_LEVEL - 1 {
            return if t < -0.45 { Biome::FrozenOcean } else { Biome::Ocean };
        }
        if height <= SEA_LEVEL + 1 && t > -0.45 {
            return Biome::Beach;
        }
        if height > SEA_LEVEL + 45 {
            return Biome::Mountains;
        }
        if t < -0.4 {
            Biome::SnowyPlains
        } else if t > 0.35 && h < 0.1 {
            Biome::Desert
        } else if h > 0.25 {
            Biome::BirchForest
        } else if h > 0.0 {
            Biome::Forest
        } else {
            Biome::Plains
        }
    }

    /// Point d'apparition : la première terre ferme en partant de (0, 0) et en tournant
    /// autour. Renvoie (x, hauteur du sol, z).
    pub fn spawn(&self) -> (i32, i32, i32) {
        if self.kind == Kind::Plugin {
            return self.plugin.as_ref().and_then(|p| p.spawn()).unwrap_or((8, 64, 8));
        }
        if let Some(v) = &self.vanilla {
            return v.spawn();
        }
        for r in 0..64i32 {
            for (dx, dz) in ring(r) {
                let (x, z) = (dx * 8, dz * 8);
                let y = self.surface(x, z);
                if y >= SEA_LEVEL && !matches!(self.biome(x, z), Biome::Mountains) {
                    return (x, y, z);
                }
            }
        }
        (0, self.surface(0, 0).max(SEA_LEVEL), 0)
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
        let b = &self.blocks;
        let height = (MAX_Y - MIN_Y + 1) as usize;
        let mut blocks = vec![b.air; 16 * 16 * height];
        let at = |x: usize, z: usize, y: i32| (x * 16 + z) * height + (y - MIN_Y) as usize;
        if self.kind == Kind::Flat {
            for x in 0..16 {
                for z in 0..16 {
                    blocks[at(x, z, -64)] = b.stone;
                    blocks[at(x, z, -63)] = b.dirt;
                    blocks[at(x, z, -62)] = b.dirt;
                    blocks[at(x, z, -61)] = b.grass;
                }
            }
            return Column::from_blocks(&blocks, b.air, Biome::Plains);
        }
        let (bx, bz) = (cx * 16, cz * 16);
        let mut rng = Rng::new(mix(self.seed, cx as i64, cz as i64, 100));

        // 1. Relief et couches de surface
        let mut heights = [[0i32; 16]; 16];
        let mut biomes = [[Biome::Plains; 16]; 16];
        for x in 0..16 {
            for z in 0..16 {
                let (wx, wz) = (bx + x as i32, bz + z as i32);
                let top = self.surface(wx, wz);
                let biome = self.biome(wx, wz);
                heights[x][z] = top;
                biomes[x][z] = biome;
                let depth = 3 + (self.detail.noise2(wx as f64 / 8.0, wz as f64 / 8.0) * 1.5).round() as i32;
                for y in MIN_Y..=top {
                    let block = if y == MIN_Y || (y < MIN_Y + 5 && rng.below(5) as i32 >= y - MIN_Y) {
                        b.bedrock
                    } else if y < top - depth {
                        if y < 0 || (y < 8 && rng.below(8) as i32 > y) {
                            b.deepslate
                        } else {
                            b.stone
                        }
                    } else {
                        let under_water = top < SEA_LEVEL;
                        match biome {
                            Biome::Desert => {
                                if y >= top - 3 {
                                    b.sand
                                } else {
                                    b.sandstone
                                }
                            }
                            Biome::Beach => {
                                if y >= top - 3 {
                                    b.sand
                                } else {
                                    b.sandstone
                                }
                            }
                            Biome::Ocean | Biome::FrozenOcean => {
                                if top < SEA_LEVEL - 12 {
                                    b.gravel
                                } else if y == top && rng.below(12) == 0 {
                                    b.clay
                                } else {
                                    b.sand
                                }
                            }
                            Biome::Mountains if top > SEA_LEVEL + 60 => b.stone,
                            _ => {
                                if y == top && !under_water {
                                    b.grass
                                } else {
                                    b.dirt
                                }
                            }
                        }
                    };
                    blocks[at(x, z, y)] = block;
                }
                // Mer, glace en eau froide
                for y in (top + 1)..=SEA_LEVEL {
                    blocks[at(x, z, y)] = if y == SEA_LEVEL && biome == Biome::FrozenOcean { b.ice } else { b.water };
                }
                // Neige sur les sommets et les plaines froides
                if top >= SEA_LEVEL && (biome == Biome::SnowyPlains || (biome == Biome::Mountains && top > SEA_LEVEL + 55)) {
                    if top > SEA_LEVEL + 85 {
                        blocks[at(x, z, top)] = b.snow;
                    }
                    blocks[at(x, z, top + 1)] = b.snow_layer;
                }
            }
        }

        // 2. Poches de granite, diorite, andésite et tuf dans la pierre
        for (block, count, low, high) in [(b.granite, 3, 0, 80), (b.diorite, 3, 0, 80), (b.andesite, 3, 0, 80), (b.tuff, 2, -60, 0)] {
            for _ in 0..count {
                let (x, y, z) = (rng.below(16) as i32, low + rng.below((high - low) as u32) as i32, rng.below(16) as i32);
                self.blob(&mut blocks, at, (x, y, z), 2.5 + rng.unit() * 1.5, &[b.stone, b.deepslate], block);
            }
        }

        // 3. Minerais : (minerai, essais, hauteur min, hauteur max, taille)
        let ores = [(b.coal, 18, 0, 128, 1.6), (b.iron, 12, -24, 72, 1.3), (b.copper, 8, -16, 96, 1.4), (b.gold, 4, -64, 32, 1.1), (b.redstone, 6, -64, 15, 1.2), (b.lapis, 2, -32, 32, 1.1), (b.diamond, 2, -64, 16, 1.0)];
        for (ore, tries, low, high, size) in ores {
            for _ in 0..tries {
                let (x, y, z) = (rng.below(16) as i32, low + rng.below((high - low) as u32) as i32, rng.below(16) as i32);
                if y > heights[x as usize][z as usize] - 4 {
                    continue;
                }
                let block = if y < 0 { ore.1 } else { ore.0 };
                self.blob(&mut blocks, at, (x, y, z), size + rng.unit(), &[b.stone, b.deepslate, b.granite, b.diorite, b.andesite, b.tuff], block);
            }
        }

        // 4. Grottes : grandes cavités (« fromage ») et galeries (« spaghettis »), calculées
        // sur une grille de 4 blocs puis interpolées
        self.carve(&mut blocks, at, bx, bz, &heights);

        // 5. Arbres, cactus et plantes (y compris ceux des chunks voisins qui débordent ici)
        for ncx in (cx - 1)..=(cx + 1) {
            for ncz in (cz - 1)..=(cz + 1) {
                for (wx, wz, tree) in self.trees(ncx, ncz) {
                    self.place_tree(&mut blocks, at, bx, bz, wx, wz, tree);
                }
            }
        }
        let mut plants = Rng::new(mix(self.seed, cx as i64, cz as i64, 200));
        for x in 0..16 {
            for z in 0..16 {
                let top = heights[x][z];
                if top < SEA_LEVEL || top + 1 > MAX_Y {
                    continue;
                }
                let ground = blocks[at(x, z, top)];
                let above = blocks[at(x, z, top + 1)];
                if above != b.air {
                    continue;
                }
                let roll = plants.below(100);
                match biomes[x][z] {
                    Biome::Desert if ground == b.sand && roll < 1 => {
                        let tall = 1 + plants.below(3) as i32;
                        for dy in 1..=tall {
                            blocks[at(x, z, top + dy)] = b.cactus;
                        }
                    }
                    Biome::Plains | Biome::Forest | Biome::BirchForest if ground == b.grass => {
                        if roll < 12 {
                            blocks[at(x, z, top + 1)] = b.short_grass;
                        } else if roll < 14 {
                            blocks[at(x, z, top + 1)] = if plants.below(2) == 0 { b.dandelion } else { b.poppy };
                        }
                    }
                    _ => {}
                }
            }
        }

        // Biome du chunk : celui de son centre
        Column::from_blocks(&blocks, b.air, biomes[8][8])
    }

    /// Boule irrégulière d'un bloc, qui ne remplace que les blocs de `into`, sans sortir du chunk.
    fn blob(&self, blocks: &mut [u32], at: impl Fn(usize, usize, i32) -> usize, center: (i32, i32, i32), radius: f64, into: &[u32], block: u32) {
        let r = radius.ceil() as i32;
        for dx in -r..=r {
            for dy in -r..=r {
                for dz in -r..=r {
                    let (x, y, z) = (center.0 + dx, center.1 + dy, center.2 + dz);
                    if !(0..16).contains(&x) || !(0..16).contains(&z) || y <= MIN_Y || y > MAX_Y {
                        continue;
                    }
                    if ((dx * dx + dy * dy + dz * dz) as f64) > radius * radius {
                        continue;
                    }
                    let i = at(x as usize, z as usize, y);
                    if into.contains(&blocks[i]) {
                        blocks[i] = block;
                    }
                }
            }
        }
    }

    /// Grottes : creuse le chunk sous la surface.
    fn carve(&self, blocks: &mut [u32], at: impl Fn(usize, usize, i32) -> usize, bx: i32, bz: i32, heights: &[[i32; 16]; 16]) {
        let b = &self.blocks;
        let top = heights.iter().flatten().copied().max().unwrap_or(0).min(MAX_Y - 8);
        // Grille : 5 × 5 points horizontaux, un point tous les 4 blocs en hauteur
        let ny = ((top - MIN_Y) / 4 + 2) as usize;
        let mut grid = vec![0f64; 5 * 5 * ny];
        for gx in 0..5 {
            for gz in 0..5 {
                for gy in 0..ny {
                    let (x, y, z) = ((bx + gx as i32 * 4) as f64, (MIN_Y + gy as i32 * 4) as f64, (bz + gz as i32 * 4) as f64);
                    let cheese = self.caves.noise3(x / 48.0, y / 24.0, z / 48.0);
                    let a = self.tunnels_a.noise3(x / 64.0, y / 40.0, z / 64.0);
                    let c = self.tunnels_b.noise3(x / 64.0 + 100.0, y / 40.0, z / 64.0);
                    // Négatif : vide. Les galeries sont là où deux bruits passent tous deux par zéro
                    let tunnel = a * a + c * c - 0.012;
                    let open = (0.42 - cheese).min(tunnel * 8.0);
                    grid[(gx * 5 + gz) * ny + gy] = open;
                }
            }
        }
        for x in 0..16usize {
            for z in 0..16usize {
                let (gx, fx) = (x / 4, (x % 4) as f64 / 4.0);
                let (gz, fz) = (z / 4, (z % 4) as f64 / 4.0);
                let limit = heights[x][z] - 7;
                for y in (MIN_Y + 5)..=limit.min(top) {
                    let gy = ((y - MIN_Y) / 4) as usize;
                    let fy = ((y - MIN_Y) % 4) as f64 / 4.0;
                    let g = |i: usize, j: usize, k: usize| grid[((gx + i) * 5 + gz + j) * ny + gy + k];
                    let v = lerp(
                        fx,
                        lerp(fz, lerp(fy, g(0, 0, 0), g(0, 0, 1)), lerp(fy, g(0, 1, 0), g(0, 1, 1))),
                        lerp(fz, lerp(fy, g(1, 0, 0), g(1, 0, 1)), lerp(fy, g(1, 1, 0), g(1, 1, 1))),
                    );
                    if v < 0.0 {
                        let i = at(x, z, y);
                        if blocks[i] != b.bedrock && blocks[i] != b.water {
                            // Lave tout en bas, comme dans le jeu
                            blocks[i] = if y < -54 { b.lava } else { b.air };
                        }
                    }
                }
            }
        }
    }

    /// Arbres dont le tronc est dans ce chunk : (x, z du tronc, essence).
    fn trees(&self, cx: i32, cz: i32) -> Vec<(i32, i32, Tree)> {
        let mut rng = Rng::new(mix(self.seed, cx as i64, cz as i64, 300));
        let center = self.biome(cx * 16 + 8, cz * 16 + 8);
        let (count, kind) = match center {
            Biome::Forest => (5 + rng.below(4), None),
            Biome::BirchForest => (5 + rng.below(4), Some(Tree::Birch)),
            Biome::Plains => (if rng.below(3) == 0 { 1 } else { 0 }, Some(Tree::Oak)),
            Biome::SnowyPlains => (rng.below(2), Some(Tree::Spruce)),
            Biome::Mountains => (rng.below(2), Some(Tree::Spruce)),
            _ => (0, None),
        };
        let mut out = Vec::new();
        for _ in 0..count {
            let (x, z) = (cx * 16 + 2 + rng.below(12) as i32, cz * 16 + 2 + rng.below(12) as i32);
            let tree = kind.unwrap_or(if rng.below(5) == 0 { Tree::Birch } else { Tree::Oak });
            let roll = rng.next();
            // Pas d'arbre dans l'eau, sur la plage ou sur le sable
            let top = self.surface(x, z);
            if top < SEA_LEVEL || matches!(self.biome(x, z), Biome::Beach | Biome::Desert | Biome::Ocean | Biome::FrozenOcean) {
                continue;
            }
            if center == Biome::Mountains && top > SEA_LEVEL + 70 {
                continue;
            }
            let _ = roll;
            out.push((x, z, tree));
        }
        out
    }

    /// Pose un arbre dont le tronc est en (wx, wz), coupé aux limites du chunk.
    #[allow(clippy::too_many_arguments)]
    fn place_tree(&self, blocks: &mut [u32], at: impl Fn(usize, usize, i32) -> usize, bx: i32, bz: i32, wx: i32, wz: i32, tree: Tree) {
        let b = &self.blocks;
        let ground = self.surface(wx, wz);
        let mut rng = Rng::new(mix(self.seed, wx as i64, wz as i64, 400));
        let (log, leaves, trunk) = match tree {
            Tree::Oak => (b.oak_log, b.oak_leaves, 4 + rng.below(3) as i32),
            Tree::Birch => (b.birch_log, b.birch_leaves, 5 + rng.below(3) as i32),
            Tree::Spruce => (b.spruce_log, b.spruce_leaves, 6 + rng.below(4) as i32),
        };
        let mut put = |x: i32, y: i32, z: i32, block: u32, replace_only_air: bool| {
            let (lx, lz) = (x - bx, z - bz);
            if !(0..16).contains(&lx) || !(0..16).contains(&lz) || y <= MIN_Y || y > MAX_Y {
                return;
            }
            let i = at(lx as usize, lz as usize, y);
            let current = blocks[i];
            if !replace_only_air || current == b.air || current == b.snow_layer || current == b.short_grass {
                blocks[i] = block;
            }
        };
        let base = ground + 1;
        let top = base + trunk;
        match tree {
            Tree::Spruce => {
                // Cône de branches qui s'élargit vers le bas
                let mut radius: i32 = 0;
                for y in ((base + 2)..=(top + 1)).rev() {
                    for dx in -radius..=radius {
                        for dz in -radius..=radius {
                            if dx.abs() + dz.abs() <= radius + 1 && !(dx == 0 && dz == 0 && y <= top) {
                                put(wx + dx, y, wz + dz, leaves, true);
                            }
                        }
                    }
                    radius = if radius >= 2 { 1 } else { radius + 1 };
                }
            }
            _ => {
                for y in (top - 3)..=(top + 1) {
                    let r: i32 = if y >= top { 1 } else { 2 };
                    for dx in -r..=r {
                        for dz in -r..=r {
                            // Coins arrondis, un peu au hasard
                            if dx.abs() == r && dz.abs() == r && (y >= top || rng.below(2) == 0) {
                                continue;
                            }
                            put(wx + dx, y, wz + dz, leaves, true);
                        }
                    }
                }
            }
        }
        for y in base..top {
            put(wx, y, wz, log, false);
        }
        // Sous le tronc : de la terre plutôt que de l'herbe
        put(wx, ground, wz, b.dirt, false);
    }
}

/// Points d'un carré de rayon `r` autour de l'origine (son pourtour seulement).
fn ring(r: i32) -> Vec<(i32, i32)> {
    if r == 0 {
        return vec![(0, 0)];
    }
    let mut out = Vec::new();
    for i in -r..=r {
        out.push((i, -r));
        out.push((i, r));
    }
    for i in (-r + 1)..r {
        out.push((-r, i));
        out.push((r, i));
    }
    out
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
    fn the_same_seed_gives_the_same_terrain() {
        let (a, b) = (generator(Kind::Normal, 42), generator(Kind::Normal, 42));
        let (ca, cb) = (a.generate(3, -7), b.generate(3, -7));
        for y in [-64, -30, 0, 40, 62, 70, 100] {
            for (x, z) in [(0, 0), (5, 9), (15, 15)] {
                assert_eq!(ca.get(x, y, z), cb.get(x, y, z));
            }
        }
        let other = generator(Kind::Normal, 43).generate(3, -7);
        let differs = (0..16).any(|x| (MIN_Y..100).any(|y| other.get(x, y, 8) != ca.get(x, y, 8)));
        assert!(differs, "une autre graine donne un autre terrain");
    }

    #[test]
    fn terrain_looks_like_minecraft() {
        let g = generator(Kind::Normal, 12345);
        let b = g.blocks.clone();
        let mut land = 0;
        let mut water = 0;
        let mut heights = Vec::new();
        let mut ores = 0;
        let mut caves = 0;
        for cx in -4..4 {
            for cz in -4..4 {
                let col = g.generate(cx, cz);
                for x in 0..16 {
                    for z in 0..16 {
                        assert_eq!(col.get(x, MIN_Y, z), b.bedrock, "roche-mère tout en bas");
                        let top = g.surface(cx * 16 + x as i32, cz * 16 + z as i32);
                        heights.push(top);
                        if top >= SEA_LEVEL {
                            land += 1;
                        } else {
                            water += 1;
                            assert_eq!(col.get(x, SEA_LEVEL, z) == b.water || col.get(x, SEA_LEVEL, z) == b.ice, true);
                        }
                        for y in (MIN_Y + 5)..(top - 8) {
                            let block = col.get(x, y, z);
                            if [b.coal.0, b.iron.0, b.coal.1, b.iron.1, b.diamond.1, b.redstone.1, b.copper.0].contains(&block) {
                                ores += 1;
                            }
                            if block == b.air {
                                caves += 1;
                            }
                        }
                    }
                }
            }
        }
        let (min, max) = (heights.iter().min().unwrap(), heights.iter().max().unwrap());
        assert!(max - min >= 8, "du relief : de {} à {}", min, max);
        assert!(land > 0 && water + land == 64 * 256, "terres et mers : {} / {}", land, water);
        assert!(ores > 50, "des minerais : {}", ores);
        assert!(caves > 100, "des grottes : {}", caves);
    }

    #[test]
    fn spawn_is_on_dry_land() {
        for seed in [1, 2, 3, 4, 5] {
            let g = generator(Kind::Normal, seed);
            let (x, y, z) = g.spawn();
            assert!(y >= SEA_LEVEL, "graine {} : apparition en {} {} {}", seed, x, y, z);
        }
    }

    #[test]
    fn flat_terrain_is_unchanged() {
        let g = generator(Kind::Flat, 0);
        let c = g.generate(10, -3);
        let b = &g.blocks;
        assert_eq!((c.get(0, -64, 0), c.get(3, -63, 4), c.get(3, -62, 4), c.get(15, -61, 15), c.get(1, -60, 1)), (b.stone, b.dirt, b.dirt, b.grass, b.air));
    }

    #[test]
    #[ignore]
    fn generation_speed() {
        let g = generator(Kind::Normal, 99);
        let start = std::time::Instant::now();
        for cx in 0..20 {
            for cz in 0..20 {
                g.generate(cx, cz);
            }
        }
        eprintln!("400 chunks en {:?}", start.elapsed());
    }

    #[test]
    fn text_seeds_match_java() {
        // "Hello".hashCode() en Java
        assert_eq!(parse_seed("Hello"), 69609650u64);
        assert_eq!(parse_seed("-5"), (-5i64) as u64);
    }
}
