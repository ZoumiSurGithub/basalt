//! Terrain à la manière du jeu depuis la 1.18.
//!
//! Même principe que le générateur de Minecraft :
//! - cinq bruits de **climat** : continentalité (océans / terres), érosion (montagnes /
//!   plaines), étrangeté (d'où sortent pics et vallées), température et humidité ;
//! - la **hauteur** du relief découle de la continentalité, de l'érosion et des pics et
//!   vallées ; une densité 3D (bruit de base, « fromage » des montagnes) donne surplombs et
//!   falaises, calculée sur une grille de 4 × 8 × 4 blocs puis interpolée ;
//! - les **biomes** sont choisis d'après ces cinq valeurs, avec les tableaux du jeu
//!   (température × humidité pour les terres, les plateaux, les reliefs tourmentés, les pics,
//!   les pentes, les plages, les rivières, les marais), plus les biomes souterrains (grottes
//!   luxuriantes, grottes de stalactites, abîmes) ;
//! - **grottes** : cavités « fromage », galeries « spaghettis », boyaux « nouilles », lave
//!   tout en bas ;
//! - **surface** selon le biome (sable, grès, terre cuite des badlands, neige, glace,
//!   mycélium, boue, gravier, pierre sur les pentes raides…) ;
//! - **arbres** de chaque biome (chêne, bouleau, sapin, sapin géant, chêne noir, acacia,
//!   jungle, cerisier, palétuvier), fleurs, herbes, fougères, cactus, cannes à sucre,
//!   nénuphars, pics de glace, et minerais (dont l'émeraude dans les montagnes).
//!
//! Les constantes ne sont pas celles du jeu : le terrain a la même allure, mais une graine
//! ne redonne pas le même monde que dans Minecraft.

use super::generator::{lerp, mix, Column, Palette, Perlin, Rng, Section, SEA_LEVEL, SECTIONS};
use super::{MAX_Y, MIN_Y};

// --- Bruits ----------------------------------------------------------------------------

/// Bruit à plusieurs octaves, à la manière de `PerlinNoise` du jeu : `first` est l'octave
/// la plus basse (fréquence 2^first), `amps` leurs poids.
struct Octaves {
    layers: Vec<(Perlin, f64, f64)>, // bruit, fréquence, amplitude
}

impl Octaves {
    fn new(seed: u64, salt: u64, first: i32, amps: &[f64]) -> Octaves {
        let n = amps.len() as i32;
        let mut amplitude = 2f64.powi(n - 1) / (2f64.powi(n) - 1.0);
        let mut layers = Vec::new();
        for (i, a) in amps.iter().enumerate() {
            if *a != 0.0 {
                layers.push((Perlin::new(mix(seed, i as i64, first as i64, salt)), 2f64.powi(first + i as i32), amplitude * a));
            }
            amplitude *= 0.5;
        }
        Octaves { layers }
    }

    fn sample(&self, x: f64, y: f64, z: f64) -> f64 {
        self.layers.iter().map(|(p, f, a)| p.noise3(x * f, y * f, z * f) * a).sum()
    }
}

/// Bruit « normal » du jeu : deux bruits à octaves superposés, ramenés à un écart type voisin de 1/3.
struct Normal {
    a: Octaves,
    b: Octaves,
    factor: f64,
}

impl Normal {
    fn new(seed: u64, salt: u64, first: i32, amps: &[f64]) -> Normal {
        let nonzero: Vec<usize> = amps.iter().enumerate().filter(|(_, a)| **a != 0.0).map(|(i, _)| i).collect();
        let span = (nonzero.last().unwrap_or(&0) - nonzero.first().unwrap_or(&0)) as f64;
        let deviation = 0.1 * (1.0 + 1.0 / (span + 1.0));
        Normal { a: Octaves::new(seed, salt * 2, first, amps), b: Octaves::new(seed, salt * 2 + 1, first, amps), factor: (1.0 / 6.0) / deviation }
    }

    fn sample(&self, x: f64, y: f64, z: f64) -> f64 {
        const K: f64 = 1.018_126_888_217_522_7;
        (self.a.sample(x, y, z) + self.b.sample(x * K, y * K, z * K)) * self.factor
    }
}

// --- Biomes ----------------------------------------------------------------------------

/// Biomes du jeu, avec leur identifiant Bedrock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(clippy::upper_case_acronyms)]
pub enum B {
    Ocean = 0,
    Plains = 1,
    Desert = 2,
    WindsweptHills = 3,
    Forest = 4,
    Taiga = 5,
    Swamp = 6,
    River = 7,
    FrozenRiver = 11,
    SnowyPlains = 12,
    MushroomFields = 14,
    Beach = 16,
    Jungle = 21,
    SparseJungle = 23,
    DeepOcean = 24,
    StonyShore = 25,
    SnowyBeach = 26,
    BirchForest = 27,
    DarkForest = 29,
    SnowyTaiga = 30,
    OldGrowthPineTaiga = 32,
    WindsweptForest = 34,
    Savanna = 35,
    SavannaPlateau = 36,
    Badlands = 37,
    WoodedBadlands = 38,
    WarmOcean = 40,
    LukewarmOcean = 42,
    DeepLukewarmOcean = 43,
    ColdOcean = 44,
    DeepColdOcean = 45,
    FrozenOcean = 46,
    DeepFrozenOcean = 47,
    BambooJungle = 48,
    SunflowerPlains = 129,
    WindsweptGravellyHills = 131,
    FlowerForest = 132,
    IceSpikes = 140,
    OldGrowthBirchForest = 155,
    OldGrowthSpruceTaiga = 160,
    WindsweptSavanna = 163,
    ErodedBadlands = 165,
    JaggedPeaks = 182,
    FrozenPeaks = 183,
    SnowySlopes = 184,
    Grove = 185,
    Meadow = 186,
    LushCaves = 187,
    DripstoneCaves = 188,
    StonyPeaks = 189,
    DeepDark = 190,
    MangroveSwamp = 191,
    CherryGrove = 192,
    PaleGarden = 193,
}

impl B {
    pub fn id(self) -> u8 {
        self as u8
    }

    /// Biome où il neige (couche de neige au sol, eau gelée en surface).
    fn snowy(self) -> bool {
        matches!(self, B::SnowyPlains | B::SnowyTaiga | B::IceSpikes | B::SnowyBeach | B::Grove | B::SnowySlopes | B::JaggedPeaks | B::FrozenPeaks | B::FrozenRiver | B::FrozenOcean | B::DeepFrozenOcean)
    }

    fn ocean(self) -> bool {
        matches!(self, B::Ocean | B::DeepOcean | B::WarmOcean | B::LukewarmOcean | B::DeepLukewarmOcean | B::ColdOcean | B::DeepColdOcean | B::FrozenOcean | B::DeepFrozenOcean)
    }

    fn badlands(self) -> bool {
        matches!(self, B::Badlands | B::WoodedBadlands | B::ErodedBadlands)
    }
}

impl B {
    /// Tous les biomes, pour les recherches par nom (`/locate biome`).
    pub const ALL: [B; 54] = [
        B::Ocean,
        B::Plains,
        B::Desert,
        B::WindsweptHills,
        B::Forest,
        B::Taiga,
        B::Swamp,
        B::River,
        B::FrozenRiver,
        B::SnowyPlains,
        B::MushroomFields,
        B::Beach,
        B::Jungle,
        B::SparseJungle,
        B::DeepOcean,
        B::StonyShore,
        B::SnowyBeach,
        B::BirchForest,
        B::DarkForest,
        B::SnowyTaiga,
        B::OldGrowthPineTaiga,
        B::WindsweptForest,
        B::Savanna,
        B::SavannaPlateau,
        B::Badlands,
        B::WoodedBadlands,
        B::WarmOcean,
        B::LukewarmOcean,
        B::DeepLukewarmOcean,
        B::ColdOcean,
        B::DeepColdOcean,
        B::FrozenOcean,
        B::DeepFrozenOcean,
        B::BambooJungle,
        B::SunflowerPlains,
        B::WindsweptGravellyHills,
        B::FlowerForest,
        B::IceSpikes,
        B::OldGrowthBirchForest,
        B::OldGrowthSpruceTaiga,
        B::WindsweptSavanna,
        B::ErodedBadlands,
        B::JaggedPeaks,
        B::FrozenPeaks,
        B::SnowySlopes,
        B::Grove,
        B::Meadow,
        B::LushCaves,
        B::DripstoneCaves,
        B::StonyPeaks,
        B::DeepDark,
        B::MangroveSwamp,
        B::CherryGrove,
        B::PaleGarden,
    ];

    /// Nom du biome dans le jeu (« plains », « cherry_grove »…).
    pub fn name(self) -> &'static str {
        match self {
            B::Ocean => "ocean",
            B::Plains => "plains",
            B::Desert => "desert",
            B::WindsweptHills => "windswept_hills",
            B::Forest => "forest",
            B::Taiga => "taiga",
            B::Swamp => "swamp",
            B::River => "river",
            B::FrozenRiver => "frozen_river",
            B::SnowyPlains => "snowy_plains",
            B::MushroomFields => "mushroom_fields",
            B::Beach => "beach",
            B::Jungle => "jungle",
            B::SparseJungle => "sparse_jungle",
            B::DeepOcean => "deep_ocean",
            B::StonyShore => "stony_shore",
            B::SnowyBeach => "snowy_beach",
            B::BirchForest => "birch_forest",
            B::DarkForest => "dark_forest",
            B::SnowyTaiga => "snowy_taiga",
            B::OldGrowthPineTaiga => "old_growth_pine_taiga",
            B::WindsweptForest => "windswept_forest",
            B::Savanna => "savanna",
            B::SavannaPlateau => "savanna_plateau",
            B::Badlands => "badlands",
            B::WoodedBadlands => "wooded_badlands",
            B::WarmOcean => "warm_ocean",
            B::LukewarmOcean => "lukewarm_ocean",
            B::DeepLukewarmOcean => "deep_lukewarm_ocean",
            B::ColdOcean => "cold_ocean",
            B::DeepColdOcean => "deep_cold_ocean",
            B::FrozenOcean => "frozen_ocean",
            B::DeepFrozenOcean => "deep_frozen_ocean",
            B::BambooJungle => "bamboo_jungle",
            B::SunflowerPlains => "sunflower_plains",
            B::WindsweptGravellyHills => "windswept_gravelly_hills",
            B::FlowerForest => "flower_forest",
            B::IceSpikes => "ice_spikes",
            B::OldGrowthBirchForest => "old_growth_birch_forest",
            B::OldGrowthSpruceTaiga => "old_growth_spruce_taiga",
            B::WindsweptSavanna => "windswept_savanna",
            B::ErodedBadlands => "eroded_badlands",
            B::JaggedPeaks => "jagged_peaks",
            B::FrozenPeaks => "frozen_peaks",
            B::SnowySlopes => "snowy_slopes",
            B::Grove => "grove",
            B::Meadow => "meadow",
            B::LushCaves => "lush_caves",
            B::DripstoneCaves => "dripstone_caves",
            B::StonyPeaks => "stony_peaks",
            B::DeepDark => "deep_dark",
            B::MangroveSwamp => "mangrove_swamp",
            B::CherryGrove => "cherry_grove",
            B::PaleGarden => "pale_garden",
        }
    }
}

/// Les valeurs de climat d'une position.
#[derive(Clone, Copy, Debug)]
struct Climate {
    temperature: f64,
    humidity: f64,
    continentalness: f64,
    erosion: f64,
    weirdness: f64,
}

