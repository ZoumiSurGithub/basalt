//! Créatures : les animaux paisibles (vache, cochon, mouton, poule, lapin).
//!
//! Comme dans le jeu, les animaux apparaissent quand une terre est découverte : à la
//! première visite d'un chunk, un petit troupeau s'y installe parfois, selon le biome. Ils
//! sont enregistrés avec leur région (`entities/`, voir `mob_store`) : seuls ceux qui sont
//! près d'un joueur sont en mémoire, et ils ne bougent qu'à la distance de simulation
//! (`tick-distance`). Ils se promènent, sautent les marches, évitent les chutes, nagent, suivent le
//! joueur qui tient leur nourriture, fuient quand on les frappe, meurent et laissent leur
//! butin (le jeu joue lui-même leurs cris et leurs bruits). Nourris, deux adultes font un petit, qui grandit en vingt minutes. Les moutons se
//! tondent et refont leur laine en broutant, les vaches se traient, les poules pondent.

use super::*;
use crate::world::generator::Rng;
use crate::world::vanilla::B;

/// Espèces connues.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Species {
    Cow,
    Pig,
    Sheep,
    Chicken,
    Rabbit,
    Mooshroom,
    Goat,
    Llama,
    Horse,
    Donkey,
    Mule,
}

impl Species {
    const FARM: [Species; 4] = [Species::Cow, Species::Pig, Species::Sheep, Species::Chicken];
    const ALL: [Species; 11] = [
        Species::Cow,
        Species::Pig,
        Species::Sheep,
        Species::Chicken,
        Species::Rabbit,
        Species::Mooshroom,
        Species::Goat,
        Species::Llama,
        Species::Horse,
        Species::Donkey,
        Species::Mule,
    ];

    fn id(self) -> &'static str {
        match self {
            Species::Cow => "minecraft:cow",
            Species::Pig => "minecraft:pig",
            Species::Sheep => "minecraft:sheep",
            Species::Chicken => "minecraft:chicken",
            Species::Rabbit => "minecraft:rabbit",
            Species::Mooshroom => "minecraft:mooshroom",
            Species::Goat => "minecraft:goat",
            Species::Llama => "minecraft:llama",
            Species::Horse => "minecraft:horse",
            Species::Donkey => "minecraft:donkey",
            Species::Mule => "minecraft:mule",
        }
    }
    fn from_id(s: &str) -> Option<Species> {
        Species::ALL.into_iter().find(|k| k.id() == s)
    }
    /// Équidés et lamas : vie tirée au sort, apprivoisés avant de se reproduire
    fn mount(self) -> bool {
        matches!(self, Species::Llama | Species::Horse | Species::Donkey | Species::Mule)
    }
    /// Se dirige au clavier une fois apprivoisée et sellée (pas le lama)
    fn steerable(self) -> bool {
        matches!(self, Species::Horse | Species::Donkey | Species::Mule)
    }
    /// Patience maximale au dressage (« max_temper » du jeu)
    fn max_temper(self) -> u32 {
        if self == Species::Llama {
            30
        } else {
            100
        }
    }
    /// Ce qui l'amadoue pendant le dressage : (objet, patience gagnée)
    fn temper_items(self) -> &'static [(&'static str, u32)] {
        match self {
            Species::Horse | Species::Donkey | Species::Mule => &[("wheat", 3), ("sugar", 3), ("apple", 3), ("golden_carrot", 5), ("golden_apple", 10), ("enchanted_golden_apple", 10)],
            Species::Llama => &[("wheat", 3), ("hay_block", 6)],
            _ => &[],
        }
    }
    /// Position du cavalier (ses yeux) par rapport à la monture : le siège du jeu (1,1 bloc pour
    /// le cheval) plus la hauteur des yeux du joueur assis
    fn seat(self) -> (f32, f32, f32) {
        match self {
            Species::Donkey | Species::Mule => (0.0, 2.175, -0.2),
            Species::Llama => (0.0, 2.37, -0.3),
            _ => (0.0, 2.3, -0.2),
        }
    }
    /// Vie maximale (celle du jeu ; les montures tirent la leur entre 15 et 30)
    fn health(self) -> f32 {
        match self {
            Species::Cow | Species::Pig | Species::Mooshroom | Species::Goat => 10.0,
            Species::Sheep => 8.0,
            Species::Chicken => 4.0,
            Species::Rabbit => 3.0,
            Species::Llama | Species::Horse | Species::Donkey | Species::Mule => 30.0,
        }
    }
    /// Largeur et hauteur de la boîte de collision d'un adulte (celles du jeu)
    fn size(self) -> (f32, f32) {
        match self {
            Species::Cow | Species::Mooshroom | Species::Goat => (0.9, 1.3),
            Species::Pig => (0.9, 0.9),
            Species::Sheep => (0.9, 1.3),
            Species::Chicken => (0.4, 0.7),
            Species::Rabbit => (0.4, 0.5),
            Species::Llama => (0.9, 1.87),
            Species::Horse | Species::Donkey | Species::Mule => (1.4, 1.6),
        }
    }
    /// Taille d'affichage (« minecraft:scale » des fichiers du jeu) : le lapin est un petit
    /// modèle réduit (0,6 adulte, 0,4 bébé), les autres bébés font moitié.
    fn scale(self, baby: bool) -> f32 {
        match (self, baby) {
            (Species::Rabbit, false) => 0.6,
            (Species::Rabbit, true) => 0.4,
            (_, true) => 0.5,
            (_, false) => 1.0,
        }
    }
    /// Vitesse de marche (blocs par tick)
    fn speed(self) -> f32 {
        match self {
            Species::Chicken => 0.07,
            Species::Rabbit => 0.12,
            Species::Goat | Species::Horse => 0.1,
            _ => 0.09,
        }
    }
    /// Ce qui l'attire : il suit le joueur qui le tient
    fn tempt(self) -> &'static [&'static str] {
        match self {
            Species::Cow | Species::Sheep | Species::Mooshroom | Species::Goat => &["wheat"],
            Species::Pig => &["carrot", "potato", "beetroot"],
            Species::Chicken => &["wheat_seeds", "pumpkin_seeds", "melon_seeds", "beetroot_seeds", "torchflower_seeds", "pitcher_pod"],
            Species::Rabbit => &["carrot", "golden_carrot", "dandelion"],
            Species::Llama => &["hay_block"],
            Species::Horse | Species::Donkey | Species::Mule => &[],
        }
    }
    /// Ce qui le rend amoureux (les montures doivent d'abord être apprivoisées ; la mule
    /// ne se reproduit pas)
    fn breed_items(self) -> &'static [&'static str] {
        match self {
            Species::Llama => &["hay_block"],
            Species::Horse | Species::Donkey => &["golden_carrot", "golden_apple", "enchanted_golden_apple"],
            Species::Mule => &[],
            other => other.tempt(),
        }
    }
    /// Ce qui fait grandir un petit : (objet, part de la croissance totale)
    fn grow_items(self) -> &'static [(&'static str, f32)] {
        const EQUINE: [(&str, f32); 7] = [("wheat", 0.016667), ("sugar", 0.025), ("hay_block", 0.15), ("apple", 0.05), ("golden_carrot", 0.05), ("golden_apple", 0.2), ("enchanted_golden_apple", 0.2)];
        match self {
            Species::Horse | Species::Donkey | Species::Mule => &EQUINE,
            Species::Llama => &[("wheat", 0.1), ("hay_block", 0.9)],
            Species::Cow | Species::Sheep | Species::Mooshroom | Species::Goat => &[("wheat", 0.1)],
            Species::Pig => &[("carrot", 0.1), ("potato", 0.1), ("beetroot", 0.1)],
            Species::Chicken => &[("wheat_seeds", 0.1), ("pumpkin_seeds", 0.1), ("melon_seeds", 0.1), ("beetroot_seeds", 0.1), ("torchflower_seeds", 0.1), ("pitcher_pod", 0.1)],
            Species::Rabbit => &[("carrot", 0.1), ("golden_carrot", 0.1), ("dandelion", 0.1)],
        }
    }
    /// Ce qui le soigne : (objet, points de vie rendus)
    fn heal_items(self) -> &'static [(&'static str, f32)] {
        const EQUINE: [(&str, f32); 7] = [("wheat", 2.0), ("sugar", 1.0), ("hay_block", 20.0), ("apple", 3.0), ("golden_carrot", 4.0), ("golden_apple", 10.0), ("enchanted_golden_apple", 10.0)];
        match self {
            Species::Horse | Species::Donkey | Species::Mule => &EQUINE,
            Species::Llama => &[("wheat", 2.0), ("hay_block", 10.0)],
            _ => &[],
        }
    }
    /// Butin : (objet, quantité minimale, maximale)
    fn drops(self) -> &'static [(&'static str, u8, u8)] {
        match self {
            Species::Cow | Species::Mooshroom => &[("minecraft:beef", 1, 3), ("minecraft:leather", 0, 2)],
            Species::Pig => &[("minecraft:porkchop", 1, 3)],
            Species::Sheep => &[("minecraft:mutton", 1, 2), ("minecraft:white_wool", 1, 1)],
            Species::Chicken => &[("minecraft:chicken", 1, 1), ("minecraft:feather", 0, 2)],
            Species::Rabbit => &[("minecraft:rabbit", 0, 1), ("minecraft:rabbit_hide", 0, 1)],
            Species::Llama | Species::Horse | Species::Donkey | Species::Mule => &[("minecraft:leather", 0, 2)],
            Species::Goat => &[],
        }
    }
}

