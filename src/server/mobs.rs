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

/// Qui tient la laisse : un joueur, ou un nœud attaché à une barrière (son identifiant et
/// la position de la barrière).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Leash {
    Player(SocketAddr),
    Knot(i64, (i32, i32, i32)),
}

/// Distances de la laisse du jeu (« minecraft:leashable ») : au-delà de la première elle
/// tire doucement, de la deuxième fort, et casse à la troisième.
const LEASH_SOFT: f32 = 4.0;
const LEASH_HARD: f32 = 6.0;
const LEASH_MAX: f32 = 10.0;

/// Vitesse au sol (blocs par tick) d'une créature qui veut aller à `speed` (vitesse du jeu ×
/// multiplicateur) : le jeu s'en sert à la fois comme élan et comme vitesse, d'où le carré
/// (le sol en garde 0,546 à chaque tick). Sous 0,2 (promenade des équidés et des lamas), le
/// carré les rendait bien plus lents que dans le jeu : la vitesse y reste proportionnelle.
fn ground_speed(speed: f32) -> f32 {
    if speed < 0.2 {
        speed * 0.2 / 0.454
    } else {
        (speed * speed / 0.454).min(0.4)
    }
}

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
            // Siège du cochon : 0,63 dans le jeu
            Species::Pig => (0.0, 1.83, 0.0),
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
    /// Vitesse du jeu (« minecraft:movement ») ; celle du cheval est tirée à sa naissance
    fn movement(self) -> f32 {
        match self {
            Species::Rabbit => 0.3,
            Species::Goat => 0.4,
            Species::Donkey | Species::Mule => 0.175,
            _ => 0.25,
        }
    }
    /// Multiplicateurs de vitesse du jeu : promenade, fuite, attirée par la nourriture,
    /// petit qui suit un parent (« speed_multiplier » des comportements du jeu)
    fn multipliers(self) -> (f32, f32, f32, f32) {
        match self {
            Species::Cow | Species::Mooshroom | Species::Sheep => (0.8, 1.25, 1.25, 1.1),
            Species::Pig => (1.0, 1.25, 1.2, 1.1),
            Species::Chicken => (1.0, 1.5, 1.0, 1.1),
            Species::Rabbit => (0.6, 2.2, 1.0, 1.1),
            Species::Goat => (0.6, 1.0, 0.75, 1.0),
            Species::Llama | Species::Horse | Species::Donkey | Species::Mule => (0.7, 1.2, 1.2, 1.0),
        }
    }
    /// Ce qui l'attire : il suit le joueur qui le tient
    fn tempt(self) -> &'static [&'static str] {
        match self {
            Species::Cow | Species::Sheep | Species::Mooshroom | Species::Goat => &["wheat"],
            Species::Pig => &["carrot", "potato", "beetroot"],
            Species::Chicken => &["wheat_seeds", "pumpkin_seeds", "melon_seeds", "beetroot_seeds", "torchflower_seeds", "pitcher_pod"],
            Species::Rabbit => &["carrot", "golden_carrot", "dandelion"],
            // Le lama ne suit personne ; les équidés seulement une fois apprivoisés
            Species::Llama => &[],
            Species::Horse | Species::Donkey | Species::Mule => &["golden_apple", "enchanted_golden_apple", "golden_carrot"],
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
    /// Écart entre la hauteur que donne le jeu du cavalier et celle de la monture (mesuré
    /// au sol, il laisse voir ses sauts)
    ride_offset: Option<f32>,
    /// Ticks restants cabrée
    rearing: u32,
    /// En l'air (saut d'une monture dirigée)
    airborne: bool,
    /// Plus haute position depuis que la monture dirigée a quitté le sol (dégâts de chute)
    fall_peak: Option<f32>,
    /// Tenue en laisse
    pub leash: Option<Leash>,
    /// Porte un coffre (âne, mule, lama)
    chested: bool,
    /// Ticks d'accélération restants (cochon piqué par la carotte sur un bâton)
    boost: u32,
    /// Chèvre : cornes restantes (une tombe quand elle fonce dans un bloc dur), attente avant
    /// la prochaine charge, charge en cours (direction, ticks restants), saut en cours
    horns: u8,
    ram_cooldown: u32,
    ram: Option<(f32, f32, u32)>,
    leap: u32,
    /// Lapin : plant de carottes visé
    garden: Option<(i32, i32, i32)>,
    /// Lama : celui qu'il suit en caravane ; joueur qui l'a frappé (il lui crache dessus)
    caravan: Option<i64>,
    angry: Option<(SocketAddr, u32)>,
    /// Champimeuh brune qui a mangé une fleur : son prochain bol sera une soupe suspecte
    stew: bool,
    /// Inventaire d'une monture : 0 selle, 1 armure (cheval) ou tapis (lama), puis le coffre ;
    /// chaque case : (nom de l'objet, quantité)
    items: Vec<Option<(String, u8)>>,
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
        if self.rearing > 0 {
            f |= bedrock::FLAG_REARING;
        }
        if self.leash.is_some() {
            f |= bedrock::FLAG_LEASHED;
        }
        if self.chested {
            f |= bedrock::FLAG_CHESTED;
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
    /// Taille de l'inventaire d'une monture : selle et armure, plus le coffre (15 cases pour
    /// l'âne et la mule, 3 par point de force pour le lama)
    fn inventory_size(&self) -> usize {
        if !self.species.mount() {
            return 0;
        }
        let chest = match (self.chested, self.species) {
            (true, Species::Llama) => 3 * self.strength.max(1) as usize,
            (true, _) => 15,
            _ => 0,
        };
        2 + chest
    }
    /// L'inventaire à la bonne taille (coffre ajouté, ancienne monture)
    fn fit_items(&mut self) {
        let size = self.inventory_size();
        if self.items.len() != size {
            self.items.resize(size, None);
        }
    }
    /// Armure ou tapis porté (case 1)
    fn body_item(&self) -> Option<&str> {
        self.items.get(1).and_then(|i| i.as_ref()).map(|(n, _)| n.as_str())
    }
    /// Vitesse du jeu de cette créature (le cheval a la sienne)
    fn movement(&self) -> f32 {
        if self.species == Species::Horse && self.move_speed > 0.0 {
            self.move_speed
        } else {
            self.species.movement()
        }
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
    /// Boîte annoncée au jeu (données 53 et 54) : celle des fichiers du jeu, avant la taille
    /// d'affichage, que le jeu applique lui-même. Réduite ici aussi, celle d'un petit (ou d'un
    /// lapin) était réduite deux fois : trop petite pour le viser.
    fn hitbox(&self) -> (f32, f32) {
        let (w, h) = self.species.size();
        let adult = self.species.scale(false);
        (w / adult, h / adult)
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
    /// Attachée à la barrière de ce bloc
    pub knot: Option<(i32, i32, i32)>,
    pub chested: bool,
    /// Cornes de la chèvre
    pub horns: u8,
    /// Inventaire d'une monture : (case, objet, quantité)
    pub items: Vec<(u8, String, u8)>,
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
            ("knot".into(), match self.knot {
                Some((x, y, z)) => Json::Arr(vec![Json::Num(x as f64), Json::Num(y as f64), Json::Num(z as f64)]),
                None => Json::Null,
            }),
            ("chested".into(), Json::Bool(self.chested)),
            ("horns".into(), Json::Num(self.horns as f64)),
            ("items".into(), Json::Arr(self.items.iter().map(|(slot, name, count)| Json::Arr(vec![Json::Num(*slot as f64), Json::Str(name.clone()), Json::Num(*count as f64)])).collect())),
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
            knot: v.get("knot").and_then(|k| k.as_arr()).and_then(|k| match k {
                [x, y, z] => Some((x.as_f64()? as i32, y.as_f64()? as i32, z.as_f64()? as i32)),
                _ => None,
            }),
            chested: v.get("chested").and_then(|s| s.as_bool()).unwrap_or(false),
            horns: num("horns").unwrap_or(2.0).clamp(0.0, 2.0) as u8,
            items: v
                .get("items")
                .and_then(|i| i.as_arr())
                .unwrap_or(&[])
                .iter()
                .filter_map(|e| match e.as_arr()? {
                    [slot, name, count] => Some((slot.as_f64()?.clamp(0.0, 64.0) as u8, name.as_str()?.to_string(), count.as_f64()?.clamp(1.0, 64.0) as u8)),
                    _ => None,
                })
                .collect(),
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
            knot: match self.leash {
                Some(Leash::Knot(_, pos)) => Some(pos),
                _ => None,
            },
            chested: self.chested,
            horns: self.horns,
            items: self.items.iter().enumerate().filter_map(|(i, it)| it.as_ref().map(|(n, c)| (i as u8, n.clone(), *c))).collect(),
        }
    }
}

impl Server {
    /// Installe des animaux dans les chunks qu'un joueur découvre (une fois par chunk, comme
    /// à la génération dans le jeu : un chunk sur dix environ reçoit un troupeau).
    pub(super) fn populate_chunks(&mut self) {
        if !self.cfg.spawn_animals {
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
            ride_offset: None,
            rearing: 0,
            airborne: false,
            fall_peak: None,
            leash: None,
            chested: false,
            boost: 0,
            horns: 2,
            ram_cooldown: 600 + self.mob_rng.below(5400),
            ram: None,
            leap: 0,
            garden: None,
            caravan: None,
            angry: None,
            stew: false,
            items: Vec::new(),
        };
        self.mobs.insert(id, mob);
        self.mobs_dirty = true;
        id
    }

    /// Données d'entité d'une créature (taille, couleur, bébé, tondu…).
    fn mob_meta(m: &Mob, holder: i64) -> Vec<(u32, bedrock::Meta)> {
        let (w, h) = m.hitbox();
        let mut meta = bedrock::mob_metadata(w, h, m.variant, m.color, m.flags());
        let scale = m.species.scale(m.baby());
        for (k, v) in meta.iter_mut() {
            match *k {
                38 => *v = bedrock::Meta::Float(scale),
                // Motif de la robe du cheval
                43 => *v = bedrock::Meta::Int(m.mark),
                // Qui tient sa laisse
                37 => *v = bedrock::Meta::Long(holder),
                _ => {}
            }
        }
        for (k, v) in Self::container_meta(m) {
            if let Some(cell) = meta.iter_mut().find(|(key, _)| *key == k) {
                cell.1 = v;
            }
        }
        meta
    }

    /// Inventaire porté par une monture, tel que le jeu le lit dans ses données d'entité :
    /// type d'écran (44), nombre de cases (45), cases par point de force (46). Laissé à zéro,
    /// le jeu affichait l'écran mais plantait dès qu'on y prenait un objet : il cherchait les
    /// cases acceptant l'objet tenu dans un inventaire de taille nulle.
    fn container_meta(m: &Mob) -> [(u32, bedrock::Meta); 3] {
        let size = m.inventory_size() as i32;
        let kind = if size > 0 { bedrock::SCREEN_HORSE } else { 0 };
        [(44, bedrock::Meta::Byte(kind)), (45, bedrock::Meta::Int(size)), (46, bedrock::Meta::Int(0))]
    }

    /// Paquet d'apparition d'une créature.
    fn mob_packet(m: &Mob, rider: Option<i64>, holder: i64) -> Vec<u8> {
        bedrock::add_actor(m.id, m.species.id(), m.pos, m.vel, m.yaw, (m.health.max(0.5), m.max_health), m.species.mount().then_some((m.move_speed, m.jump)), rider, &Self::mob_meta(m, holder))
    }

    /// Identifiant de celui qui tient la laisse (-1 : personne).
    fn holder_id(&self, m: &Mob) -> i64 {
        match m.leash {
            Some(Leash::Player(a)) => self.clients.get(&a).map(|c| c.id).unwrap_or(-1),
            Some(Leash::Knot(k, _)) => k,
            None => -1,
        }
    }

    /// Envoie aux joueurs qui la voient l'état changé d'une créature (bébé devenu adulte,
    /// tondu, amoureux).
    fn mob_changed(&mut self, id: i64) {
        let Some(m) = self.mobs.get(&id) else { return };
        let scale = m.species.scale(m.baby());
        let (w, h) = m.hitbox();
        let holder = self.holder_id(m);
        let packet = bedrock::set_actor_data(
            id,
            &[
                (0u32, bedrock::Meta::Long(m.flags())),
                (2, bedrock::Meta::Int(m.variant)),
                (3, bedrock::Meta::Byte(m.color)),
                (bedrock::META_LEAD_HOLDER, bedrock::Meta::Long(holder)),
                (38, bedrock::Meta::Float(scale)),
                (53, bedrock::Meta::Float(w)),
                (54, bedrock::Meta::Float(h)),
            ]
            .into_iter()
            .chain(Self::container_meta(m))
            .collect::<Vec<_>>()[..],
        );
        self.send_to_viewers(id, bedrock::ID_SET_ACTOR_DATA, &packet);
        self.mobs_dirty = true;
    }

    /// Montre à chaque joueur les créatures proches de lui et retire les autres.
    pub(super) fn show_mobs(&mut self) {
        let mut out: Vec<(SocketAddr, u32, Vec<u8>)> = Vec::new();
        let ids: HashMap<SocketAddr, i64> = self.clients.iter().map(|(a, c)| (*a, c.id)).collect();
        let refused_attributes = self.refused_extras.contains(&bedrock::ID_UPDATE_ATTRIBUTES);
        // Identifiants des armures et tapis portés
        let item_ids: HashMap<String, i16> = self.mobs.values().filter_map(|m| m.body_item()).filter_map(|n| self.item_id(n).map(|id| (n.to_string(), id))).collect();
        let holder_of = |m: &Mob| match m.leash {
            Some(Leash::Player(a)) => ids.get(&a).copied().unwrap_or(-1),
            Some(Leash::Knot(k, _)) => k,
            None => -1,
        };
        for (addr, c) in self.clients.iter_mut() {
            if !c.joined || c.stage != Stage::InGame {
                continue;
            }
            // Nœuds de laisse aux barrières (avant les animaux qui y sont attachés)
            for (k, pos) in self.knots.iter() {
                let at = (pos.0 as f32 + 0.5, pos.1 as f32 + 0.25, pos.2 as f32 + 0.5);
                let far = (at.0 - c.pos.0).powi(2) + (at.2 - c.pos.2).powi(2);
                let has_chunk = c.delivered.contains(&(pos.0 >> 4, pos.2 >> 4));
                if far < VIEW * VIEW && has_chunk {
                    if c.mobs_seen.insert(*k) {
                        let meta = bedrock::mob_metadata(0.375, 0.5, 0, 0, 0);
                        out.push((*addr, bedrock::ID_ADD_ACTOR, bedrock::add_actor(*k, "minecraft:leash_knot", at, (0.0, 0.0, 0.0), 0.0, (1.0, 1.0), None, None, &meta)));
                    }
                } else if (far > (VIEW + 16.0) * (VIEW + 16.0) || !has_chunk) && c.mobs_seen.remove(k) {
                    out.push((*addr, bedrock::ID_REMOVE_ACTOR, bedrock::remove_actor(*k)));
                }
            }
            for m in self.mobs.values() {
                let far = (m.pos.0 - c.pos.0).powi(2) + (m.pos.2 - c.pos.2).powi(2);
                // Le jeu oublie une créature arrivée avant son chunk : elle attend qu'il soit parti
                let chunk = ((m.pos.0.floor() as i32) >> 4, (m.pos.2.floor() as i32) >> 4);
                let has_chunk = c.delivered.contains(&chunk);
                if far < VIEW * VIEW && m.dead.is_none() && has_chunk {
                    if c.mobs_seen.insert(m.id) {
                        out.push((*addr, bedrock::ID_ADD_ACTOR, Self::mob_packet(m, m.rider.and_then(|r| ids.get(&r).copied()), holder_of(m))));
                        // Monture : sa vitesse et son saut (le jeu du cavalier s'en sert)
                        if m.species.mount() && !refused_attributes {
                            out.push((*addr, bedrock::ID_UPDATE_ATTRIBUTES, bedrock::mount_attributes(m.id, m.movement(), m.jump)));
                        }
                        // Armure du cheval, tapis du lama
                        if let Some(body) = m.body_item().and_then(|n| item_ids.get(n)) {
                            out.push((*addr, bedrock::ID_MOB_ARMOR_EQUIPMENT, bedrock::mob_armor(m.id, Some((*body, 1)))));
                        }
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

    /// Un son de la créature, joué chez ceux qui la voient (sauf `except`, dont le jeu le
    /// joue déjà lui-même).
    fn mob_sound(&mut self, id: i64, sound: &str, except: Option<SocketAddr>) {
        if self.refused_extras.contains(&bedrock::ID_LEVEL_SOUND_EVENT) {
            return;
        }
        let Some(m) = self.mobs.get(&id) else { return };
        let packet = bedrock::level_sound(sound, m.pos, m.species.id(), m.baby(), id);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| Some(**a) != except && c.mobs_seen.contains(&id)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_LEVEL_SOUND_EVENT, &packet);
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
            // Fin de la ruade : elle se remet sur ses quatre pattes
            if m.rearing > 0 {
                m.rearing -= 1;
                if let Some(live) = self.mobs.get_mut(&id) {
                    live.rearing = m.rearing;
                }
                if m.rearing == 0 {
                    self.mob_changed(id);
                }
            }
            if let Some(rider) = m.rider {
                // En selle : la monture suit son cavalier (ou attend le dressage)
                let had_love = m.love > 0;
                self.mob_age(&mut m);
                let tame = !m.tamed && m.tame_in > 0 && {
                    m.tame_in -= 1;
                    m.tame_in == 0
                };
                // Le serveur la déplace lui-même : cochon dirigé par la carotte sur un bâton,
                // monture sauvage qui gambade avant d'accepter son cavalier (ou de le jeter)
                let (rider_yaw, held) = self.clients.get(&rider).map(|c| (c.yaw, self.held_name(c))).unwrap_or((0.0, String::new()));
                let moves = if m.species == Species::Pig && m.saddled {
                    if held == "carrot_on_a_stick" {
                        m.yaw = rider_yaw.rem_euclid(360.0);
                        let (dx, dz) = (-m.yaw.to_radians().sin(), m.yaw.to_radians().cos());
                        let speed = if m.boost > 0 { 0.23 } else { 0.115 };
                        m.boost = m.boost.saturating_sub(1);
                        m.vel.0 = dx * speed;
                        m.vel.2 = dz * speed;
                    } else {
                        m.vel.0 = 0.0;
                        m.vel.2 = 0.0;
                    }
                    true
                } else if m.species.mount() && !m.tamed {
                    if m.target.is_none() || self.mob_rng.below(15) == 0 {
                        let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
                        m.target = Some((m.pos.0 + a.cos() * 5.0, m.pos.2 + a.sin() * 5.0));
                    }
                    if let Some((tx, tz)) = m.target {
                        let (dx, dz) = (tx - m.pos.0, tz - m.pos.2);
                        let d = (dx * dx + dz * dz).sqrt().max(0.01);
                        let speed = ground_speed(m.movement() * 1.2);
                        m.vel.0 = dx / d * speed;
                        m.vel.2 = dz / d * speed;
                        m.yaw = (-dx).atan2(dz).to_degrees().rem_euclid(360.0);
                    }
                    true
                } else {
                    false
                };
                if moves && self.world.is_ready((m.pos.0.floor() as i32) >> 4, (m.pos.2.floor() as i32) >> 4) {
                    self.mob_physics(&mut m);
                    let moved = (m.pos.0 - m.sent.0).abs() + (m.pos.1 - m.sent.1).abs() + (m.pos.2 - m.sent.2).abs() > 0.01 || (m.yaw - m.sent_yaw).abs() > 3.0;
                    if moved {
                        m.sent = m.pos;
                        m.sent_yaw = m.yaw;
                        let packet = bedrock::move_actor_absolute(m.id, m.pos, 0.0, m.yaw, m.yaw, m.on_ground);
                        self.send_to_viewers(m.id, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
                        self.mobs_dirty = true;
                    }
                }
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
            // En laisse : où est celui qui la tient ? Trop loin (10 blocs), elle casse
            let leash_to = match m.leash {
                Some(Leash::Player(a)) => self.clients.get(&a).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2)),
                Some(Leash::Knot(_, p)) => Some((p.0 as f32 + 0.5, p.1 as f32, p.2 as f32 + 0.5)),
                None => None,
            };
            if let Some(h) = leash_to {
                if (h.0 - m.pos.0).powi(2) + (h.1 - m.pos.1).powi(2) + (h.2 - m.pos.2).powi(2) > LEASH_MAX * LEASH_MAX {
                    self.mobs.insert(id, m);
                    self.unleash(id, true);
                    continue;
                }
            }
            let had_love = m.love > 0;
            let grew = self.mob_age(&mut m);
            self.mob_think(&mut m, &players, leash_to);
            let before = m.pos;
            self.mob_physics(&mut m);
            // Chèvre arrêtée net par un bloc en pleine charge : une corne peut tomber
            if let Some((dx, dz, _)) = m.ram {
                if (m.pos.0 - before.0).abs() + (m.pos.2 - before.2).abs() < 0.01 {
                    m.ram = None;
                    self.goat_impact(&mut m, dx, dz);
                }
            }
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
            // Lapin : cherche de temps en temps des carottes plantées (16 blocs autour), et
            // grignote le plant arrivé dessus (il perd un stade, ou disparaît)
            Species::Rabbit if !m.baby() => {
                let (pos, garden, idle) = (m.pos, m.garden, m.target.is_none() && m.panic == 0);
                match garden {
                    Some(g) => {
                        let carrots = self.world.is_ready(g.0 >> 4, g.2 >> 4) && self.block_name_of(self.world.get(g.0, g.1, g.2)).as_deref() == Some("minecraft:carrots");
                        if !carrots {
                            if let Some(m) = self.mobs.get_mut(&id) {
                                m.garden = None;
                            }
                        } else if (pos.0 - g.0 as f32 - 0.5).powi(2) + (pos.2 - g.2 as f32 - 0.5).powi(2) < 0.8 && (pos.1 - g.1 as f32).abs() < 1.5 {
                            self.nibble_carrots(g);
                            if let Some(m) = self.mobs.get_mut(&id) {
                                m.garden = None;
                                m.target = None;
                            }
                        }
                    }
                    None if idle && self.mob_rng.below(400) == 0 => {
                        if let Some(g) = self.find_carrots(pos) {
                            if let Some(m) = self.mobs.get_mut(&id) {
                                m.garden = Some(g);
                            }
                        }
                    }
                    None => {}
                }
            }
            Species::Chicken if !m.baby() => {
                m.egg = m.egg.saturating_sub(1);
                if m.egg == 0 {
                    m.egg = 6000 + self.mob_rng.below(6000);
                    let pos = m.pos;
                    if let Some(egg) = self.item_id("egg") {
                        self.drop_item((pos.0, pos.1 + 0.3, pos.2), (0.0, 0.1, 0.0), egg, 1, 10);
                    }
                    self.mob_sound(id, bedrock::SOUND_PLOP, None);
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
    fn mob_think(&mut self, m: &mut Mob, players: &[((f32, f32, f32), String)], leash_to: Option<(f32, f32, f32)>) {
        if m.knock > 0 {
            m.knock -= 1;
            m.vel.0 *= 0.7;
            m.vel.2 *= 0.7;
            return;
        }
        // Chèvre en plein bond : elle garde son élan
        if m.leap > 0 {
            m.leap -= 1;
            return;
        }
        // Chèvre qui charge, tête baissée, en ligne droite
        if let Some((dx, dz, left)) = m.ram {
            m.vel.0 = dx * 0.4;
            m.vel.2 = dz * 0.4;
            m.yaw = (-dx).atan2(dz).to_degrees().rem_euclid(360.0);
            m.target = None;
            m.ram = (left > 0).then_some((dx, dz, left.saturating_sub(1)));
            // Un joueur sur sa route : renversé
            let hit = self
                .clients
                .iter()
                .filter(|(_, c)| c.joined && c.dead.is_none() && c.mode.takes_damage())
                .find(|(_, c)| (c.pos.0 - m.pos.0).powi(2) + (c.pos.2 - m.pos.2).powi(2) < 1.4 * 1.4 && (c.pos.1 - EYE - m.pos.1).abs() < 1.5)
                .map(|(a, c)| (*a, c.id));
            if let Some((a, pid)) = hit {
                m.ram = None;
                m.vel.0 = 0.0;
                m.vel.2 = 0.0;
                self.hurt(&a, if m.baby() { 1.0 } else { 2.0 }, Cause::Mob("une chèvre"));
                if !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) {
                    self.send(&a, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(pid, (dx * 1.2, 0.5, dz * 1.2)));
                }
            }
            return;
        }
        if m.species == Species::Goat {
            m.ram_cooldown = m.ram_cooldown.saturating_sub(1);
        }
        if m.species == Species::Llama {
            m.caravan = None;
        }
        let (stroll, panic, tempt, follow) = m.species.multipliers();
        let attr = m.movement();
        let mut speed = ground_speed(attr * stroll);
        let mut stop_at = 0.4;
        let tempting: &[&str] = if m.species.mount() && !m.tamed { &[] } else { m.species.tempt() };
        let near_player = |p: &(f32, f32, f32), r: f32| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2) < r * r && (p.1 - m.pos.1).abs() < 4.0;
        // Lapin : fuit le joueur qui ne lui tend pas sa nourriture (à moins de 8 blocs)
        let scary = if m.species == Species::Rabbit {
            players.iter().filter(|(p, held)| near_player(p, 8.0) && !tempting.contains(&held.as_str())).map(|(p, _)| *p).min_by(|a, b| {
                let d = |p: &(f32, f32, f32)| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2);
                d(a).total_cmp(&d(b))
            })
        } else {
            None
        };
        if let Some(h) = leash_to.filter(|h| (h.0 - m.pos.0).powi(2) + (h.2 - m.pos.2).powi(2) > LEASH_SOFT * LEASH_SOFT) {
            // En laisse, trop loin de celui qui la tient : elle le rejoint (tirée fort au-delà
            // de 6 blocs)
            let d = ((h.0 - m.pos.0).powi(2) + (h.2 - m.pos.2).powi(2)).sqrt();
            m.target = Some((h.0, h.2));
            stop_at = LEASH_SOFT - 1.0;
            speed = if d > LEASH_HARD { (ground_speed(attr * panic) + (d - LEASH_HARD) * 0.1).min(0.6) } else { ground_speed(attr * tempt) };
        } else if m.panic > 0 {
            m.panic -= 1;
            speed = ground_speed(attr * panic);
            // En fuite : une nouvelle direction de temps en temps
            if m.target.is_none() || self.mob_rng.below(20) == 0 {
                let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
                m.target = Some((m.pos.0 + a.cos() * 8.0, m.pos.2 + a.sin() * 8.0));
            }
        } else if let Some(p) = scary {
            let (dx, dz) = (m.pos.0 - p.0, m.pos.2 - p.2);
            let d = (dx * dx + dz * dz).sqrt().max(0.1);
            m.target = Some((m.pos.0 + dx / d * 6.0, m.pos.2 + dz / d * 6.0));
            // Au pas de course quand on s'approche (règles du jeu : 1,5 puis 1,8)
            speed = ground_speed(attr * if d < 4.0 { 1.8 } else { 1.5 });
        } else if let Some(partner) = self.partner_of(m) {
            // Amoureux : rejoint l'autre
            m.target = Some(partner);
            stop_at = 0.8;
            speed = ground_speed(attr);
        } else if let Some(p) = players.iter().filter(|(_, held)| tempting.contains(&held.as_str())).map(|(p, _)| *p).find(|p| near_player(p, 10.0)) {
            // Attiré par sa nourriture : suit le joueur, s'arrête à deux pas
            m.target = Some((p.0, p.2));
            stop_at = 2.5;
            speed = ground_speed(attr * tempt);
        } else if let Some(parent) = self.parent_of(m) {
            // Petit : suit un adulte de son espèce
            m.target = Some(parent);
            stop_at = 2.5;
            speed = ground_speed(attr * follow);
        } else if let Some((a, left)) = m.angry {
            // Lama en colère : il fait face à celui qui l'a frappé et lui crache dessus
            m.angry = (left > 1).then_some((a, left - 1));
            m.target = None;
            if let Some(p) = self.clients.get(&a).filter(|c| c.dead.is_none()).map(|c| c.pos) {
                m.yaw = (-(p.0 - m.pos.0)).atan2(p.2 - m.pos.2).to_degrees().rem_euclid(360.0);
                let d2 = (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2);
                if d2 < 10.0 * 10.0 && left == 1 {
                    self.spit(m, a);
                }
            }
        } else if m.species == Species::Goat && m.ram_cooldown == 0 && !m.baby() {
            // Chèvre : de temps en temps, elle fonce sur un joueur à 4 à 7 blocs
            let target = self
                .clients
                .values()
                .filter(|c| c.joined && c.dead.is_none() && c.mode.takes_damage())
                .map(|c| c.pos)
                .find(|p| {
                    let d2 = (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2);
                    (16.0..49.0).contains(&d2) && (p.1 - EYE - m.pos.1).abs() < 2.0
                });
            // Hurleuse : bien plus souvent
            m.ram_cooldown = if m.variant == 1 { 100 + self.mob_rng.below(200) } else { 600 + self.mob_rng.below(5400) };
            if let Some(p) = target {
                let (dx, dz) = (p.0 - m.pos.0, p.2 - m.pos.2);
                let d = (dx * dx + dz * dz).sqrt().max(0.1);
                m.ram = Some((dx / d, dz / d, 30));
            }
        } else if let Some(leader) = (m.species == Species::Llama && m.leash.is_none()).then(|| self.caravan_leader(m)).flatten() {
            // Lama : suit en file celui de devant (caravane derrière un lama tenu en laisse)
            m.caravan = Some(leader.0);
            m.target = Some((leader.1, leader.2));
            stop_at = 2.5;
            speed = ground_speed(attr * 2.1);
        } else if let Some(g) = m.garden {
            // Lapin : va grignoter un plant de carottes
            m.target = Some((g.0 as f32 + 0.5, g.2 as f32 + 0.5));
            stop_at = 0.6;
            speed = ground_speed(attr);
        } else if m.species == Species::Goat && m.target.is_none() && m.on_ground && self.mob_rng.below(600) == 0 {
            // Chèvre : bondit sur un bloc plus haut ou plus bas
            if let Some((dx, dy, dz)) = self.leap_spot(m) {
                const T: f32 = 12.0;
                m.vel = (dx / T, (dy + 0.5 * 0.08 * T * T) / T, dz / T);
                m.yaw = (-dx).atan2(dz).to_degrees().rem_euclid(360.0);
                m.on_ground = false;
                m.leap = T as u32;
                return;
            }
        } else if m.target.is_none() && self.mob_rng.below(120) == 0 {
            // Promenade, en évitant l'eau
            let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
            let d = 2.0 + self.mob_rng.unit() as f32 * 8.0;
            let t = (m.pos.0 + a.cos() * d, m.pos.2 + a.sin() * d);
            if !self.mob_water(t.0, m.pos.1 - 0.5, t.1) && !self.mob_water(t.0, m.pos.1 + 0.2, t.1) {
                m.target = Some(t);
            }
        } else if m.target.is_none() && self.mob_rng.below(50) == 0 {
            // Regarde un joueur proche de temps en temps (2 chances sur 100 par tick, 6 blocs)
            if let Some(p) = players.iter().map(|(p, _)| *p).find(|p| near_player(p, 6.0)) {
                m.yaw = (-(p.0 - m.pos.0)).atan2(p.2 - m.pos.2).to_degrees().rem_euclid(360.0);
            }
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

    /// Nom du bloc de cet état.
    fn block_name_of(&self, state: u32) -> Option<String> {
        self.data.as_ref().and_then(|d| d.block_name(state)).map(|n| n.to_string())
    }

    /// Un plant de carottes à moins de 16 blocs (même hauteur à un bloc près).
    fn find_carrots(&self, pos: (f32, f32, f32)) -> Option<(i32, i32, i32)> {
        let (x0, y0, z0) = (pos.0.floor() as i32, pos.1.floor() as i32, pos.2.floor() as i32);
        let mut best: Option<((i32, i32, i32), i32)> = None;
        for dx in -16..=16 {
            for dz in -16..=16 {
                let (x, z) = (x0 + dx, z0 + dz);
                if !self.world.is_ready(x >> 4, z >> 4) {
                    continue;
                }
                for dy in -1..=1 {
                    let y = y0 + dy;
                    if self.block_name_of(self.world.get(x, y, z)).as_deref() == Some("minecraft:carrots") {
                        let d = dx * dx + dz * dz;
                        if best.map(|b| d < b.1).unwrap_or(true) {
                            best = Some(((x, y, z), d));
                        }
                    }
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Le lapin grignote un plant de carottes : il perd un stade de croissance, ou disparaît.
    fn nibble_carrots(&mut self, g: (i32, i32, i32)) {
        let state = self.world.get(g.0, g.1, g.2);
        let next = match self.data.as_ref().and_then(|d| d.prop(state, "growth")) {
            Some(crate::data::Prop::Int(n)) if n > 0 => self.data.as_ref().and_then(|d| d.with_props(state, &[("growth", crate::data::Prop::Int(n - 1))])),
            _ => Some(self.world.air),
        };
        if let Some(next) = next {
            self.world.set(g.0, g.1, g.2, next);
            self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(g.0, g.1, g.2, next), None);
        }
    }

    /// Chèvre qui a foncé dans un bloc : sur du bois, de la pierre, un minerai ou de la glace
    /// compacte, une de ses cornes tombe (deux au plus).
    fn goat_impact(&mut self, m: &mut Mob, dx: f32, dz: f32) {
        let (x, y, z) = ((m.pos.0 + dx * 0.9).floor() as i32, m.pos.1.floor() as i32, (m.pos.2 + dz * 0.9).floor() as i32);
        let hard = [y, y + 1].iter().any(|&yy| {
            self.block_name_of(self.world.get(x, yy, z))
                .map(|n| n.ends_with("_log") || n.ends_with("_ore") || n == "minecraft:stone" || n == "minecraft:packed_ice" || n.ends_with("_stem"))
                .unwrap_or(false)
        });
        if hard && m.horns > 0 && !m.baby() {
            m.horns -= 1;
            if let Some(horn) = self.item_id("goat_horn") {
                self.drop_item((m.pos.0, m.pos.1 + 0.8, m.pos.2), (-dx * 0.2, 0.25, -dz * 0.2), horn, 1, 10);
            }
            self.mobs_dirty = true;
        }
    }

    /// Lama : celui qu'il peut suivre en caravane (à moins de 8 blocs, tenu en laisse ou déjà
    /// dans une file, sans boucle, 10 lamas au plus, un seul suiveur chacun).
    fn caravan_leader(&self, m: &Mob) -> Option<(i64, f32, f32)> {
        let depth = |mut id: i64| -> Option<usize> {
            for n in 0..10 {
                let o = self.mobs.get(&id)?;
                if o.id == m.id {
                    return None; // boucle
                }
                if matches!(o.leash, Some(Leash::Player(_))) {
                    return Some(n);
                }
                id = o.caravan?;
            }
            None
        };
        self.mobs
            .values()
            .filter(|o| o.id != m.id && o.species == Species::Llama && o.dead.is_none())
            .filter(|o| (o.pos.0 - m.pos.0).powi(2) + (o.pos.2 - m.pos.2).powi(2) < 64.0)
            .filter(|o| !self.mobs.values().any(|f| f.id != m.id && f.caravan == Some(o.id)))
            .filter(|o| depth(o.id).map(|d| d < 9).unwrap_or(false))
            .min_by(|a, b| {
                let d = |o: &Mob| (o.pos.0 - m.pos.0).powi(2) + (o.pos.2 - m.pos.2).powi(2);
                // Celui qu'il suivait déjà d'abord
                (Some(a.id) != m.caravan).cmp(&(Some(b.id) != m.caravan)).then(d(a).total_cmp(&d(b)))
            })
            .map(|o| (o.id, o.pos.0, o.pos.2))
    }

    /// Chèvre : un bloc où bondir, plus haut ou plus bas d'1 à 3 blocs, à 2 à 4 blocs de là
    /// (écart par rapport à sa position).
    fn leap_spot(&mut self, m: &Mob) -> Option<(f32, f32, f32)> {
        for _ in 0..6 {
            let (dx, dz) = (self.mob_rng.below(9) as i32 - 4, self.mob_rng.below(9) as i32 - 4);
            if dx.abs().max(dz.abs()) < 2 {
                continue;
            }
            let (x, z) = (m.pos.0.floor() as i32 + dx, m.pos.2.floor() as i32 + dz);
            let base = m.pos.1.floor() as i32;
            for dy in [3, 2, 1, -1, -2] {
                let y = base + dy;
                if !self.mob_free(x, y - 1, z) && self.mob_free(x, y, z) && self.mob_free(x, y + 1, z) && !self.mob_water(x as f32 + 0.5, y as f32 + 0.2, z as f32 + 0.5) {
                    return Some((x as f32 + 0.5 - m.pos.0, y as f32 - m.pos.1, z as f32 + 0.5 - m.pos.2));
                }
            }
        }
        None
    }

    /// Le lama crache sur ce joueur : un crachat part de sa bouche, droit sur lui.
    fn spit(&mut self, m: &Mob, at: SocketAddr) {
        let Some(target) = self.clients.get(&at).map(|c| (c.pos.0, c.pos.1 - 0.6, c.pos.2)) else { return };
        // Part de la bouche : devant la tête, dans la direction de la cible
        let (fx, fz) = (target.0 - m.pos.0, target.2 - m.pos.2);
        let flat = (fx * fx + fz * fz).sqrt().max(0.1);
        let from = (m.pos.0 + fx / flat * 0.7, m.pos.1 + 1.6, m.pos.2 + fz / flat * 0.7);
        let (dx, dy, dz) = (target.0 - from.0, target.1 - from.1, target.2 - from.2);
        let d = (dx * dx + dy * dy + dz * dz).sqrt().max(0.1);
        // Un peu moins vite que le vrai crachat, pour qu'on le voie partir même de près
        let vel = (dx / d * 0.9, dy / d * 0.9, dz / d * 0.9);
        let id = self.next_id;
        self.next_id += 1;
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined && (c.pos.0 - from.0).powi(2) + (c.pos.2 - from.2).powi(2) < 64.0 * 64.0).map(|(a, _)| *a).collect();
        let meta = bedrock::projectile_metadata(m.id);
        let yaw = (-fx).atan2(fz).to_degrees();
        let packet = bedrock::add_actor(id, "minecraft:llama_spit", from, vel, yaw, (1.0, 1.0), None, None, &meta);
        // Son élan aussi, pour que le jeu le fasse voler entre deux positions
        let motion = bedrock::set_actor_motion(id, vel);
        for v in &viewers {
            self.send(v, bedrock::ID_ADD_ACTOR, &packet);
            if !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) {
                self.send(v, bedrock::ID_SET_ACTOR_MOTION, &motion);
            }
        }
        self.mob_sound(m.id, bedrock::SOUND_SPIT, None);
        self.spits.push(Spit { id, pos: from, vel, ttl: 30, target: at, viewers });
    }

    /// Les crachats en vol avancent ; celui qui touche son joueur lui retire un demi-cœur.
    pub(super) fn move_spits(&mut self) {
        let mut spits = std::mem::take(&mut self.spits);
        spits.retain_mut(|s| {
            s.pos = (s.pos.0 + s.vel.0, s.pos.1 + s.vel.1, s.pos.2 + s.vel.2);
            s.ttl = s.ttl.saturating_sub(1);
            let hit = self.clients.get(&s.target).filter(|c| c.dead.is_none()).map(|c| (c.pos.0 - s.pos.0).powi(2) + (c.pos.1 - 0.8 - s.pos.1).powi(2) + (c.pos.2 - s.pos.2).powi(2) < 1.2 * 1.2).unwrap_or(false);
            let wall = !self.mob_free(s.pos.0.floor() as i32, s.pos.1.floor() as i32, s.pos.2.floor() as i32);
            if hit {
                let takes = self.clients.get(&s.target).map(|c| c.mode.takes_damage()).unwrap_or(false);
                if takes {
                    self.hurt(&s.target, 1.0, Cause::Mob("un lama"));
                }
            }
            if hit || wall || s.ttl == 0 {
                for v in &s.viewers {
                    self.send(v, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(s.id));
                }
                return false;
            }
            let packet = bedrock::move_actor_snap(s.id, s.pos, (-s.vel.0).atan2(s.vel.2).to_degrees());
            let motion = bedrock::set_actor_motion(s.id, s.vel);
            let motion_ok = !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION);
            for v in &s.viewers {
                self.send(v, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
                if motion_ok {
                    self.send(v, bedrock::ID_SET_ACTOR_MOTION, &motion);
                }
            }
            true
        });
        self.spits.append(&mut spits);
    }

    /// Petit : position d'un adulte de son espèce entre 3 et 8 blocs (il le rejoint).
    fn parent_of(&self, m: &Mob) -> Option<(f32, f32)> {
        if !m.baby() {
            return None;
        }
        let near = self
            .mobs
            .values()
            .filter(|o| o.species == m.species && !o.baby() && o.dead.is_none())
            .map(|o| (o.pos.0, o.pos.2, (o.pos.0 - m.pos.0).powi(2) + (o.pos.2 - m.pos.2).powi(2)))
            .filter(|o| o.2 < 64.0)
            .min_by(|a, b| a.2.total_cmp(&b.2))?;
        (near.2 > 9.0).then_some((near.0, near.1))
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
                // Qualités des parents (montures) : le petit tient de leur moyenne et d'un tirage
                let stats = |s: &Self, id: i64| s.mobs.get(&id).map(|m| (m.max_health, m.move_speed, m.jump)).unwrap_or((0.0, 0.0, 0.0));
                let (pa, pb) = (stats(self, a.0), stats(self, b.0));
                let baby = self.add_mob(kind, at);
                if let Some(m) = self.mobs.get_mut(&baby) {
                    m.age = -GROW_TICKS;
                    m.color = color;
                    m.variant = variant;
                    m.mark = if kind == Species::Horse { mark } else { 0 };
                    if kind.mount() {
                        // Né de parents apprivoisés : apprivoisé lui aussi, comme dans le jeu
                        m.tamed = true;
                        if kind == Species::Horse {
                            // (vie, vitesse, saut) : la moyenne des deux parents et d'un tirage
                            m.max_health = ((pa.0 + pb.0 + m.max_health) / 3.0).round().clamp(15.0, 30.0);
                            m.health = m.max_health;
                            m.move_speed = ((pa.1 + pb.1 + m.move_speed) / 3.0).clamp(0.1125, 0.3375);
                            m.jump = ((pa.2 + pb.2 + m.jump) / 3.0).clamp(0.4, 1.0);
                        }
                    }
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
        // Au sol en début de tick (le bond du lapin ne compte pas : il peut aussi gravir une marche)
        let grounded = m.on_ground;
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
            } else if grounded && body_free(self, ex, m.pos.1.floor() + 1.01, ez) {
                // Marche d'un bloc : saute
                m.vel.1 = m.vel.1.max(0.42);
                m.on_ground = false;
            } else if !grounded && m.vel.1 > -0.3 && body_free(self, ex, m.pos.1.floor() + 1.01, ez) {
                // En plein saut vers la marche : il continue de monter, sans changer d'idée
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
    /// Dégâts de chute d'une monture : vie retirée, sans recul ni fuite (son cavalier la mène).
    fn fall_hurt_mob(&mut self, id: i64, damage: f32) {
        let tick = self.tick;
        let Some(m) = self.mobs.get_mut(&id) else { return };
        if m.dead.is_some() {
            return;
        }
        m.hurt_tick = tick;
        m.health -= damage;
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
    }

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
        // Lama frappé : il crache sur le joueur le plus proche du coup, une dizaine de secondes
        if m.species == Species::Llama && !dead {
            let attacker = self.clients.iter().filter(|(_, c)| c.joined).min_by(|a, b| {
                let d = |c: &Client| (c.pos.0 - from.0).powi(2) + (c.pos.2 - from.2).powi(2);
                d(a.1).total_cmp(&d(b.1))
            });
            // (un seul crachat par coup reçu, juste après)
            if let (Some((a, _)), Some(m)) = (attacker.map(|(a, c)| (*a, c.id)), self.mobs.get_mut(&id)) {
                m.angry = Some((a, 10));
                m.panic = 0;
            }
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
        let (held, creative, near, riding, sneaking) = match self.clients.get(addr) {
            Some(c) => {
                let h = c.inv.held();
                let name = if h.is_empty() { String::new() } else { self.data.as_ref().and_then(|d| d.item_by_id(h.id)).map(|d| d.name.trim_start_matches("minecraft:").to_string()).unwrap_or_default() };
                let near = (c.pos.0 - m.pos.0).powi(2) + (c.pos.2 - m.pos.2).powi(2) < 8.0 * 8.0;
                (name, c.mode == GameMode::Creative, near, c.riding.is_some(), c.sneaking)
            }
            None => return true,
        };
        // Accroupi devant une monture apprivoisée : son inventaire (selle, armure, coffre)
        if sneaking && !riding && m.species.mount() && m.tamed && !m.baby() && self.open_mob_inventory(addr, id) {
            return true;
        }
        if !near {
            return true;
        }
        // En laisse : main vide ou une laisse en main, on la détache (elle tombe)
        if m.leash.is_some() && (held.is_empty() || held == "lead") {
            self.unleash(id, true);
            return true;
        }
        // Une laisse en main : il est attaché au joueur
        if held == "lead" && m.leash.is_none() && m.rider.is_none() {
            if let Some(mob) = self.mobs.get_mut(&id) {
                mob.leash = Some(Leash::Player(*addr));
                mob.target = None;
            }
            self.mob_changed(id);
            self.mob_sound(id, bedrock::SOUND_LEASH_PLACE, None);
            if !creative {
                if let Some(c) = self.clients.get_mut(addr) {
                    c.inv.consume_held();
                }
                self.send_inventory(addr);
                self.show_held(addr);
            }
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
            self.mob_sound(id, bedrock::SOUND_SHEAR, None);
            changed = true;
        } else if held == "shears" && m.species == Species::Mooshroom && !m.baby() {
            // Champimeuh tondue : 5 champignons de sa couleur, et elle redevient une vache
            let mushroom = if m.variant == 1 { "brown_mushroom" } else { "red_mushroom" };
            if let Some(item) = self.item_id(mushroom) {
                let vel = ((self.mob_rng.unit() as f32 - 0.5) * 0.2, 0.25, (self.mob_rng.unit() as f32 - 0.5) * 0.2);
                self.drop_item((m.pos.0, m.pos.1 + 1.0, m.pos.2), vel, item, 5, 10);
            }
            self.mob_sound(id, bedrock::SOUND_SHEAR, None);
            self.transform_mob(id, Species::Cow);
        } else if held == "bucket" && matches!(m.species, Species::Cow | Species::Mooshroom | Species::Goat) && !m.baby() {
            // Traite : le seau vide devient un seau de lait
            self.mob_sound(id, bedrock::SOUND_MILK, None);
            swap(self, "milk_bucket");
        } else if held == "bowl" && m.species == Species::Mooshroom && !m.baby() {
            // Champimeuh : un bol devient une soupe de champignons (suspecte si la brune a mangé
            // une fleur)
            self.mob_sound(id, bedrock::SOUND_MILK, None);
            if m.stew {
                if let Some(s) = self.mobs.get_mut(&id) {
                    s.stew = false;
                }
                swap(self, "suspicious_stew");
            } else {
                swap(self, "mushroom_stew");
            }
        } else if m.species == Species::Mooshroom && m.variant == 1 && !m.stew && !m.baby() && FLOWERS.contains(&held.as_str()) {
            // Champimeuh brune : elle mange la fleur
            if let Some(s) = self.mobs.get_mut(&id) {
                s.stew = true;
            }
            used = true;
        } else if held == "saddle" && !m.saddled && !m.baby() && ((m.tamed && m.species.steerable()) || m.species == Species::Pig) {
            // Selle posée : son cavalier pourra la diriger (le cochon, avec une carotte sur un bâton)
            if let Some(s) = self.mobs.get_mut(&id) {
                s.saddled = true;
                s.fit_items();
                if let Some(cell) = s.items.first_mut() {
                    *cell = Some(("saddle".into(), 1));
                }
            }
            self.mob_sound(id, bedrock::SOUND_SADDLE, None);
            changed = true;
            used = true;
        } else if held == "chest" && m.tamed && !m.chested && !m.baby() && matches!(m.species, Species::Donkey | Species::Mule | Species::Llama) {
            // Un coffre sur le dos : de quoi transporter des objets
            if let Some(s) = self.mobs.get_mut(&id) {
                s.chested = true;
                s.fit_items();
            }
            self.mob_sound(id, bedrock::SOUND_ARMOR, None);
            changed = true;
            used = true;
        } else if m.tamed && !m.baby() && m.body_item().is_none() && ((m.species == Species::Horse && held.ends_with("_horse_armor")) || (m.species == Species::Llama && held.ends_with("_carpet"))) {
            // Armure du cheval, tapis du lama
            if let Some(s) = self.mobs.get_mut(&id) {
                s.fit_items();
                if let Some(cell) = s.items.get_mut(1) {
                    *cell = Some((held.clone(), 1));
                }
            }
            if let Some(item) = self.item_id(&held) {
                self.send_to_viewers(id, bedrock::ID_MOB_ARMOR_EQUIPMENT, &bedrock::mob_armor(id, Some((item, 1))));
            }
            self.mob_sound(id, bedrock::SOUND_ARMOR, None);
            self.mobs_dirty = true;
            used = true;
        } else if let (Species::Sheep, Some(color)) = (m.species, dye_color(&held)) {
            // Teinture : la laine prend sa couleur
            if color != m.color {
                if let Some(s) = self.mobs.get_mut(&id) {
                    s.color = color;
                }
                changed = true;
                used = true;
            }
        } else if breeds && !m.baby() && m.age == 0 && m.love == 0 {
            // Prête à avoir un petit (une monture blessée est soignée en même temps)
            if let Some(s) = self.mobs.get_mut(&id) {
                s.love = LOVE_TICKS;
                s.health = (s.health + heal.unwrap_or(0.0)).min(s.max_health);
            }
            self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_LOVE_PARTICLES, 0));
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
        } else if (m.species.mount() || (m.species == Species::Pig && m.saddled)) && !m.baby() && m.rider.is_none() && !riding && !m.species.breed_items().contains(&held.as_str()) {
            // Main vide (ou un objet sans usage ici) : en selle
            self.mount_mob(addr, id);
        }
        // Un repas donné : elle mange
        let food = grow.is_some() || heal.is_some() || breeds || temper.is_some() || FLOWERS.contains(&held.as_str());
        if used && food {
            self.mob_sound(id, bedrock::SOUND_EAT, None);
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
    pub(super) fn summon_mob(&mut self, name: &str, pos: (f32, f32, f32), baby: bool, tamed: bool) -> Result<(), String> {
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
            // Événement « minecraft:on_tame » du jeu : une monture déjà apprivoisée
            m.tamed = tamed && species.mount();
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

    /// Nom (sans « minecraft: ») de l'objet tenu en main par ce joueur.
    pub(super) fn held_name(&self, c: &Client) -> String {
        let h = c.inv.held();
        if h.is_empty() {
            return String::new();
        }
        self.data.as_ref().and_then(|d| d.item_by_id(h.id)).map(|d| d.name.trim_start_matches("minecraft:").to_string()).unwrap_or_default()
    }

    /// Clic dans le vide : sur un cochon sellé, la carotte sur un bâton le fait accélérer
    /// (le double de sa vitesse pendant 3 secondes, comme dans le jeu). Vrai si c'est le cas.
    pub(super) fn boost_mount(&mut self, addr: &SocketAddr) -> bool {
        let Some(c) = self.clients.get(addr) else { return false };
        let Some(id) = c.riding else { return false };
        if self.held_name(c) != "carrot_on_a_stick" {
            return false;
        }
        match self.mobs.get_mut(&id) {
            Some(m) if m.species == Species::Pig && m.saddled && m.boost == 0 => {
                m.boost = 60;
                true
            }
            _ => false,
        }
    }

    /// Ouvre l'inventaire d'une monture apprivoisée (selle, armure ou tapis, coffre). Faux
    /// si ce n'est pas possible (sauvage, petit, un autre écran ouvert).
    pub(super) fn open_mob_inventory(&mut self, addr: &SocketAddr, id: i64) -> bool {
        match self.mobs.get(&id) {
            Some(m) if m.species.mount() && m.tamed && !m.baby() && m.dead.is_none() => {}
            _ => return false,
        }
        if self.clients.get(addr).map(|c| c.inv.open_window != 0).unwrap_or(true) {
            return false;
        }
        // Un seul joueur à la fois dans un même inventaire
        if self.clients.values().any(|c| c.inv.block_mob == Some(id)) {
            return false;
        }
        let Some(slots) = self.mob_inventory_view(id) else { return false };
        const WINDOW: u8 = 2;
        if let Some(c) = self.clients.get_mut(addr) {
            c.inv.open_window = WINDOW;
            c.inv.block_mob = Some(id);
            c.inv.block_is_furnace = false;
            c.inv.block.slots = slots.clone();
        }
        self.send(addr, bedrock::ID_CONTAINER_OPEN, &bedrock::container_open_entity(WINDOW, bedrock::SCREEN_HORSE, id));
        // Les cases d'équipement et ce qu'elles acceptent (selle ; armure du cheval, tapis du lama)
        let species = self.mobs.get(&id).map(|m| m.species);
        const COLORS: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
        let carpets: Vec<String> = COLORS.iter().map(|c| format!("{}_carpet", c)).collect();
        // Icônes des cases vides : la vraie selle, la vraie armure (les icônes « horsesaddle » /
        // « horsearmor » d'autres serveurs n'ont rien changé au plantage)
        let variant = self.cfg.mount_screen;
        let (saddle_icon, armor_icon, carpet_icon) = ("saddle", "iron_horse_armor", "white_carpet");
        let mut equip: Vec<(i32, &str, Vec<&str>)> = Vec::new();
        match species {
            Some(Species::Horse) => {
                equip.push((0, saddle_icon, vec!["saddle"]));
                equip.push((1, armor_icon, vec!["leather_horse_armor", "iron_horse_armor", "golden_horse_armor", "diamond_horse_armor"]));
            }
            Some(Species::Donkey | Species::Mule) => equip.push((0, saddle_icon, vec!["saddle"])),
            Some(Species::Llama) => equip.push((1, carpet_icon, carpets.iter().map(|s| s.as_str()).collect())),
            _ => {}
        }
        let size = slots.len() as i32;
        info!("{} : inventaire de monture, écriture {} (debug-mount-screen)", addr, variant);
        self.send(addr, bedrock::ID_UPDATE_EQUIP, &bedrock::update_equip(WINDOW, id, size, &equip));
        self.send_block_content(addr, &slots);
        true
    }

    /// L'inventaire d'une monture, en piles d'objets (numérotées pour le jeu).
    pub(super) fn mob_inventory_view(&mut self, id: i64) -> Option<Vec<inventory::ItemStack>> {
        let m = self.mobs.get_mut(&id).filter(|m| m.dead.is_none())?;
        m.fit_items();
        let items = m.items.clone();
        let mut slots = Vec::with_capacity(items.len());
        for cell in items {
            let stack = match cell.and_then(|(name, count)| self.item_id(&name).map(|i| (i, count))) {
                Some((item, count)) => {
                    self.tile_net_id = if self.tile_net_id >= i32::MAX - 1 { 1_000_000_000 } else { self.tile_net_id + 1 };
                    inventory::ItemStack::new(item, count, self.tile_net_id)
                }
                None => inventory::ItemStack::EMPTY,
            };
            slots.push(stack);
        }
        Some(slots)
    }

    /// Reprend l'inventaire d'une monture après une demande du jeu. Selle et armure ne
    /// prennent que ce qui leur va : le reste retourne dans le sac du joueur.
    pub(super) fn store_mob_inventory(&mut self, addr: &SocketAddr, id: i64, slots: &[inventory::ItemStack]) {
        let names: Vec<Option<(String, u8)>> = slots
            .iter()
            .map(|s| if s.is_empty() { None } else { self.data.as_ref().and_then(|d| d.item_by_id(s.id)).map(|d| (d.name.trim_start_matches("minecraft:").to_string(), s.count)) })
            .collect();
        let Some(m) = self.mobs.get_mut(&id) else { return };
        let (species, before_body, before_saddle) = (m.species, m.body_item().map(|s| s.to_string()), m.saddled);
        let mut refused: Vec<(String, u8)> = Vec::new();
        let mut items = names;
        items.resize(m.inventory_size(), None);
        let fits = |slot: usize, name: &str| match slot {
            0 => name == "saddle" && species.steerable(),
            1 => match species {
                Species::Horse => name.ends_with("_horse_armor"),
                Species::Llama => name.ends_with("_carpet"),
                _ => false,
            },
            _ => true,
        };
        for (slot, cell) in items.iter_mut().enumerate() {
            if let Some((name, count)) = cell.clone() {
                if !fits(slot, &name) || (slot < 2 && count > 1) {
                    refused.push((name, count));
                    *cell = None;
                }
            }
        }
        m.items = items;
        m.saddled = m.items.first().map(|i| i.is_some()).unwrap_or(false);
        let after_body = m.body_item().map(|s| s.to_string());
        let after_saddle = m.saddled;
        self.mobs_dirty = true;
        if before_body != after_body {
            let body = after_body.as_deref().and_then(|n| self.item_id(n)).map(|i| (i, 1));
            self.send_to_viewers(id, bedrock::ID_MOB_ARMOR_EQUIPMENT, &bedrock::mob_armor(id, body));
            self.mob_sound(id, bedrock::SOUND_ARMOR, None);
        }
        if before_saddle != after_saddle {
            if after_saddle {
                self.mob_sound(id, bedrock::SOUND_SADDLE, None);
            }
            self.mob_changed(id);
        }
        if !refused.is_empty() {
            for (name, count) in refused {
                if let Some(item) = self.item_id(&name) {
                    self.add_item(addr, item, count as u16);
                }
            }
            if let Some(view) = self.mob_inventory_view(id) {
                if let Some(c) = self.clients.get_mut(addr) {
                    c.inv.block.slots = view.clone();
                }
                self.send_block_content(addr, &view);
            }
            self.send_inventory(addr);
        }
    }

    /// Une barrière (pas un portillon) : on peut y attacher une laisse.
    pub(super) fn is_fence(&self, state: u32) -> bool {
        self.data.as_ref().and_then(|d| d.block_name(state)).map(|n| n.ends_with("fence") && !n.contains("gate")).unwrap_or(false)
    }

    /// Nœud de laisse de la barrière de ce bloc (créé s'il n'y en a pas encore).
    fn knot_at(&mut self, pos: (i32, i32, i32)) -> i64 {
        if let Some((k, _)) = self.knots.iter().find(|(_, p)| **p == pos) {
            return *k;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.knots.insert(id, pos);
        id
    }

    /// Retire un nœud de laisse (chez tous ceux qui le voient).
    fn remove_knot(&mut self, k: i64) {
        self.knots.remove(&k);
        let viewers: Vec<SocketAddr> = self.clients.iter_mut().filter_map(|(a, c)| c.mobs_seen.remove(&k).then_some(*a)).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(k));
        }
    }

    /// Détache une créature ; `drop` : la laisse tombe par terre (rompue, ou détachée à la main).
    fn unleash(&mut self, id: i64, drop: bool) {
        let Some(m) = self.mobs.get_mut(&id) else { return };
        if m.leash.take().is_none() {
            return;
        }
        let pos = m.pos;
        if drop {
            if let Some(lead) = self.item_id("lead") {
                self.drop_item((pos.0, pos.1 + 0.5, pos.2), (0.0, 0.2, 0.0), lead, 1, 10);
            }
            self.mob_sound(id, bedrock::SOUND_LEASH_BREAK, None);
        }
        self.mob_changed(id);
    }

    /// Clic sur un nœud de laisse : les animaux attachés sont libérés (les laisses tombent),
    /// le nœud disparaît. Rend vrai si c'était un nœud.
    pub(super) fn knot_clicked(&mut self, k: i64) -> bool {
        if !self.knots.contains_key(&k) {
            return false;
        }
        let tied: Vec<i64> = self.mobs.values().filter(|m| matches!(m.leash, Some(Leash::Knot(id, _)) if id == k)).map(|m| m.id).collect();
        for id in tied {
            self.unleash(id, true);
        }
        self.remove_knot(k);
        true
    }

    /// Clic droit sur une barrière : les animaux que le joueur tient en laisse (à moins de
    /// 10 blocs) y sont attachés, par un nœud. Rend vrai si au moins un l'a été.
    pub(super) fn tie_to_fence(&mut self, addr: &SocketAddr, pos: (i32, i32, i32)) -> bool {
        let held: Vec<i64> = self
            .mobs
            .values()
            .filter(|m| m.leash == Some(Leash::Player(*addr)))
            .filter(|m| (m.pos.0 - pos.0 as f32 - 0.5).powi(2) + (m.pos.1 - pos.1 as f32).powi(2) + (m.pos.2 - pos.2 as f32 - 0.5).powi(2) < LEASH_MAX * LEASH_MAX)
            .map(|m| m.id)
            .collect();
        if held.is_empty() {
            return false;
        }
        let k = self.knot_at(pos);
        // Le nœud doit exister chez les joueurs avant que les laisses s'y accrochent
        self.show_mobs();
        for id in held {
            if let Some(m) = self.mobs.get_mut(&id) {
                m.leash = Some(Leash::Knot(k, pos));
            }
            self.mob_changed(id);
            self.mob_sound(id, bedrock::SOUND_LEASH_PLACE, None);
        }
        true
    }

    /// Laisses à vérifier de temps en temps : joueur parti ou mort, barrière cassée, nœud
    /// qui ne retient plus personne.
    fn check_leashes(&mut self) {
        // (Chunk pas encore prêt : la barrière est supposée toujours là)
        let fence = |s: &Self, p: (i32, i32, i32)| !s.world.is_ready(p.0 >> 4, p.2 >> 4) || s.is_fence(s.world.get(p.0, p.1, p.2));
        let broken: Vec<i64> = self
            .mobs
            .values()
            .filter(|m| match m.leash {
                Some(Leash::Player(a)) => self.clients.get(&a).map(|c| c.dead.is_some() || !c.joined).unwrap_or(true),
                Some(Leash::Knot(_, p)) => !fence(self, p),
                None => false,
            })
            .map(|m| m.id)
            .collect();
        for id in broken {
            self.unleash(id, true);
        }
        let used: HashSet<i64> = self.mobs.values().filter_map(|m| match m.leash {
            Some(Leash::Knot(k, _)) => Some(k),
            _ => None,
        }).collect();
        let unused: Vec<i64> = self.knots.keys().filter(|k| !used.contains(k)).copied().collect();
        for k in unused {
            self.remove_knot(k);
        }
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
        m.ride_offset = None;
        m.target = None;
        m.vel = (0.0, 0.0, 0.0);
        // (le cochon sellé n'a pas à être apprivoisé)
        let (seat, wild) = (m.species.seat(), !m.tamed && m.species.mount());
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
        if let Some(m) = self.mobs.get(&id) {
            let attributes = bedrock::mount_attributes(id, m.movement(), m.jump);
            self.send_to_viewers(id, bedrock::ID_UPDATE_ATTRIBUTES, &attributes);
        }
        self.rider_data(addr, Some(seat));
        self.mob_changed(id);
    }

    /// Siège d'une monture (pour montrer son cavalier aux autres joueurs).
    pub(super) fn mob_seat(&self, id: i64) -> Option<((f32, f32, f32), (f32, f32, f32))> {
        self.mobs.get(&id).map(|m| (m.species.seat(), m.pos))
    }

    /// Le cavalier descend (de lui-même, jeté par une monture sauvage, mort ou parti).
    pub(super) fn dismount(&mut self, addr: &SocketAddr) {
        // Son inventaire ouvert se ferme
        if self.clients.get(addr).map(|c| c.inv.block_mob.is_some()).unwrap_or(false) {
            self.force_close(addr);
        }
        let tick = self.tick + 1;
        // Il descend debout à la place de sa monture : la position que donnait son jeu en selle
        // est sous elle, dans le sol (enregistrée telle quelle, il reviendrait ailleurs)
        let stand = self.clients.get(addr).and_then(|c| c.riding).and_then(|id| self.mobs.get(&id)).map(|m| (m.pos.0, m.pos.1 + crate::player::EYE_HEIGHT, m.pos.2));
        let (mob, player) = match self.clients.get_mut(addr) {
            Some(c) => match c.riding.take() {
                // Sa position est de nouveau envoyée aux autres
                Some(m) => {
                    c.moved_tick = tick;
                    if let Some(p) = stand {
                        c.pos = p;
                    }
                    (m, c.id)
                }
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
                // Elle se cabre une seconde
                m.rearing = 20;
            }
            self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_TAME_FAIL, 0));
            self.mob_sound(id, bedrock::SOUND_MAD, None);
            self.mob_changed(id);
            if let Some(r) = rider {
                self.dismount(&r);
            }
        }
    }

    /// Position de sa monture envoyée par le jeu du cavalier (il la dirige lui-même) : elle
    /// est reprise et montrée aux autres joueurs.
    pub(super) fn on_rider_move(&mut self, addr: &SocketAddr, body: &[u8]) {
        let Some((runtime, pos, _pitch, yaw)) = bedrock::read_move_actor(body) else { return };
        if self.cfg.debug && self.tick % 20 == 0 {
            info!("[selle] le jeu déplace sa monture {} en {:.2} {:.2} {:.2}", runtime, pos.0, pos.1, pos.2);
        }
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
        let rider = m.rider;
        let was_airborne = m.airborne;
        // Au sol si un bloc la porte (sinon le jeu des autres la montre en plein saut)
        let (bx, bz) = (pos.0.floor() as i32, pos.2.floor() as i32);
        // (au moins un demi-bloc de vide dessous : une marche descendue n'est pas un saut)
        let airborne = self.mob_free(bx, (pos.1 - 0.05).floor() as i32, bz) && self.mob_free(bx, (pos.1 - 0.6).floor() as i32, bz);
        // Chute : la plus haute position en l'air ; à l'atterrissage, des dégâts comme dans le
        // jeu (au-delà de 3 blocs, moins 5 pour les chevaux, ânes et mules ; aucun dans l'eau)
        let mut landed_from = None;
        let in_water = self.mob_water(pos.0, pos.1 + 0.1, pos.2) || self.mob_water(pos.0, pos.1 - 0.5, pos.2);
        if let Some(m) = self.mobs.get_mut(&id) {
            m.airborne = airborne;
            if in_water {
                m.fall_peak = None;
            } else if airborne {
                m.fall_peak = Some(m.fall_peak.map(|p| p.max(pos.1)).unwrap_or(pos.1));
            } else if let Some(peak) = m.fall_peak.take() {
                landed_from = Some(peak - pos.1);
            }
        }
        if let Some(distance) = landed_from {
            let horse_like = self.mobs.get(&id).map(|m| matches!(m.species, Species::Horse | Species::Donkey | Species::Mule)).unwrap_or(false);
            let damage = if horse_like { mount_fall_damage(distance) } else { ((distance - 3.0).ceil()).max(0.0) };
            if damage > 0.0 && !in_water {
                self.fall_hurt_mob(id, damage);
            }
        }
        if airborne != was_airborne {
            // Saut et atterrissage : son jeu les joue pour lui, pas pour les autres
            self.mob_sound(id, if airborne { bedrock::SOUND_JUMP } else { bedrock::SOUND_LAND }, rider);
        }
        let Some(m) = self.mobs.get_mut(&id) else { return };
        let moved = (m.pos.0 - m.sent.0).abs() + (m.pos.1 - m.sent.1).abs() + (m.pos.2 - m.sent.2).abs() > 0.01 || (m.yaw - m.sent_yaw).abs() > 3.0;
        if !moved {
            return;
        }
        m.sent = m.pos;
        m.sent_yaw = m.yaw;
        let packet = bedrock::move_actor_absolute(id, pos, 0.0, m.yaw, m.yaw, !airborne);
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
        let driven = m.driven;
        // Sous le cavalier, posée sur le sol : la hauteur que donne son jeu en selle n'est pas
        // fiable (elle mettait la monture sous terre)
        let (x, z) = (pos.0.floor() as i32, pos.2.floor() as i32);
        let offset = m.ride_offset;
        // Hauteur d'après l'écart mesuré au sol : un saut se voit. Inconnu, ou la monture se
        // retrouverait dans un bloc (marche, pente) : elle est reposée sur le sol et l'écart
        // remesuré.
        let y = match offset.map(|o| pos.1 - o) {
            Some(y) if self.mob_free(x, y.floor() as i32, z) => y,
            _ => {
                let top = match offset {
                    Some(o) => (pos.1 - o).floor() as i32 + 2,
                    None => pos.1.floor() as i32 + 1,
                };
                let ground = (top - 6..=top).rev().find(|&y| !self.mob_free(x, y - 1, z) && self.mob_free(x, y, z)).unwrap_or(m.pos.1.floor() as i32) as f32;
                if let Some(m) = self.mobs.get_mut(&id) {
                    m.ride_offset = Some(pos.1 - ground);
                }
                ground
            }
        };
        if self.cfg.debug && tick % 20 == 0 {
            info!("[selle] cavalier en {:.2} {:.2} {:.2}, monture en y={:.2}", pos.0, pos.1, pos.2, y);
        }
        self.steer_mount(id, (pos.0, y, pos.2), yaw, None);
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
        // Ce qu'elle portait : selle, armure ou tapis, coffre et son contenu
        let mut worn: Vec<(String, u8)> = m.items.iter().flatten().cloned().collect();
        if m.saddled && !m.species.mount() {
            worn.push(("saddle".into(), 1));
        }
        if m.chested {
            worn.push(("chest".into(), 1));
        }
        for (name, count) in worn {
            if let Some(item) = self.item_id(&name) {
                let vel = ((self.mob_rng.unit() as f32 - 0.5) * 0.2, 0.2, (self.mob_rng.unit() as f32 - 0.5) * 0.2);
                self.drop_item((m.pos.0, m.pos.1 + 0.5, m.pos.2), vel, item, count, 10);
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
        self.check_leashes();
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
            .filter(|m| m.dead.is_none() && m.rider.is_none() && !matches!(m.leash, Some(Leash::Player(_))))
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
        let knot = r.knot.map(|pos| (self.knot_at(pos), pos));
        let id = self.add_mob(r.species, r.pos);
        if let (Some((k, pos)), Some(m)) = (knot, self.mobs.get_mut(&id)) {
            m.leash = Some(Leash::Knot(k, pos));
        }
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
            m.chested = r.chested;
            m.horns = r.horns;
            m.items = vec![None; m.inventory_size()];
            for (slot, name, count) in r.items {
                if let Some(cell) = m.items.get_mut(slot as usize) {
                    *cell = Some((name, count));
                }
            }
            // Selle d'avant l'inventaire : elle y prend sa place
            if m.saddled && m.species.steerable() && m.items.first().map(|i| i.is_none()).unwrap_or(false) {
                m.items[0] = Some(("saddle".into(), 1));
            }
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

/// Crachat de lama en vol : identifiant, position, élan, ticks restants, joueur visé et
/// joueurs qui le voient.
pub(super) struct Spit {
    id: i64,
    pos: (f32, f32, f32),
    vel: (f32, f32, f32),
    ttl: u32,
    target: SocketAddr,
    viewers: Vec<SocketAddr>,
}

/// Petites fleurs (pour la soupe suspecte de la champimeuh brune)
const FLOWERS: [&str; 16] = [
    "dandelion",
    "poppy",
    "blue_orchid",
    "allium",
    "azure_bluet",
    "red_tulip",
    "orange_tulip",
    "white_tulip",
    "pink_tulip",
    "oxeye_daisy",
    "cornflower",
    "lily_of_the_valley",
    "wither_rose",
    "torchflower",
    "open_eyeblossom",
    "closed_eyeblossom",
];

/// Couleur de laine d'une teinture (None si ce n'en est pas une).
fn dye_color(item: &str) -> Option<u8> {
    const COLORS: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
    match item {
        "ink_sac" => Some(15),
        "cocoa_beans" => Some(12),
        "lapis_lazuli" => Some(11),
        "bone_meal" => Some(0),
        _ => item.strip_suffix("_dye").and_then(|c| COLORS.iter().position(|x| *x == c)).map(|i| i as u8),
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

/// Dégâts de chute d'une monture (cheval, âne, mule) : comme un joueur (un demi-cœur par bloc
/// au-delà de 3), moins 5 comme dans les fichiers du jeu.
pub(crate) fn mount_fall_damage(distance: f32) -> f32 {
    ((distance - 3.0).ceil() - 5.0).max(0.0)
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
        assert!(Species::Horse.breed_items().contains(&"golden_carrot") && Species::Horse.tempt().contains(&"golden_apple"));
        assert!(Species::Llama.tempt().is_empty(), "le lama ne suit pas la nourriture");
        // Vitesses du jeu : une vache se promène (0,25 × 0,8) à moins de 2 blocs par seconde,
        // fuit (× 1,25) à peu près au pas d'un joueur
        let walk = ground_speed(0.25 * 0.8) * 20.0;
        let flee = ground_speed(0.25 * 1.25) * 20.0;
        assert!((1.5..2.0).contains(&walk) && (4.0..4.6).contains(&flee), "{} {}", walk, flee);
        assert!(ground_speed(10.0) <= 0.4);
        assert_eq!(Species::Llama.breed_items(), &["hay_block"]);
        assert!(Species::Mule.breed_items().is_empty());
        assert!(Species::Horse.heal_items().iter().any(|(i, h)| *i == "hay_block" && *h == 20.0));
        for s in Species::ALL {
            assert_eq!(Species::from_id(s.id()), Some(s));
            assert!(!s.grow_items().is_empty(), "{:?} : un petit doit pouvoir grandir plus vite", s);
        }
    }

    #[test]
    fn dyes_and_inventories() {
        assert_eq!(dye_color("red_dye"), Some(14));
        assert_eq!(dye_color("light_blue_dye"), Some(3));
        assert_eq!((dye_color("ink_sac"), dye_color("bone_meal")), (Some(15), Some(0)));
        assert_eq!(dye_color("wheat"), None);
        assert!(FLOWERS.contains(&"cornflower"));
    }

    #[test]
    fn mount_fall_damage_follows_the_game() {
        assert_eq!(mount_fall_damage(7.0), 0.0);
        assert_eq!(mount_fall_damage(8.5), 1.0);
        assert_eq!(mount_fall_damage(20.0), 12.0);
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
            knot: Some((4, 65, -7)),
            chested: true,
            horns: 1,
            items: vec![(0, "saddle".into(), 1), (5, "diamond".into(), 3)],
        };
        assert_eq!(MobRecord::from_json(&r.to_json()), Some(r));
        // Ancien enregistrement (sans les nouveaux champs) : valeurs du jeu par défaut
        let old = crate::json::parse(r#"{"kind":"minecraft:cow","x":0,"y":0,"z":0}"#).unwrap();
        let cow = MobRecord::from_json(&old).unwrap();
        assert_eq!((cow.max_health, cow.health, cow.tamed, cow.mark), (10.0, 10.0, false, 0));
    }
}