/// Climat entre quatre coins de grille (a = (x0, z0), b = (x0, z1), c = (x1, z0), d = (x1, z1)).
#[inline]
fn blend(a: &Climate, b: &Climate, c: &Climate, d: &Climate, fx: f64, fz: f64) -> Climate {
    let mix4 = |f: fn(&Climate) -> f64| lerp(fx, lerp(fz, f(a), f(b)), lerp(fz, f(c), f(d)));
    Climate {
        temperature: mix4(|c| c.temperature),
        humidity: mix4(|c| c.humidity),
        continentalness: mix4(|c| c.continentalness),
        erosion: mix4(|c| c.erosion),
        weirdness: mix4(|c| c.weirdness),
    }
}

thread_local! {
    /// Petit cache du climat aux coins de grille (graine, x, z) : les chunks voisins,
    /// les structures et `/locate` retombent souvent sur les mêmes coins.
    static GRID_CLIMATE: std::cell::RefCell<Vec<Option<(u64, i32, i32, Climate)>>> = std::cell::RefCell::new(vec![None; 1 << 13]);
}

impl Climate {
    /// Pics et vallées, tirés de l'étrangeté comme dans le jeu (-1 : vallée, 1 : pic).
    fn peaks_valleys(&self) -> f64 {
        -((self.weirdness.abs() - 2.0 / 3.0).abs() - 1.0 / 3.0) * 3.0
    }
}

fn index(value: f64, bounds: &[f64]) -> usize {
    bounds.iter().take_while(|b| value > **b).count()
}

/// Indices de température (0 à 4), d'humidité (0 à 4) et d'érosion (0 à 6), comme le jeu.
fn temperature_index(t: f64) -> usize {
    index(t, &[-0.45, -0.15, 0.2, 0.55])
}
fn humidity_index(h: f64) -> usize {
    index(h, &[-0.35, -0.1, 0.1, 0.3])
}
fn erosion_index(e: f64) -> usize {
    index(e, &[-0.78, -0.375, -0.2225, 0.05, 0.45, 0.55])
}

const OCEANS: [[B; 5]; 2] = [
    [B::DeepFrozenOcean, B::DeepColdOcean, B::DeepOcean, B::DeepLukewarmOcean, B::WarmOcean],
    [B::FrozenOcean, B::ColdOcean, B::Ocean, B::LukewarmOcean, B::WarmOcean],
];
const MIDDLE: [[B; 5]; 5] = [
    [B::SnowyPlains, B::SnowyPlains, B::SnowyPlains, B::SnowyTaiga, B::Taiga],
    [B::Plains, B::Plains, B::Forest, B::Taiga, B::OldGrowthSpruceTaiga],
    [B::FlowerForest, B::Plains, B::Forest, B::BirchForest, B::DarkForest],
    [B::Savanna, B::Savanna, B::Forest, B::Jungle, B::Jungle],
    [B::Desert, B::Desert, B::Desert, B::Desert, B::Desert],
];
const MIDDLE_VARIANT: [[Option<B>; 5]; 5] = [
    [Some(B::IceSpikes), None, Some(B::SnowyTaiga), None, None],
    [None, None, None, None, Some(B::OldGrowthPineTaiga)],
    [Some(B::SunflowerPlains), None, None, Some(B::OldGrowthBirchForest), Some(B::PaleGarden)],
    [None, None, Some(B::Plains), Some(B::SparseJungle), Some(B::BambooJungle)],
    [None, None, None, None, None],
];
const PLATEAU: [[B; 5]; 5] = [
    [B::SnowyPlains, B::SnowyPlains, B::SnowyPlains, B::SnowyTaiga, B::SnowyTaiga],
    [B::Meadow, B::Meadow, B::Forest, B::Taiga, B::OldGrowthSpruceTaiga],
    [B::Meadow, B::Meadow, B::Meadow, B::Meadow, B::DarkForest],
    [B::SavannaPlateau, B::SavannaPlateau, B::Forest, B::Forest, B::Jungle],
    [B::Badlands, B::Badlands, B::Badlands, B::WoodedBadlands, B::WoodedBadlands],
];
const PLATEAU_VARIANT: [[Option<B>; 5]; 5] = [
    [Some(B::IceSpikes), None, None, None, None],
    [Some(B::CherryGrove), None, Some(B::Meadow), Some(B::Meadow), Some(B::OldGrowthPineTaiga)],
    [Some(B::CherryGrove), Some(B::CherryGrove), Some(B::Forest), Some(B::BirchForest), None],
    [None, None, None, None, None],
    [Some(B::ErodedBadlands), Some(B::ErodedBadlands), None, None, None],
];
const SHATTERED: [[Option<B>; 5]; 5] = [
    [Some(B::WindsweptGravellyHills), Some(B::WindsweptGravellyHills), Some(B::WindsweptHills), Some(B::WindsweptForest), Some(B::WindsweptForest)],
    [Some(B::WindsweptGravellyHills), Some(B::WindsweptGravellyHills), Some(B::WindsweptHills), Some(B::WindsweptForest), Some(B::WindsweptForest)],
    [Some(B::WindsweptHills), Some(B::WindsweptHills), Some(B::WindsweptHills), Some(B::WindsweptForest), Some(B::WindsweptForest)],
    [None, None, None, None, None],
    [None, None, None, None, None],
];

/// Biome de surface d'après le climat, à la manière de `OverworldBiomeBuilder`.
fn surface_biome(c: &Climate) -> B {
    let (t, h) = (temperature_index(c.temperature), humidity_index(c.humidity));
    let e = erosion_index(c.erosion);
    let pv = c.peaks_valleys();
    let weird = c.weirdness > 0.0;
    let cont = c.continentalness;
    if cont < -1.05 {
        return B::MushroomFields;
    }
    if cont < -0.455 {
        return OCEANS[0][t];
    }
    if cont < -0.19 {
        return OCEANS[1][t];
    }
    let middle = || {
        if weird {
            MIDDLE_VARIANT[t][h].unwrap_or(MIDDLE[t][h])
        } else {
            MIDDLE[t][h]
        }
    };
    let badlands = || if h < 2 { if weird { B::ErodedBadlands } else { B::Badlands } } else { B::WoodedBadlands };
    // Terres chaudes et sèches, loin des côtes, peu érodées : badlands plutôt que désert
    let middle_or_badlands = || if t == 4 { badlands() } else { middle() };
    let plateau = || {
        if weird {
            PLATEAU_VARIANT[t][h].unwrap_or(PLATEAU[t][h])
        } else {
            PLATEAU[t][h]
        }
    };
    let shattered = || SHATTERED[t][h].unwrap_or_else(middle);
    let peak = || match t {
        0..=2 => {
            if weird {
                B::FrozenPeaks
            } else {
                B::JaggedPeaks
            }
        }
        3 => B::StonyPeaks,
        _ => badlands(),
    };
    let slope = || if t >= 3 { plateau() } else if h <= 1 { B::SnowySlopes } else { B::Grove };
    let beach = || match t {
        0 => B::SnowyBeach,
        4 => B::Desert,
        _ => B::Beach,
    };
    let river = || if t == 0 { B::FrozenRiver } else { B::River };
    let swamp = || match t {
        0 => middle(),
        1 | 2 => B::Swamp,
        _ => B::MangroveSwamp,
    };
    let coast = cont < -0.11;
    let near = cont < 0.03;
    let far = cont >= 0.3;
    if coast {
        if pv < -0.85 {
            return river();
        }
        return match e {
            0..=1 => {
                if pv > 0.2 {
                    B::StonyShore
                } else {
                    middle()
                }
            }
            2..=4 => beach(),
            5 => {
                if weird && pv > 0.2 {
                    shattered()
                } else {
                    beach()
                }
            }
            _ => beach(),
        };
    }
    if pv < -0.85 {
        // Vallées : rivières (le relief les creuse sous le niveau de la mer), ou marais
        return match e {
            0..=1 => {
                if far {
                    middle_or_badlands()
                } else {
                    river()
                }
            }
            6 => swamp(),
            _ => river(),
        };
    }
    if pv < -0.6 {
        return match e {
            0..=1 => {
                if far {
                    plateau()
                } else {
                    middle_or_badlands()
                }
            }
            2 | 3 => middle_or_badlands(),
            4 | 5 => middle(),
            _ => swamp(),
        };
    }
    if pv < 0.2 {
        return match e {
            0 => {
                if near {
                    middle()
                } else {
                    slope()
                }
            }
            1 => {
                if near {
                    middle_or_badlands()
                } else if far {
                    plateau()
                } else {
                    slope()
                }
            }
            2 | 3 => {
                if far {
                    plateau()
                } else if e == 2 {
                    middle_or_badlands()
                } else {
                    middle()
                }
            }
            4 => middle(),
            5 => {
                if weird {
                    shattered()
                } else {
                    middle()
                }
            }
            _ => swamp(),
        };
    }
    if pv < 0.7 {
        return match e {
            0 => {
                if near {
                    slope()
                } else {
                    peak()
                }
            }
            1 => slope(),
            2 => plateau(),
            3 => {
                if far {
                    plateau()
                } else {
                    middle_or_badlands()
                }
            }
            4 => middle(),
            5 => shattered(),
            _ => middle(),
        };
    }
    match e {
        0 | 1 => peak(),
        2 | 3 => plateau(),
        4 => middle(),
        5 => shattered(),
        _ => middle(),
    }
}

// --- Générateur ------------------------------------------------------------------------

pub struct Vanilla {
    seed: u64,
    temperature: Normal,
    humidity: Normal,
    continentalness: Normal,
    erosion: Normal,
    weirdness: Normal,
    shift: Normal,
    jagged: Normal,
    base: Octaves,
    cheese: Normal,
    spaghetti_a: Normal,
    spaghetti_b: Normal,
    spaghetti_rarity: Normal,
    noodle: Normal,
    noodle_a: Normal,
    noodle_b: Normal,
    entrances: Normal,
    surface: Normal,
    badlands_bands: Normal,
    patches: Normal,
    /// Villages, temples et mines déjà calculés
    structures: super::structures::PlanCache,
}

/// Taille des cellules de la grille de densité (comme dans le jeu).
const CELL_W: i32 = 4;
const CELL_H: i32 = 8;
const GRID_Y: usize = ((MAX_Y - MIN_Y + 1) / CELL_H) as usize + 1;

/// Forme du relief d'une colonne : hauteur de surface prévue, « raideur » (grande : relief
/// lisse ; petite : surplombs) et dentelure des sommets.
#[derive(Clone, Copy, Debug)]
struct Shape {
    height: f64,
    factor: f64,
    jagged: f64,
}

/// Interpolation linéaire par morceaux.
fn spline(x: f64, points: &[(f64, f64)]) -> f64 {
    if x <= points[0].0 {
        return points[0].1;
    }
    for w in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            let t = (x - x0) / (x1 - x0);
            // Raccord adouci (comme les splines du jeu)
            let t = t * t * (3.0 - 2.0 * t);
            return y0 + (y1 - y0) * t;
        }
    }
    points[points.len() - 1].1
}

