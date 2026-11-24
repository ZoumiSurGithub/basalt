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
    /// Terrain à la manière du jeu depuis la 1.18 (bruits de climat, biomes, grottes)
    Vanilla,
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
    /// Nom de la sorte de terrain dans server.properties.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Flat => "flat",
            Kind::Vanilla => "vanilla",
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
        let at = |x: usize, z: usize, y: i32| (x * 16 + z) * height + (y - MIN_Y) as usize;
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