/// Un petit met 20 minutes à grandir ; un adulte attend 5 minutes avant un nouveau petit
const GROW_TICKS: i64 = 24_000;
const BREED_COOLDOWN: i64 = 6_000;
/// Durée de l'envie de se reproduire après un repas
const LOVE_TICKS: u32 = 600;

/// Une créature dans le monde.
#[derive(Clone)]
pub(super) struct Mob {
    pub id: i64,
    pub species: Species,
    /// Position des pieds
    pub pos: (f32, f32, f32),
    pub vel: (f32, f32, f32),
    pub yaw: f32,
    pub on_ground: bool,
    pub health: f32,
    /// Âge : négatif pour un petit (ticks avant d'être adulte), positif pour un adulte qui
    /// vient d'avoir un petit (ticks avant le suivant), 0 sinon
    age: i64,
    /// Ticks d'envie de se reproduire
    love: u32,
    /// Mouton tondu
    sheared: bool,
    /// Destination de la promenade en cours
    target: Option<(f32, f32)>,
    /// Ticks de fuite restants (après un coup)
    panic: u32,
    /// Ticks de recul restants (après un coup)
    knock: u32,
    /// Tick du dernier coup reçu (un coup par demi-seconde)
    hurt_tick: u64,
    /// Tick de la mort (retirée une seconde plus tard)
    pub dead: Option<u64>,
    /// Dernière position et orientation envoyées
    sent: (f32, f32, f32),
    sent_yaw: f32,
    /// Couleur de la laine (mouton)
    color: u8,
    /// Variante (pelage du lapin)
    variant: i32,
    /// Ticks avant le prochain œuf (poule)
    egg: u32,
    /// Vie maximale (tirée au sort pour les montures)
    max_health: f32,
    /// Motif de la robe (cheval)
    mark: i32,
    /// Apprivoisé (montures)
    tamed: bool,
    /// Qualités d'une monture, tirées à sa naissance : vitesse, saut, force (lama)
    move_speed: f32,
    jump: f32,
    strength: u8,
    /// Sellée (monture)
    saddled: bool,
    /// Patience gagnée au dressage (chaque essai raté la fait monter)
    temper: u32,
    /// Joueur en selle
    pub rider: Option<SocketAddr>,
    /// Ticks avant l'essai de dressage (cavalier sur une monture sauvage)
    tame_in: u32,
    /// Tick du dernier déplacement imposé par le cavalier
    driven: u64,
}

impl Mob {
    fn baby(&self) -> bool {
        self.age < 0
    }
    fn flags(&self) -> i64 {
        let mut f = bedrock::MOB_FLAGS;
        if self.baby() {
            f |= bedrock::FLAG_BABY;
        }
        if self.love > 0 {
            f |= bedrock::FLAG_IN_LOVE;
        }
        if self.sheared {
            f |= bedrock::FLAG_SHEARED;
        }
        if self.tamed {
            f |= bedrock::FLAG_TAMED;
        }
        if self.saddled {
            f |= bedrock::FLAG_SADDLED;
            // Apprivoisée et sellée : son cavalier la dirige (saut chargé du cheval)
            if self.tamed && self.species.steerable() {
                f |= bedrock::FLAG_WASD_CONTROLLED | bedrock::FLAG_CAN_POWER_JUMP;
            }
        }
        f
    }
    /// Dirigée par son cavalier (le jeu de celui-ci la déplace)
    fn driven_by_rider(&self) -> bool {
        self.rider.is_some() && self.tamed && self.saddled && self.species.steerable()
    }
    fn size(&self) -> (f32, f32) {
        let (w, h) = self.species.size();
        if self.baby() {
            (w / 2.0, h / 2.0)
        } else {
            (w, h)
        }
    }
}

/// Au plus tant de créatures en mémoire à la fois (au-delà, les terres neuves n'en reçoivent
/// plus ; celles qui sont enregistrées reviennent toujours)
const MAX_MOBS: usize = 4000;
/// Rayon d'affichage
const VIEW: f32 = 80.0;
/// Chunks examinés autour de chaque joueur pour y installer des animaux
const POPULATE_RADIUS: i32 = 4;
/// Ce qu'on enregistre d'une créature.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct MobRecord {
    pub species: Species,
    pub pos: (f32, f32, f32),
    pub yaw: f32,
    pub health: f32,
    pub age: i64,
    pub color: u8,
    pub variant: i32,
    pub sheared: bool,
    pub max_health: f32,
    pub mark: i32,
    pub tamed: bool,
    pub move_speed: f32,
    pub jump: f32,
    pub strength: u8,
    pub saddled: bool,
    pub temper: u32,
}

impl MobRecord {
    pub fn to_json(&self) -> Json {
        let n = |v: f32| Json::Num((v as f64 * 100.0).round() / 100.0);
        Json::Obj(vec![
            ("kind".into(), Json::Str(self.species.id().into())),
            ("x".into(), n(self.pos.0)),
            ("y".into(), n(self.pos.1)),
            ("z".into(), n(self.pos.2)),
            ("yaw".into(), n(self.yaw)),
            ("health".into(), n(self.health)),
            ("age".into(), Json::Num(self.age as f64)),
            ("color".into(), Json::Num(self.color as f64)),
            ("variant".into(), Json::Num(self.variant as f64)),
            ("sheared".into(), Json::Bool(self.sheared)),
            ("max_health".into(), n(self.max_health)),
            ("mark".into(), Json::Num(self.mark as f64)),
            ("tamed".into(), Json::Bool(self.tamed)),
            ("speed".into(), Json::Num((self.move_speed as f64 * 10000.0).round() / 10000.0)),
            ("jump".into(), Json::Num((self.jump as f64 * 10000.0).round() / 10000.0)),
            ("strength".into(), Json::Num(self.strength as f64)),
            ("saddled".into(), Json::Bool(self.saddled)),
            ("temper".into(), Json::Num(self.temper as f64)),
        ])
    }

    pub fn from_json(v: &Json) -> Option<MobRecord> {
        let species = v.get("kind").and_then(|k| k.as_str()).and_then(Species::from_id)?;
        let num = |k: &str| v.get(k).and_then(|n| n.as_f64()).filter(|n| n.abs() < 3.0e7);
        let max_health = (num("max_health").unwrap_or(species.health() as f64) as f32).clamp(1.0, species.health());
        Some(MobRecord {
            species,
            pos: (num("x")? as f32, num("y")? as f32, num("z")? as f32),
            yaw: num("yaw").unwrap_or(0.0) as f32,
            health: (num("health").unwrap_or(max_health as f64) as f32).clamp(0.5, max_health),
            max_health,
            mark: num("mark").unwrap_or(0.0) as i32,
            tamed: v.get("tamed").and_then(|s| s.as_bool()).unwrap_or(false),
            move_speed: num("speed").unwrap_or(0.0) as f32,
            jump: num("jump").unwrap_or(0.0) as f32,
            strength: num("strength").unwrap_or(0.0).clamp(0.0, 5.0) as u8,
            saddled: v.get("saddled").and_then(|s| s.as_bool()).unwrap_or(false),
            temper: num("temper").unwrap_or(0.0).clamp(0.0, 100.0) as u32,
            age: num("age").unwrap_or(0.0) as i64,
            color: num("color").unwrap_or(0.0) as u8 & 15,
            variant: num("variant").unwrap_or(0.0) as i32,
            sheared: v.get("sheared").and_then(|s| s.as_bool()).unwrap_or(false),
        })
    }
}

impl Mob {
    fn record(&self) -> MobRecord {
        MobRecord {
            species: self.species,
            pos: self.pos,
            yaw: self.yaw,
            health: self.health,
            age: self.age,
            color: self.color,
            variant: self.variant,
            sheared: self.sheared,
            max_health: self.max_health,
            mark: self.mark,
            tamed: self.tamed,
            move_speed: self.move_speed,
            jump: self.jump,
            strength: self.strength,
            saddled: self.saddled,
            temper: self.temper,
        }
    }
}