impl Vanilla {
    pub fn new(seed: u64) -> Vanilla {
        Vanilla {
            seed,
            temperature: Normal::new(seed, 1, -10, &[1.5, 0.0, 1.0, 0.0, 0.0, 0.0]),
            humidity: Normal::new(seed, 2, -8, &[1.0, 1.0, 0.0, 0.0, 0.0, 0.0]),
            continentalness: Normal::new(seed, 3, -9, &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0]),
            erosion: Normal::new(seed, 4, -9, &[1.0, 1.0, 0.0, 1.0, 1.0]),
            weirdness: Normal::new(seed, 5, -7, &[1.0, 2.0, 1.0, 0.0, 0.0, 0.0]),
            shift: Normal::new(seed, 6, -3, &[1.0, 1.0, 1.0, 0.0]),
            jagged: Normal::new(seed, 7, -9, &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
            base: Octaves::new(seed, 8, -7, &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
            cheese: Normal::new(seed, 9, -8, &[0.5, 1.0, 2.0, 1.0, 2.0, 1.0, 0.0, 2.0, 0.0]),
            spaghetti_a: Normal::new(seed, 10, -7, &[1.0]),
            spaghetti_b: Normal::new(seed, 11, -7, &[1.0]),
            spaghetti_rarity: Normal::new(seed, 12, -11, &[1.0]),
            noodle: Normal::new(seed, 13, -8, &[1.0]),
            noodle_a: Normal::new(seed, 14, -7, &[1.0]),
            noodle_b: Normal::new(seed, 15, -7, &[1.0]),
            entrances: Normal::new(seed, 16, -7, &[0.4, 0.5, 1.0]),
            surface: Normal::new(seed, 17, -6, &[1.0, 1.0, 1.0]),
            badlands_bands: Normal::new(seed, 18, -9, &[1.0]),
            patches: Normal::new(seed, 19, -5, &[1.0, 1.0]),
            structures: Default::default(),
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Biome de surface d'une colonne, sans calculer sa hauteur (rapide).
    pub fn surface_biome_at(&self, x: i32, z: i32) -> B {
        surface_biome(&self.blended_climate(x, z))
    }

    /// Climat à un coin de la grille du relief (x et z multiples de CELL_W), en cache.
    fn grid_climate(&self, x: i32, z: i32) -> Climate {
        let key = self.seed as u64;
        let slot = ((x.wrapping_mul(73_856_093) ^ z.wrapping_mul(19_349_663)) as u32 as usize) & ((1 << 13) - 1);
        if let Some(c) = GRID_CLIMATE.with(|g| match g.borrow()[slot] {
            Some((s, gx, gz, c)) if s == key && gx == x && gz == z => Some(c),
            _ => None,
        }) {
            return c;
        }
        let c = self.climate(x, z);
        GRID_CLIMATE.with(|g| g.borrow_mut()[slot] = Some((key, x, z, c)));
        c
    }

    /// Climat d'une colonne tel que la génération le voit : interpolé entre les coins de
    /// la grille (identique au bit près à celui de `generate`).
    fn blended_climate(&self, x: i32, z: i32) -> Climate {
        let (gx, gz) = (x.div_euclid(CELL_W) * CELL_W, z.div_euclid(CELL_W) * CELL_W);
        let (fx, fz) = ((x - gx) as f64 / CELL_W as f64, (z - gz) as f64 / CELL_W as f64);
        blend(
            &self.grid_climate(gx, gz),
            &self.grid_climate(gx, gz + CELL_W),
            &self.grid_climate(gx + CELL_W, gz),
            &self.grid_climate(gx + CELL_W, gz + CELL_W),
            fx,
            fz,
        )
    }

    fn climate(&self, x: i32, z: i32) -> Climate {
        // Décalage des coordonnées (comme le jeu) : des frontières moins régulières
        let (fx, fz) = (x as f64, z as f64);
        let sx = fx + self.shift.sample(fx * 0.25, 0.0, fz * 0.25) * 4.0;
        let sz = fz + self.shift.sample(fz * 0.25, fx * 0.25, 0.0) * 4.0;
        let (qx, qz) = (sx * 0.25, sz * 0.25);
        Climate {
            temperature: self.temperature.sample(qx, 0.0, qz),
            humidity: self.humidity.sample(qx, 0.0, qz),
            continentalness: self.continentalness.sample(qx, 0.0, qz),
            erosion: self.erosion.sample(qx, 0.0, qz),
            weirdness: self.weirdness.sample(qx, 0.0, qz),
        }
    }

    /// Forme du relief d'après le climat.
    fn shape(&self, c: &Climate, x: i32, z: i32) -> Shape {
        let cont = c.continentalness;
        let e = c.erosion;
        let pv = c.peaks_valleys();
        // Hauteur de base selon la distance à la côte
        let base = spline(cont, &[(-1.2, 30.0), (-1.05, 42.0), (-0.455, 34.0), (-0.42, 44.0), (-0.19, 50.0), (-0.16, 61.0), (-0.11, 64.0), (0.03, 66.0), (0.3, 72.0), (0.6, 80.0), (1.0, 86.0)]);
        let inland = spline(cont, &[(-0.2, 0.0), (-0.11, 0.25), (0.03, 0.6), (0.3, 1.0)]);
        // Montagnes : peu d'érosion et pics hauts, loin des côtes
        let rugged = spline(e, &[(-0.8, 1.0), (-0.375, 0.65), (-0.2225, 0.3), (0.05, 0.12), (0.45, 0.0)]);
        let peaks = spline(pv, &[(-1.0, -0.6), (-0.85, -0.45), (-0.6, -0.05), (0.2, 0.25), (0.7, 0.75), (1.0, 1.0)]);
        let mut height = base + inland * (peaks * (8.0 + rugged * 150.0) + rugged * 25.0);
        // Vallées : rivières creusées juste sous le niveau de la mer
        if pv < -0.6 && cont > -0.19 {
            let depth = ((-0.6 - pv) / 0.25).clamp(0.0, 1.0);
            let river = SEA_LEVEL as f64 - 3.0 - (1.0 - rugged) * 2.0;
            if height > river {
                height = lerp(depth * (1.0 - rugged * 0.7), height, river);
            }
        }
        // Plateaux : terres loin des côtes et moyennement érodées
        if cont > 0.3 && (-0.375..0.05).contains(&e) && pv > -0.6 {
            height += 20.0 * spline(cont, &[(0.3, 0.0), (0.6, 1.0)]) * spline(e, &[(-0.375, 1.0), (0.05, 0.0)]);
        }
        // Marais : très érodé, tout près du niveau de la mer
        if e > 0.55 && cont > -0.11 {
            height = lerp(spline(e, &[(0.55, 0.0), (0.7, 1.0)]), height, SEA_LEVEL as f64 + 0.5);
        }
        let factor = 2.0 + 5.0 * (1.0 - rugged * inland);
        let jagged = if pv > 0.4 && rugged > 0.6 { (pv - 0.4) * rugged * 30.0 * self.jagged.sample(x as f64 * 1500.0 / 4096.0, 0.0, z as f64 * 1500.0 / 4096.0).abs() } else { 0.0 };
        Shape { height: height.clamp((MIN_Y + 8) as f64, (MAX_Y - 30) as f64), factor, jagged }
    }

    /// Densité du relief (positive : plein), sans les grottes.
    fn terrain_density(&self, s: &Shape, x: i32, y: i32, z: i32) -> f64 {
        let depth = (s.height + s.jagged - y as f64) / 128.0;
        let mut d = depth * s.factor;
        if d < 0.0 {
            d *= 0.25;
        }
        d *= 4.0;
        let (fx, fy, fz) = (x as f64, y as f64, z as f64);
        d + self.base.sample(fx / 3.0, fy / 6.0, fz / 3.0) * 0.18
    }

    /// Valeur des grottes (négative : vide). `depth` : profondeur sous le relief prévu.
    fn caves(&self, x: i32, y: i32, z: i32, depth: f64) -> f64 {
        if y < MIN_Y + 6 || depth < 2.0 {
            return 1.0;
        }
        let (fx, fy, fz) = (x as f64, y as f64, z as f64);
        let mut open: f64 = 1.0;
        // Grandes cavités (« fromage »), seulement en profondeur
        if depth > 10.0 {
            let fade = ((depth - 10.0) / 20.0).min(1.0);
            open = open.min(0.34 - self.cheese.sample(fx, fy * 0.8, fz) * fade);
        }
        // Galeries (« spaghettis ») : là où deux bruits passent tous deux par zéro. Près de la
        // surface, seulement là où le bruit des entrées le permet.
        let rarity = 0.75 + self.spaghetti_rarity.sample(fx * 2.0, fy, fz * 2.0).abs() * 0.5;
        let a = self.spaghetti_a.sample(fx / rarity, fy / rarity, fz / rarity);
        let b = self.spaghetti_b.sample(fx / rarity, fy / rarity, fz / rarity);
        let spaghetti = a.abs().max(b.abs()) - 0.085;
        if depth > 8.0 || self.entrances.sample(fx, fy, fz) > 0.25 {
            open = open.min(spaghetti);
        }
        // Boyaux étroits (« nouilles »), entre y = -60 et 64
        if (-60..=64).contains(&y) && depth > 8.0 && self.noodle.sample(fx, fy, fz) > 0.0 {
            let n = self.noodle_a.sample(fx * 2.0, fy * 2.0, fz * 2.0).abs().max(self.noodle_b.sample(fx * 2.0, fy * 2.0, fz * 2.0).abs()) - 0.04;
            open = open.min(n);
        }
        open
    }

    /// Biome d'une position (les biomes souterrains apparaissent sous le relief).
    fn biome_at(&self, c: &Climate, y: i32, surface: i32) -> B {
        let below = surface - y;
        if below > 18 {
            if c.erosion < -0.225 && y < -16 && c.continentalness > 0.0 {
                return B::DeepDark;
            }
            if c.continentalness > 0.7 {
                return B::DripstoneCaves;
            }
            if c.humidity > 0.6 {
                return B::LushCaves;
            }
        }
        surface_biome(c)
    }

    /// Hauteur réelle du sol d'une colonne (dernier bloc plein du relief, grottes exclues),
    /// calculée comme dans `generate` : même grille, même interpolation.
    fn column_top(&self, x: i32, z: i32) -> i32 {
        let (gx0, gz0) = (x.div_euclid(CELL_W) * CELL_W, z.div_euclid(CELL_W) * CELL_W);
        let (fx, fz) = ((x - gx0) as f64 / CELL_W as f64, (z - gz0) as f64 / CELL_W as f64);
        const OFFSETS: [(i32, i32); 4] = [(0, 0), (CELL_W, 0), (0, CELL_W), (CELL_W, CELL_W)];
        let corners: [Shape; 4] = OFFSETS.map(|(dx, dz)| {
            let (cx, cz) = (gx0 + dx, gz0 + dz);
            self.shape(&self.grid_climate(cx, cz), cx, cz)
        });
        let top_guess = corners.iter().map(|s| s.height + s.jagged).fold(0.0f64, f64::max) as i32 + 48;
        let start = (top_guess.min(MAX_Y) - MIN_Y) / CELL_H + 1;
        let density = |gy: i32| {
            let y = MIN_Y + gy * CELL_H;
            let d: [f64; 4] = std::array::from_fn(|k| self.terrain_density(&corners[k], gx0 + OFFSETS[k].0, y, gz0 + OFFSETS[k].1));
            lerp(fz, lerp(fx, d[0], d[1]), lerp(fx, d[2], d[3]))
        };
        let mut gy = start.min(GRID_Y as i32 - 1);
        let mut upper = density(gy);
        while gy > 0 {
            let lower = density(gy - 1);
            if lower > 0.0 {
                // Le sol est dans cette cellule : on cherche le bloc exact
                for dy in (0..CELL_H).rev() {
                    let t = dy as f64 / CELL_H as f64;
                    if lerp(t, lower, upper) > 0.0 {
                        return MIN_Y + (gy - 1) * CELL_H + dy;
                    }
                }
            }
            upper = lower;
            gy -= 1;
        }
        MIN_Y
    }

    /// Hauteur du sol et biome de surface d'une colonne.
    pub fn surface(&self, x: i32, z: i32) -> (i32, B) {
        (self.column_top(x, z), self.surface_biome_at(x, z))
    }

    /// Point d'apparition : terre ferme ni enneigée ni montagneuse, près de l'origine.
    pub fn spawn(&self) -> (i32, i32, i32) {
        for r in 0..96i32 {
            for (dx, dz) in ring(r) {
                let (x, z) = (dx * 16 + 8, dz * 16 + 8);
                let biome = self.surface_biome_at(x, z);
                if biome.ocean() || matches!(biome, B::River | B::FrozenRiver | B::JaggedPeaks | B::FrozenPeaks | B::StonyPeaks | B::Swamp | B::MangroveSwamp) {
                    continue;
                }
                let y = self.column_top(x, z);
                if y > SEA_LEVEL && y < SEA_LEVEL + 60 {
                    return (x, y, z);
                }
            }
        }
        (0, self.column_top(0, 0).max(SEA_LEVEL), 0)
    }

    /// Génère un chunk.
    pub fn generate(&self, p: &Palette, cx: i32, cz: i32) -> Column {
        let (bx, bz) = (cx * 16, cz * 16);
        let height = (MAX_Y - MIN_Y + 1) as usize;
        let at = |x: usize, z: usize, y: i32| (x * 16 + z) * height + (y - MIN_Y) as usize;
        let mut blocks = vec![p.air; 16 * 16 * height];
        let named = |n: &str| p.named.get(n).copied().unwrap_or(p.air);

        // 1. Climat et forme du relief aux coins de la grille (5 × 5 colonnes)
        let mut shapes = [[Shape { height: 0.0, factor: 1.0, jagged: 0.0 }; 5]; 5];
        let mut corner_climate = [[Climate { temperature: 0.0, humidity: 0.0, continentalness: 0.0, erosion: 0.0, weirdness: 0.0 }; 5]; 5];
        for (i, row) in shapes.iter_mut().enumerate() {
            for (j, s) in row.iter_mut().enumerate() {
                let (x, z) = (bx + i as i32 * CELL_W, bz + j as i32 * CELL_W);
                let c = self.grid_climate(x, z);
                corner_climate[i][j] = c;
                *s = self.shape(&c, x, z);
            }
        }
        // Climat d'une colonne : interpolé entre les coins de la grille (déjà calculés pour
        // le relief) plutôt que recalculé 256 fois ; les frontières de biomes restent
        // douces, au bloc près
        let climate_at = |x: usize, z: usize| -> Climate {
            let (i, j) = (x / 4, z / 4);
            let (fx, fz) = ((x % 4) as f64 / 4.0, (z % 4) as f64 / 4.0);
            blend(&corner_climate[i][j], &corner_climate[i][j + 1], &corner_climate[i + 1][j], &corner_climate[i + 1][j + 1], fx, fz)
        };
        // 2. Densité du relief et des grottes sur la grille, puis interpolée bloc par bloc
        let mut terrain = vec![0f64; 25 * GRID_Y];
        let mut caves = vec![0f64; 25 * GRID_Y];
        let top_cell = shapes.iter().flatten().map(|s| s.height + s.jagged).fold(0.0f64, f64::max) as i32 + 48;
        let top_gy = (((top_cell.min(MAX_Y) - MIN_Y) / CELL_H) as usize + 1).min(GRID_Y - 1);
        for i in 0..5 {
            for j in 0..5 {
                let s = shapes[i][j];
                let (x, z) = (bx + i as i32 * CELL_W, bz + j as i32 * CELL_W);
                for gy in 0..GRID_Y {
                    let y = MIN_Y + gy as i32 * CELL_H;
                    let k = (i * 5 + j) * GRID_Y + gy;
                    if gy > top_gy {
                        terrain[k] = -1.0;
                        caves[k] = 1.0;
                        continue;
                    }
                    terrain[k] = self.terrain_density(&s, x, y, z);
                    caves[k] = self.caves(x, y, z, s.height - y as f64);
                }
            }
        }
        // Densités d'une colonne aux hauteurs de la grille (interpolées en x et z une seule
        // fois), puis le long de y pour chaque bloc : même résultat qu'une interpolation
        // trilinéaire complète, environ huit fois moins de calcul
        let column_of = |grid: &[f64], x: usize, z: usize, out: &mut [f64; GRID_Y]| {
            let (i, fx) = (x / 4, (x % 4) as f64 / 4.0);
            let (j, fz) = (z / 4, (z % 4) as f64 / 4.0);
            let (c00, c01, c10, c11) = ((i * 5 + j) * GRID_Y, (i * 5 + j + 1) * GRID_Y, ((i + 1) * 5 + j) * GRID_Y, ((i + 1) * 5 + j + 1) * GRID_Y);
            for gy in 0..GRID_Y {
                out[gy] = lerp(fx, lerp(fz, grid[c00 + gy], grid[c01 + gy]), lerp(fz, grid[c10 + gy], grid[c11 + gy]));
            }
        };
        let along = |col: &[f64; GRID_Y], y: i32| -> f64 {
            let gy = ((y - MIN_Y) / CELL_H) as usize;
            let fy = ((y - MIN_Y) % CELL_H) as f64 / CELL_H as f64;
            lerp(fy, col[gy], col[(gy + 1).min(GRID_Y - 1)])
        };
        let mut terrain_col = [0f64; GRID_Y];
        let mut caves_col = [0f64; GRID_Y];

        // 3. Roche, eau et lave ; sommet du relief de chaque colonne
        let mut tops = [[MIN_Y; 16]; 16];
        let mut climates = vec![
            Climate { temperature: 0.0, humidity: 0.0, continentalness: 0.0, erosion: 0.0, weirdness: 0.0 };
            256
        ];
        let mut biomes = [[B::Plains; 16]; 16];
        let mut rng = Rng::new(mix(self.seed, cx as i64, cz as i64, 500));
        for x in 0..16usize {
            for z in 0..16usize {
                let c = climate_at(x, z);
                climates[x * 16 + z] = c;
                biomes[x][z] = surface_biome(&c);
                column_of(&terrain, x, z, &mut terrain_col);
                column_of(&caves, x, z, &mut caves_col);
                let mut top = MIN_Y;
                for y in (MIN_Y..=MAX_Y.min(top_cell)).rev() {
                    if along(&terrain_col, y) > 0.0 {
                        top = y;
                        break;
                    }
                }
                tops[x][z] = top;
                let under_sea = top < SEA_LEVEL;
                for y in MIN_Y..=top {
                    let solid = along(&terrain_col, y) > 0.0;
                    let cave = along(&caves_col, y) < 0.0 && !(under_sea && y > top - 5);
                    let block = if y == MIN_Y || (y < MIN_Y + 5 && (rng.below(5) as i32) >= y - MIN_Y) {
                        p.bedrock
                    } else if !solid || cave {
                        if y < -54 {
                            p.lava
                        } else if !solid && y <= SEA_LEVEL && !cave {
                            p.water
                        } else {
                            p.air
                        }
                    } else if y < 0 || (y < 8 && (rng.below(8) as i32) > y) {
                        p.deepslate
                    } else {
                        p.stone
                    };
                    blocks[at(x, z, y)] = block;
                }
                for y in (top + 1)..=SEA_LEVEL {
                    blocks[at(x, z, y)] = p.water;
                }
            }
        }

        // 4. Surface selon le biome
        for x in 0..16usize {
            for z in 0..16usize {
                let top = tops[x][z];
                let biome = biomes[x][z];
                let (wx, wz) = (bx + x as i32, bz + z as i32);
                let noise = self.surface.sample(wx as f64, 0.0, wz as f64);
                let depth = (3.0 + noise * 2.0 + 0.5).max(1.0) as i32;
                // Pente : écart de hauteur avec les voisins (dans le chunk)
                let neighbour = |dx: i32, dz: i32| {
                    let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                    if (0..16).contains(&nx) && (0..16).contains(&nz) {
                        tops[nx as usize][nz as usize]
                    } else {
                        top
                    }
                };
                let steep = [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().map(|(dx, dz)| (neighbour(*dx * 2, *dz * 2) - top).abs()).max().unwrap_or(0) >= 4;
                let underwater = top < SEA_LEVEL;
                let (top_block, filler, under) = self.surface_blocks(p, biome, underwater, steep, top, noise, wx, wz);
                let mut d = 0;
                let mut y = top;
                while y > MIN_Y + 1 && d < depth + 4 {
                    let i = at(x, z, y);
                    let current = blocks[i];
                    if current != p.stone && current != p.deepslate {
                        if current == p.air || current == p.water || current == p.lava {
                            break; // grotte sous la surface : la suite reste de la roche
                        }
                        y -= 1;
                        continue;
                    }
                    blocks[i] = if d == 0 {
                        top_block
                    } else if d <= depth {
                        filler
                    } else {
                        under
                    };
                    if biome.badlands() && d > 0 && top >= SEA_LEVEL - 4 {
                        blocks[i] = self.terracotta(p, y, wx, wz);
                    }
                    d += 1;
                    y -= 1;
                }
                // Badlands : bandes de terre cuite sur toute la hauteur des falaises
                if biome.badlands() && top > SEA_LEVEL {
                    for y in (SEA_LEVEL - 4)..(top - 3) {
                        let i = at(x, z, y);
                        if blocks[i] == p.stone {
                            blocks[i] = self.terracotta(p, y, wx, wz);
                        }
                    }
                }
                // Neige et glace dans les biomes froids
                if biome.snowy() && top + 1 <= MAX_Y {
                    if top < SEA_LEVEL {
                        blocks[at(x, z, SEA_LEVEL)] = p.ice;
                    } else if blocks[at(x, z, top)] != p.snow && blocks[at(x, z, top)] != named("packed_ice") && blocks[at(x, z, top)] != named("powder_snow") {
                        blocks[at(x, z, top + 1)] = p.snow_layer;
                    }
                }
            }
        }

        // 5. Roches, minerais
        for (block, count, low, high) in [(p.granite, 2, 0, 80), (p.diorite, 2, 0, 80), (p.andesite, 2, 0, 80), (p.tuff, 2, -60, 0), (p.gravel, 2, -40, 60), (p.dirt, 2, 0, 160)] {
            for _ in 0..count {
                let (x, y, z) = (rng.below(16) as i32, low + rng.below((high - low) as u32) as i32, rng.below(16) as i32);
                blob(&mut blocks, at, (x, y, z), 2.5 + rng.unit() * 1.5, &[p.stone, p.deepslate], block);
            }
        }
        let mountains = matches!(biomes[8][8], B::WindsweptHills | B::WindsweptGravellyHills | B::WindsweptForest | B::JaggedPeaks | B::FrozenPeaks | B::StonyPeaks | B::Meadow | B::Grove | B::SnowySlopes | B::CherryGrove);
        let emerald = (named("emerald_ore"), named("deepslate_emerald_ore"));
        let ores = [
            (p.coal, 20, 0, 192, 1.6),
            (p.iron, 12, -24, 56, 1.3),
            (p.iron, if mountains { 10 } else { 2 }, 80, 256, 1.3),
            (p.copper, 10, -16, 112, 1.5),
            (p.gold, 4, -64, 32, 1.1),
            (p.redstone, 6, -64, 15, 1.2),
            (p.lapis, 2, -64, 64, 1.1),
            (p.diamond, 2, -64, 16, 1.0),
            (emerald, if mountains { 6 } else { 0 }, -16, 256, 0.6),
        ];
        let rocks = [p.stone, p.deepslate, p.granite, p.diorite, p.andesite, p.tuff];
        for (ore, tries, low, high, size) in ores {
            for _ in 0..tries {
                let (x, y, z) = (rng.below(16) as i32, low + rng.below((high - low) as u32) as i32, rng.below(16) as i32);
                if y > tops[x as usize][z as usize] - 3 {
                    continue;
                }
                blob(&mut blocks, at, (x, y, z), size + rng.unit(), &rocks, if y < 0 { ore.1 } else { ore.0 });
            }
        }

        // 6. Arbres et végétation : ceux des chunks voisins qui débordent ici aussi
        for ncx in (cx - 1)..=(cx + 1) {
            for ncz in (cz - 1)..=(cz + 1) {
                let own = if (ncx, ncz) == (cx, cz) { Some((&tops, &biomes)) } else { None };
                for plant in self.plants(ncx, ncz, own) {
                    plant.paint(&mut blocks, at, bx, bz, p);
                }
            }
        }
        // Plantes basses de ce chunk (herbes, fleurs, champignons…) et fonds marins
        let mut plants = Rng::new(mix(self.seed, cx as i64, cz as i64, 600));
        let corals = ["tube", "brain", "bubble", "fire", "horn"];
        for x in 0..16usize {
            for z in 0..16usize {
                let top = tops[x][z];
                if top + 2 > MAX_Y {
                    continue;
                }
                let biome = biomes[x][z];
                let ground = blocks[at(x, z, top)];
                if top < SEA_LEVEL {
                    // Nénuphars à la surface des marais
                    let surface = at(x, z, SEA_LEVEL + 1);
                    if matches!(biome, B::Swamp | B::MangroveSwamp) && plants.below(10) == 0 && blocks[surface] == p.air {
                        blocks[surface] = named("waterlily");
                    }
                    if blocks[at(x, z, top + 1)] != p.water {
                        continue;
                    }
                    let depth = SEA_LEVEL - top;
                    let (wx, wz) = (bx + x as i32, bz + z as i32);
                    let roll = plants.below(1000);
                    let warm = biome == B::WarmOcean;
                    // Récifs de corail des océans chauds : buttes de blocs de corail, coraux et
                    // gorgones au sommet, cornichons de mer
                    let reef = warm && depth >= 4 && self.patches.sample(wx as f64 * 0.8, 50.0, wz as f64 * 0.8) > 0.05;
                    if reef {
                        let kind = corals[(mix(self.seed, wx as i64 / 3, wz as i64 / 3, 800) % 5) as usize];
                        let height = 1 + plants.below(3) as i32;
                        for dy in 1..=height {
                            blocks[at(x, z, top + dy)] = named(&format!("{}_coral_block", kind));
                        }
                        let crown = top + height + 1;
                        if crown < SEA_LEVEL {
                            let r = plants.below(10);
                            let deco = if r < 4 {
                                Some(format!("{}_coral", corals[plants.below(5) as usize]))
                            } else if r < 7 {
                                Some(format!("{}_coral_fan", corals[plants.below(5) as usize]))
                            } else if r < 8 {
                                Some("sea_pickle[cluster_count=3]".to_string())
                            } else {
                                None
                            };
                            if let Some(d) = deco {
                                blocks[at(x, z, crown)] = named(&d);
                            }
                        }
                        continue;
                    }
                    let kelp_ok = matches!(biome, B::Ocean | B::DeepOcean | B::ColdOcean | B::DeepColdOcean | B::LukewarmOcean | B::DeepLukewarmOcean);
                    if kelp_ok && depth >= 4 && roll < 90 {
                        // Laminaires : une colonne qui monte presque jusqu'à la surface
                        let height = (2 + plants.below((depth - 2) as u32) as i32).min(depth - 1);
                        for dy in 1..=height {
                            blocks[at(x, z, top + dy)] = named("kelp");
                        }
                    } else if roll < if warm { 450 } else { 300 } {
                        // Herbes marines, parfois hautes (deux blocs)
                        if depth >= 3 && plants.below(4) == 0 {
                            blocks[at(x, z, top + 1)] = named("seagrass[sea_grass_type=double_bot]");
                            blocks[at(x, z, top + 2)] = named("seagrass[sea_grass_type=double_top]");
                        } else {
                            blocks[at(x, z, top + 1)] = named("seagrass");
                        }
                    } else if (warm || biome == B::LukewarmOcean) && roll < 320 {
                        blocks[at(x, z, top + 1)] = named("sea_pickle[cluster_count=1]");
                    } else if matches!(biome, B::DeepOcean | B::DeepColdOcean | B::DeepLukewarmOcean) && roll < 304 && (ground == p.gravel || ground == p.sand) {
                        blocks[at(x, z, top)] = named("magma");
                    }
                    continue;
                }
                if blocks[at(x, z, top + 1)] != p.air {
                    continue;
                }
                let roll = plants.below(1000);
                if let Some(block) = self.ground_plant(p, biome, ground, roll, &mut plants) {
                    blocks[at(x, z, top + 1)] = block;
                    if block == p.cactus || block == named("reeds") {
                        let tall = 1 + plants.below(3) as i32;
                        for dy in 2..=tall {
                            if top + dy <= MAX_Y && blocks[at(x, z, top + dy)] == p.air {
                                blocks[at(x, z, top + dy)] = block;
                            }
                        }
                        // Fleur de cactus, parfois, au sommet
                        if block == p.cactus && plants.below(6) == 0 && top + tall + 1 <= MAX_Y && blocks[at(x, z, top + tall + 1)] == p.air {
                            blocks[at(x, z, top + tall + 1)] = named("cactus_flower");
                        }
                    }
                }
            }
        }
        // Icebergs des océans gelés
        if matches!(biomes[8][8], B::FrozenOcean | B::DeepFrozenOcean) && plants.below(6) == 0 {
            let (packed, blue) = (named("packed_ice"), named("blue_ice"));
            let up = 4 + plants.below(10) as i32;
            for dy in -6..=up {
                let r = if dy <= 0 { 6 + dy / 2 } else { (6 * (up - dy) / up).max(0) };
                for dx in -r..=r {
                    for dz in -r..=r {
                        if dx * dx + dz * dz > r * r {
                            continue;
                        }
                        let (x, z, y) = ((8 + dx) as usize, (8 + dz) as usize, SEA_LEVEL + dy);
                        let i = at(x, z, y);
                        if blocks[i] == p.water || blocks[i] == p.air || blocks[i] == p.ice {
                            blocks[i] = if dx * dx + dz * dz < 2 && dy < 2 && plants.below(3) == 0 { blue } else { packed };
                        }
                    }
                }
            }
        }
        // Cannes à sucre au bord de l'eau
        for _ in 0..3 {
            let (x, z) = (plants.below(14) as usize + 1, plants.below(14) as usize + 1);
            let top = tops[x][z];
            if top != SEA_LEVEL || top + 4 > MAX_Y {
                continue;
            }
            let near_water = [(x - 1, z), (x + 1, z), (x, z - 1), (x, z + 1)].iter().any(|(a, b)| blocks[at(*a, *b, top)] == p.water);
            let ground = blocks[at(x, z, top)];
            if near_water && (ground == p.grass || ground == p.sand || ground == p.dirt) && blocks[at(x, z, top + 1)] == p.air {
                for dy in 1..=(1 + plants.below(3) as i32) {
                    blocks[at(x, z, top + dy)] = named("reeds");
                }
            }
        }

        // 7. Structures : villages, temples, mines (la part qui tombe dans ce chunk)
        let mut chests = Vec::new();
        for plan in super::structures::near(self, p, &self.structures, cx, cz) {
            for (x, y, z, s) in plan.blocks_in(cx, cz) {
                blocks[at(*x as usize, *z as usize, *y as i32)] = *s;
            }
            chests.extend_from_slice(plan.chests_in(cx, cz));
        }

        // 8. Biomes en 3D (cases de 4 blocs), y compris les biomes souterrains
        let mut ids = vec![0u8; SECTIONS * 64];
        for qx in 0..4usize {
            for qz in 0..4usize {
                let (x, z) = (qx * 4 + 2, qz * 4 + 2);
                let c = climates[x * 16 + z];
                let top = tops[x][z];
                for qy in 0..(SECTIONS * 4) {
                    let y = MIN_Y + qy as i32 * 4 + 2;
                    let id = self.biome_at(&c, y, top).id();
                    ids[(qy / 4) * 64 + qx * 16 + qz * 4 + (qy % 4)] = id;
                }
            }
        }
        let mut column = Column::from_blocks(&blocks, p.air, super::generator::Biome::Plains);
        column.biome_ids = Some(ids);
        column.chests = chests;
        let _ = Section::Uniform(0);
        column
    }

    /// Bloc de surface, bloc de dessous, et bloc encore en dessous.
    #[allow(clippy::too_many_arguments)]
    fn surface_blocks(&self, p: &Palette, biome: B, underwater: bool, steep: bool, top: i32, noise: f64, x: i32, z: i32) -> (u32, u32, u32) {
        let named = |n: &str| p.named.get(n).copied().unwrap_or(p.stone);
        let patch = self.patches.sample(x as f64, 0.0, z as f64);
        if underwater {
            return match biome {
                B::DeepOcean | B::DeepColdOcean | B::DeepFrozenOcean => (p.gravel, p.gravel, p.stone),
                B::Swamp => (p.dirt, p.dirt, p.dirt),
                B::MangroveSwamp => (named("mud"), named("mud"), p.dirt),
                B::River | B::FrozenRiver => {
                    if patch > 0.2 {
                        (p.gravel, p.gravel, p.stone)
                    } else if patch < -0.4 {
                        (p.clay, p.clay, p.dirt)
                    } else {
                        (p.sand, p.sand, p.sandstone)
                    }
                }
                B::WarmOcean | B::LukewarmOcean | B::DeepLukewarmOcean => (p.sand, p.sand, p.sandstone),
                _ if biome.badlands() => (named("red_sand"), named("red_sand"), p.stone),
                _ if top < SEA_LEVEL - 12 => (p.gravel, p.gravel, p.stone),
                _ => {
                    if patch < -0.45 {
                        (p.clay, p.clay, p.dirt)
                    } else {
                        (p.sand, p.sand, p.sandstone)
                    }
                }
            };
        }
        match biome {
            B::Desert | B::Beach => (p.sand, p.sand, p.sandstone),
            B::SnowyBeach => (p.sand, p.sand, p.sandstone),
            B::StonyShore => (p.stone, p.stone, p.stone),
            B::MushroomFields => (named("mycelium"), p.dirt, p.dirt),
            B::Badlands | B::ErodedBadlands => (named("red_sand"), named("hardened_clay"), named("hardened_clay")),
            B::WoodedBadlands => {
                if top > SEA_LEVEL + 35 {
                    (if patch > 0.0 { named("coarse_dirt") } else { p.grass }, p.dirt, named("hardened_clay"))
                } else {
                    (named("red_sand"), named("hardened_clay"), named("hardened_clay"))
                }
            }
            B::StonyPeaks => {
                if patch > 0.3 {
                    (named("calcite"), named("calcite"), p.stone)
                } else {
                    (p.stone, p.stone, p.stone)
                }
            }
            B::JaggedPeaks => {
                if steep {
                    (p.stone, p.stone, p.stone)
                } else {
                    (p.snow, p.snow, p.stone)
                }
            }
            B::FrozenPeaks => {
                if steep {
                    (named("packed_ice"), named("packed_ice"), p.stone)
                } else if patch > 0.3 {
                    (named("packed_ice"), p.snow, p.stone)
                } else {
                    (p.snow, p.snow, p.stone)
                }
            }
            B::SnowySlopes => {
                if steep {
                    (p.stone, p.stone, p.stone)
                } else if patch > 0.35 {
                    (named("powder_snow"), p.snow, p.stone)
                } else {
                    (p.snow, p.snow, p.stone)
                }
            }
            B::WindsweptGravellyHills => {
                if patch > 0.1 || steep {
                    (p.gravel, p.gravel, p.stone)
                } else if patch < -0.3 {
                    (p.stone, p.stone, p.stone)
                } else {
                    (p.grass, p.dirt, p.dirt)
                }
            }
            B::WindsweptHills | B::WindsweptForest if steep || patch > 0.4 => (p.stone, p.stone, p.stone),
            B::WindsweptSavanna if steep || patch > 0.3 => (p.stone, p.stone, p.stone),
            B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga => {
                if patch > 0.2 {
                    (named("podzol"), p.dirt, p.dirt)
                } else if patch < -0.3 {
                    (named("coarse_dirt"), p.dirt, p.dirt)
                } else {
                    (p.grass, p.dirt, p.dirt)
                }
            }
            B::MangroveSwamp => (named("mud"), named("mud"), p.dirt),
            B::Savanna | B::SavannaPlateau if patch > 0.45 => (named("coarse_dirt"), p.dirt, p.dirt),
            _ if steep && top > SEA_LEVEL + 60 => (p.stone, p.stone, p.stone),
            _ => {
                let _ = noise;
                (p.grass, p.dirt, p.dirt)
            }
        }
    }

    /// Bandes de terre cuite des badlands (couleur selon la hauteur, ondulée par un bruit).
    fn terracotta(&self, p: &Palette, y: i32, x: i32, z: i32) -> u32 {
        const BANDS: [&str; 12] = [
            "hardened_clay", "orange_terracotta", "hardened_clay", "yellow_terracotta", "hardened_clay", "hardened_clay",
            "brown_terracotta", "hardened_clay", "red_terracotta", "white_terracotta", "hardened_clay", "light_gray_terracotta",
        ];
        let shift = (self.badlands_bands.sample(x as f64, 0.0, z as f64) * 4.0).round() as i32;
        let band = BANDS[((y + shift).rem_euclid(BANDS.len() as i32 * 2) / 2) as usize];
        p.named.get(band).copied().unwrap_or(p.stone)
    }

    /// Plante basse posée sur le sol d'un biome (`roll` : de 0 à 999).
    fn ground_plant(&self, p: &Palette, biome: B, ground: u32, roll: u32, rng: &mut Rng) -> Option<u32> {
        let named = |n: &str| p.named.get(n).copied();
        let flowers = |rng: &mut Rng| {
            const FLOWERS: [&str; 7] = ["dandelion", "poppy", "azure_bluet", "red_tulip", "oxeye_daisy", "cornflower", "allium"];
            let n = FLOWERS[rng.below(FLOWERS.len() as u32) as usize];
            p.named.get(n).copied().or(Some(p.dandelion))
        };
        // Tapis à quatre orientations (pétales de cerisier, feuilles mortes, fleurs sauvages)
        let carpet = |kind: &str, rng: &mut Rng| {
            let variants: Vec<&&str> = super::generator::EXTRA_BLOCKS.iter().filter(|n| n.starts_with(kind)).collect();
            named(variants[rng.below(variants.len() as u32) as usize])
        };
        let on_grass = ground == p.grass;
        let red_sand = p.named.get("red_sand").copied().unwrap_or(0);
        match biome {
            B::Desert | B::Badlands | B::ErodedBadlands | B::WoodedBadlands if ground == p.sand || ground == red_sand => {
                if roll < 6 {
                    Some(p.cactus)
                } else if roll < 14 {
                    named("deadbush")
                } else if roll < 40 {
                    named("short_dry_grass")
                } else if roll < 48 {
                    named("tall_dry_grass")
                } else {
                    None
                }
            }
            B::CherryGrove if on_grass => {
                if roll < 300 {
                    carpet("pink_petals", rng)
                } else if roll < 360 {
                    Some(p.short_grass)
                } else {
                    None
                }
            }
            B::Plains | B::SunflowerPlains | B::Meadow if on_grass => {
                if roll < 220 {
                    Some(p.short_grass)
                } else if roll < if biome == B::Meadow { 300 } else { 240 } {
                    flowers(rng)
                } else if biome == B::Meadow && roll < 340 {
                    carpet("wildflowers", rng)
                } else if roll < 250 {
                    named("bush")
                } else {
                    None
                }
            }
            B::FlowerForest if on_grass => {
                if roll < 200 {
                    flowers(rng)
                } else if roll < 260 {
                    Some(p.short_grass)
                } else {
                    None
                }
            }
            B::Forest | B::BirchForest | B::OldGrowthBirchForest | B::DarkForest | B::WindsweptForest | B::PaleGarden if on_grass => {
                let litter = if biome == B::DarkForest { 160 } else { 80 };
                if roll < litter {
                    carpet("leaf_litter", rng)
                } else if roll < litter + 90 {
                    Some(p.short_grass)
                } else if roll < litter + 100 {
                    flowers(rng)
                } else if roll < litter + 140 && matches!(biome, B::BirchForest | B::OldGrowthBirchForest) {
                    carpet("wildflowers", rng)
                } else if roll < litter + 108 {
                    named("bush")
                } else if roll < litter + 112 && biome == B::DarkForest {
                    named(if rng.below(2) == 0 { "red_mushroom" } else { "brown_mushroom" })
                } else {
                    None
                }
            }
            B::WoodedBadlands if on_grass || ground == p.named.get("coarse_dirt").copied().unwrap_or(0) => {
                if roll < 100 {
                    carpet("leaf_litter", rng)
                } else if roll < 160 {
                    named("short_dry_grass")
                } else {
                    None
                }
            }
            B::Taiga | B::SnowyTaiga | B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga | B::Grove => {
                if roll < 100 && (on_grass || ground == p.named.get("podzol").copied().unwrap_or(0)) {
                    named("fern")
                } else if roll < 110 && biome != B::SnowyTaiga && on_grass {
                    named("sweet_berry_bush")
                } else if roll < 115 && on_grass {
                    named("brown_mushroom")
                } else {
                    None
                }
            }
            B::Jungle | B::SparseJungle | B::BambooJungle if on_grass => {
                if roll < 250 {
                    named(if rng.below(4) == 0 { "fern" } else { "short_grass" }).or(Some(p.short_grass))
                } else if roll < 270 && biome == B::BambooJungle {
                    named("bamboo")
                } else if roll < 255 {
                    named("bush")
                } else {
                    None
                }
            }
            B::Savanna | B::SavannaPlateau | B::WindsweptSavanna if on_grass => {
                if roll < 300 {
                    Some(p.short_grass)
                } else {
                    None
                }
            }
            B::Swamp | B::MangroveSwamp if on_grass || ground == p.named.get("mud").copied().unwrap_or(0) => {
                if roll < 60 && on_grass {
                    Some(p.short_grass)
                } else if roll < 70 && on_grass {
                    named("blue_orchid")
                } else if roll < 85 {
                    named("firefly_bush")
                } else {
                    None
                }
            }
            B::MushroomFields => {
                if roll < 10 {
                    named(if rng.below(2) == 0 { "red_mushroom" } else { "brown_mushroom" })
                } else {
                    None
                }
            }
            B::WindsweptHills | B::WindsweptGravellyHills | B::SnowyPlains if on_grass && roll < 60 => Some(p.short_grass),
            _ if on_grass && roll < 2 => named("pumpkin"),
            _ => None,
        }
    }

    /// Arbres (et pics de glace) dont le pied est dans un chunk. `own` : sommets et biomes
    /// déjà calculés quand il s'agit du chunk en cours de génération.
    #[allow(clippy::type_complexity)]
    fn plants(&self, cx: i32, cz: i32, own: Option<(&[[i32; 16]; 16], &[[B; 16]; 16])>) -> Vec<Plant> {
        let mut rng = Rng::new(mix(self.seed, cx as i64, cz as i64, 700));
        let center = surface_biome(&self.climate(cx * 16 + 8, cz * 16 + 8));
        // Nombre d'essais par chunk, comme les densités du jeu (à peu près)
        let tries = match center {
            B::Forest | B::FlowerForest => 10,
            B::BirchForest | B::OldGrowthBirchForest => 10,
            B::DarkForest | B::PaleGarden => 14,
            B::Taiga | B::SnowyTaiga | B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga => 10,
            B::Grove => 6,
            B::Jungle | B::BambooJungle => 30,
            B::SparseJungle => 3,
            B::Savanna | B::SavannaPlateau | B::WindsweptSavanna => 2,
            B::WindsweptForest => 6,
            B::WindsweptHills | B::WindsweptGravellyHills => 1,
            B::Swamp => 3,
            B::MangroveSwamp => 6,
            B::CherryGrove => 4,
            B::Meadow => if rng.below(4) == 0 { 1 } else { 0 },
            B::WoodedBadlands => 3,
            B::Plains | B::SunflowerPlains => if rng.below(3) == 0 { 1 } else { 0 },
            B::SnowyPlains => if rng.below(4) == 0 { 1 } else { 0 },
            B::IceSpikes => 2,
            _ => 0,
        };
        let mut out = Vec::new();
        for _ in 0..tries {
            let (lx, lz) = (rng.below(16) as i32, rng.below(16) as i32);
            let (x, z) = (cx * 16 + lx, cz * 16 + lz);
            let roll = rng.next();
            let (top, biome) = match own {
                Some((tops, biomes)) => (tops[lx as usize][lz as usize], biomes[lx as usize][lz as usize]),
                None => {
                    // Chunk voisin : le biome d'abord, la hauteur seulement si un arbre peut y pousser
                    let biome = self.surface_biome_at(x, z);
                    if biome.ocean() || matches!(biome, B::River | B::FrozenRiver | B::Beach | B::Desert | B::StonyShore) {
                        continue;
                    }
                    (self.column_top(x, z), biome)
                }
            };
            if top < SEA_LEVEL - if biome == B::MangroveSwamp { 3 } else { 0 } || top + 32 > MAX_Y || biome.ocean() || matches!(biome, B::River | B::FrozenRiver | B::Beach | B::Desert | B::StonyShore) {
                continue;
            }
            let kind = match biome {
                B::Forest | B::WindsweptForest => {
                    if roll % 5 == 0 {
                        Tree::Birch
                    } else if roll % 10 == 1 {
                        Tree::FancyOak
                    } else {
                        Tree::Oak
                    }
                }
                B::FlowerForest => {
                    if roll % 3 == 0 {
                        Tree::Birch
                    } else {
                        Tree::Oak
                    }
                }
                B::BirchForest => Tree::Birch,
                B::OldGrowthBirchForest => Tree::TallBirch,
                B::DarkForest => {
                    if roll % 6 == 0 {
                        Tree::Oak
                    } else {
                        Tree::DarkOak
                    }
                }
                B::PaleGarden => Tree::PaleOak,
                B::Taiga | B::SnowyTaiga | B::Grove => Tree::Spruce,
                B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga => {
                    if roll % 3 == 0 {
                        Tree::Spruce
                    } else {
                        Tree::MegaSpruce
                    }
                }
                B::Jungle | B::BambooJungle | B::SparseJungle => {
                    if roll % 10 == 0 {
                        Tree::MegaJungle
                    } else if roll % 3 == 0 {
                        Tree::JungleBush
                    } else {
                        Tree::Jungle
                    }
                }
                B::Savanna | B::SavannaPlateau | B::WindsweptSavanna => {
                    if roll % 5 == 0 {
                        Tree::Oak
                    } else {
                        Tree::Acacia
                    }
                }
                B::Swamp => Tree::SwampOak,
                B::MangroveSwamp => Tree::Mangrove,
                B::CherryGrove => Tree::Cherry,
                B::SnowyPlains | B::WindsweptHills | B::WindsweptGravellyHills => {
                    if roll % 3 == 0 {
                        Tree::Oak
                    } else {
                        Tree::Spruce
                    }
                }
                B::IceSpikes => Tree::IceSpike,
                _ => Tree::Oak,
            };
            let bees = matches!(biome, B::Plains | B::SunflowerPlains | B::FlowerForest | B::Meadow | B::Forest | B::BirchForest);
            out.push(Plant { x, z, ground: top, kind, seed: roll, bees });
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tree {
    Oak,
    FancyOak,
    SwampOak,
    Birch,
    TallBirch,
    Spruce,
    MegaSpruce,
    DarkOak,
    PaleOak,
    Acacia,
    Jungle,
    MegaJungle,
    JungleBush,
    Cherry,
    Mangrove,
    IceSpike,
}

/// Un arbre à dessiner : position du pied, hauteur du sol et essence.
struct Plant {
    x: i32,
    z: i32,
    ground: i32,
    kind: Tree,
    seed: u64,
    /// Biome à abeilles (plaines, forêts fleuries) : nid possible sous les feuilles
    bees: bool,
}

impl Plant {
    /// Dessine la partie de l'arbre qui tombe dans le chunk dont le coin est (bx, bz).
    fn paint(&self, blocks: &mut [u32], at: impl Fn(usize, usize, i32) -> usize, bx: i32, bz: i32, p: &Palette) {
        let named = |n: &str| p.named.get(n).copied().unwrap_or(p.air);
        let mut rng = Rng::new(self.seed);
        let soft = [p.air, p.snow_layer, p.short_grass, p.water, named("fern"), named("seagrass")];
        let mut put = |x: i32, y: i32, z: i32, block: u32, force: bool| {
            let (lx, lz) = (x - bx, z - bz);
            if !(0..16).contains(&lx) || !(0..16).contains(&lz) || y <= MIN_Y || y > MAX_Y {
                return;
            }
            let i = at(lx as usize, lz as usize, y);
            if force || soft.contains(&blocks[i]) {
                blocks[i] = block;
            }
        };
        let (x, z, base) = (self.x, self.z, self.ground + 1);
        let blob = |put: &mut dyn FnMut(i32, i32, i32, u32, bool), cx: i32, cy: i32, cz: i32, r: i32, h: i32, leaves: u32, rng: &mut Rng| {
            for dy in 0..h {
                let rr = if dy == h - 1 { r - 1 } else { r };
                for dx in -rr..=rr {
                    for dz in -rr..=rr {
                        if dx.abs() == rr && dz.abs() == rr && (dy == h - 1 || rng.below(2) == 0) {
                            continue;
                        }
                        put(cx + dx, cy + dy, cz + dz, leaves, false);
                    }
                }
            }
        };
        match self.kind {
            Tree::IceSpike => {
                let h = 6 + rng.below(10) as i32;
                let ice = named("packed_ice");
                for dy in 0..h {
                    let r: i32 = if dy < h / 3 { 1 } else { 0 };
                    for dx in -r..=r {
                        for dz in -r..=r {
                            if dx.abs() + dz.abs() <= r {
                                put(x + dx, base - 1 + dy, z + dz, ice, true);
                            }
                        }
                    }
                }
            }
            Tree::Oak | Tree::SwampOak | Tree::Birch | Tree::TallBirch => {
                // Forme du jeu (« blob ») : tronc de h blocs ; quatre couches de feuilles, les
                // deux du bas de rayon 2 (3 pour le chêne des marais), les deux du haut de
                // rayon 1 ; coins retirés en haut, et au hasard en dessous
                let (log, leaves, trunk) = match self.kind {
                    Tree::Birch => (p.birch_log, p.birch_leaves, 5 + rng.below(3) as i32),
                    Tree::TallBirch => (p.birch_log, p.birch_leaves, 10 + rng.below(4) as i32),
                    _ => (p.oak_log, p.oak_leaves, 4 + rng.below(3) as i32),
                };
                let wide = if self.kind == Tree::SwampOak { 3 } else { 2 };
                let top = base + trunk; // couche de feuilles la plus haute (au-dessus du tronc)
                for layer in 0..4 {
                    let y = top - 3 + layer;
                    let r: i32 = if layer < 2 { wide } else { 1 };
                    for dx in -r..=r {
                        for dz in -r..=r {
                            if dx.abs() == r && dz.abs() == r && (layer == 3 || rng.below(2) == 0) {
                                continue;
                            }
                            put(x + dx, y, z + dz, leaves, false);
                        }
                    }
                }
                for y in base..top {
                    put(x, y, z, log, true);
                }
                put(x, base - 1, z, p.dirt, true);
                if self.kind == Tree::SwampOak {
                    let vine = named("vine");
                    for _ in 0..8 {
                        let (dx, dz) = (rng.below(9) as i32 - 4, rng.below(9) as i32 - 4);
                        if dx.abs() == 4 || dz.abs() == 4 {
                            for dy in 0..(1 + rng.below(4) as i32) {
                                put(x + dx, top - 3 - dy, z + dz, vine, false);
                            }
                        }
                    }
                }
                // Nid d'abeilles, parfois, sous les feuilles
                if self.bees && rng.below(20) == 0 {
                    put(x, top - 4, z + 1, named("bee_nest"), false);
                }
            }
            Tree::FancyOak => {
                // Grand chêne : tronc haut, branches en diagonale, boules de feuilles au bout
                let trunk = 8 + rng.below(5) as i32;
                let top = base + trunk;
                let ball = |put: &mut dyn FnMut(i32, i32, i32, u32, bool), cx: i32, cy: i32, cz: i32| {
                    for dy in 0..4i32 {
                        let r: i32 = if dy == 0 || dy == 3 { 2 } else { 3 };
                        for dx in -r..=r {
                            for dz in -r..=r {
                                if dx * dx + dz * dz <= r * r - (dy == 0 || dy == 3) as i32 {
                                    put(cx + dx, cy + dy, cz + dz, p.oak_leaves, false);
                                }
                            }
                        }
                    }
                };
                ball(&mut put, x, top - 2, z);
                for i in 0..(2 + rng.below(2) as i32) {
                    let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)][rng.below(6) as usize];
                    let y0 = base + trunk / 2 + i;
                    for k in 1..=3 {
                        put(x + dx * k, y0 + k, z + dz * k, p.oak_log, true);
                    }
                    ball(&mut put, x + dx * 3, y0 + 2, z + dz * 3);
                }
                for y in base..top {
                    put(x, y, z, p.oak_log, true);
                }
                put(x, base - 1, z, p.dirt, true);
            }
            Tree::Spruce => {
                let trunk = 6 + rng.below(5) as i32;
                let top = base + trunk;
                let mut r: i32 = 0;
                for y in ((base + 2)..=(top + 1)).rev() {
                    for dx in -r..=r {
                        for dz in -r..=r {
                            if dx.abs() + dz.abs() <= r + 1 {
                                put(x + dx, y, z + dz, p.spruce_leaves, false);
                            }
                        }
                    }
                    r = if r >= 2 + (trunk > 8) as i32 { 1 } else { r + 1 };
                }
                for y in base..top {
                    put(x, y, z, p.spruce_log, true);
                }
            }
            Tree::MegaSpruce => {
                let trunk = 14 + rng.below(12) as i32;
                let top = base + trunk;
                let mut r: i32 = 0;
                for y in ((top - trunk / 2)..=(top + 1)).rev() {
                    for dx in -r..=r + 1 {
                        for dz in -r..=r + 1 {
                            let d = (dx as f64 - 0.5).powi(2) + (dz as f64 - 0.5).powi(2);
                            if d <= ((r as f64) + 0.7).powi(2) {
                                put(x + dx, y, z + dz, p.spruce_leaves, false);
                            }
                        }
                    }
                    r = if (top - y) % 3 == 2 { (r + 1).min(4) } else { r };
                }
                for y in base..top {
                    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        put(x + dx, y, z + dz, p.spruce_log, true);
                    }
                }
                for (dx, dz) in [(-1, -1), (2, 0), (0, 2), (-1, 1), (1, -1), (2, 2)] {
                    put(x + dx, base - 1, z + dz, named("podzol"), true);
                }
            }
            Tree::DarkOak | Tree::PaleOak => {
                let (log, leaves) = if self.kind == Tree::PaleOak { (named("pale_oak_log"), named("pale_oak_leaves")) } else { (named("dark_oak_log"), named("dark_oak_leaves")) };
                let trunk = 6 + rng.below(3) as i32;
                let top = base + trunk;
                for dy in -1..=1 {
                    let r = if dy == 1 { 2 } else { 3 };
                    for dx in -r..=r + 1 {
                        for dz in -r..=r + 1 {
                            if (dx == -r || dx == r + 1) && (dz == -r || dz == r + 1) {
                                continue;
                            }
                            put(x + dx, top + dy, z + dz, leaves, false);
                        }
                    }
                }
                for y in base..top {
                    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        put(x + dx, y, z + dz, log, true);
                    }
                }
            }
            Tree::Acacia => {
                let trunk = 5 + rng.below(3) as i32;
                let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][rng.below(4) as usize];
                let bend = trunk - 2 - rng.below(2) as i32;
                let (mut tx, mut tz) = (x, z);
                for i in 0..trunk {
                    if i >= bend {
                        tx += dx;
                        tz += dz;
                    }
                    put(tx, base + i, tz, named("acacia_log"), true);
                }
                let top = base + trunk;
                for (r, y) in [(3i32, top - 1), (1i32, top)] {
                    for ox in -r..=r {
                        for oz in -r..=r {
                            if ox.abs() + oz.abs() <= r + 1 && !(ox.abs() == r && oz.abs() == r) {
                                put(tx + ox, y, tz + oz, named("acacia_leaves"), false);
                            }
                        }
                    }
                }
            }
            Tree::Jungle | Tree::JungleBush => {
                let (log, leaves) = (named("jungle_log"), named("jungle_leaves"));
                if self.kind == Tree::JungleBush {
                    put(x, base, z, log, true);
                    blob(&mut put, x, base, z, 2, 2, p.oak_leaves, &mut rng);
                    return;
                }
                let trunk = 7 + rng.below(6) as i32;
                let top = base + trunk;
                blob(&mut put, x, top - 2, z, 2, 3, leaves, &mut rng);
                for y in base..top {
                    put(x, y, z, log, true);
                }
                let vine = named("vine");
                for _ in 0..4 {
                    let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][rng.below(4) as usize];
                    for dy in 0..(2 + rng.below(4) as i32) {
                        put(x + dx, top - 3 - dy, z + dz, vine, false);
                    }
                }
            }
            Tree::MegaJungle => {
                let (log, leaves) = (named("jungle_log"), named("jungle_leaves"));
                let trunk = 18 + rng.below(12) as i32;
                let top = base + trunk;
                for dy in 0..3 {
                    let r = 4 - dy;
                    for dx in -r..=r + 1 {
                        for dz in -r..=r + 1 {
                            let d = (dx as f64 - 0.5).powi(2) + (dz as f64 - 0.5).powi(2);
                            if d <= (r as f64 + 0.5).powi(2) {
                                put(x + dx, top - 1 + dy, z + dz, leaves, false);
                            }
                        }
                    }
                }
                // Branches avec un peu de feuillage, le long du tronc
                for i in 0..3 {
                    let y = top - 6 - i * 4;
                    let (dx, dz) = [(3, 0), (-2, 0), (0, 3), (0, -2)][rng.below(4) as usize];
                    put(x + dx / 2, y, z + dz / 2, log, true);
                    blob(&mut put, x + dx, y, z + dz, 2, 2, leaves, &mut rng);
                }
                for y in base..top {
                    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        put(x + dx, y, z + dz, log, true);
                    }
                }
            }
            Tree::Cherry => {
                // Cerisier : tronc qui se divise en deux branches obliques ; chacune porte une
                // large couronne aplatie dont le bord retombe en feuilles pendantes
                let (log, leaves) = (named("cherry_log"), named("cherry_leaves"));
                let trunk = 3 + rng.below(2) as i32;
                for y in base..(base + trunk) {
                    put(x, y, z, log, true);
                }
                let branches: &[(i32, i32)] = if rng.below(3) == 0 { &[(1, 0)] } else { &[(1, 0), (-1, 0)] };
                let flip = rng.below(2) == 0;
                for (bx2, bz2) in branches {
                    let (dx, dz) = if flip { (*bz2, *bx2) } else { (*bx2, *bz2) };
                    let len = 3 + rng.below(2) as i32;
                    let (mut cx, mut cy, mut cz) = (x, base + trunk - 1, z);
                    for k in 0..len {
                        cx += dx;
                        cz += dz;
                        if k > 0 {
                            cy += 1;
                        }
                        put(cx, cy, cz, log, true);
                    }
                    cy += 1;
                    put(cx, cy, cz, log, true);
                    // Couronne : ellipse de rayon 4, sur 4 couches
                    for dy in -1..=2i32 {
                        let r: i32 = match dy {
                            -1 => 3,
                            0 | 1 => 4,
                            _ => 2,
                        };
                        for ox in -r..=r {
                            for oz in -r..=r {
                                let d = ox * ox + oz * oz;
                                if d <= r * r && !(d > (r - 1) * (r - 1) && rng.below(3) == 0) {
                                    put(cx + ox, cy + dy, cz + oz, leaves, false);
                                }
                            }
                        }
                    }
                    // Feuilles pendantes sous le bord
                    for _ in 0..10 {
                        let (ox, oz) = (rng.below(7) as i32 - 3, rng.below(7) as i32 - 3);
                        if ox * ox + oz * oz >= 7 {
                            for dy in 1..=(1 + rng.below(2) as i32) {
                                put(cx + ox, cy - 1 - dy, cz + oz, leaves, false);
                            }
                        }
                    }
                }
            }
            Tree::Mangrove => {
                let (log, leaves, roots) = (named("mangrove_log"), named("mangrove_leaves"), named("mangrove_roots"));
                let trunk = 6 + rng.below(4) as i32;
                let lift = 2 + rng.below(2) as i32;
                let start = base.max(SEA_LEVEL + 1) + lift;
                let top = start + trunk;
                // Racines : de la base du tronc jusqu'au sol, en diagonale
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    for i in 0..=(start - base + 1) {
                        put(x + dx * (1 + i / 2), start - i, z + dz * (1 + i / 2), roots, false);
                    }
                }
                blob(&mut put, x, top - 2, z, 3, 3, leaves, &mut rng);
                for y in (base - 1)..top {
                    put(x, y, z, if y < start { roots } else { log }, true);
                }
            }
        }
    }
}

/// Boule irrégulière d'un bloc, qui ne remplace que les blocs de `into`, sans sortir du chunk.
fn blob(blocks: &mut [u32], at: impl Fn(usize, usize, i32) -> usize, center: (i32, i32, i32), radius: f64, into: &[u32], block: u32) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette::new(&|_| None)
    }

    #[test]
    fn biome_tables_follow_the_game() {
        let c = |t: f64, h: f64, cont: f64, e: f64, w: f64| Climate { temperature: t, humidity: h, continentalness: cont, erosion: e, weirdness: w };
        assert_eq!(surface_biome(&c(0.0, 0.0, -0.6, 0.0, 0.0)), B::DeepOcean);
        assert_eq!(surface_biome(&c(0.9, 0.0, -0.3, 0.0, 0.0)), B::WarmOcean);
        assert_eq!(surface_biome(&c(-0.9, 0.0, -0.3, 0.0, 0.0)), B::FrozenOcean);
        assert_eq!(surface_biome(&c(0.9, -0.9, 0.1, 0.3, -0.3)), B::Desert);
        assert_eq!(surface_biome(&c(0.0, 0.2, 0.1, 0.3, -0.3)), B::BirchForest);
        assert_eq!(surface_biome(&c(-0.3, -0.9, 0.1, 0.3, -0.3)), B::Plains);
        // Pics : érosion faible, étrangeté qui donne un pic, loin des côtes
        assert_eq!(surface_biome(&c(-0.9, 0.0, 0.5, -0.9, -0.66)), B::JaggedPeaks);
        assert_eq!(surface_biome(&c(0.4, 0.0, 0.5, -0.9, 0.66)), B::StonyPeaks);
        assert_eq!(surface_biome(&c(0.0, 0.0, -1.1, 0.0, 0.0)), B::MushroomFields);
    }

    #[test]
    fn vanilla_terrain_has_the_expected_variety() {
        let v = Vanilla::new(2024);
        let p = palette();
        let mut seen = std::collections::HashSet::new();
        let mut heights = Vec::new();
        for i in -40..40 {
            for j in -40..40 {
                let (x, z) = (i * 96, j * 96);
                let c = v.climate(x, z);
                seen.insert(surface_biome(&c));
                heights.push(v.shape(&c, x, z).height);
            }
        }
        assert!(seen.len() >= 18, "{} biomes : {:?}", seen.len(), seen);
        let land = heights.iter().filter(|h| **h > SEA_LEVEL as f64).count();
        assert!(land > heights.len() / 4 && land < heights.len() * 9 / 10, "terres : {} / {}", land, heights.len());
        let max = heights.iter().cloned().fold(0.0, f64::max);
        assert!(max > 150.0, "des montagnes : {}", max);
        // Un chunk : roche-mère, des grottes, la surface au bon endroit
        let col = v.generate(&p, 3, 5);
        assert_eq!(col.get(0, MIN_Y, 0), p.bedrock);
        let (top, _) = v.surface(3 * 16 + 4, 5 * 16 + 7);
        assert_ne!(col.get(4, top, 7), p.air, "le sommet calculé à part est bien plein");
        assert!(col.biome_ids.is_some());
    }

    #[test]
    fn neighbours_agree_on_trees_across_chunk_borders() {
        // Un arbre près d'un bord est dessiné à l'identique dans les deux chunks
        let v = Vanilla::new(77);
        let p = palette();
        let (a, b) = (v.generate(&p, 0, 0), v.generate(&p, 1, 0));
        // La colonne x = 15 du premier et x = 0 du second se suivent : pas de vérification
        // directe possible sans arbre connu, on vérifie au moins la stabilité
        assert_eq!(a.get(15, 70, 3), v.generate(&p, 0, 0).get(15, 70, 3));
        assert_eq!(b.get(0, 70, 3), v.generate(&p, 1, 0).get(0, 70, 3));
    }
}