impl Server {
    /// Installe des animaux dans les chunks qu'un joueur découvre (une fois par chunk, comme
    /// à la génération dans le jeu : un chunk sur dix environ reçoit un troupeau).
    pub(super) fn populate_chunks(&mut self) {
        if !self.cfg.spawn_animals || self.mobs_refused {
            return;
        }
        let players: Vec<(f32, f32, f32)> = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2)).collect();
        // Quelques chunks par passage au plus : un joueur qui arrive dans une région neuve (ou
        // qui se connecte) ne provoque pas d'à-coup
        let mut budget = 12;
        for p in players {
            let (pcx, pcz) = ((p.0.floor() as i32) >> 4, (p.2.floor() as i32) >> 4);
            for cx in pcx - POPULATE_RADIUS..=pcx + POPULATE_RADIUS {
                for cz in pcz - POPULATE_RADIUS..=pcz + POPULATE_RADIUS {
                    // Les animaux déjà enregistrés de ce chunk doivent être là avant de décider
                    if !self.mob_chunks.contains(&(cx, cz)) || !self.world.is_ready(cx, cz) {
                        continue;
                    }
                    let Some(region) = self.mob_store.regions.get_mut(&mob_store::region_of((cx, cz))) else { continue };
                    if region.populated.contains(&(cx, cz)) {
                        continue;
                    }
                    if budget == 0 {
                        return;
                    }
                    budget -= 1;
                    region.populated.insert((cx, cz));
                    region.dirty = true;
                    if self.mobs.len() < MAX_MOBS && self.mob_rng.below(10) == 0 {
                        self.spawn_herd(cx, cz, p.1 as i32);
                    }
                }
            }
        }
    }

    /// Un troupeau dans ce chunk, de l'espèce qui convient au biome (règles d'apparition du
    /// jeu : chevaux et ânes dans les plaines, lamas sur les collines venteuses et les plateaux
    /// de savane, chèvres sur les sommets, champimeuhs dans les champs de champignons).
    fn spawn_herd(&mut self, cx: i32, cz: i32, near_y: i32) {
        let (x, z) = (cx * 16 + self.mob_rng.below(16) as i32, cz * 16 + self.mob_rng.below(16) as i32);
        let biome = self.world.generator().vanilla().map(|v| v.surface_biome_at(x, z));
        let roll = self.mob_rng.below(10);
        let farm = Species::FARM[self.mob_rng.below(4) as usize];
        let species = match biome {
            None => farm,
            Some(b) => match b {
                B::MushroomFields => Species::Mooshroom,
                B::JaggedPeaks | B::FrozenPeaks => Species::Goat,
                B::SnowySlopes => {
                    if roll < 5 {
                        Species::Goat
                    } else {
                        Species::Rabbit
                    }
                }
                B::Desert | B::SnowyPlains | B::IceSpikes | B::Grove => Species::Rabbit,
                B::Plains | B::SunflowerPlains => match roll {
                    0 | 1 => Species::Horse,
                    2 => Species::Donkey,
                    _ => farm,
                },
                B::Savanna => match roll {
                    0 | 1 => Species::Horse,
                    2 => Species::Donkey,
                    _ => farm,
                },
                B::SavannaPlateau | B::WindsweptHills | B::WindsweptGravellyHills | B::WindsweptForest => {
                    if roll < 3 {
                        Species::Llama
                    } else {
                        farm
                    }
                }
                B::Meadow => {
                    if roll < 2 {
                        Species::Donkey
                    } else {
                        farm
                    }
                }
                B::Forest | B::BirchForest | B::OldGrowthBirchForest | B::DarkForest | B::WindsweptSavanna | B::Jungle | B::SparseJungle | B::BambooJungle | B::CherryGrove => farm,
                B::FlowerForest | B::Taiga | B::SnowyTaiga | B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga => {
                    if self.mob_rng.below(4) == 0 {
                        Species::Rabbit
                    } else {
                        farm
                    }
                }
                _ => return,
            },
        };
        // Taille du troupeau, part de petits et pelage commun (règles du jeu)
        let (lo, hi, babies) = match species {
            Species::Rabbit => (2, 3, 0),
            Species::Mooshroom => (4, 8, 5),
            Species::Goat => (1, 3, 5),
            Species::Llama => (4, 6, 10),
            Species::Horse => (2, 6, 20),
            Species::Donkey => (1, 3, 20),
            _ => (2, 4, 0),
        };
        let herd_variant = match species {
            Species::Rabbit => match biome {
                Some(B::Desert) => 4,
                Some(B::SnowyPlains | B::IceSpikes | B::SnowySlopes | B::Grove | B::SnowyTaiga) => {
                    if self.mob_rng.below(5) == 0 {
                        3
                    } else {
                        1
                    }
                }
                _ => [0, 0, 2, 5][self.mob_rng.below(4) as usize],
            },
            // Robe du cheval (7 couleurs), lama (crème, blanc, marron, gris)
            Species::Horse => self.mob_rng.below(7) as i32,
            Species::Llama => self.mob_rng.below(4) as i32,
            _ => 0,
        };
        let count = lo + self.mob_rng.below(hi - lo + 1);
        let g = &self.world.generator().blocks;
        let (grass, sand, snow, snow_layer) = (g.grass, g.sand, g.snow, g.snow_layer);
        for _ in 0..count {
            let (dx, dz) = (self.mob_rng.below(5) as i32 - 2, self.mob_rng.below(5) as i32 - 2);
            let (bx, bz) = (x + dx, z + dz);
            let Some(y) = self.surface_y(bx, bz, near_y) else { continue };
            let below = self.mob_get(bx, y - 1, bz);
            let ok = match species {
                Species::Rabbit => below == grass || below == sand || below == snow || below == snow_layer,
                // Sommets, collines et champs de champignons : tout sol solide
                Species::Goat | Species::Llama | Species::Mooshroom => true,
                _ => below == grass,
            };
            if !ok {
                continue;
            }
            let baby = babies > 0 && self.mob_rng.below(100) < babies;
            let mark = if species == Species::Horse { self.mob_rng.below(5) as i32 } else { 0 };
            // Chèvre « hurleuse » : 2 sur 100
            let variant = if species == Species::Goat { (self.mob_rng.below(100) < 2) as i32 } else { herd_variant };
            let id = self.add_mob(species, (bx as f32 + 0.5, y as f32, bz as f32 + 0.5));
            if let Some(m) = self.mobs.get_mut(&id) {
                m.variant = variant;
                m.mark = mark;
                if baby {
                    m.age = -GROW_TICKS;
                }
            }
        }
    }

    /// Hauteur où se tenir au sommet de cette colonne (deux blocs libres sur un sol solide),
    /// cherchée dans les 64 blocs autour de `near` (la hauteur du joueur).
    fn surface_y(&self, x: i32, z: i32, near: i32) -> Option<i32> {
        let water = self.world.generator().blocks.water;
        let mut y = (near + 64).min(crate::world::MAX_Y - 2);
        let bottom = (near - 64).max(crate::world::MIN_Y + 1);
        while y > bottom {
            if !self.mob_free(x, y - 1, z) {
                if self.mob_get(x, y - 1, z) == water || !self.mob_free(x, y, z) || !self.mob_free(x, y + 1, z) {
                    return None;
                }
                return Some(y);
            }
            if self.mob_get(x, y - 1, z) == water {
                return None; // eau en surface
            }
            y -= 1;
        }
        None
    }

    /// Bloc à une position, sans jamais faire générer de terrain dans le tick : un chunk pas
    /// encore prêt compte comme un mur de pierre (la génération d'un chunk, surtout s'il
    /// contient une grande structure, prend jusqu'à plusieurs dixièmes de seconde).
    fn mob_get(&self, x: i32, y: i32, z: i32) -> u32 {
        if self.world.is_ready(x >> 4, z >> 4) {
            self.world.get(x, y, z)
        } else {
            self.world.generator().blocks.stone
        }
    }

    /// Un bloc que l'on traverse (air, herbe haute, fleur, eau…).
    fn mob_free(&self, x: i32, y: i32, z: i32) -> bool {
        let s = self.mob_get(x, y, z);
        s == self.world.generator().blocks.water || self.npc_passable(s)
    }

    fn mob_water(&self, x: f32, y: f32, z: f32) -> bool {
        self.mob_get(x.floor() as i32, y.floor() as i32, z.floor() as i32) == self.world.generator().blocks.water
    }

    /// Ajoute une créature adulte.
    pub(super) fn add_mob(&mut self, species: Species, pos: (f32, f32, f32)) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        let color = if species == Species::Sheep {
            // Laine : surtout blanche, parfois grise, noire ou marron, rarement rose (le jeu)
            match self.mob_rng.below(1000) {
                0..=49 => 7,
                50..=99 => 8,
                100..=149 => 15,
                150..=179 => 12,
                180..=181 => 6,
                _ => 0,
            }
        } else {
            0
        };
        let yaw = self.mob_rng.unit() as f32 * 360.0;
        let egg = 6000 + self.mob_rng.below(6000);
        // Montures : vie, vitesse, saut et force tirés au sort comme dans le jeu
        let r = |rng: &mut Rng, lo: f32, hi: f32| lo + rng.unit() as f32 * (hi - lo);
        let max_health = if species.mount() { (15 + self.mob_rng.below(16)) as f32 } else { species.health() };
        let (move_speed, jump, strength) = match species {
            Species::Horse => (r(&mut self.mob_rng, 0.1125, 0.3375), r(&mut self.mob_rng, 0.4, 1.0), 0),
            Species::Donkey | Species::Mule => (0.175, 0.5, 0),
            Species::Llama => (0.25, 0.0, 1 + self.mob_rng.below(5) as u8),
            _ => (0.0, 0.0, 0),
        };
        let mob = Mob {
            id,
            species,
            pos,
            vel: (0.0, 0.0, 0.0),
            yaw,
            on_ground: true,
            health: max_health,
            age: 0,
            love: 0,
            sheared: false,
            target: None,
            panic: 0,
            knock: 0,
            hurt_tick: 0,
            dead: None,
            sent: pos,
            sent_yaw: yaw,
            color,
            variant: 0,
            egg,
            max_health,
            mark: 0,
            tamed: false,
            move_speed,
            jump,
            strength,
            saddled: false,
            temper: 0,
            rider: None,
            tame_in: 0,
            driven: 0,
        };
        self.mobs.insert(id, mob);
        self.mobs_dirty = true;
        id
    }

    /// Données d'entité d'une créature (taille, couleur, bébé, tondu…).
    fn mob_meta(m: &Mob) -> Vec<(u32, bedrock::Meta)> {
        let (w, h) = m.size();
        let mut meta = bedrock::mob_metadata(w, h, m.variant, m.color, m.flags());
        let scale = m.species.scale(m.baby());
        for (k, v) in meta.iter_mut() {
            match *k {
                38 => *v = bedrock::Meta::Float(scale),
                // Motif de la robe du cheval
                43 => *v = bedrock::Meta::Int(m.mark),
                _ => {}
            }
        }
        meta
    }

    /// Paquet d'apparition d'une créature.
    fn mob_packet(m: &Mob, rider: Option<i64>) -> Vec<u8> {
        bedrock::add_actor(m.id, m.species.id(), m.pos, m.vel, m.yaw, (m.health.max(0.5), m.max_health), m.species.mount().then_some((m.move_speed, m.jump)), rider, &Self::mob_meta(m))
    }

    /// Envoie aux joueurs qui la voient l'état changé d'une créature (bébé devenu adulte,
    /// tondu, amoureux).
    fn mob_changed(&mut self, id: i64) {
        let Some(m) = self.mobs.get(&id) else { return };
        let scale = m.species.scale(m.baby());
        let (w, h) = m.size();
        let packet = bedrock::set_actor_data(id, &[(0, bedrock::Meta::Long(m.flags())), (38, bedrock::Meta::Float(scale)), (53, bedrock::Meta::Float(w)), (54, bedrock::Meta::Float(h))]);
        self.send_to_viewers(id, bedrock::ID_SET_ACTOR_DATA, &packet);
        self.mobs_dirty = true;
    }

    /// Montre à chaque joueur les créatures proches de lui et retire les autres.
    pub(super) fn show_mobs(&mut self) {
        let mut out: Vec<(SocketAddr, u32, Vec<u8>)> = Vec::new();
        let ids: HashMap<SocketAddr, i64> = self.clients.iter().map(|(a, c)| (*a, c.id)).collect();
        for (addr, c) in self.clients.iter_mut() {
            if !c.joined || c.stage != Stage::InGame {
                continue;
            }
            for m in self.mobs.values() {
                let far = (m.pos.0 - c.pos.0).powi(2) + (m.pos.2 - c.pos.2).powi(2);
                // Le jeu oublie une créature arrivée avant son chunk : elle attend qu'il soit parti
                let chunk = ((m.pos.0.floor() as i32) >> 4, (m.pos.2.floor() as i32) >> 4);
                let has_chunk = c.delivered.contains(&chunk);
                if far < VIEW * VIEW && m.dead.is_none() && has_chunk {
                    if c.mobs_seen.insert(m.id) {
                        out.push((*addr, bedrock::ID_ADD_ACTOR, Self::mob_packet(m, m.rider.and_then(|r| ids.get(&r).copied()))));
                    }
                } else if (far > (VIEW + 16.0) * (VIEW + 16.0) || !has_chunk) && c.mobs_seen.remove(&m.id) {
                    out.push((*addr, bedrock::ID_REMOVE_ACTOR, bedrock::remove_actor(m.id)));
                }
            }
        }
        for (addr, id, packet) in out {
            self.send(&addr, id, &packet);
        }
    }

    /// Retire une créature (chez tous les joueurs qui la voient).
    pub(super) fn remove_mob(&mut self, id: i64) {
        self.mobs.remove(&id);
        self.mobs_dirty = true;
        let viewers: Vec<SocketAddr> = self.clients.iter_mut().filter_map(|(a, c)| c.mobs_seen.remove(&id).then_some(*a)).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id));
        }
    }

    /// Envoie un paquet aux joueurs qui voient cette créature.
    fn send_to_viewers(&mut self, id: i64, packet_id: u32, packet: &[u8]) {
        if self.refused_extras.contains(&packet_id) {
            return;
        }
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.mobs_seen.contains(&id)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, packet_id, packet);
        }
    }

    /// Un tick de vie des créatures proches des joueurs : promenade, nourriture, amours,
    /// fuite, gravité, collisions.
    pub(super) fn move_mobs(&mut self) {
        // Joueurs : position des pieds et nourriture tenue en main
        let players: Vec<((f32, f32, f32), String)> = self
            .clients
            .values()
            .filter(|c| c.joined && c.stage == Stage::InGame && c.dead.is_none())
            .map(|c| {
                let held = c.inv.held();
                let name = if held.is_empty() { String::new() } else { self.data.as_ref().and_then(|d| d.item_by_id(held.id)).map(|d| d.name.trim_start_matches("minecraft:").to_string()).unwrap_or_default() };
                ((c.pos.0, c.pos.1 - EYE, c.pos.2), name)
            })
            .collect();
        let ids: Vec<i64> = self.mobs.keys().copied().collect();
        let tick = self.tick;
        // Distance de simulation, comme le jeu : au-delà, les créatures restent figées
        let active = (self.cfg.tick_distance * 16) as f32;
        for id in ids {
            let Some(mut m) = self.mobs.get(&id).cloned() else { continue };
            if let Some(t) = m.dead {
                if tick.saturating_sub(t) >= 20 {
                    self.remove_mob(id);
                }
                continue;
            }
            let near = players.iter().any(|(p, _)| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2) < active * active);
            if !near {
                continue;
            }
            if m.rider.is_some() {
                // En selle : la monture suit son cavalier (ou attend le dressage)
                let had_love = m.love > 0;
                self.mob_age(&mut m);
                let tame = !m.tamed && m.tame_in > 0 && {
                    m.tame_in -= 1;
                    m.tame_in == 0
                };
                let love = m.love;
                self.mobs.insert(id, m);
                if had_love && love == 0 {
                    self.mob_changed(id);
                }
                if tame {
                    self.try_tame(id);
                }
                continue;
            }
            let (cx, cz) = ((m.pos.0.floor() as i32) >> 4, (m.pos.2.floor() as i32) >> 4);
            if !self.world.is_ready(cx, cz) {
                // Demandé aux fils de génération ; l'animal attend qu'il soit prêt
                self.world.prepare(cx, cz);
                continue;
            }
            let had_love = m.love > 0;
            let grew = self.mob_age(&mut m);
            self.mob_think(&mut m, &players);
            self.mob_physics(&mut m);
            let moved = (m.pos.0 - m.sent.0).abs() + (m.pos.1 - m.sent.1).abs() + (m.pos.2 - m.sent.2).abs() > 0.01 || (m.yaw - m.sent_yaw).abs() > 3.0;
            if moved {
                m.sent = m.pos;
                m.sent_yaw = m.yaw;
                let packet = bedrock::move_actor_absolute(m.id, m.pos, 0.0, m.yaw, m.yaw, m.on_ground);
                self.send_to_viewers(m.id, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
                self.mobs_dirty = true;
            }
            let love = m.love;
            self.mobs.insert(id, m);
            if grew {
                self.mob_changed(id);
            }
            if love > 0 && love % 10 == 0 {
                self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_LOVE_PARTICLES, 0));
            }
            if had_love && love == 0 {
                self.mob_changed(id); // l'envie passe
            }
            self.mob_habits(id);
        }
        self.breed();
    }

    /// Le temps qui passe : un petit grandit, un parent se repose, l'envie s'éteint. Vrai si
    /// l'apparence change (petit devenu adulte).
    fn mob_age(&mut self, m: &mut Mob) -> bool {
        if m.love > 0 {
            m.love -= 1;
        }
        if m.age < 0 {
            m.age += 1;
            return m.age == 0;
        }
        if m.age > 0 {
            m.age -= 1;
        }
        false
    }

    /// Habitudes : la poule pond, le mouton broute (et refait sa laine).
    fn mob_habits(&mut self, id: i64) {
        let Some(m) = self.mobs.get_mut(&id) else { return };
        match m.species {
            Species::Chicken if !m.baby() => {
                m.egg = m.egg.saturating_sub(1);
                if m.egg == 0 {
                    m.egg = 6000 + self.mob_rng.below(6000);
                    let pos = m.pos;
                    if let Some(egg) = self.item_id("egg") {
                        self.drop_item((pos.0, pos.1 + 0.3, pos.2), (0.0, 0.1, 0.0), egg, 1, 10);
                    }
                }
            }
            Species::Sheep if m.on_ground && self.mob_rng.below(if m.baby() { 50 } else { 1000 }) == 0 => {
                let (x, y, z) = (m.pos.0.floor() as i32, m.pos.1.floor() as i32, m.pos.2.floor() as i32);
                let g = &self.world.generator().blocks;
                let (short_grass, air, grass, dirt) = (g.short_grass, g.air, g.grass, g.dirt);
                let feet = self.mob_get(x, y, z);
                let below = self.mob_get(x, y - 1, z);
                let eaten = if feet == short_grass {
                    self.world.set(x, y, z, air);
                    self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y, z, air), None);
                    true
                } else if below == grass {
                    self.world.set(x, y - 1, z, dirt);
                    self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(x, y - 1, z, dirt), None);
                    true
                } else {
                    false
                };
                if eaten {
                    let mut changed = false;
                    if let Some(m) = self.mobs.get_mut(&id) {
                        m.target = None;
                        if m.sheared {
                            m.sheared = false;
                            changed = true;
                        }
                        if m.age < 0 {
                            m.age = (m.age + 1200).min(-1);
                        }
                    }
                    self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_EAT_GRASS, 0));
                    if changed {
                        self.mob_changed(id);
                    }
                }
            }
            _ => {}
        }
    }

    /// Choix de la direction : suivre la nourriture, rejoindre l'âme sœur, se promener,
    /// s'arrêter, fuir.
    fn mob_think(&mut self, m: &mut Mob, players: &[((f32, f32, f32), String)]) {
        if m.knock > 0 {
            m.knock -= 1;
            m.vel.0 *= 0.7;
            m.vel.2 *= 0.7;
            return;
        }
        let mut speed = if m.panic > 0 { m.species.speed() * 2.0 } else { m.species.speed() };
        let mut stop_at = 0.4;
        if m.panic > 0 {
            m.panic -= 1;
            // En fuite : une nouvelle direction de temps en temps
            if m.target.is_none() || self.mob_rng.below(20) == 0 {
                let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
                m.target = Some((m.pos.0 + a.cos() * 8.0, m.pos.2 + a.sin() * 8.0));
            }
        } else if let Some(partner) = self.partner_of(m) {
            // Amoureux : rejoint l'autre
            m.target = Some(partner);
            stop_at = 0.8;
        } else if let Some(p) = players.iter().filter(|(_, held)| m.species.tempt().contains(&held.as_str())).map(|(p, _)| *p).find(|p| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2) < 100.0 && (p.1 - m.pos.1).abs() < 4.0) {
            // Attiré par sa nourriture : suit le joueur, s'arrête à deux pas
            m.target = Some((p.0, p.2));
            stop_at = 2.5;
            speed *= 1.25;
        } else if m.target.is_none() && self.mob_rng.below(120) == 0 {
            let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
            let d = 2.0 + self.mob_rng.unit() as f32 * 8.0;
            m.target = Some((m.pos.0 + a.cos() * d, m.pos.2 + a.sin() * d));
        }
        match m.target {
            Some((tx, tz)) => {
                let (dx, dz) = (tx - m.pos.0, tz - m.pos.2);
                let d = (dx * dx + dz * dz).sqrt();
                // Orientation du jeu : 0 vers le sud (+z), 90 vers l'ouest (-x)
                let want = (-dx).atan2(dz).to_degrees();
                let diff = (want - m.yaw + 540.0).rem_euclid(360.0) - 180.0;
                m.yaw = (m.yaw + diff.clamp(-20.0, 20.0)).rem_euclid(360.0);
                if d < stop_at {
                    if stop_at <= 0.4 {
                        m.target = None;
                    }
                    m.vel.0 = 0.0;
                    m.vel.2 = 0.0;
                } else {
                    m.vel.0 = dx / d * speed;
                    m.vel.2 = dz / d * speed;
                }
            }
            None => {
                m.vel.0 = 0.0;
                m.vel.2 = 0.0;
            }
        }
    }

    /// Position d'un autre animal de la même espèce, adulte et amoureux, à moins de 8 blocs.
    fn partner_of(&self, m: &Mob) -> Option<(f32, f32)> {
        if m.love == 0 || m.baby() {
            return None;
        }
        self.mobs
            .values()
            .filter(|o| o.id != m.id && mate(m.species, o.species).is_some() && o.love > 0 && !o.baby() && o.dead.is_none())
            .map(|o| (o.pos.0, o.pos.2, (o.pos.0 - m.pos.0).powi(2) + (o.pos.2 - m.pos.2).powi(2)))
            .filter(|o| o.2 < 64.0)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|o| (o.0, o.1))
    }

    /// Deux amoureux qui se touchent ont un petit.
    fn breed(&mut self) {
        let lovers: Vec<(i64, Species, (f32, f32, f32))> = self.mobs.values().filter(|m| m.love > 0 && !m.baby() && m.dead.is_none()).map(|m| (m.id, m.species, m.pos)).collect();
        let mut done: HashSet<i64> = HashSet::new();
        for (i, a) in lovers.iter().enumerate() {
            for b in &lovers[i + 1..] {
                let Some(kind) = mate(a.1, b.1) else { continue };
                if done.contains(&a.0) || done.contains(&b.0) {
                    continue;
                }
                let d = (a.2 .0 - b.2 .0).powi(2) + (a.2 .1 - b.2 .1).powi(2) + (a.2 .2 - b.2 .2).powi(2);
                if d > 1.6 * 1.6 {
                    continue;
                }
                done.insert(a.0);
                done.insert(b.0);
                for id in [a.0, b.0] {
                    if let Some(m) = self.mobs.get_mut(&id) {
                        m.love = 0;
                        m.age = BREED_COOLDOWN;
                    }
                    self.mob_changed(id);
                }
                let at = ((a.2 .0 + b.2 .0) / 2.0, a.2 .1.max(b.2 .1), (a.2 .2 + b.2 .2) / 2.0);
                // Le petit tient de l'un de ses parents (couleur de laine, pelage, robe) ; le motif
                // de la robe d'un poulain vient de l'un ou l'autre
                let (color, mut variant) = self.mobs.get(&if self.mob_rng.below(2) == 0 { a.0 } else { b.0 }).map(|m| (m.color, m.variant)).unwrap_or((0, 0));
                let mark = self.mobs.get(&if self.mob_rng.below(2) == 0 { a.0 } else { b.0 }).map(|m| m.mark).unwrap_or(0);
                match kind {
                    // Champimeuh : très rarement (1 sur 1024) l'autre couleur que ses parents
                    Species::Mooshroom if self.mob_rng.below(1024) == 0 => variant = 1 - variant.clamp(0, 1),
                    Species::Goat => variant = (self.mob_rng.below(100) < 2) as i32,
                    Species::Mule => variant = 0,
                    _ => {}
                }
                let baby = self.add_mob(kind, at);
                if let Some(m) = self.mobs.get_mut(&baby) {
                    m.age = -GROW_TICKS;
                    m.color = color;
                    m.variant = variant;
                    m.mark = if kind == Species::Horse { mark } else { 0 };
                }
                self.send_to_viewers(a.0, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(a.0, bedrock::EVENT_LOVE_PARTICLES, 0));
                // Un peu d'expérience pour l'éleveur, comme dans le jeu : pas encore gérée
            }
        }
    }

    /// Gravité, nage et collisions avec les blocs (marches d'un bloc sautées, vide évité).
    fn mob_physics(&mut self, m: &mut Mob) {
        let (w, h) = m.size();
        let half = w / 2.0;
        let in_water = self.mob_water(m.pos.0, m.pos.1 + 0.2, m.pos.2);
        // Vertical
        if in_water {
            m.vel.1 = (m.vel.1 + 0.02).min(0.08);
        } else {
            m.vel.1 = (m.vel.1 - 0.08) * 0.98;
            if m.species == Species::Chicken && !m.on_ground {
                m.vel.1 = m.vel.1.max(-0.1); // les poules battent des ailes
            }
        }
        // Le lapin avance par bonds
        if m.species == Species::Rabbit && m.on_ground && (m.vel.0 != 0.0 || m.vel.2 != 0.0) && !in_water {
            m.vel.1 = 0.3;
            m.on_ground = false;
        }
        // Le bloc occupé par un point du corps est-il libre ?
        let body_free = |s: &Self, x: f32, y: f32, z: f32| -> bool {
            let mut yy = y;
            while yy < y + h {
                if !s.mob_free(x.floor() as i32, yy.floor() as i32, z.floor() as i32) {
                    return false;
                }
                yy += 1.0;
            }
            s.mob_free(x.floor() as i32, (y + h - 0.01).floor() as i32, z.floor() as i32)
        };
        // Horizontal, axe par axe : le bord du corps dans le sens du déplacement
        for axis in 0..2 {
            let v = if axis == 0 { m.vel.0 } else { m.vel.2 };
            if v == 0.0 {
                continue;
            }
            let edge = v.signum() * half;
            let (nx, nz) = if axis == 0 { (m.pos.0 + v, m.pos.2) } else { (m.pos.0, m.pos.2 + v) };
            let (ex, ez) = if axis == 0 { (nx + edge, nz) } else { (nx, nz + edge) };
            let ok = body_free(self, ex, m.pos.1 + 0.01, ez);
            if ok {
                // Pas de chute de plus de 3 blocs (sauf en fuite ou dans l'eau)
                let ground_below = (1..=4).any(|d| !self.mob_free(ex.floor() as i32, m.pos.1.floor() as i32 - d, ez.floor() as i32));
                if !ground_below && m.panic == 0 && m.knock == 0 && !in_water {
                    m.target = None;
                    continue;
                }
                if axis == 0 {
                    m.pos.0 = nx;
                } else {
                    m.pos.2 = nz;
                }
            } else if m.on_ground && body_free(self, ex, m.pos.1.floor() + 1.01, ez) {
                // Marche d'un bloc : saute
                m.vel.1 = 0.42;
                m.on_ground = false;
            } else if in_water {
                m.vel.1 = 0.1; // sort de l'eau en grimpant au bord
            } else {
                m.target = None;
            }
        }
        // Vertical : atterrissage et plafond
        let ny = m.pos.1 + m.vel.1;
        if m.vel.1 <= 0.0 {
            let feet = ny.floor() as i32;
            if !self.mob_free(m.pos.0.floor() as i32, feet, m.pos.2.floor() as i32) && !in_water {
                m.pos.1 = (feet + 1) as f32;
                m.vel.1 = 0.0;
                m.on_ground = true;
            } else {
                m.pos.1 = ny;
                m.on_ground = false;
            }
        } else if self.mob_free(m.pos.0.floor() as i32, (ny + h).floor() as i32, m.pos.2.floor() as i32) {
            m.pos.1 = ny;
            m.on_ground = false;
        } else {
            m.vel.1 = 0.0;
        }
        // Coincé dans un bloc (bloc posé dessus) : remonte
        if !self.mob_free(m.pos.0.floor() as i32, m.pos.1.floor() as i32, m.pos.2.floor() as i32) {
            m.pos.1 = m.pos.1.floor() + 1.0;
            m.vel.1 = 0.0;
        }
        if m.pos.1 < crate::world::MIN_Y as f32 - 16.0 {
            m.health = 0.0;
            m.dead = Some(self.tick);
        }
    }

    /// Coup porté par un joueur à une créature. Vrai si la cible est une créature.
    pub(super) fn hit_mob(&mut self, id: i64, from: (f32, f32, f32), damage: f32) -> bool {
        let tick = self.tick;
        let Some(m) = self.mobs.get_mut(&id) else { return false };
        if m.dead.is_some() || tick.saturating_sub(m.hurt_tick) < 10 {
            return true;
        }
        let (dx, dz) = (m.pos.0 - from.0, m.pos.2 - from.2);
        if dx * dx + dz * dz > 6.0 * 6.0 {
            return true;
        }
        m.hurt_tick = tick;
        m.health -= damage;
        let flat = (dx * dx + dz * dz).sqrt().max(0.01);
        m.vel = (dx / flat * 0.4, 0.36, dz / flat * 0.4);
        m.knock = 8;
        m.on_ground = false;
        m.panic = 100;
        m.target = None;
        m.love = 0;
        let dead = m.health <= 0.0;
        let rider = m.rider;
        if dead {
            m.dead = Some(tick);
        }
        self.mobs_dirty = true;
        self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_HURT, 0));
        if dead {
            if let Some(r) = rider {
                self.dismount(&r);
            }
            self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_DEATH, 0));
            self.mob_loot(id);
        }
        // Aucun son envoyé : le jeu joue lui-même les cris, les bruits de coup et de mort
        true
    }

    /// Clic droit d'un joueur sur une créature : nourrir, tondre, traire. Vrai si la cible est
    /// une créature.
    pub(super) fn interact_mob(&mut self, addr: &SocketAddr, id: i64) -> bool {
        let Some(m) = self.mobs.get(&id).cloned() else { return false };
        if m.dead.is_some() {
            return true;
        }
        let (held, creative, near, riding) = match self.clients.get(addr) {
            Some(c) => {
                let h = c.inv.held();
                let name = if h.is_empty() { String::new() } else { self.data.as_ref().and_then(|d| d.item_by_id(h.id)).map(|d| d.name.trim_start_matches("minecraft:").to_string()).unwrap_or_default() };
                let near = (c.pos.0 - m.pos.0).powi(2) + (c.pos.2 - m.pos.2).powi(2) < 8.0 * 8.0;
                (name, c.mode == GameMode::Creative, near, c.riding.is_some())
            }
            None => return true,
        };
        if !near {
            return true;
        }
        let mut changed = false;
        let mut used = false;
        const COLORS: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
        // Un objet changé contre un autre dans la main (seau → seau de lait, bol → soupe)
        let swap = |server: &mut Server, into: &str| {
            if !creative {
                if let Some(item) = server.item_id(into) {
                    if let Some(c) = server.clients.get_mut(addr) {
                        c.inv.replace_held(item);
                    }
                }
                server.send_inventory(addr);
                server.show_held(addr);
            }
        };
        let grow = m.species.grow_items().iter().find(|(name, _)| *name == held).map(|(_, g)| *g);
        let heal = m.species.heal_items().iter().find(|(name, _)| *name == held).map(|(_, h)| *h);
        let breeds = m.species.breed_items().contains(&held.as_str()) && (!m.species.mount() || m.tamed);
        let temper = if m.tamed { None } else { m.species.temper_items().iter().find(|(name, _)| *name == held).map(|(_, t)| *t) };
        if held == "shears" && m.species == Species::Sheep && !m.baby() && !m.sheared {
            // Tonte : 1 à 3 laines de sa couleur
            if let Some(wool) = self.item_id(&format!("{}_wool", COLORS[m.color as usize & 15])) {
                let count = 1 + self.mob_rng.below(3) as u8;
                for _ in 0..count {
                    let vel = ((self.mob_rng.unit() as f32 - 0.5) * 0.2, 0.25, (self.mob_rng.unit() as f32 - 0.5) * 0.2);
                    self.drop_item((m.pos.0, m.pos.1 + 1.0, m.pos.2), vel, wool, 1, 10);
                }
            }
            if let Some(s) = self.mobs.get_mut(&id) {
                s.sheared = true;
            }
            changed = true;
        } else if held == "shears" && m.species == Species::Mooshroom && !m.baby() {
            // Champimeuh tondue : 5 champignons de sa couleur, et elle redevient une vache
            let mushroom = if m.variant == 1 { "brown_mushroom" } else { "red_mushroom" };
            if let Some(item) = self.item_id(mushroom) {
                let vel = ((self.mob_rng.unit() as f32 - 0.5) * 0.2, 0.25, (self.mob_rng.unit() as f32 - 0.5) * 0.2);
                self.drop_item((m.pos.0, m.pos.1 + 1.0, m.pos.2), vel, item, 5, 10);
            }
            self.transform_mob(id, Species::Cow);
        } else if held == "bucket" && matches!(m.species, Species::Cow | Species::Mooshroom | Species::Goat) && !m.baby() {
            // Traite : le seau vide devient un seau de lait
            swap(self, "milk_bucket");
        } else if held == "bowl" && m.species == Species::Mooshroom && !m.baby() {
            // Champimeuh : un bol devient une soupe de champignons
            swap(self, "mushroom_stew");
        } else if held == "saddle" && m.tamed && !m.saddled && m.species.steerable() && !m.baby() {
            // Selle posée : son cavalier pourra la diriger
            if let Some(s) = self.mobs.get_mut(&id) {
                s.saddled = true;
            }
            changed = true;
            used = true;
        } else if heal.is_some() && (m.health < m.max_health || temper.is_some()) {
            // Monture blessée : le repas la soigne ; sauvage, il l'amadoue (dressage plus facile)
            if let Some(s) = self.mobs.get_mut(&id) {
                s.health = (s.health + heal.unwrap_or(0.0)).min(s.max_health);
                s.temper = (s.temper + temper.unwrap_or(0)).min(s.species.max_temper());
            }
            used = true;
        } else if let (Some(g), true) = (grow, m.baby()) {
            // Un petit nourri grandit plus vite (la part de croissance de ce repas)
            if let Some(s) = self.mobs.get_mut(&id) {
                s.age = (s.age + (GROW_TICKS as f32 * g) as i64).min(-1);
            }
            used = true;
        } else if breeds && !m.baby() && m.age == 0 && m.love == 0 {
            if let Some(s) = self.mobs.get_mut(&id) {
                s.love = LOVE_TICKS;
            }
            self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_LOVE_PARTICLES, 0));
            changed = true;
            used = true;
        } else if m.species.mount() && !m.baby() && m.rider.is_none() && !riding && !m.species.breed_items().contains(&held.as_str()) {
            // Main vide (ou un objet sans usage ici) : en selle
            self.mount_mob(addr, id);
        }
        if used && !creative {
            if let Some(c) = self.clients.get_mut(addr) {
                c.inv.consume_held();
            }
            self.send_inventory(addr);
            self.show_held(addr);
        }
        if changed {
            self.mob_changed(id);
        }
        true
    }

    /// `/summon` : fait apparaître un animal (pelage, robe, qualités tirés au sort comme à
    /// l'apparition naturelle). `baby` : un petit (événement « minecraft:entity_born »).
    pub(super) fn summon_mob(&mut self, name: &str, pos: (f32, f32, f32), baby: bool) -> Result<(), String> {
        let short = name.trim_start_matches("minecraft:").to_ascii_lowercase();
        let Some(species) = Species::from_id(&format!("minecraft:{}", short)) else {
            return Err(format!("Basalt ne sait pas encore faire apparaître : {}", name));
        };
        if !pos.1.is_finite() || pos.1 < crate::world::MIN_Y as f32 || pos.1 > crate::world::MAX_Y as f32 {
            return Err("Position hors du monde".into());
        }
        let variant = match species {
            Species::Horse => self.mob_rng.below(7) as i32,
            Species::Llama => self.mob_rng.below(4) as i32,
            Species::Goat => (self.mob_rng.below(100) < 2) as i32,
            _ => 0,
        };
        let mark = if species == Species::Horse { self.mob_rng.below(5) as i32 } else { 0 };
        let id = self.add_mob(species, pos);
        if let Some(m) = self.mobs.get_mut(&id) {
            m.variant = variant;
            m.mark = mark;
            if baby {
                m.age = -GROW_TICKS;
            }
        }
        // Son chunk compte désormais parmi ceux qui sont en mémoire (sinon il serait rangé aussitôt)
        let chunk = mob_store::chunk_of(pos.0, pos.2);
        if !self.mob_chunks.contains(&chunk) {
            if let Some(region) = self.mob_store.region(mob_store::region_of(chunk)) {
                for r in region.chunks.remove(&chunk).unwrap_or_default() {
                    self.spawn_record(r);
                }
                self.mob_chunks.insert(chunk);
            }
        }
        Ok(())
    }

    /// Le joueur monte sur la monture : lien envoyé à tous ceux qui la voient, et le joueur
    /// devient cavalier (assis sur le siège). Une monture sauvage tente ensuite de le jeter.
    pub(super) fn mount_mob(&mut self, addr: &SocketAddr, id: i64) {
        let Some(player) = self.clients.get(addr).map(|c| c.id) else { return };
        let Some(m) = self.mobs.get_mut(&id) else { return };
        if m.rider.is_some() || m.dead.is_some() {
            return;
        }
        m.rider = Some(*addr);
        m.target = None;
        m.vel = (0.0, 0.0, 0.0);
        let (seat, wild) = (m.species.seat(), !m.tamed);
        if wild {
            // Quelques secondes avant de décider : apprivoisée ou jeté à terre
            m.tame_in = 40 + self.mob_rng.below(60);
        }
        if let Some(c) = self.clients.get_mut(addr) {
            c.riding = Some(id);
            c.fall_peak = None;
        }
        let link = bedrock::set_actor_link(id, player, bedrock::LINK_RIDER, false);
        self.send_to_viewers(id, bedrock::ID_SET_ACTOR_LINK, &link);
        self.rider_data(addr, Some(seat));
        self.mob_changed(id);
    }

    /// Le cavalier descend (de lui-même, jeté par une monture sauvage, mort ou parti).
    pub(super) fn dismount(&mut self, addr: &SocketAddr) {
        let (mob, player) = match self.clients.get_mut(addr) {
            Some(c) => match c.riding.take() {
                Some(m) => (m, c.id),
                None => return,
            },
            None => return,
        };
        self.release_mount(mob, player);
        self.rider_data(addr, None);
    }

    /// La monture n'a plus de cavalier (lien retiré chez tous ceux qui la voient).
    pub(super) fn release_mount(&mut self, mob: i64, player: i64) {
        if let Some(m) = self.mobs.get_mut(&mob) {
            m.rider = None;
            m.tame_in = 0;
            m.sent = (f32::MAX, 0.0, 0.0); // position renvoyée à tous au prochain tick
        }
        let link = bedrock::set_actor_link(mob, player, bedrock::LINK_REMOVE, false);
        self.send_to_viewers(mob, bedrock::ID_SET_ACTOR_LINK, &link);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.visible.contains(&player)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_SET_ACTOR_LINK, &link);
        }
    }

    /// Données du joueur cavalier (ou redevenu piéton, `seat` à None), chez lui et chez ceux
    /// qui le voient : indicateur « en selle » et position du siège.
    fn rider_data(&mut self, addr: &SocketAddr, seat: Option<(f32, f32, f32)>) {
        let Some(c) = self.clients.get(addr) else { return };
        let (id, sneaking, sprinting) = (c.id, c.sneaking, c.sprinting);
        let riding = if seat.is_some() { bedrock::FLAG_RIDING } else { 0 };
        let seat = seat.unwrap_or((0.0, 0.0, 0.0));
        let own = bedrock::OWN_PLAYER_FLAGS | riding;
        let packet = bedrock::set_actor_data(id, &[(0, bedrock::Meta::Long(own)), (bedrock::META_SEAT_POSITION, bedrock::Meta::Vec3(seat.0, seat.1, seat.2))]);
        self.send(addr, bedrock::ID_SET_ACTOR_DATA, &packet);
        let others = Self::remote_flags(sneaking, sprinting) | riding;
        let packet = bedrock::set_actor_data(id, &[(0, bedrock::Meta::Long(others)), (bedrock::META_SEAT_POSITION, bedrock::Meta::Vec3(seat.0, seat.1, seat.2))]);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| *a != addr && c.visible.contains(&id)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_SET_ACTOR_DATA, &packet);
        }
    }

    /// Essai de dressage d'une monture sauvage montée : réussi si le hasard tombe sous sa
    /// patience (comme le jeu) ; sinon elle jette son cavalier et sera plus patiente ensuite.
    fn try_tame(&mut self, id: i64) {
        let Some(m) = self.mobs.get(&id) else { return };
        let (max, temper, rider) = (m.species.max_temper(), m.temper, m.rider);
        if self.mob_rng.below(max) < temper {
            if let Some(m) = self.mobs.get_mut(&id) {
                m.tamed = true;
            }
            self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_TAME_SUCCESS, 0));
            self.mob_changed(id);
        } else {
            if let Some(m) = self.mobs.get_mut(&id) {
                m.temper = (m.temper + 5).min(max);
            }
            self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_TAME_FAIL, 0));
            if let Some(r) = rider {
                self.dismount(&r);
            }
        }
    }

    /// Position de sa monture envoyée par le jeu du cavalier (il la dirige lui-même) : elle
    /// est reprise et montrée aux autres joueurs.
    pub(super) fn on_rider_move(&mut self, addr: &SocketAddr, body: &[u8]) {
        let Some((runtime, pos, _pitch, yaw)) = bedrock::read_move_actor(body) else { return };
        let Some((riding, player)) = self.clients.get(addr).map(|c| (c.riding, c.pos)) else { return };
        if riding != Some(runtime as i64) {
            return;
        }
        self.steer_mount(runtime as i64, pos, yaw, Some(player));
    }

    /// Déplace une monture dirigée par son cavalier (refus si elle s'écarte de lui).
    pub(super) fn steer_mount(&mut self, id: i64, pos: (f32, f32, f32), yaw: f32, player: Option<(f32, f32, f32)>) {
        let tick = self.tick;
        let Some(m) = self.mobs.get_mut(&id) else { return };
        if !m.driven_by_rider() {
            return;
        }
        if let Some(p) = player {
            if (p.0 - pos.0).powi(2) + (p.2 - pos.2).powi(2) > 8.0 * 8.0 {
                return;
            }
        }
        m.pos = pos;
        m.yaw = yaw.rem_euclid(360.0);
        m.driven = tick;
        let moved = (m.pos.0 - m.sent.0).abs() + (m.pos.1 - m.sent.1).abs() + (m.pos.2 - m.sent.2).abs() > 0.01 || (m.yaw - m.sent_yaw).abs() > 3.0;
        if !moved {
            return;
        }
        m.sent = m.pos;
        m.sent_yaw = m.yaw;
        let rider = m.rider;
        let packet = bedrock::move_actor_absolute(id, pos, 0.0, m.yaw, m.yaw, true);
        self.mobs_dirty = true;
        // Le cavalier la déplace lui-même : on ne la lui renvoie pas
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| Some(**a) != rider && c.mobs_seen.contains(&id)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
        }
    }

    /// Le cavalier a bougé : si son jeu n'a pas envoyé la position de la monture depuis un
    /// moment, elle est déduite de la sienne (sous le siège).
    pub(super) fn follow_rider(&mut self, addr: &SocketAddr) {
        let Some((Some(id), pos, yaw)) = self.clients.get(addr).map(|c| (c.riding, c.pos, c.yaw)) else { return };
        let tick = self.tick;
        let Some(m) = self.mobs.get(&id) else { return };
        if !m.driven_by_rider() || tick.saturating_sub(m.driven) < 10 {
            return;
        }
        let seat = m.species.seat();
        let driven = m.driven;
        self.steer_mount(id, (pos.0, pos.1 - seat.1, pos.2), yaw, None);
        // (Position déduite : n'empêche pas une vraie position du jeu d'arriver ensuite)
        if let Some(m) = self.mobs.get_mut(&id) {
            m.driven = driven;
        }
    }

    /// Change l'espèce d'une créature (champimeuh tondue → vache) : elle est retirée puis
    /// remontrée aux joueurs sous sa nouvelle forme, à la même place.
    fn transform_mob(&mut self, id: i64, into: Species) {
        let Some(m) = self.mobs.get_mut(&id) else { return };
        m.species = into;
        m.variant = 0;
        m.max_health = m.max_health.min(into.health());
        m.health = m.health.min(m.max_health);
        let viewers: Vec<SocketAddr> = self.clients.iter_mut().filter_map(|(a, c)| c.mobs_seen.remove(&id).then_some(*a)).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id));
        }
        self.mobs_dirty = true;
        // Remontrée au prochain passage de show_mobs
    }

    /// Butin d'une créature morte, lâché sur place (rien pour un petit).
    fn mob_loot(&mut self, id: i64) {
        let Some(m) = self.mobs.get(&id).cloned() else { return };
        if m.saddled {
            if let Some(saddle) = self.item_id("saddle") {
                self.drop_item((m.pos.0, m.pos.1 + 0.5, m.pos.2), (0.0, 0.2, 0.0), saddle, 1, 10);
            }
        }
        if m.baby() {
            return;
        }
        for &(name, lo, hi) in m.species.drops() {
            let count = lo + self.mob_rng.below((hi - lo) as u32 + 1) as u8;
            // La laine prend la couleur du mouton ; un mouton tondu n'en a plus
            let name = if name == "minecraft:white_wool" {
                if m.sheared {
                    continue;
                }
                const COLORS: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
                format!("minecraft:{}_wool", COLORS[m.color as usize & 15])
            } else {
                name.to_string()
            };
            let Some(item) = self.item_id(&name) else { continue };
            let vel = ((self.mob_rng.unit() as f32 - 0.5) * 0.2, 0.2, (self.mob_rng.unit() as f32 - 0.5) * 0.2);
            self.drop_item((m.pos.0, m.pos.1 + 0.5, m.pos.2), vel, item, count, 10);
        }
    }

    /// Rayon (en chunks) autour d'un joueur où les créatures sont en mémoire : de quoi les
    /// voir et les faire vivre ; au-delà de `+ 2`, elles retournent dans leur région.
    fn mob_load_radius(&self) -> i32 {
        ((VIEW / 16.0).ceil() as i32).max(self.cfg.tick_distance) + 1
    }

    /// Sort de leur région les créatures des chunks qui s'approchent d'un joueur, et y range
    /// celles dont plus personne n'est proche (comme le jeu, qui les garde avec leur chunk).
    pub(super) fn stream_mobs(&mut self) {
        self.mob_store.receive();
        let centers: Vec<(i32, i32)> = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame).map(|c| mob_store::chunk_of(c.pos.0, c.pos.2)).collect();
        let load = self.mob_load_radius();
        let keep = load + 2;
        // 1. Chunks qui s'approchent : leurs créatures reviennent
        for &(pcx, pcz) in &centers {
            for cx in pcx - load..=pcx + load {
                for cz in pcz - load..=pcz + load {
                    if self.mob_chunks.contains(&(cx, cz)) {
                        continue;
                    }
                    let Some(region) = self.mob_store.region(mob_store::region_of((cx, cz))) else { continue };
                    let records = region.chunks.remove(&(cx, cz)).unwrap_or_default();
                    self.mob_chunks.insert((cx, cz));
                    for r in records {
                        self.spawn_record(r);
                    }
                }
            }
        }
        // 2. Chunks loin de tout joueur : oubliés
        let near = |c: &(i32, i32), r: i32| centers.iter().any(|p| (c.0 - p.0).abs() <= r && (c.1 - p.1).abs() <= r);
        self.mob_chunks.retain(|c| near(c, keep));
        // 3. Créatures hors des chunks gardés : rangées dans leur région
        let away: Vec<(i64, (i32, i32))> = self
            .mobs
            .values()
            // (Une monture en selle suit son cavalier : jamais rangée sous lui)
            .filter(|m| m.dead.is_none() && m.rider.is_none())
            .map(|m| (m.id, mob_store::chunk_of(m.pos.0, m.pos.2)))
            .filter(|(_, c)| !self.mob_chunks.contains(c))
            .collect();
        for (id, chunk) in away {
            let Some(record) = self.mobs.get(&id).map(|m| m.record()) else { continue };
            // Région pas encore lue : la créature attend en mémoire
            let Some(region) = self.mob_store.region(mob_store::region_of(chunk)) else { continue };
            region.chunks.entry(chunk).or_default().push(record);
            region.dirty = true;
            self.remove_mob(id);
        }
    }

    /// Fait revenir une créature enregistrée.
    fn spawn_record(&mut self, r: MobRecord) {
        let id = self.add_mob(r.species, r.pos);
        if let Some(m) = self.mobs.get_mut(&id) {
            m.yaw = r.yaw;
            m.sent_yaw = r.yaw;
            m.health = r.health;
            m.age = r.age;
            m.color = r.color;
            m.variant = r.variant;
            m.sheared = r.sheared;
            m.max_health = r.max_health;
            m.mark = r.mark;
            m.tamed = r.tamed;
            m.saddled = r.saddled;
            m.temper = r.temper;
            // Monture enregistrée avant que ses qualités existent : celles tirées à l'apparition restent
            if r.move_speed > 0.0 {
                m.move_speed = r.move_speed;
                m.jump = r.jump;
                m.strength = r.strength;
            }
        }
    }

    /// Enregistre les régions changées : les créatures rangées et celles en mémoire. À l'arrêt
    /// (`last`), tout est écrit, quitte à lire d'abord les régions manquantes.
    pub(super) fn save_mobs(&mut self, last: bool) {
        let mut live: HashMap<(i32, i32), Vec<MobRecord>> = HashMap::new();
        for m in self.mobs.values().filter(|m| m.dead.is_none()) {
            live.entry(mob_store::region_of(mob_store::chunk_of(m.pos.0, m.pos.2))).or_default().push(m.record());
        }
        if last {
            // Les écritures en attente d'abord : les fichiers relus sont à jour
            crate::background::DISK.flush(Duration::from_secs(60));
            for r in live.keys() {
                self.mob_store.load_now(*r);
            }
        }
        let mut waiting = false;
        let touched: Vec<(i32, i32)> = self
            .mob_store
            .regions
            .iter()
            .filter(|(r, region)| region.dirty || (self.mobs_dirty || last) && live.contains_key(*r))
            .map(|(r, _)| *r)
            .collect();
        for r in live.keys() {
            if !self.mob_store.regions.contains_key(r) && (self.mobs_dirty || last) {
                waiting = true; // région encore en lecture : ce sera pour le prochain enregistrement
            }
        }
        for r in touched {
            let Some(region) = self.mob_store.regions.get_mut(&r) else { continue };
            let text = region.encode(live.get(&r).map(|v| v.as_slice()).unwrap_or(&[]));
            region.dirty = false;
            self.mob_store.write(r, text);
        }
        self.mobs_dirty = waiting;
        if last {
            info!("Animaux enregistrés : {} autour des joueurs", live.values().map(|v| v.len()).sum::<usize>());
            return;
        }
        // Régions qui ne servent plus (déjà écrites) : libérées
        let used: HashSet<(i32, i32)> = self.mob_chunks.iter().map(|c| mob_store::region_of(*c)).chain(live.keys().copied()).collect();
        self.mob_store.regions.retain(|r, region| region.dirty || used.contains(r));
    }

    /// Prépare l'enregistrement des créatures du monde ouvert ; convertit l'ancien fichier
    /// unique (`mobs.json`) en régions.
    pub(super) fn load_mobs(&mut self) {
        let dir = std::path::PathBuf::from(&self.cfg.world_dir);
        self.mob_store = mob_store::MobStore::new(dir.join("entities"));
        self.mob_chunks.clear();
        let old = dir.join("mobs.json");
        let Ok(text) = std::fs::read_to_string(&old) else { return };
        let Some(all) = mob_store::Region::parse(&text) else {
            warn!("{} illisible : animaux non repris", old.display());
            return;
        };
        let mut regions: HashMap<(i32, i32), mob_store::Region> = HashMap::new();
        for (chunk, mobs) in all.chunks {
            regions.entry(mob_store::region_of(chunk)).or_default().chunks.insert(chunk, mobs);
        }
        for chunk in all.populated {
            regions.entry(mob_store::region_of(chunk)).or_default().populated.insert(chunk);
        }
        let count: usize = regions.values().map(|r| r.stored()).sum();
        for (r, region) in &regions {
            if let Err(e) = self.mob_store.write_now(*r, &region.encode(&[])) {
                warn!("Conversion des animaux impossible ({}) : l'ancien fichier est gardé", e);
                return;
            }
        }
        let _ = std::fs::rename(&old, dir.join("mobs.json.ancien"));
        info!("Animaux convertis au rangement par région : {}", count);
    }
}