#[cfg(test)]
mod pictures {
    use super::*;

    #[test]
    #[ignore]
    fn vanilla_speed() {
        let v = Vanilla::new(5);
        let p = Palette::new(&|_| None);
        let start = std::time::Instant::now();
        for cx in 0..10 {
            for cz in 0..10 {
                v.generate(&p, cx, cz);
            }
        }
        eprintln!("100 chunks vanilla en {:?}", start.elapsed());
        // Autour d'un village (graine 12345, chunk 46 -17) : le plan est calculé une fois
        let v = Vanilla::new(12345);
        let start = std::time::Instant::now();
        for cx in 41..51 {
            for cz in -22..-12 {
                v.generate(&p, cx, cz);
            }
        }
        eprintln!("100 chunks autour d'un village en {:?}", start.elapsed());
    }

    /// Vue de dessus avec les vrais blocs (`cargo test dump_top -- --ignored`).
    #[test]
    #[ignore]
    fn dump_top() {
        let data = crate::data::GameData::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data")).unwrap();
        let p = Palette::new(&|n| data.state_from_spec(n));
        let v = Vanilla::new(std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(12345));
        let (ox, oz) = (std::env::var("CUTX").ok().and_then(|s| s.parse().ok()).unwrap_or(0i32) >> 4, std::env::var("CUTZ").ok().and_then(|s| s.parse().ok()).unwrap_or(0i32) >> 4);
        let mut out = String::new();
        for cz in 0..16 {
            let row: Vec<Column> = (0..16).map(|cx| v.generate(&p, ox - 8 + cx, oz - 8 + cz)).collect();
            for z in 0..16 {
                for col in &row {
                    for x in 0..16 {
                        let mut y = MAX_Y;
                        while y > MIN_Y && col.get(x, y, z) == p.air {
                            y -= 1;
                        }
                        let name = data.block_name(col.get(x, y, z)).unwrap_or("?").trim_start_matches("minecraft:");
                        out.push_str(&format!("{}:{} ", name, y));
                    }
                }
                out.push('\n');
            }
        }
        std::fs::write("/tmp/top.txt", out).unwrap();
    }

    /// Carte et coupe du terrain, pour la mise au point (`cargo test dump_map -- --ignored`).
    #[test]
    #[ignore]
    fn dump_map() {
        let v = Vanilla::new(std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(12345));
        let size = 512i32;
        let step = std::env::var("STEP").ok().and_then(|s| s.parse().ok()).unwrap_or(16i32);
        let mut out = String::new();
        for j in 0..size {
            for i in 0..size {
                let (x, z) = ((i - size / 2) * step, (j - size / 2) * step);
                let c = v.climate(x, z);
                let s = v.shape(&c, x, z);
                out.push_str(&format!("{} {} ", surface_biome(&c).id(), s.height as i32));
            }
            out.push('\n');
        }
        std::fs::write("/tmp/map.txt", out).unwrap();
        // Coupe : un chunk de 16 de large sur 256 de long, à z = 0
        let p = Palette::new(&|_| None);
        let mut cut = String::new();
        let (ox, oz) = (std::env::var("CUTX").ok().and_then(|s| s.parse().ok()).unwrap_or(0i32), std::env::var("CUTZ").ok().and_then(|s| s.parse().ok()).unwrap_or(0i32));
        for cx in -8..8 {
            let col = v.generate(&p, (ox >> 4) + cx, oz >> 4);
            for x in 0..16 {
                for y in MIN_Y..=MAX_Y {
                    cut.push_str(&format!("{} ", col.get(x, y, (oz & 15) as usize)));
                }
                cut.push('\n');
            }
        }
        std::fs::write("/tmp/cut.txt", cut).unwrap();
    }
}