/// Espèce du petit de deux parents (None s'ils ne peuvent pas en avoir) : même espèce, ou
/// cheval et âne qui font une mule ; la mule n'a pas de petit.
fn mate(a: Species, b: Species) -> Option<Species> {
    match (a, b) {
        (Species::Mule, _) | (_, Species::Mule) => None,
        (Species::Horse, Species::Donkey) | (Species::Donkey, Species::Horse) => Some(Species::Mule),
        (a, b) if a == b => Some(a),
        _ => None,
    }
}

/// Générateur de hasard des créatures, d'après l'heure de démarrage.
pub(super) fn mob_rng() -> Rng {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    Rng::new(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mates_and_foods_follow_the_game() {
        assert_eq!(mate(Species::Horse, Species::Donkey), Some(Species::Mule));
        assert_eq!(mate(Species::Donkey, Species::Horse), Some(Species::Mule));
        assert_eq!(mate(Species::Mule, Species::Mule), None, "la mule n'a pas de petit");
        assert_eq!(mate(Species::Cow, Species::Mooshroom), None);
        assert_eq!(mate(Species::Goat, Species::Goat), Some(Species::Goat));
        assert!(Species::Horse.mount() && Species::Llama.mount() && !Species::Goat.mount());
        assert!(Species::Horse.breed_items().contains(&"golden_carrot") && Species::Horse.tempt().is_empty());
        assert_eq!(Species::Llama.breed_items(), &["hay_block"]);
        assert!(Species::Mule.breed_items().is_empty());
        assert!(Species::Horse.heal_items().iter().any(|(i, h)| *i == "hay_block" && *h == 20.0));
        for s in Species::ALL {
            assert_eq!(Species::from_id(s.id()), Some(s));
            assert!(!s.grow_items().is_empty(), "{:?} : un petit doit pouvoir grandir plus vite", s);
        }
    }

    #[test]
    fn mount_qualities_are_saved() {
        let r = MobRecord {
            species: Species::Horse,
            pos: (1.0, 64.0, 2.0),
            yaw: 0.0,
            health: 22.0,
            age: 0,
            color: 0,
            variant: 3,
            sheared: false,
            max_health: 27.0,
            mark: 4,
            tamed: true,
            move_speed: 0.3125,
            jump: 0.75,
            strength: 0,
            saddled: true,
            temper: 15,
        };
        assert_eq!(MobRecord::from_json(&r.to_json()), Some(r));
        // Ancien enregistrement (sans les nouveaux champs) : valeurs du jeu par défaut
        let old = crate::json::parse(r#"{"kind":"minecraft:cow","x":0,"y":0,"z":0}"#).unwrap();
        let cow = MobRecord::from_json(&old).unwrap();
        assert_eq!((cow.max_health, cow.health, cow.tamed, cow.mark), (10.0, 10.0, false, 0));
    }
}