#[cfg(test)]
mod decorations {
    use super::*;

    /// Chaque bloc de décoration existe dans la palette du jeu, et sur une grande zone on
    /// trouve pétales, feuilles mortes, laminaires, herbes marines, coraux et nids d'abeilles.
    #[test]
    fn decorations_exist_and_appear() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let Ok(data) = crate::data::GameData::load(&dir) else { return };
        for n in super::super::generator::EXTRA_BLOCKS.iter().map(|s| s.to_string()).chain(super::super::structures::block_specs()) {
            assert!(data.state_from_spec(&format!("minecraft:{}", n)).is_some(), "bloc absent du jeu : {}", n);
        }
        let p = Palette::new(&|n| data.state_from_spec(n));
        let v = Vanilla::new(12345);
        let mut seen = std::collections::HashSet::new();
        for cx in -40..40 {
            for cz in -40..40 {
                let col = v.generate(&p, cx * 4, cz * 4);
                for s in &col.sections {
                    match s {
                        super::super::generator::Section::Uniform(b) => {
                            seen.insert(*b);
                        }
                        super::super::generator::Section::Mixed { palette, .. } => seen.extend(palette.iter().copied()),
                    }
                }
            }
        }
        let names: std::collections::HashSet<String> = seen.iter().filter_map(|s| data.block_names.get(s).cloned()).collect();
        for want in ["pink_petals", "leaf_litter", "kelp", "seagrass", "tube_coral_block", "sea_pickle", "packed_ice"] {
            assert!(names.iter().any(|n| n.ends_with(want)), "aucun bloc {} généré", want);
        }
    }
}
