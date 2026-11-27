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
    Wolf,
    Cat,
    Villager,
    IronGolem,
    /// Monstres : ils s'en prennent aux joueurs qu'ils voient
    Zombie,
    Spider,
    Skeleton,
    Stray,
    Creeper,
    Husk,
    Drowned,
    ZombieVillager,
    CaveSpider,
    Silverfish,
    WitherSkeleton,
    Vindicator,
    Pillager,
    Enderman,
    Slime,
    Witch,
    /// Créatures volantes
    Blaze,
    Ghast,
    Phantom,
    /// Le reste du bestiaire du jeu : comportement générique (promenade, vol, nage ou
    /// attaque au contact), sans leurs particularités
    Fox,
    Ocelot,
    Panda,
    PolarBear,
    Turtle,
    Frog,
    Camel,
    Sniffer,
    Armadillo,
    Strider,
    SnowGolem,
    SkeletonHorse,
    ZombieHorse,
    TraderLlama,
    WanderingTrader,
    Bat,
    Bee,
    Parrot,
    Allay,
    Cod,
    Salmon,
    TropicalFish,
    Pufferfish,
    Squid,
    GlowSquid,
    Dolphin,
    Axolotl,
    Tadpole,
    Piglin,
    PiglinBrute,
    ZombifiedPiglin,
    Hoglin,
    Zoglin,
    MagmaCube,
    Endermite,
    Ravager,
    Evoker,
    Bogged,
    Warden,
}

/// Basalt sait-il faire vivre cette créature du jeu (`minecraft:cow`) ?
pub(super) fn species_known(id: &str) -> bool {
    Species::from_id(id).is_some()
}

/// Les créatures du jeu que Basalt sait faire vivre.
pub(super) fn species_names() -> Vec<&'static str> {
    Species::ALL.iter().map(|s| s.id()).collect()
}

/// Ce qu'un loup attaque.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Prey {
    Mob(i64),
    Player(SocketAddr),
}

/// Qui a porté un coup à une créature.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Attacker {
    Player(SocketAddr),
    Mob(i64),
}

/// Secondes d'air d'une créature (« total_supply » des fiches du jeu), et secondes sous l'eau
/// avant qu'un zombie ne devienne un noyé, ou un zombie momifié un zombie.
pub(super) const BREATH: u8 = 15;
const SOAKED: u8 = 15;

/// Nombre maximal de blocs gelés par les semelles givrantes en même temps.
const MAX_FROST: usize = 4096;

/// Viandes que mange un loup apprivoisé : (objet, points de vie rendus)
const WOLF_FOOD: [(&str, f32); 11] = [
    ("beef", 3.0),
    ("cooked_beef", 8.0),
    ("porkchop", 3.0),
    ("cooked_porkchop", 8.0),
    ("chicken", 2.0),
    ("cooked_chicken", 6.0),
    ("mutton", 2.0),
    ("cooked_mutton", 6.0),
    ("rabbit", 3.0),
    ("cooked_rabbit", 5.0),
    ("rotten_flesh", 4.0),
];
const WOLF_MEATS: [&str; 11] = ["beef", "cooked_beef", "porkchop", "cooked_porkchop", "chicken", "cooked_chicken", "mutton", "cooked_mutton", "rabbit", "cooked_rabbit", "rotten_flesh"];
const WOLF_GROW: [(&str, f32); 11] = [
    ("beef", 0.1),
    ("cooked_beef", 0.1),
    ("porkchop", 0.1),
    ("cooked_porkchop", 0.1),
    ("chicken", 0.1),
    ("cooked_chicken", 0.1),
    ("mutton", 0.1),
    ("cooked_mutton", 0.1),
    ("rabbit", 0.1),
    ("cooked_rabbit", 0.1),
    ("rotten_flesh", 0.1),
];
/// Fleurs qui attirent les abeilles (« breed_items » de la fiche du jeu)
const BEE_FLOWERS: [&str; 28] = ["poppy", "blue_orchid", "allium", "azure_bluet", "red_tulip", "orange_tulip", "white_tulip", "pink_tulip", "oxeye_daisy", "cornflower", "lily_of_the_valley", "dandelion", "wither_rose", "sunflower", "lilac", "rose_bush", "peony", "flowering_azalea", "azalea_leaves_flowered", "mangrove_propagule", "pitcher_plant", "torchflower", "cherry_leaves", "pink_petals", "wildflowers", "cactus_flower", "chorus_flower", "spore_blossom"];
const BEE_GROW: [(&str, f32); 28] = {
    let mut out = [("", 0.1f32); 28];
    let mut i = 0;
    while i < 28 {
        out[i] = (BEE_FLOWERS[i], 0.1);
        i += 1;
    }
    out
};
/// Vie d'un loup sauvage, puis apprivoisé (celles du jeu)
const WOLF_WILD_HEALTH: f32 = 8.0;
const WOLF_TAMED_HEALTH: f32 = 40.0;

impl Species {
    const FARM: [Species; 4] = [Species::Cow, Species::Pig, Species::Sheep, Species::Chicken];
    const ALL: [Species; 73] = [
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
        Species::Wolf,
        Species::Cat,
        Species::Villager,
        Species::IronGolem,
        Species::Zombie,
        Species::Spider,
        Species::Skeleton,
        Species::Stray,
        Species::Creeper,
        Species::Husk,
        Species::Drowned,
        Species::ZombieVillager,
        Species::CaveSpider,
        Species::Silverfish,
        Species::WitherSkeleton,
        Species::Vindicator,
        Species::Pillager,
        Species::Enderman,
        Species::Slime,
        Species::Witch,
        Species::Blaze,
        Species::Ghast,
        Species::Phantom,
        Species::Fox,
        Species::Ocelot,
        Species::Panda,
        Species::PolarBear,
        Species::Turtle,
        Species::Frog,
        Species::Camel,
        Species::Sniffer,
        Species::Armadillo,
        Species::Strider,
        Species::SnowGolem,
        Species::SkeletonHorse,
        Species::ZombieHorse,
        Species::TraderLlama,
        Species::WanderingTrader,
        Species::Bat,
        Species::Bee,
        Species::Parrot,
        Species::Allay,
        Species::Cod,
        Species::Salmon,
        Species::TropicalFish,
        Species::Pufferfish,
        Species::Squid,
        Species::GlowSquid,
        Species::Dolphin,
        Species::Axolotl,
        Species::Tadpole,
        Species::Piglin,
        Species::PiglinBrute,
        Species::ZombifiedPiglin,
        Species::Hoglin,
        Species::Zoglin,
        Species::MagmaCube,
        Species::Endermite,
        Species::Ravager,
        Species::Evoker,
        Species::Bogged,
        Species::Warden,
    ];

    /// Monstre : (dégâts d'un coup, distance à laquelle il repère un joueur, son nom dans le
    /// message de mort). None pour une créature paisible.
    fn hostile(self) -> Option<(f32, f32, &'static str)> {
        match self {
            Species::Zombie => Some((3.0, 24.0, "un zombie")),
            Species::Spider => Some((2.0, 16.0, "une araignée")),
            Species::Skeleton => Some((2.0, 16.0, "un squelette")),
            Species::Stray => Some((2.0, 16.0, "un vagabond")),
            Species::Creeper => Some((0.0, 16.0, "un creeper")),
            Species::Husk => Some((3.0, 24.0, "un zombie momifié")),
            Species::Drowned => Some((3.0, 24.0, "un noyé")),
            Species::ZombieVillager => Some((3.0, 24.0, "un zombie-villageois")),
            Species::CaveSpider => Some((2.0, 16.0, "une araignée venimeuse")),
            Species::Silverfish => Some((1.0, 16.0, "un poisson d'argent")),
            Species::WitherSkeleton => Some((8.0, 16.0, "un wither squelette")),
            Species::Vindicator => Some((13.0, 16.0, "un vindicateur")),
            Species::Pillager => Some((3.0, 16.0, "un pillard")),
            Species::Enderman => Some((7.0, 64.0, "un enderman")),
            Species::Slime => Some((2.0, 16.0, "un slime")),
            Species::Witch => Some((0.0, 16.0, "une sorcière")),
            Species::Blaze => Some((0.0, 48.0, "un blaze")),
            Species::Ghast => Some((0.0, 64.0, "un ghast")),
            Species::Phantom => Some((6.0, 64.0, "un phantom")),
            Species::Piglin => Some((5.0, 16.0, "un piglin")),
            Species::PiglinBrute => Some((7.0, 16.0, "un piglin barbare")),
            Species::ZombifiedPiglin => Some((5.0, 16.0, "un piglin zombifié")),
            Species::Hoglin => Some((6.0, 16.0, "un hoglin")),
            Species::Zoglin => Some((6.0, 16.0, "un zoglin")),
            Species::MagmaCube => Some((4.0, 16.0, "un cube de magma")),
            Species::Endermite => Some((2.0, 16.0, "une endermite")),
            Species::Ravager => Some((12.0, 24.0, "un ravageur")),
            // (l'évocateur ne frappe pas : il fait jaillir des crocs, voir `cast_fangs`)
            Species::Evoker => Some((0.0, 16.0, "un évocateur")),
            Species::Bogged => Some((2.0, 16.0, "un squelette des marais")),
            Species::Warden => Some((30.0, 24.0, "un warden")),
            // Neutres : ils ne répondent qu'aux coups (voir `neutral`)
            Species::PolarBear => Some((6.0, 16.0, "un ours polaire")),
            Species::Bee => Some((2.0, 16.0, "une abeille")),
            Species::Dolphin => Some((3.0, 16.0, "un dauphin")),
            _ => None,
        }
    }

    /// Prend feu au soleil, comme le zombie et le squelette du jeu.
    fn burns(self) -> bool {
        matches!(self, Species::Zombie | Species::Skeleton | Species::Stray | Species::Drowned | Species::ZombieVillager | Species::Phantom | Species::Bogged)
    }

    /// Tireur : (projectile, dégâts, ticks entre deux tirs, portée en blocs).
    fn ranged(self) -> Option<(&'static str, f32, u32, f32)> {
        match self {
            Species::Skeleton | Species::Stray | Species::Bogged => Some(("minecraft:arrow", 3.0, 60, 15.0)),
            Species::Pillager => Some(("minecraft:arrow", 4.0, 50, 8.0)),
            Species::Witch => Some(("minecraft:splash_potion", 0.0, 60, 10.0)),
            Species::Blaze => Some(("minecraft:small_fireball", 5.0, 30, 48.0)),
            Species::Ghast => Some(("minecraft:fireball", 6.0, 60, 64.0)),
            _ => None,
        }
    }

    /// Multiplicateur de vitesse en poursuite (« speed_multiplier » de l'attaque dans les
    /// fichiers du jeu) : la plupart des monstres poursuivent à leur vitesse de marche.
    fn chase(self) -> f32 {
        match self {
            Species::Creeper | Species::WitherSkeleton => 1.25,
            // (en colère, l'enderman passe de 0,3 à 0,45)
            Species::Enderman => 1.5,
            _ => 1.0,
        }
    }

    /// Créature volante : sa vitesse de vol, en blocs par tick (None : elle marche).
    fn flies(self) -> Option<f32> {
        match self {
            Species::Blaze => Some(0.12),
            Species::Ghast => Some(0.08),
            Species::Phantom => Some(0.3),
            Species::Bat => Some(0.1),
            Species::Bee => Some(0.1),
            Species::Parrot => Some(0.12),
            Species::Allay => Some(0.1),
            _ => None,
        }
    }

    /// Créature aquatique : sa vitesse de nage, en blocs par tick (None : elle ne vit pas dans l'eau).
    fn swims(self) -> Option<f32> {
        match self {
            Species::Cod => Some(0.08),
            Species::Salmon => Some(0.08),
            Species::TropicalFish => Some(0.08),
            Species::Pufferfish => Some(0.06),
            Species::Squid => Some(0.05),
            Species::GlowSquid => Some(0.05),
            Species::Dolphin => Some(0.2),
            Species::Axolotl => Some(0.06),
            Species::Tadpole => Some(0.05),
            _ => None,
        }
    }

    /// Neutre : il ne s'en prend qu'au joueur qui l'a frappé (l'enderman).
    fn neutral(self) -> bool {
        matches!(self, Species::Enderman | Species::ZombifiedPiglin | Species::PolarBear | Species::Bee | Species::Dolphin)
    }

    /// Points d'armure naturels (ceux du jeu) : chaque point retire 4 % des dégâts reçus.
    fn armor(self) -> u32 {
        match self {
            Species::Zombie | Species::Husk | Species::Drowned | Species::ZombieVillager => 2,
            Species::WitherSkeleton => 4,
            _ => 0,
        }
    }

    /// Grimpe aux murs, comme les araignées du jeu.
    fn climbs(self) -> bool {
        matches!(self, Species::Spider | Species::CaveSpider)
    }

    /// Explose au contact : la puissance de l'explosion (3 pour le creeper).
    fn explodes(self) -> Option<f32> {
        (self == Species::Creeper).then_some(3.0)
    }

    /// Ce qu'il tient en main (montré aux joueurs).
    fn held(self) -> Option<&'static str> {
        match self {
            Species::Skeleton | Species::Stray => Some("minecraft:bow"),
            Species::WitherSkeleton => Some("minecraft:stone_sword"),
            Species::Vindicator => Some("minecraft:iron_axe"),
            Species::Pillager => Some("minecraft:crossbow"),
            Species::Piglin | Species::ZombifiedPiglin => Some("minecraft:golden_sword"),
            Species::PiglinBrute => Some("minecraft:golden_axe"),
            Species::Bogged => Some("minecraft:bow"),
            _ => None,
        }
    }


    pub(super) fn id(self) -> &'static str {
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
            Species::Wolf => "minecraft:wolf",
            Species::Zombie => "minecraft:zombie",
            Species::Spider => "minecraft:spider",
            Species::Skeleton => "minecraft:skeleton",
            Species::Stray => "minecraft:stray",
            Species::Creeper => "minecraft:creeper",
            Species::Husk => "minecraft:husk",
            Species::Drowned => "minecraft:drowned",
            Species::ZombieVillager => "minecraft:zombie_villager_v2",
            Species::CaveSpider => "minecraft:cave_spider",
            Species::Silverfish => "minecraft:silverfish",
            Species::WitherSkeleton => "minecraft:wither_skeleton",
            Species::Vindicator => "minecraft:vindicator",
            Species::Pillager => "minecraft:pillager",
            Species::Enderman => "minecraft:enderman",
            Species::Slime => "minecraft:slime",
            Species::Witch => "minecraft:witch",
            Species::Blaze => "minecraft:blaze",
            Species::Ghast => "minecraft:ghast",
            Species::Phantom => "minecraft:phantom",
            Species::Cat => "minecraft:cat",
            Species::Villager => "minecraft:villager_v2",
            Species::IronGolem => "minecraft:iron_golem",
            Species::Fox => "minecraft:fox",
            Species::Ocelot => "minecraft:ocelot",
            Species::Panda => "minecraft:panda",
            Species::PolarBear => "minecraft:polar_bear",
            Species::Turtle => "minecraft:turtle",
            Species::Frog => "minecraft:frog",
            Species::Camel => "minecraft:camel",
            Species::Sniffer => "minecraft:sniffer",
            Species::Armadillo => "minecraft:armadillo",
            Species::Strider => "minecraft:strider",
            Species::SnowGolem => "minecraft:snow_golem",
            Species::SkeletonHorse => "minecraft:skeleton_horse",
            Species::ZombieHorse => "minecraft:zombie_horse",
            Species::TraderLlama => "minecraft:trader_llama",
            Species::WanderingTrader => "minecraft:wandering_trader",
            Species::Bat => "minecraft:bat",
            Species::Bee => "minecraft:bee",
            Species::Parrot => "minecraft:parrot",
            Species::Allay => "minecraft:allay",
            Species::Cod => "minecraft:cod",
            Species::Salmon => "minecraft:salmon",
            Species::TropicalFish => "minecraft:tropicalfish",
            Species::Pufferfish => "minecraft:pufferfish",
            Species::Squid => "minecraft:squid",
            Species::GlowSquid => "minecraft:glow_squid",
            Species::Dolphin => "minecraft:dolphin",
            Species::Axolotl => "minecraft:axolotl",
            Species::Tadpole => "minecraft:tadpole",
            Species::Piglin => "minecraft:piglin",
            Species::PiglinBrute => "minecraft:piglin_brute",
            Species::ZombifiedPiglin => "minecraft:zombie_pigman",
            Species::Hoglin => "minecraft:hoglin",
            Species::Zoglin => "minecraft:zoglin",
            Species::MagmaCube => "minecraft:magma_cube",
            Species::Endermite => "minecraft:endermite",
            Species::Ravager => "minecraft:ravager",
            Species::Evoker => "minecraft:evocation_illager",
            Species::Bogged => "minecraft:bogged",
            Species::Warden => "minecraft:warden",
        }
    }
    fn from_id(s: &str) -> Option<Species> {
        if s == "minecraft:villager" {
            return Some(Species::Villager);
        }
        if s == "minecraft:zombie_villager" {
            return Some(Species::ZombieVillager);
        }
        Species::ALL.into_iter().find(|k| k.id() == s)
    }
    /// Équidés et lamas : vie tirée au sort, apprivoisés avant de se reproduire
    fn mount(self) -> bool {
        matches!(self, Species::Llama | Species::Horse | Species::Donkey | Species::Mule) || self.born_tame()
    }
    /// Monture apprivoisée d'emblée (« is_tamed » des fiches du jeu) : il suffit de la seller
    fn born_tame(self) -> bool {
        matches!(self, Species::Camel | Species::SkeletonHorse | Species::ZombieHorse)
    }
    /// S'apprivoise avant de se reproduire (montures, loup)
    fn tameable(self) -> bool {
        self.mount() || self.pet()
    }
    /// Compagnon : apprivoisé, il a un maître qu'il suit, et s'assoit sur demande
    fn pet(self) -> bool {
        matches!(self, Species::Wolf | Species::Cat | Species::Parrot)
    }
    /// Ce qui l'apprivoise (une chance sur trois par objet donné)
    fn taming_items(self) -> &'static [&'static str] {
        match self {
            Species::Wolf => &["bone"],
            Species::Cat => &["cod", "salmon"],
            Species::Parrot => &["wheat_seeds", "pumpkin_seeds", "melon_seeds", "beetroot_seeds", "pitcher_pod", "torchflower_seeds"],
            _ => &[],
        }
    }
    /// Se dirige au clavier une fois apprivoisée et sellée (pas le lama)
    fn steerable(self) -> bool {
        matches!(self, Species::Horse | Species::Donkey | Species::Mule | Species::Camel | Species::SkeletonHorse | Species::ZombieHorse)
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
            // (sièges du jeu : 1,7 pour l'arpenteur, 1,905 et un peu en avant pour le dromadaire)
            Species::Strider => (0.0, 2.9, -0.2),
            Species::Camel => (0.0, 3.1, 0.5),
            _ => (0.0, 2.3, -0.2),
        }
    }
    /// Vie maximale (celle du jeu ; les montures tirent la leur entre 15 et 30)
    fn health(self) -> f32 {
        match self {
            Species::Fox => 10.0,
            Species::Ocelot => 10.0,
            Species::Panda => 20.0,
            Species::PolarBear => 30.0,
            Species::Turtle => 30.0,
            Species::Frog => 10.0,
            Species::Camel => 32.0,
            Species::Sniffer => 14.0,
            Species::Armadillo => 12.0,
            Species::Strider => 20.0,
            Species::SnowGolem => 4.0,
            Species::SkeletonHorse => 15.0,
            Species::ZombieHorse => 25.0,
            Species::TraderLlama => 30.0,
            Species::WanderingTrader => 20.0,
            Species::Bat => 6.0,
            Species::Bee => 10.0,
            Species::Parrot => 6.0,
            Species::Allay => 20.0,
            Species::Cod => 3.0,
            Species::Salmon => 3.0,
            Species::TropicalFish => 3.0,
            Species::Pufferfish => 3.0,
            Species::Squid => 10.0,
            Species::GlowSquid => 10.0,
            Species::Dolphin => 10.0,
            Species::Axolotl => 14.0,
            Species::Tadpole => 6.0,
            Species::Piglin => 16.0,
            Species::PiglinBrute => 50.0,
            Species::ZombifiedPiglin => 20.0,
            Species::Hoglin => 40.0,
            Species::Zoglin => 40.0,
            Species::MagmaCube => 4.0,
            Species::Endermite => 8.0,
            Species::Ravager => 100.0,
            Species::Evoker => 24.0,
            Species::Bogged => 16.0,
            Species::Warden => 500.0,
            Species::Cow | Species::Pig | Species::Mooshroom | Species::Goat | Species::Cat => 10.0,
            Species::Sheep => 8.0,
            Species::Chicken => 4.0,
            Species::Rabbit => 3.0,
            Species::Llama | Species::Horse | Species::Donkey | Species::Mule => 30.0,
            // (apprivoisé ; sauvage : 8)
            Species::Wolf => WOLF_TAMED_HEALTH,
            Species::Villager | Species::Zombie | Species::Skeleton | Species::Stray | Species::Creeper | Species::Husk | Species::Drowned | Species::ZombieVillager | Species::WitherSkeleton => 20.0,
            Species::IronGolem => 100.0,
            Species::Spider => 16.0,
            Species::CaveSpider => 12.0,
            Species::Silverfish => 8.0,
            Species::Vindicator | Species::Pillager => 24.0,
            Species::Witch => 26.0,
            Species::Enderman => 40.0,
            Species::Blaze | Species::Phantom => 20.0,
            Species::Ghast => 10.0,
            // (taille 1 ; une plus grosse a sa taille au carré)
            Species::Slime => 1.0,
        }
    }
    /// Largeur et hauteur de la boîte de collision d'un adulte (celles du jeu)
    fn size(self) -> (f32, f32) {
        match self {
            Species::Fox => (0.6, 0.7),
            Species::Ocelot => (0.6, 0.7),
            Species::Panda => (1.3, 1.25),
            Species::PolarBear => (1.4, 1.4),
            Species::Turtle => (1.2, 0.4),
            Species::Frog => (0.5, 0.55),
            Species::Camel => (1.7, 2.375),
            Species::Sniffer => (1.9, 1.75),
            Species::Armadillo => (0.7, 0.65),
            Species::Strider => (0.9, 1.7),
            Species::SnowGolem => (0.4, 1.8),
            Species::SkeletonHorse => (1.4, 1.6),
            Species::ZombieHorse => (1.4, 1.6),
            Species::TraderLlama => (0.9, 1.87),
            Species::WanderingTrader => (0.6, 1.9),
            Species::Bat => (0.5, 0.9),
            Species::Bee => (0.55, 0.5),
            Species::Parrot => (0.5, 1.0),
            Species::Allay => (0.35, 0.6),
            Species::Cod => (0.6, 0.3),
            Species::Salmon => (0.5, 0.5),
            Species::TropicalFish => (0.4, 0.4),
            Species::Pufferfish => (0.8, 0.8),
            Species::Squid => (0.8, 0.8),
            Species::GlowSquid => (0.8, 0.8),
            Species::Dolphin => (0.9, 0.6),
            Species::Axolotl => (0.75, 0.42),
            Species::Tadpole => (0.8, 0.6),
            Species::Piglin => (0.6, 1.9),
            Species::PiglinBrute => (0.6, 1.9),
            Species::ZombifiedPiglin => (0.6, 1.9),
            Species::Hoglin => (1.4, 1.4),
            Species::Zoglin => (1.4, 1.4),
            Species::MagmaCube => (1.04, 1.04),
            Species::Endermite => (0.4, 0.3),
            Species::Ravager => (1.95, 2.2),
            Species::Evoker => (0.6, 1.9),
            Species::Bogged => (0.6, 1.9),
            Species::Warden => (0.9, 2.9),
            Species::Cow | Species::Mooshroom | Species::Goat => (0.9, 1.3),
            Species::Pig => (0.9, 0.9),
            Species::Sheep => (0.9, 1.3),
            Species::Chicken => (0.6, 0.8),
            Species::Rabbit => (0.4, 0.5),
            Species::Llama => (0.9, 1.87),
            Species::Horse | Species::Donkey | Species::Mule => (1.4, 1.6),
            Species::Wolf => (0.6, 0.8),
            Species::Cat => (0.6, 0.7),
            Species::Villager | Species::Zombie | Species::Skeleton | Species::Stray | Species::Husk | Species::Drowned | Species::ZombieVillager | Species::Vindicator | Species::Pillager | Species::Witch => (0.6, 1.9),
            Species::IronGolem => (1.4, 2.9),
            Species::Spider => (1.4, 0.9),
            Species::Creeper => (0.6, 1.8),
            Species::CaveSpider => (0.7, 0.5),
            Species::Silverfish => (0.4, 0.3),
            Species::WitherSkeleton => (0.72, 2.01),
            Species::Enderman => (0.6, 2.9),
            Species::Blaze => (0.5, 1.8),
            Species::Ghast => (4.0, 4.0),
            Species::Phantom => (0.9, 0.5),
            // (taille 1 ; multipliée par sa taille)
            Species::Slime => (0.52, 0.52),
        }
    }
    /// Taille d'affichage (« minecraft:scale » des fichiers du jeu) : le lapin est un petit
    /// modèle réduit (0,6 adulte, 0,4 bébé), les autres bébés font moitié.
    fn scale(self, baby: bool) -> f32 {
        match (self, baby) {
            (Species::Rabbit, false) => 0.6,
            (Species::Rabbit, true) => 0.4,
            (Species::Cat, false) => 0.8,
            (Species::Cat, true) => 0.4,
            // (le têtard est « petit » toute sa vie de têtard : il garde sa taille)
            (Species::Tadpole, _) => 1.0,
            // Petits à part (« minecraft:scale » des fiches du jeu, que le jeu compense dans
            // son dessin : un bébé tortue à 0,5 y paraîtrait trois fois trop gros)
            (Species::Turtle, true) => 0.16,
            (Species::Panda, true) => 0.4,
            (Species::Camel | Species::Sniffer, true) => 0.45,
            (Species::Armadillo, true) => 0.6,
            (Species::Dolphin, true) => 0.65,
            // Adultes plus grands que leur modèle : poisson-globe, poisson tropical, wither squelette
            (Species::Pufferfish, false) => 1.2,
            (Species::TropicalFish, false) => 1.3,
            (Species::WitherSkeleton, false) => 1.2,
            (_, true) => 0.5,
            (_, false) => 1.0,
        }
    }
    /// Vitesse du jeu (« minecraft:movement ») ; celle du cheval est tirée à sa naissance
    fn movement(self) -> f32 {
        match self {
            Species::Fox => 0.3,
            Species::Ocelot => 0.3,
            Species::Panda => 0.15,
            Species::PolarBear => 0.25,
            Species::Turtle => 0.1,
            Species::Frog => 0.1,
            Species::Camel => 0.09,
            Species::Sniffer => 0.09,
            Species::Armadillo => 0.14,
            Species::Strider => 0.16,
            Species::SnowGolem => 0.2,
            Species::SkeletonHorse => 0.2,
            Species::ZombieHorse => 0.275,
            Species::TraderLlama => 0.25,
            Species::WanderingTrader => 0.5,
            Species::Piglin => 0.35,
            Species::PiglinBrute => 0.35,
            Species::ZombifiedPiglin => 0.23,
            Species::Hoglin => 0.3,
            Species::Zoglin => 0.25,
            Species::MagmaCube => 0.3,
            Species::Endermite => 0.25,
            Species::Ravager => 0.3,
            Species::Evoker => 0.3,
            Species::Bogged => 0.25,
            Species::Warden => 0.3,
            Species::Rabbit | Species::Wolf | Species::Cat | Species::Spider | Species::CaveSpider | Species::Enderman => 0.3,
            Species::Vindicator | Species::Pillager => 0.35,
            Species::Zombie | Species::Husk | Species::Drowned | Species::ZombieVillager => 0.23,
            Species::Creeper => 0.2,
            Species::Goat => 0.4,
            Species::Villager => 0.5,
            Species::Donkey | Species::Mule => 0.175,
            _ => 0.25,
        }
    }
    /// Multiplicateurs de vitesse du jeu : promenade, fuite, attirée par la nourriture,
    /// petit qui suit un parent (« speed_multiplier » des comportements du jeu)
    fn multipliers(self) -> (f32, f32, f32, f32) {
        match self {
            // (« random_stroll », « panic », « tempt » et « follow_parent » des fiches du jeu)
            Species::Fox | Species::Ocelot => (0.8, 1.25, 0.5, 1.1),
            Species::Panda => (0.8, 1.25, 1.25, 1.1),
            Species::PolarBear => (1.0, 1.25, 1.0, 1.25),
            Species::Turtle => (1.0, 1.2, 1.1, 1.1),
            Species::Frog | Species::Tadpole => (1.0, 2.0, 1.25, 1.0),
            Species::Camel => (2.0, 4.0, 2.5, 1.1),
            Species::Sniffer | Species::Armadillo => (1.0, 2.0, 1.25, 1.1),
            Species::Strider => (0.8, 1.1, 1.2, 1.1),
            Species::SkeletonHorse | Species::ZombieHorse => (0.7, 1.2, 1.2, 1.0),
            Species::Hoglin => (0.4, 1.0, 1.0, 1.0),
            Species::Piglin => (0.6, 1.1, 1.0, 1.0),
            Species::TraderLlama | Species::WanderingTrader | Species::Bat | Species::Bee | Species::Parrot | Species::Allay | Species::Cod | Species::Salmon | Species::TropicalFish | Species::Pufferfish | Species::Squid | Species::GlowSquid | Species::Dolphin | Species::Axolotl | Species::SnowGolem | Species::PiglinBrute | Species::ZombifiedPiglin | Species::Zoglin | Species::MagmaCube | Species::Endermite | Species::Ravager | Species::Evoker | Species::Bogged | Species::Warden => (0.8, 1.25, 1.0, 1.0),
            Species::Cow | Species::Mooshroom | Species::Sheep => (0.8, 1.25, 1.25, 1.1),
            Species::Pig => (1.0, 1.25, 1.2, 1.1),
            Species::Chicken => (1.0, 1.5, 1.0, 1.1),
            Species::Rabbit => (0.6, 2.2, 1.0, 1.1),
            Species::Goat => (0.6, 1.0, 0.75, 1.0),
            Species::Llama | Species::Horse | Species::Donkey | Species::Mule => (0.7, 1.2, 1.2, 1.0),
            Species::Wolf => (0.8, 1.0, 1.0, 1.0),
            Species::Cat => (0.8, 1.33, 0.5, 1.0),
            Species::Villager => (0.6, 1.0, 0.6, 0.6),
            Species::IronGolem | Species::Zombie | Species::Spider | Species::Skeleton | Species::Stray | Species::Creeper | Species::Husk | Species::Drowned | Species::ZombieVillager | Species::CaveSpider | Species::Silverfish | Species::WitherSkeleton | Species::Vindicator | Species::Pillager | Species::Enderman | Species::Slime | Species::Blaze | Species::Ghast | Species::Phantom | Species::Witch => (0.6, 1.0, 1.0, 1.0),
        }
    }
    /// Ce qui l'attire : il suit le joueur qui le tient
    fn tempt(self) -> &'static [&'static str] {
        match self {
            // (« minecraft:behavior.tempt » des fiches du jeu)
            Species::Fox => &["sweet_berries", "glow_berries"],
            Species::Ocelot => &["cod", "salmon"],
            Species::Panda => &["bamboo"],
            Species::Turtle => &["seagrass"],
            Species::Frog | Species::Tadpole => &["slime_ball"],
            Species::Camel => &["cactus"],
            Species::Sniffer => &["torchflower_seeds"],
            Species::Armadillo => &["spider_eye"],
            Species::Strider => &["warped_fungus", "warped_fungus_on_a_stick"],
            Species::ZombieHorse => &["red_mushroom"],
            Species::Axolotl => &["tropical_fish_bucket"],
            Species::Bee => &BEE_FLOWERS,
            Species::PolarBear | Species::SnowGolem | Species::SkeletonHorse | Species::TraderLlama | Species::WanderingTrader | Species::Bat | Species::Parrot | Species::Allay | Species::Cod | Species::Salmon | Species::TropicalFish | Species::Pufferfish | Species::Squid | Species::GlowSquid | Species::Dolphin | Species::Piglin | Species::PiglinBrute | Species::ZombifiedPiglin | Species::Hoglin | Species::Zoglin | Species::MagmaCube | Species::Endermite | Species::Ravager | Species::Evoker | Species::Bogged | Species::Warden => &[],
            Species::Cow | Species::Sheep | Species::Mooshroom | Species::Goat => &["wheat"],
            Species::Pig => &["carrot", "potato", "beetroot"],
            Species::Chicken => &["wheat_seeds", "pumpkin_seeds", "melon_seeds", "beetroot_seeds", "torchflower_seeds", "pitcher_pod"],
            Species::Rabbit => &["carrot", "golden_carrot", "dandelion"],
            // Le lama ne suit personne ; les équidés seulement une fois apprivoisés
            Species::Llama => &[],
            Species::Horse | Species::Donkey | Species::Mule => &["golden_apple", "enchanted_golden_apple", "golden_carrot"],
            Species::Wolf => &[],
            Species::Cat => &["cod", "salmon"],
            Species::Villager | Species::IronGolem | Species::Zombie | Species::Spider | Species::Skeleton | Species::Stray | Species::Creeper | Species::Husk | Species::Drowned | Species::ZombieVillager | Species::CaveSpider | Species::Silverfish | Species::WitherSkeleton | Species::Vindicator | Species::Pillager | Species::Enderman | Species::Slime | Species::Blaze | Species::Ghast | Species::Phantom | Species::Witch => &[],
        }
    }
    /// Ce qui le rend amoureux (les montures doivent d'abord être apprivoisées ; la mule
    /// ne se reproduit pas)
    fn breed_items(self) -> &'static [&'static str] {
        match self {
            Species::Llama => &["hay_block"],
            Species::Horse | Species::Donkey => &["golden_carrot", "golden_apple", "enchanted_golden_apple"],
            Species::Mule => &[],
            Species::Wolf => &WOLF_MEATS,
            // (le têtard grandit, le cheval zombie ne se reproduit pas)
            Species::Strider => &["warped_fungus"],
            Species::Hoglin => &["crimson_fungus"],
            Species::Tadpole | Species::ZombieHorse => &[],
            other => other.tempt(),
        }
    }
    /// Ce qui fait grandir un petit : (objet, part de la croissance totale)
    fn grow_items(self) -> &'static [(&'static str, f32)] {
        const EQUINE: [(&str, f32); 7] = [("wheat", 0.016667), ("sugar", 0.025), ("hay_block", 0.15), ("apple", 0.05), ("golden_carrot", 0.05), ("golden_apple", 0.2), ("enchanted_golden_apple", 0.2)];
        match self {
            Species::Fox => &[("sweet_berries", 0.1), ("glow_berries", 0.1)],
            Species::Ocelot => &[("cod", 0.1), ("salmon", 0.1)],
            Species::Panda => &[("bamboo", 0.1)],
            Species::Turtle => &[("seagrass", 0.1)],
            Species::Frog | Species::Tadpole => &[("slime_ball", 0.1)],
            Species::Camel => &[("cactus", 0.1)],
            Species::Sniffer => &[("torchflower_seeds", 0.1)],
            Species::Armadillo => &[("spider_eye", 0.1)],
            Species::Strider => &[("warped_fungus", 0.1)],
            Species::Hoglin => &[("crimson_fungus", 0.1)],
            Species::Axolotl => &[("tropical_fish_bucket", 0.1)],
            Species::Bee => &BEE_GROW,
            Species::PolarBear | Species::SnowGolem | Species::SkeletonHorse | Species::ZombieHorse | Species::TraderLlama | Species::WanderingTrader | Species::Bat | Species::Parrot | Species::Allay | Species::Cod | Species::Salmon | Species::TropicalFish | Species::Pufferfish | Species::Squid | Species::GlowSquid | Species::Dolphin | Species::Piglin | Species::PiglinBrute | Species::ZombifiedPiglin | Species::Zoglin | Species::MagmaCube | Species::Endermite | Species::Ravager | Species::Evoker | Species::Bogged | Species::Warden => &[],
            Species::Horse | Species::Donkey | Species::Mule => &EQUINE,
            Species::Llama => &[("wheat", 0.1), ("hay_block", 0.9)],
            Species::Cow | Species::Sheep | Species::Mooshroom | Species::Goat => &[("wheat", 0.1)],
            Species::Pig => &[("carrot", 0.1), ("potato", 0.1), ("beetroot", 0.1)],
            Species::Chicken => &[("wheat_seeds", 0.1), ("pumpkin_seeds", 0.1), ("melon_seeds", 0.1), ("beetroot_seeds", 0.1), ("torchflower_seeds", 0.1), ("pitcher_pod", 0.1)],
            Species::Rabbit => &[("carrot", 0.1), ("golden_carrot", 0.1), ("dandelion", 0.1)],
            Species::Wolf => &WOLF_GROW,
            Species::Cat => &[("cod", 0.1), ("salmon", 0.1)],
            Species::Villager | Species::IronGolem | Species::Zombie | Species::Spider | Species::Skeleton | Species::Stray | Species::Creeper | Species::Husk | Species::Drowned | Species::ZombieVillager | Species::CaveSpider | Species::Silverfish | Species::WitherSkeleton | Species::Vindicator | Species::Pillager | Species::Enderman | Species::Slime | Species::Blaze | Species::Ghast | Species::Phantom | Species::Witch => &[],
        }
    }
    /// Ce qui le soigne : (objet, points de vie rendus)
    fn heal_items(self) -> &'static [(&'static str, f32)] {
        const EQUINE: [(&str, f32); 7] = [("wheat", 2.0), ("sugar", 1.0), ("hay_block", 20.0), ("apple", 3.0), ("golden_carrot", 4.0), ("golden_apple", 10.0), ("enchanted_golden_apple", 10.0)];
        match self {
            // (« minecraft:healable » des fiches du jeu)
            Species::Camel => &[("cactus", 2.0)],
            Species::Sniffer => &[("torchflower_seeds", 2.0)],
            Species::ZombieHorse => &[("red_mushroom", 3.0)],
            Species::Horse | Species::Donkey | Species::Mule => &EQUINE,
            Species::Llama => &[("wheat", 2.0), ("hay_block", 10.0)],
            Species::Wolf => &WOLF_FOOD,
            Species::Cat => &[("cod", 2.0), ("salmon", 2.0)],
            // (un lingot répare le golem d'un quart de sa vie)
            Species::IronGolem => &[("iron_ingot", 25.0)],
            _ => &[],
        }
    }
    /// Ne se noie pas : les morts-vivants respirent sous l'eau, le blaze et le golem de fer
    /// ne respirent pas (d'après les fiches du jeu, composant « breathable »).
    pub(super) fn breathes_water(self) -> bool {
        self.undead() || matches!(self, Species::Blaze | Species::IronGolem | Species::Turtle | Species::Frog | Species::Axolotl)
    }
    /// Coule dans l'eau au lieu d'y flotter : les morts-vivants qui marchent, noyé compris (il
    /// vit au fond ; le phantom vole).
    fn sinks(self) -> bool {
        // (la tortue et la grenouille nagent sous l'eau et marchent au fond, comme dans le jeu)
        (self.undead() && self != Species::Phantom) || matches!(self, Species::Turtle | Species::Frog)
    }
    /// Défonce les portes en bois qui le séparent de sa proie (zombies et leurs cousins).
    fn breaks_doors(self) -> bool {
        matches!(self, Species::Zombie | Species::Husk | Species::Drowned | Species::ZombieVillager | Species::Vindicator)
    }
    /// Mort-vivant : le châtiment le blesse davantage.
    pub(super) fn undead(self) -> bool {
        matches!(self, Species::Zombie | Species::Husk | Species::Drowned | Species::ZombieVillager | Species::Skeleton | Species::Stray | Species::WitherSkeleton | Species::Phantom | Species::Bogged | Species::SkeletonHorse | Species::ZombieHorse | Species::ZombifiedPiglin | Species::Zoglin)
    }
    /// Arthropode : le fléau des arthropodes le blesse davantage.
    pub(super) fn arthropod(self) -> bool {
        matches!(self, Species::Spider | Species::CaveSpider | Species::Silverfish | Species::Bee | Species::Endermite)
    }
    /// Résistance au recul (« minecraft:knockback_resistance » des fiches du jeu).
    pub(super) fn knockback_resistance(self) -> f32 {
        match self {
            Species::IronGolem | Species::Warden => 1.0,
            Species::Ravager => 0.75,
            Species::Hoglin | Species::Zoglin => 0.6,
            _ => 0.0,
        }
    }
    /// Ne vit que dans l'eau : hors de l'eau il s'étouffe (« breathes_air: false » des fiches
    /// du jeu : poissons, calamars, têtard).
    pub(super) fn gills(self) -> bool {
        matches!(self, Species::Cod | Species::Salmon | Species::TropicalFish | Species::Pufferfish | Species::Squid | Species::GlowSquid | Species::Tadpole)
    }
    /// Secondes d'air (« total_supply ») : le dauphin tient quatre minutes sous l'eau.
    pub(super) fn air_supply(self) -> u8 {
        match self {
            Species::Dolphin => 240,
            Species::Tadpole => 8,
            _ => BREATH,
        }
    }
    /// Seau dans lequel on l'attrape avec un seau d'eau
    pub(super) fn bucket(self) -> Option<&'static str> {
        Some(match self {
            Species::Cod => "cod_bucket",
            Species::Salmon => "salmon_bucket",
            Species::TropicalFish => "tropical_fish_bucket",
            Species::Pufferfish => "pufferfish_bucket",
            Species::Axolotl => "axolotl_bucket",
            Species::Tadpole => "tadpole_bucket",
            _ => return None,
        })
    }
    /// Dirigé par son cavalier avec cet objet au bout d'un bâton (le cochon, l'arpenteur)
    fn stick(self) -> Option<&'static str> {
        match self {
            Species::Pig => Some("carrot_on_a_stick"),
            Species::Strider => Some("warped_fungus_on_a_stick"),
            _ => None,
        }
    }
    /// Créature « ambiante » (aquatique, chauve-souris) : elle apparaît et disparaît autour des
    /// joueurs comme les monstres, sans compter parmi eux.
    pub(super) fn ambient(self) -> bool {
        self.swims().is_some() || self == Species::Bat
    }
    /// Cube qui saute et se divise à sa mort : le slime et le cube de magma.
    pub(super) fn cube(self) -> bool {
        matches!(self, Species::Slime | Species::MagmaCube)
    }
    /// Frappé, il ameute ses semblables dans ce rayon (« broadcast_anger » des fiches du jeu).
    fn anger_range(self) -> Option<f32> {
        match self {
            Species::ZombifiedPiglin | Species::PolarBear | Species::Silverfish => Some(20.0),
            Species::Piglin | Species::Hoglin | Species::Dolphin | Species::Bee => Some(16.0),
            _ => None,
        }
    }
    /// Ce qu'il chasse de lui-même : (proies, dégâts d'une morsure). Le renard et l'ocelot
    /// chassent poules et lapins, la grenouille gobe les petits slimes et cubes de magma.
    fn hunts(self) -> (&'static [Species], f32) {
        match self {
            Species::Wolf => (&[Species::Rabbit, Species::Sheep, Species::Fox], 3.0),
            Species::Cat => (&[Species::Rabbit], 3.0),
            Species::Fox => (&[Species::Chicken, Species::Rabbit, Species::Cod, Species::Salmon, Species::TropicalFish], 2.0),
            Species::Ocelot => (&[Species::Chicken], 3.0),
            Species::Frog => (&[Species::Slime, Species::MagmaCube], 100.0),
            Species::PolarBear => (&[], 6.0),
            _ => (&[], 0.0),
        }
    }
    /// Butin : (objet, quantité minimale, maximale)
    fn drops(self) -> &'static [(&'static str, u8, u8)] {
        match self {
            Species::Fox => &[],
            Species::Ocelot => &[],
            Species::Panda => &[("minecraft:bamboo", 0, 2)],
            Species::PolarBear => &[("minecraft:cod", 0, 2)],
            Species::Turtle => &[("minecraft:seagrass", 0, 2)],
            Species::Frog => &[],
            Species::Camel => &[],
            Species::Sniffer => &[],
            Species::Armadillo => &[],
            Species::Strider => &[("minecraft:string", 2, 5)],
            Species::SnowGolem => &[("minecraft:snowball", 0, 15)],
            Species::SkeletonHorse => &[("minecraft:bone", 0, 2)],
            Species::ZombieHorse => &[("minecraft:rotten_flesh", 0, 2)],
            Species::TraderLlama => &[("minecraft:leather", 0, 2)],
            Species::WanderingTrader => &[],
            Species::Bat => &[],
            Species::Bee => &[],
            Species::Parrot => &[("minecraft:feather", 1, 2)],
            Species::Allay => &[],
            Species::Cod => &[("minecraft:cod", 1, 1)],
            Species::Salmon => &[("minecraft:salmon", 1, 1)],
            Species::TropicalFish => &[("minecraft:tropical_fish", 1, 1)],
            Species::Pufferfish => &[("minecraft:pufferfish", 1, 1)],
            Species::Squid => &[("minecraft:ink_sac", 1, 3)],
            Species::GlowSquid => &[("minecraft:glow_ink_sac", 1, 3)],
            Species::Dolphin => &[("minecraft:cod", 0, 1)],
            Species::Axolotl => &[],
            Species::Tadpole => &[],
            Species::Piglin => &[],
            Species::PiglinBrute => &[],
            Species::ZombifiedPiglin => &[("minecraft:rotten_flesh", 0, 1), ("minecraft:gold_nugget", 0, 1)],
            Species::Hoglin => &[("minecraft:porkchop", 2, 4), ("minecraft:leather", 0, 1)],
            Species::Zoglin => &[("minecraft:rotten_flesh", 1, 3)],
            Species::MagmaCube => &[("minecraft:magma_cream", 0, 1)],
            Species::Endermite => &[],
            Species::Ravager => &[("minecraft:saddle", 1, 1)],
            Species::Evoker => &[("minecraft:totem_of_undying", 1, 1), ("minecraft:emerald", 0, 1)],
            Species::Bogged => &[("minecraft:bone", 0, 2), ("minecraft:arrow", 0, 2)],
            Species::Warden => &[("minecraft:sculk_catalyst", 1, 1)],
            Species::Cow | Species::Mooshroom => &[("minecraft:beef", 1, 3), ("minecraft:leather", 0, 2)],
            Species::Pig => &[("minecraft:porkchop", 1, 3)],
            Species::Sheep => &[("minecraft:mutton", 1, 2), ("minecraft:white_wool", 1, 1)],
            Species::Chicken => &[("minecraft:chicken", 1, 1), ("minecraft:feather", 0, 2)],
            Species::Rabbit => &[("minecraft:rabbit", 0, 1), ("minecraft:rabbit_hide", 0, 1)],
            Species::Llama | Species::Horse | Species::Donkey | Species::Mule => &[("minecraft:leather", 0, 2)],
            Species::Zombie => &[("minecraft:rotten_flesh", 0, 2)],
            Species::Spider | Species::CaveSpider => &[("minecraft:string", 0, 2)],
            Species::Skeleton | Species::Stray => &[("minecraft:bone", 0, 2), ("minecraft:arrow", 0, 2)],
            Species::Creeper => &[("minecraft:gunpowder", 0, 2)],
            Species::Husk | Species::Drowned | Species::ZombieVillager => &[("minecraft:rotten_flesh", 0, 2)],
            Species::WitherSkeleton => &[("minecraft:coal", 0, 1), ("minecraft:bone", 0, 2)],
            Species::Vindicator => &[("minecraft:emerald", 0, 1)],
            Species::Pillager => &[("minecraft:arrow", 0, 2)],
            Species::Silverfish => &[],
            Species::Enderman => &[("minecraft:ender_pearl", 0, 1)],
            Species::Slime => &[("minecraft:slime_ball", 0, 2)],
            Species::Witch => &[("minecraft:redstone", 0, 2), ("minecraft:glowstone_dust", 0, 2), ("minecraft:sugar", 0, 2)],
            Species::Blaze => &[("minecraft:blaze_rod", 0, 1)],
            Species::Ghast => &[("minecraft:gunpowder", 0, 2), ("minecraft:ghast_tear", 0, 1)],
            Species::Phantom => &[("minecraft:phantom_membrane", 0, 1)],
            Species::Goat | Species::Wolf => &[],
            Species::Cat => &[("minecraft:string", 0, 2)],
            Species::Villager => &[],
            Species::IronGolem => &[("minecraft:iron_ingot", 3, 5), ("minecraft:poppy", 0, 2)],
        }
    }
}

/// Distance à laquelle un point d'un chemin est tenu pour atteint. Courte : en coupant les
/// virages, un villageois (0,6 de large) accrochait le montant des portes.
const PATH_STOP: f32 = 0.2;

/// Premier numéro des offres d'échange : au-dessus de tous les numéros de recettes
const TRADE_NET_BASE: i32 = 1_000_000;

/// Un échange vérifié : le villageois, l'offre, le nombre d'échanges d'affilée, et les
/// quantités totales (objet, nombre) demandées et données.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TradeDeal {
    pub mob: i64,
    pub index: usize,
    pub times: i32,
    /// Expérience que le villageois y gagne
    pub xp: i32,
    pub a: (i16, u8),
    pub b: Option<(i16, u8)>,
    pub sell: (i16, u8),
}

/// Expérience demandée pour chaque niveau (novice, apprenti, compagnon, expert, maître)
const TIER_XP: [i32; 5] = [0, 10, 70, 150, 250];

/// Niveau d'un villageois d'après son expérience : 0 novice … 4 maître.
fn trade_tier(xp: i32) -> i32 {
    TIER_XP.iter().rposition(|need| xp >= *need).unwrap_or(0) as i32
}

/// Nombre de réassorts survenus avant ce moment : deux par jour, aux heures de travail
/// (2000 et 8000). Un villageois dont le compte a augmenté retrouve toutes ses offres.
fn restocks_before(time: i64) -> i64 {
    let (day, hour) = (time.div_euclid(24_000), time.rem_euclid(24_000));
    day * 2 + (hour >= 2_000) as i64 + (hour >= 8_000) as i64
}

/// Une offre telle que le jeu la définit : (objet, quantité) demandés, second lot éventuel,
/// (objet, quantité) donnés, nombre d'échanges avant épuisement, niveau qui la débloque.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Offer {
    buy_a: (&'static str, u8),
    buy_b: Option<(&'static str, u8)>,
    sell: (&'static str, u8),
    max_uses: i32,
    tier: i32,
}

impl Offer {
    /// Expérience gagnée par le villageois (celle du jeu) : plus au fil des niveaux, et le
    /// double quand c'est lui qui achète.
    fn xp(&self) -> i32 {
        let buying = self.sell.0 == "emerald";
        match (self.tier, buying) {
            (0, true) => 2,
            (0, false) => 1,
            (1, true) => 10,
            (1, false) => 5,
            (2, true) => 20,
            (2, false) => 10,
            (_, true) => 30,
            (_, false) => 15,
        }
    }
}

/// Métier que donne un poste de travail (d'après le nom du bloc).
fn station_job(block: &str) -> Option<i32> {
    Some(match block.trim_start_matches("minecraft:") {
        "composter" => 1,
        "barrel" => 2,
        "loom" => 3,
        "fletching_table" => 4,
        "lectern" => 5,
        "cartography_table" => 6,
        "brewing_stand" => 7,
        "blast_furnace" | "lit_blast_furnace" => 8,
        "grindstone" => 9,
        "smithing_table" => 10,
        "smoker" | "lit_smoker" => 11,
        "cauldron" => 12,
        "stonecutter_block" | "stonecutter" => 13,
        _ => return None,
    })
}

/// Titre de l'écran d'échange selon le métier (clé de traduction du jeu). None : sans
/// métier, ou simplet — il n'échange rien.
fn villager_title(job: i32) -> Option<&'static str> {
    Some(match job {
        1 => "entity.villager.farmer",
        2 => "entity.villager.fisherman",
        3 => "entity.villager.shepherd",
        4 => "entity.villager.fletcher",
        5 => "entity.villager.librarian",
        6 => "entity.villager.cartographer",
        7 => "entity.villager.cleric",
        8 => "entity.villager.armor",
        9 => "entity.villager.weapon",
        10 => "entity.villager.tool",
        11 => "entity.villager.butcher",
        12 => "entity.villager.leather",
        13 => "entity.villager.mason",
        _ => return None,
    })
}

/// Offres d'un métier, niveau par niveau : deux par niveau, comme dans le jeu — en général
/// il achète une matière première contre une émeraude et vend un produit contre des
/// émeraudes. Quand le jeu tire au sort entre plusieurs offres, `seed` choisit. Les objets
/// enchantés du jeu (livres, outils en diamant enchantés) sont vendus ici sans enchantement.
fn job_offers(job: i32, seed: u64) -> Vec<Offer> {
    const E: &str = "emerald";
    type Pool = Vec<Offer>;
    let buys = |item: &'static str, count: u8| Offer { buy_a: (item, count), buy_b: None, sell: (E, 1), max_uses: 16, tier: 0 };
    let sells = |price: u8, item: &'static str, count: u8| Offer { buy_a: (E, price), buy_b: None, sell: (item, count), max_uses: 12, tier: 0 };
    let with = |price: u8, extra: (&'static str, u8), item: &'static str, count: u8| Offer { buy_a: (E, price), buy_b: Some(extra), sell: (item, count), max_uses: 12, tier: 0 };
    // Par niveau : deux réserves, une offre tirée dans chacune (une réserve vide : rien)
    let tiers: Vec<(Pool, Pool)> = match job {
        1 => vec![
            (vec![buys("wheat", 20), buys("potato", 26), buys("carrot", 22), buys("beetroot", 15)], vec![sells(1, "bread", 6)]),
            (vec![buys("pumpkin", 6)], vec![sells(1, "pumpkin_pie", 4), sells(1, "apple", 4)]),
            (vec![buys("melon_block", 4)], vec![sells(3, "cookie", 18)]),
            (vec![sells(1, "cake", 1)], vec![buys("egg", 16)]),
            (vec![sells(3, "golden_carrot", 3)], vec![sells(4, "glistering_melon_slice", 3)]),
        ],
        2 => vec![
            (vec![buys("string", 20), buys("coal", 10)], vec![with(1, ("cod", 6), "cooked_cod", 6), sells(3, "cod_bucket", 1)]),
            (vec![buys("cod", 15)], vec![with(1, ("salmon", 6), "cooked_salmon", 6), sells(2, "campfire", 1)]),
            (vec![buys("salmon", 13)], vec![sells(3, "fishing_rod", 1)]),
            (vec![buys("tropical_fish", 6)], vec![]),
            (vec![buys("pufferfish", 4)], vec![buys("oak_boat", 1)]),
        ],
        3 => vec![
            (vec![buys("white_wool", 18), buys("brown_wool", 18), buys("black_wool", 18), buys("gray_wool", 18)], vec![sells(2, "shears", 1)]),
            (vec![buys("white_dye", 12), buys("gray_dye", 12), buys("black_dye", 12), buys("light_blue_dye", 12), buys("lime_dye", 12)], vec![sells(1, "white_wool", 1), sells(1, "white_carpet", 4)]),
            (vec![buys("yellow_dye", 12), buys("light_gray_dye", 12), buys("orange_dye", 12), buys("red_dye", 12), buys("pink_dye", 12)], vec![sells(1, "red_carpet", 4), sells(1, "blue_wool", 1)]),
            (vec![buys("brown_dye", 12), buys("purple_dye", 12), buys("blue_dye", 12), buys("green_dye", 12), buys("magenta_dye", 12), buys("cyan_dye", 12)], vec![]),
            (vec![sells(2, "painting", 3)], vec![]),
        ],
        4 => vec![
            (vec![buys("stick", 32)], vec![sells(1, "arrow", 16), with(1, ("gravel", 10), "flint", 10)]),
            (vec![buys("flint", 26)], vec![sells(2, "bow", 1)]),
            (vec![buys("string", 14)], vec![sells(3, "crossbow", 1)]),
            (vec![buys("feather", 24)], vec![sells(7, "bow", 1)]),
            (vec![buys("tripwire_hook", 8)], vec![sells(8, "crossbow", 1)]),
        ],
        5 => vec![
            (vec![buys("paper", 24)], vec![sells(9, "bookshelf", 1)]),
            (vec![buys("book", 4)], vec![sells(1, "lantern", 1)]),
            (vec![buys("ink_sac", 5)], vec![sells(1, "glass", 4)]),
            (vec![buys("writable_book", 2)], vec![sells(5, "clock", 1), sells(4, "compass", 1)]),
            (vec![sells(20, "name_tag", 1)], vec![]),
        ],
        6 => vec![
            (vec![buys("paper", 24)], vec![sells(7, "empty_map", 1)]),
            (vec![buys("glass_pane", 11)], vec![]),
            (vec![buys("compass", 1)], vec![]),
            (vec![sells(7, "frame", 1)], vec![]),
            (vec![sells(8, "globe_banner_pattern", 1)], vec![]),
        ],
        7 => vec![
            (vec![buys("rotten_flesh", 32)], vec![sells(1, "redstone", 2)]),
            (vec![buys("gold_ingot", 3)], vec![sells(1, "lapis_lazuli", 1)]),
            (vec![buys("rabbit_foot", 2)], vec![sells(4, "glowstone", 1)]),
            (vec![buys("turtle_scute", 4), buys("glass_bottle", 9)], vec![sells(5, "ender_pearl", 1)]),
            (vec![buys("nether_wart", 22)], vec![sells(3, "experience_bottle", 1)]),
        ],
        8 => vec![
            (vec![buys("coal", 15)], vec![sells(5, "iron_helmet", 1), sells(9, "iron_chestplate", 1), sells(7, "iron_leggings", 1), sells(4, "iron_boots", 1)]),
            (vec![buys("iron_ingot", 4)], vec![sells(36, "bell", 1), sells(3, "chainmail_leggings", 1), sells(1, "chainmail_boots", 1)]),
            (vec![buys("lava_bucket", 1), buys("diamond", 1)], vec![sells(1, "chainmail_helmet", 1), sells(4, "chainmail_chestplate", 1), sells(5, "shield", 1)]),
            (vec![sells(19, "diamond_leggings", 1)], vec![sells(13, "diamond_boots", 1)]),
            (vec![sells(13, "diamond_helmet", 1)], vec![sells(21, "diamond_chestplate", 1)]),
        ],
        9 => vec![
            (vec![buys("coal", 15)], vec![sells(3, "iron_axe", 1)]),
            (vec![buys("iron_ingot", 4)], vec![sells(36, "bell", 1)]),
            (vec![buys("flint", 24)], vec![sells(2, "iron_sword", 1)]),
            (vec![buys("diamond", 1)], vec![sells(12, "diamond_axe", 1)]),
            (vec![sells(8, "diamond_sword", 1)], vec![]),
        ],
        10 => vec![
            (vec![buys("coal", 15)], vec![sells(1, "stone_axe", 1), sells(1, "stone_shovel", 1), sells(1, "stone_pickaxe", 1), sells(1, "stone_hoe", 1)]),
            (vec![buys("iron_ingot", 4)], vec![sells(36, "bell", 1)]),
            (vec![buys("flint", 30)], vec![sells(1, "iron_axe", 1), sells(2, "iron_shovel", 1), sells(3, "iron_pickaxe", 1), sells(4, "diamond_hoe", 1)]),
            (vec![buys("diamond", 1)], vec![sells(12, "diamond_axe", 1), sells(5, "diamond_shovel", 1)]),
            (vec![sells(13, "diamond_pickaxe", 1)], vec![]),
        ],
        11 => vec![
            (vec![buys("chicken", 14), buys("porkchop", 7), buys("rabbit", 4)], vec![sells(1, "rabbit_stew", 1)]),
            (vec![buys("coal", 15)], vec![sells(1, "cooked_porkchop", 5), sells(1, "cooked_chicken", 8)]),
            (vec![buys("mutton", 7)], vec![buys("beef", 10)]),
            (vec![buys("dried_kelp_block", 10)], vec![]),
            (vec![buys("sweet_berries", 10)], vec![]),
        ],
        12 => vec![
            (vec![buys("leather", 6)], vec![sells(7, "leather_chestplate", 1), sells(3, "leather_leggings", 1)]),
            (vec![buys("flint", 26)], vec![sells(5, "leather_helmet", 1), sells(4, "leather_boots", 1)]),
            (vec![buys("rabbit_hide", 9)], vec![sells(7, "leather_chestplate", 1)]),
            (vec![buys("turtle_scute", 4)], vec![sells(6, "leather_horse_armor", 1)]),
            (vec![sells(6, "saddle", 1)], vec![sells(5, "leather_helmet", 1)]),
        ],
        13 => vec![
            (vec![buys("clay_ball", 10)], vec![sells(1, "brick", 10)]),
            (vec![buys("stone", 20)], vec![sells(1, "chiseled_stone_bricks", 4)]),
            (vec![buys("granite", 16), buys("andesite", 16), buys("diorite", 16)], vec![sells(1, "polished_andesite", 4), sells(1, "polished_granite", 4), sells(1, "polished_diorite", 4)]),
            (vec![buys("quartz", 12)], vec![sells(1, "hardened_clay", 1)]),
            (vec![sells(1, "quartz_block", 1)], vec![sells(1, "quartz_pillar", 1)]),
        ],
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for (tier, (first, second)) in tiers.into_iter().enumerate() {
        let mixed = (seed ^ (tier as u64).wrapping_mul(0x9E37_79B9)).wrapping_mul(0x2545_F491_4F6C_DD1D);
        let mixed = mixed ^ (mixed >> 29);
        for (pool, pick) in [(first, mixed), (second, mixed >> 20)] {
            if !pool.is_empty() {
                out.push(Offer { tier: tier as i32, ..pool[(pick % pool.len() as u64) as usize] });
            }
        }
    }
    out
}

/// Un petit met 20 minutes à grandir ; un adulte attend 5 minutes avant un nouveau petit
const GROW_TICKS: i64 = 24_000;
const BREED_COOLDOWN: i64 = 6_000;
/// Durée de l'envie de se reproduire après un repas
const LOVE_TICKS: u32 = 600;

/// Ce que fait un warden, chacun avec son animation (indicateurs d'entité 84, 104, 106 et
/// 107 : « rugit », « sort de terre », « creuse », « onde sonique »).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum WardenAct {
    None,
    /// 7 secondes (« behavior.emerge »)
    Emerging,
    /// 4,2 secondes (« behavior.roar »), en trouvant sa cible
    Roaring,
    /// 3 secondes (« behavior.sonic_boom ») ; l'onde part à 1,7 seconde
    SonicBoom,
    /// 5,5 secondes (« behavior.dig ») après une minute sans cible, puis il disparaît
    Digging,
    /// 4,16 secondes (« behavior.sniff ») : il flaire ce qui vit à 24 blocs
    Sniffing,
}

impl WardenAct {
    /// Durée en ticks.
    fn ticks(self) -> u32 {
        match self {
            WardenAct::None => 0,
            WardenAct::Emerging => 140,
            WardenAct::Roaring => 84,
            WardenAct::SonicBoom => 60,
            WardenAct::Digging => 110,
            WardenAct::Sniffing => 83,
        }
    }
}

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
    /// Créature créée par un plugin : le rang de sa fiche (`entity/custom.rs`). Elle garde le
    /// comportement de son espèce ; la fiche change ce qui suit.
    custom: Option<u16>,
    /// Vitesse et taille données par la fiche (0 et 1 : celles de l'espèce)
    custom_speed: f32,
    custom_scale: f32,
    /// Boîte de collision donnée par la fiche : (largeur, hauteur) en blocs
    custom_box: Option<(f32, f32)>,
    /// Nom sous lequel elle est annoncée au jeu quand son apparence vient du pack de ressources
    ident: Option<String>,
    /// Monstre apparu de lui-même la nuit : il disparaît quand plus aucun joueur n'est près
    natural: bool,
    /// En feu (un zombie au soleil)
    pub(super) burning: bool,
    /// Rendue hostile (ou paisible) par sa fiche : Some(None) paisible, Some(Some(…)) hostile
    /// avec ses dégâts, sa vue et son nom dans le message de mort ; None : comme son espèce
    custom_hostile: Option<Option<(f32, f32, &'static str)>>,
    /// Tir voulu par sa fiche : (projectile, dégâts, ticks entre deux tirs, portée)
    custom_ranged: Option<(String, f32, u32, f32)>,
    /// Explosion voulue par sa fiche (puissance)
    custom_explode: Option<f32>,
    /// Potion voulue par sa fiche pour ses projectiles (variante)
    custom_potion: Option<u16>,
    /// Blaze : boules de feu déjà tirées dans la rafale en cours
    burst: u8,
    /// En feu jusqu'à ce tick (0 : non) — feu, lave, boule de feu
    /// Niveau de butin de l'arme du dernier joueur à l'avoir frappée
    pub looting: u8,
    /// Noyé armé d'un trident : il le lance au lieu de frapper
    pub trident: bool,
    /// Secondes d'air qui lui restent sous l'eau, et secondes passées la tête sous l'eau
    pub air: u8,
    pub soak: u8,
    /// Secondes passées hors de l'eau (axolotl, dauphin : ils se dessèchent)
    pub dry: u16,
    /// Piglin : ticks restants à admirer le lingot d'or reçu, avant de rendre son troc
    pub barter: u32,
    /// Inclinaison du corps (poulpes), en degrés, et la dernière envoyée
    pub tilt: f32,
    pub sent_tilt: f32,
    /// Warden : ce qu'il est en train de faire (sortir de terre, rugir, charger l'onde sonique,
    /// s'enfouir), et depuis combien de ticks il le fait
    pub act: WardenAct,
    pub act_ticks: u32,
    /// Warden : sa colère envers chaque suspect (joueur ou créature), de 0 à 150 ; à 80 il
    /// attaque (« anger_level » de la fiche du jeu)
    pub suspects: Vec<(Prey, u32)>,
    /// Warden : ce qu'il a rugi de poursuivre, l'endroit d'un bruit à aller voir, et le tick du
    /// prochain battement de cœur et du prochain flair
    pub focus: Option<Prey>,
    pub investigate: Option<(f32, f32, f32)>,
    pub beat_at: u64,
    pub sniff_at: u64,
    /// Warden : la plus courte distance (au carré) à sa cible, et depuis combien de ticks il
    /// ne s'en rapproche plus (coincé : c'est là qu'il tire son onde sonique)
    pub best_far: f32,
    pub stuck: u32,
    pub fire_until: u64,
    /// Sa cible, telle que le jeu la connaît (0 : aucune) : un archer qui en a une lève son arc
    aim: i64,
    /// Tourné vers sa cible, à portée et la vue dégagée
    facing: bool,
    /// Phantom : hauteur du point autour duquel il tourne, et ticks passés dans son piqué
    anchor_y: f32,
    dive: u16,
    /// Creeper : ticks écoulés depuis qu'il a allumé sa mèche (0 : éteinte)
    fuse: u8,
    /// Nom affiché au-dessus d'elle, et s'il reste visible de loin
    label: String,
    label_always: bool,
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
    /// Loup apprivoisé : nom de son maître, son identifiant s'il est connecté (-1 sinon),
    /// assis, en train de le rejoindre
    owner: Option<String>,
    owner_id: i64,
    sitting: bool,
    following: bool,
    /// Loup : sa cible, ticks avant la prochaine morsure
    attack: Option<Prey>,
    bite_in: u32,
    /// État à renvoyer aux joueurs (changé pendant qu'il réfléchissait)
    refresh: bool,
    /// Villageois : là où il vit (il ne s'en éloigne pas)
    home: Option<(f32, f32, f32)>,
    /// Villageois : ticks passés la nuit loin de chez lui ; point de rassemblement du village
    late: u32,
    meet: Option<(f32, f32)>,
    /// Villageois : où aller une fois la porte visée franchie
    errand: Option<(f32, f32)>,
    /// Villageois : expérience gagnée en échangeant (elle fait son niveau)
    trade_xp: i32,
    /// Usure de ses offres : nombre d'échanges conclus, offre par offre
    trade_uses: Vec<i32>,
    /// Dernier réassort connu (voir `restocks_before`) ; -1 : pas encore regardé
    trade_restock: i64,
    /// Villageois endormi : là où il est couché (sur son lit)
    asleep: Option<(f32, f32, f32)>,
    /// Villageois : son poste de travail (le bloc qui fait son métier)
    station: Option<(i32, i32, i32)>,
    /// Pas de nouvelle recherche de poste avant ce tick
    job_wait: u64,
    /// Villageois : chemin calculé vers sa destination (points restants, le prochain à la fin)
    path: Vec<(f32, f32)>,
    /// Zombie devant une porte en bois : depuis combien de ticks il la frappe
    door: u16,
    /// Pas de nouveau calcul de chemin avant ce tick
    path_wait: u64,
    /// Ticks passés à viser le même point du chemin (au-delà de trois secondes : bloqué)
    path_ticks: u32,
}

impl Mob {
    fn baby(&self) -> bool {
        self.age < 0
    }
    /// Seconde série d'indicateurs (donnée 92) : ceux du serveur officiel, plus « endormi »
    /// (indicateur 76) pour un villageois couché.
    fn flags2(&self) -> i64 {
        const BASE: i64 = -9_223_372_036_821_221_376;
        let mut f = BASE;
        if self.asleep.is_some() {
            f |= 1 << 12;
        }
        // (indicateur 88 : face à sa cible, prêt à tirer)
        if self.facing {
            f |= 1 << 24;
        }
        // Warden : l'animation de ce qu'il fait (« query.is_roaring », « is_emerging »,
        // « is_digging », « is_sonic_boom » des animations du jeu)
        f |= match self.act {
            WardenAct::None => 0,
            WardenAct::Roaring => 1 << (84 - 64),
            WardenAct::Emerging => 1 << (104 - 64),
            WardenAct::Digging => 1 << (106 - 64),
            WardenAct::SonicBoom => 1 << (107 - 64),
            WardenAct::Sniffing => 1 << (105 - 64),
        };
        f
    }
    fn flags(&self) -> i64 {
        let mut f = bedrock::MOB_FLAGS;
        if self.burning {
            f |= 1; // en feu
        }
        // Vindicateur en chasse : il décroise les bras et brandit sa hache
        if self.species == Species::Vindicator && self.attack.is_some() {
            f |= bedrock::FLAG_ANGRY;
        }
        if self.fuse > 0 {
            f |= 1 << 10; // mèche allumée : le creeper gonfle et clignote
        }
        if self.baby() {
            f |= bedrock::FLAG_BABY;
        }
        if self.love > 0 {
            f |= bedrock::FLAG_IN_LOVE;
        }
        if self.sheared {
            f |= bedrock::FLAG_SHEARED;
        }
        // (golem « apprivoisé » : bâti par un joueur, rien à montrer)
        if self.tamed && self.species != Species::IronGolem {
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
        if self.sitting {
            f |= bedrock::FLAG_SITTING;
        }
        // Loup en colère : il s'en prend à un joueur, ou sauvage il se bat
        if self.species == Species::Wolf && matches!(self.attack, Some(Prey::Player(_))) {
            f |= bedrock::FLAG_ANGRY;
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
        if self.custom_speed > 0.0 {
            self.custom_speed
        } else if self.species == Species::Horse && self.move_speed > 0.0 {
            self.move_speed
        } else if self.baby() && matches!(self.species, Species::Zombie | Species::Husk | Species::Drowned | Species::ZombieVillager) {
            // (un bébé zombie court bien plus vite qu'un adulte)
            0.35
        } else {
            self.species.movement()
        }
    }
    /// Dirigée par son cavalier (le jeu de celui-ci la déplace)
    fn driven_by_rider(&self) -> bool {
        self.rider.is_some() && self.tamed && self.saddled && self.species.steerable()
    }
    /// Slime : sa taille (1, 2 ou 4).
    fn slime_size(&self) -> f32 {
        if self.species.cube() { self.variant.clamp(1, 4) as f32 } else { 1.0 }
    }
    /// Taille d'affichage : celle de l'espèce et de la fiche du plugin (le jeu grossit
    /// lui-même un slime d'après sa taille, donnée comme variante).
    fn shown_scale(&self) -> f32 {
        self.species.scale(self.baby()) * self.custom_scale
    }
    /// Insensible au feu et à la lave, comme dans le jeu.
    pub(super) fn fireproof(&self) -> bool {
        matches!(self.species, Species::Blaze | Species::Ghast | Species::WitherSkeleton | Species::Strider | Species::MagmaCube | Species::ZombifiedPiglin | Species::Warden)
    }

    /// L'identifiant de l'espèce (`minecraft:blaze`) : le jeu choisit ses bruits d'après lui.
    pub(super) fn sound_id(&self) -> &'static str {
        self.species.id()
    }

    /// Le nom à montrer pour cette créature : celui de sa fiche de plugin, sinon son espèce.
    pub(super) fn label(&self) -> String {
        match self.custom.and_then(crate::entity::custom::get) {
            Some(def) => def.name,
            None => self.species.id().trim_start_matches("minecraft:").to_string(),
        }
    }

    pub(super) fn size(&self) -> (f32, f32) {
        let (w, h) = self.species.size();
        let (w, h) = (w * self.slime_size(), h * self.slime_size());
        // (boîte de la fiche si le plugin en a donné une ; sinon celle de l'espèce, à sa taille)
        let (w, h) = self.custom_box.unwrap_or((w * self.custom_scale, h * self.custom_scale));
        if self.baby() {
            (w / 2.0, h / 2.0)
        } else {
            (w, h)
        }
    }
    /// Boîte annoncée au jeu (données 53 et 54) : celle des fichiers du jeu, avant la taille
    /// d'affichage, que le jeu applique lui-même. Réduite ici aussi, celle d'un petit (ou d'un
    /// lapin) était réduite deux fois : trop petite pour le viser.
    /// Reçoit les valeurs de la fiche d'une créature créée par un plugin. `fresh` : elle vient
    /// d'apparaître, sa vie est celle de la fiche (rechargée, elle garde la sienne).
    fn take_custom(&mut self, rank: u16, def: &crate::entity::custom::CustomEntity, fresh: bool) {
        self.custom = Some(rank);
        self.custom_speed = def.speed.unwrap_or(0.0);
        self.custom_scale = def.scale;
        self.custom_box = def.hitbox;
        self.ident = def.own_model.then(|| def.name.clone());
        // Hostile ou paisible d'après la fiche ; ses dégâts et sa vue, ou ceux de l'espèce
        let natural = self.species.hostile();
        self.custom_hostile = match def.hostile {
            Some(true) => Some(Some((def.attack_damage.or(natural.map(|n| n.0)).unwrap_or(3.0), def.sight.or(natural.map(|n| n.1)).unwrap_or(16.0), def.cause))),
            Some(false) => Some(None),
            // (une vache qui tire ou qui explose s'en prend forcément aux joueurs)
            None if natural.is_none() && (def.ranged.is_some() || def.explosion.is_some()) => Some(Some((def.attack_damage.unwrap_or(0.0), def.sight.unwrap_or(16.0), def.cause))),
            None if def.attack_damage.is_some() || def.sight.is_some() => natural.map(|n| Some((def.attack_damage.unwrap_or(n.0), def.sight.unwrap_or(n.1), def.cause))),
            None => None,
        };
        self.custom_ranged = def.ranged.clone();
        self.custom_explode = def.explosion;
        self.custom_potion = def.potion;
        self.label = def.display.clone();
        self.label_always = def.name_visible;
        if let Some(health) = def.health {
            self.max_health = health;
            if fresh {
                self.health = health;
            }
            self.health = self.health.min(health);
        }
    }
    /// Monstre : (dégâts, vue, nom dans le message de mort), d'après sa fiche ou son espèce.
    fn hostile(&self) -> Option<(f32, f32, &'static str)> {
        match self.custom_hostile {
            Some(own) => own,
            None => self.species.hostile(),
        }
    }
    /// Tireur : (projectile, dégâts, ticks entre deux tirs, portée), d'après sa fiche ou son espèce.
    fn ranged(&self) -> Option<(String, f32, u32, f32)> {
        self.custom_ranged.clone().or_else(|| self.species.ranged().map(|r| (r.0.to_string(), r.1, r.2, r.3)))
    }
    /// Potion que contient son prochain projectile : celle de sa fiche, ou pour la sorcière
    /// l'une des siennes, tirée au sort (`roll` de 0 à 99) : poison, lenteur, dégâts, faiblesse.
    fn potion(&self, roll: u32) -> Option<u16> {
        self.custom_potion.or_else(|| {
            (self.species == Species::Witch).then(|| match roll {
                0..=39 => 25,
                40..=64 => 17,
                65..=84 => 23,
                _ => 34,
            })
        })
    }
    /// Explose au contact : la puissance, d'après sa fiche ou son espèce.
    fn explodes(&self) -> Option<f32> {
        self.custom_explode.or(self.species.explodes())
    }
    fn hitbox(&self) -> (f32, f32) {
        let adult = self.species.scale(false);
        match self.custom_box {
            // Boîte voulue par un plugin : annoncée divisée par la taille d'affichage, pour
            // que le jeu, qui la multiplie, retombe sur les dimensions demandées
            Some((w, h)) => (w / (adult * self.custom_scale), h / (adult * self.custom_scale)),
            None => {
                let (w, h) = self.species.size();
                (w * self.slime_size() / adult, h * self.slime_size() / adult)
            }
        }
    }
}

/// Au plus tant de créatures en mémoire à la fois (au-delà, les terres neuves n'en reçoivent
/// plus ; celles qui sont enregistrées reviennent toujours)
const MAX_MOBS: usize = 4000;
/// Monstres apparus d'eux-mêmes autour d'un joueur, au plus.
const MONSTERS_PER_PLAYER: usize = 8;
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
    /// Nom de la créature créée par un plugin (vide : une créature du jeu)
    pub custom: String,
    /// Monstre apparu de lui-même (voir `Mob::natural`)
    pub natural: bool,
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
    /// Loup apprivoisé : son maître, assis
    pub owner: Option<String>,
    pub sitting: bool,
    /// Villageois : là où il vit
    pub home: Option<(f32, f32, f32)>,
    /// Villageois : expérience d'échange et usure de ses offres
    pub trade_xp: i32,
    pub trade_uses: Vec<i32>,
    /// Villageois : dernier réassort connu (-1 : jamais regardé)
    pub trade_restock: i64,
    /// Villageois : son poste de travail
    pub station: Option<(i32, i32, i32)>,
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
            ("custom".into(), Json::Str(self.custom.clone())),
            ("natural".into(), Json::Bool(self.natural)),
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
            ("owner".into(), match &self.owner {
                Some(o) => Json::Str(o.clone()),
                None => Json::Null,
            }),
            ("sitting".into(), Json::Bool(self.sitting)),
            ("home".into(), match self.home {
                Some((x, y, z)) => Json::Arr(vec![n(x), n(y), n(z)]),
                None => Json::Null,
            }),
            ("station".into(), match self.station {
                Some((x, y, z)) => Json::Arr(vec![Json::Num(x as f64), Json::Num(y as f64), Json::Num(z as f64)]),
                None => Json::Null,
            }),
            ("trade_xp".into(), Json::Num(self.trade_xp as f64)),
            ("trade_uses".into(), Json::Arr(self.trade_uses.iter().map(|u| Json::Num(*u as f64)).collect())),
            ("trade_restock".into(), Json::Num(self.trade_restock as f64)),
            ("items".into(), Json::Arr(self.items.iter().map(|(slot, name, count)| Json::Arr(vec![Json::Num(*slot as f64), Json::Str(name.clone()), Json::Num(*count as f64)])).collect())),
        ])
    }

    pub fn from_json(v: &Json) -> Option<MobRecord> {
        let species = v.get("kind").and_then(|k| k.as_str()).and_then(Species::from_id)?;
        let num = |k: &str| v.get(k).and_then(|n| n.as_f64()).filter(|n| n.abs() < 3.0e7);
        // (une créature créée par un plugin peut avoir plus de vie que son espèce)
        let custom = v.get("custom").and_then(|c| c.as_str()).unwrap_or("").to_string();
        let ceiling = if custom.is_empty() { species.health() } else { 1024.0 };
        let max_health = (num("max_health").unwrap_or(species.health() as f64) as f32).clamp(1.0, ceiling);
        Some(MobRecord {
            species,
            pos: (num("x")? as f32, num("y")? as f32, num("z")? as f32),
            yaw: num("yaw").unwrap_or(0.0) as f32,
            health: (num("health").unwrap_or(max_health as f64) as f32).clamp(0.5, max_health),
            max_health,
            custom,
            natural: v.get("natural").and_then(|s| s.as_bool()).unwrap_or(false),
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
            owner: v.get("owner").and_then(|o| o.as_str()).filter(|o| !o.is_empty() && o.len() <= 64).map(|o| o.to_string()),
            sitting: v.get("sitting").and_then(|s| s.as_bool()).unwrap_or(false),
            home: v.get("home").and_then(|k| k.as_arr()).and_then(|k| match k {
                [x, y, z] => Some((x.as_f64()? as f32, y.as_f64()? as f32, z.as_f64()? as f32)),
                _ => None,
            }),
            station: v.get("station").and_then(|k| k.as_arr()).and_then(|k| match k {
                [x, y, z] => Some((x.as_f64()? as i32, y.as_f64()? as i32, z.as_f64()? as i32)),
                _ => None,
            }),
            trade_xp: num("trade_xp").unwrap_or(0.0).clamp(0.0, 100_000.0) as i32,
            trade_restock: num("trade_restock").map(|n| n.clamp(-1.0, 1.0e15) as i64).unwrap_or(-1),
            trade_uses: v.get("trade_uses").and_then(|k| k.as_arr()).map(|k| k.iter().take(16).map(|u| u.as_f64().unwrap_or(0.0).clamp(0.0, 64.0) as i32).collect()).unwrap_or_default(),
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
            custom: self.custom.and_then(crate::entity::custom::get).map(|def| def.name).unwrap_or_default(),
            natural: self.natural,
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
            owner: self.owner.clone(),
            sitting: self.sitting,
            home: self.home,
            trade_xp: self.trade_xp,
            trade_uses: self.trade_uses.clone(),
            trade_restock: self.trade_restock,
            station: self.station,
            items: self.items.iter().enumerate().filter_map(|(i, it)| it.as_ref().map(|(n, c)| (i as u8, n.clone(), *c))).collect(),
        }
    }
}

impl Server {
    /// Installe des animaux dans les chunks qu'un joueur découvre (une fois par chunk, comme
    /// à la génération dans le jeu : un chunk sur dix environ reçoit un troupeau).
    /// Créature qui apparaît ici au Nether ou dans l'End, selon le biome (None : rien cette
    /// fois). Seules les espèces que Basalt connaît : ghast, blaze, wither squelette, squelette,
    /// enderman. Les shulkers manquent encore.
    fn other_species(&mut self, x: i32, y: i32, z: i32) -> Option<Species> {
        use crate::world::generator::{Kind, NETHER_CRIMSON, NETHER_DELTAS, NETHER_SOUL_VALLEY, NETHER_WARPED, NETHER_WASTES};
        let roll = self.mob_rng.below(100);
        let gen = self.world.generator();
        if gen.kind == Kind::End {
            return Some(Species::Enderman);
        }
        if gen.in_fortress(x, y, z) {
            return Some(match roll {
                0..=39 => Species::Blaze,
                40..=89 => Species::WitherSkeleton,
                _ => Species::Skeleton,
            });
        }
        match (gen.nether_biome(x, z), roll) {
            (NETHER_WASTES, 0..=14) | (NETHER_SOUL_VALLEY, 60..=84) | (NETHER_DELTAS, 0..=19) => Some(Species::Ghast),
            (NETHER_WASTES, 15..=19) | (NETHER_SOUL_VALLEY, 85..=94) | (NETHER_WARPED, 0..=69) => Some(Species::Enderman),
            (NETHER_SOUL_VALLEY, 0..=59) => Some(Species::Skeleton),
            (NETHER_WASTES, 20..=69) | (NETHER_CRIMSON, 0..=19) => Some(Species::ZombifiedPiglin),
            (NETHER_WASTES, 70..=79) | (NETHER_CRIMSON, 20..=59) => Some(Species::Piglin),
            (NETHER_CRIMSON, 60..=84) => Some(Species::Hoglin),
            (NETHER_WASTES, 80..=89) | (NETHER_DELTAS, 20..=89) => Some(Species::MagmaCube),
            (NETHER_WARPED, 70..=79) => Some(Species::Strider),
            _ => None,
        }
    }

    /// Une fois par seconde : la vie de l'eau et des grottes autour de chaque joueur, comme les
    /// règles d'apparition du jeu (de mémoire : les fichiers « spawn_rules » ne sont pas dans
    /// les échantillons Mojang). Dix au plus par joueur, de 16 à 40 blocs ; ils disparaissent
    /// au loin comme les monstres.
    ///
    /// - océans : morues et calamars, dauphins (rares) ; chauds : poissons tropicaux et
    ///   poissons-globes ; froids et rivières : saumons ;
    /// - eau des grottes : calamars luisants, et axolotls sous les grottes luxuriantes ;
    /// - grottes sombres : chauves-souris.
    fn ambient_spawn(&mut self) {
        let players: Vec<(f32, f32, f32)> = self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame && c.dead.is_none() && !c.mode.is_spectator()).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2)).collect();
        let water = self.world.generator().blocks.water;
        let clay = self.world.generator().blocks.clay;
        for (px, py, pz) in players {
            if self.mobs.len() >= MAX_MOBS || self.mob_rng.below(2) != 0 {
                continue;
            }
            let around = self.mobs.values().filter(|m| m.natural && m.species.ambient() && m.dead.is_none() && (m.pos.0 - px).powi(2) + (m.pos.2 - pz).powi(2) < 64.0 * 64.0).count();
            if around >= 10 {
                continue;
            }
            let angle = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
            let far = 16.0 + self.mob_rng.unit() as f32 * 24.0;
            let (x, z) = ((px + angle.cos() * far).floor() as i32, (pz + angle.sin() * far).floor() as i32);
            if !self.world.is_ready(x >> 4, z >> 4) {
                continue;
            }
            // Une fois sur deux, la surface de l'eau de cette colonne et une profondeur au
            // hasard (un point pris au hasard tombe surtout dans la terre ou l'air) ; sinon un
            // point quelconque, pour les grottes
            let mut y = py.floor() as i32 - 24 + self.mob_rng.below(40) as i32;
            if self.mob_rng.below(2) == 0 {
                let top = (py.floor() as i32 + 24).min(crate::world::MAX_Y - 1);
                if let Some(surface) = (0..64).map(|d| top - d).find(|ty| crate::world::World::in_bounds(*ty) && self.mob_get(x, *ty, z) == water) {
                    let depth = (1..32).take_while(|d| self.mob_get(x, surface - d, z) == water).count() as i32;
                    y = surface - self.mob_rng.below(depth as u32 + 1) as i32;
                }
            }
            if !crate::world::World::in_bounds(y) || !crate::world::World::in_bounds(y + 1) {
                continue;
            }
            let here = self.mob_get(x, y, z);
            let (species, count) = if here == water && self.mob_get(x, y + 1, z) != self.world.generator().blocks.stone {
                // L'eau d'une grotte : au-dessus de la colonne d'eau, pas de ciel (la lumière
                // du ciel faiblit sous l'eau : on la mesure à la surface, pas au fond)
                let top = (y + 1..y + 64).find(|ty| self.mob_get(x, *ty, z) != water).unwrap_or(y + 1);
                let cave = !self.mob_free(x, top, z) || self.light_at(x, top, z).sky == 0;
                let roll = self.mob_rng.below(100);
                if cave {
                    let vanilla = self.world.generator().vanilla().map(|v| v.lush_caves_at(x, z)).unwrap_or(false);
                    let floor_clay = (1..8).map(|d| self.mob_get(x, y - d, z)).find(|s| *s != water) == Some(clay);
                    if vanilla && floor_clay && y < 40 {
                        (Species::Axolotl, 1 + self.mob_rng.below(3))
                    } else if y < 30 {
                        (Species::GlowSquid, 1 + self.mob_rng.below(3))
                    } else {
                        continue;
                    }
                } else {
                    let biome = self.world.generator().vanilla().map(|v| v.surface_biome_at(x, z));
                    match (biome, roll) {
                        (Some(B::WarmOcean), 0..=69) | (Some(B::LukewarmOcean | B::DeepLukewarmOcean), 0..=34) | (Some(B::MangroveSwamp), 0..=29) => (Species::TropicalFish, 3 + self.mob_rng.below(6)),
                        (Some(B::WarmOcean | B::LukewarmOcean | B::DeepLukewarmOcean), 70..=84) => (Species::Pufferfish, 1 + self.mob_rng.below(3)),
                        (Some(B::Ocean | B::DeepOcean | B::WarmOcean | B::LukewarmOcean | B::DeepLukewarmOcean), 85..=89) => (Species::Dolphin, 1 + self.mob_rng.below(2)),
                        (Some(B::ColdOcean | B::DeepColdOcean | B::FrozenOcean | B::DeepFrozenOcean | B::River | B::FrozenRiver), 0..=59) => (Species::Salmon, 1 + self.mob_rng.below(4)),
                        (Some(B::Ocean | B::DeepOcean | B::LukewarmOcean | B::DeepLukewarmOcean | B::ColdOcean | B::DeepColdOcean), 35..=79) => (Species::Cod, 3 + self.mob_rng.below(4)),
                        (Some(B::Ocean | B::DeepOcean | B::ColdOcean | B::DeepColdOcean | B::River), _) => (Species::Squid, 1 + self.mob_rng.below(3)),
                        // (côtes classées plage, eaux sans règle : comme l'océan s'il y a du fond)
                        (_, 0..=49) if self.mob_get(x, y - 2, z) == water => (Species::Cod, 3 + self.mob_rng.below(4)),
                        (_, _) if self.mob_get(x, y - 2, z) == water && !matches!(biome, Some(B::Swamp | B::MangroveSwamp)) => (Species::Squid, 1 + self.mob_rng.below(3)),
                        _ => continue,
                    }
                }
            } else if self.mob_free(x, y, z) && here != water && self.mob_free(x, y + 1, z) && y < 63 && self.mob_rng.below(3) == 0 && self.dark_for_monsters(x, y, z, false) && self.light_at(x, y, z).sky == 0 {
                (Species::Bat, 1 + self.mob_rng.below(2))
            } else {
                continue;
            };
            for _ in 0..count {
                let spot = (x as f32 + 0.5 + (self.mob_rng.unit() as f32 - 0.5) * 2.0, y as f32 + 0.2, z as f32 + 0.5 + (self.mob_rng.unit() as f32 - 0.5) * 2.0);
                let fits = if species == Species::Bat { self.mob_free(spot.0.floor() as i32, spot.1.floor() as i32, spot.2.floor() as i32) } else { self.mob_water(spot.0, spot.1, spot.2) };
                if !fits || self.mobs.len() >= MAX_MOBS {
                    continue;
                }
                let id = self.add_mob(species, spot);
                let variant = if species == Species::Axolotl { axolotl_color(&mut self.mob_rng) } else { 0 };
                if let Some(m) = self.mobs.get_mut(&id) {
                    m.natural = true;
                    m.variant = variant;
                }
                if self.plugins.listens("CreatureSpawnEvent") {
                    let out = self.fire("CreatureSpawnEvent", vec![("entity", self.mob_json(id))]);
                    if super::cancelled(&out) {
                        self.remove_mob(id);
                    }
                }
            }
        }
    }

    /// Ce monde est-il le Nether ou l'End ?
    pub(super) fn other_dimension(&self) -> bool {
        matches!(self.world.generator().kind, crate::world::generator::Kind::Nether | crate::world::generator::Kind::End)
    }

    pub(super) fn populate_chunks(&mut self) {
        if !self.cfg.spawn_animals || self.other_dimension() {
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
                    // Village : ses habitants, près de leurs lits (un pour deux blocs de lit,
                    // quatre par chunk au plus), au costume du biome
                    let beds = self.world.structure_beds(cx, cz);
                    let villagers = if beds.is_empty() { 0 } else { beds.len().div_ceil(2).min(4) };
                    for i in 0..villagers {
                        if self.mobs.len() >= MAX_MOBS {
                            break;
                        }
                        let bed = beds[(i * 2).min(beds.len() - 1)];
                        // Dans la maison, au pied du lit (pas sur le toit)
                        let Some((bx, y, bz)) = self.floor_near(bed, 2) else { continue };
                        let costume = match self.world.generator().vanilla().map(|v| v.surface_biome_at(bx, bz)) {
                            Some(B::Desert) => 1,
                            Some(B::Jungle | B::SparseJungle | B::BambooJungle) => 2,
                            Some(B::Savanna | B::SavannaPlateau | B::WindsweptSavanna) => 3,
                            Some(B::SnowyPlains | B::IceSpikes | B::SnowyTaiga | B::SnowySlopes | B::Grove) => 4,
                            Some(B::Taiga | B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga) => 6,
                            _ => 0,
                        };
                        // Sans métier au départ, sauf le simplet : le poste de travail le plus
                        // proche de chez lui lui en donne un dans les secondes qui suivent
                        let job = if self.mob_rng.below(10) == 0 { 14 } else { 0 };
                        let baby = self.mob_rng.below(10) == 0;
                        let id = self.add_mob(Species::Villager, (bx as f32 + 0.5, y as f32, bz as f32 + 0.5));
                        if let Some(m) = self.mobs.get_mut(&id) {
                            m.variant = if baby { 0 } else { job };
                            m.mark = costume;
                            if baby {
                                m.age = -GROW_TICKS;
                            }
                        }
                    }
                    // Un golem de fer au pied de la cloche : le gardien du village
                    for bell in self.world.structure_bells(cx, cz) {
                        if self.mobs.len() >= MAX_MOBS {
                            break;
                        }
                        let spot = self.golem_spot(bell);
                        if let Some((x, y, z)) = spot {
                            let id = self.add_mob(Species::IronGolem, (x as f32 + 0.5, y as f32, z as f32 + 0.5));
                            if let Some(m) = self.mobs.get_mut(&id) {
                                m.age = 0;
                                m.home = Some((bell.0 as f32 + 0.5, bell.1 as f32, bell.2 as f32 + 0.5));
                            }
                        }
                    }
                    // Les enclos du village : deux à quatre bêtes d'une même espèce dans chacun
                    let village = !beds.is_empty() || (-1..=1).any(|dx| (-1..=1).any(|dz| !self.world.structure_beds(cx + dx, cz + dz).is_empty() || !self.world.structure_bells(cx + dx, cz + dz).is_empty()));
                    if village && self.cfg.vanilla.village_animals {
                        if let Some((x, y, z)) = self.find_pen(cx, cz) {
                            let species = [Species::Cow, Species::Cow, Species::Cow, Species::Sheep, Species::Sheep, Species::Sheep, Species::Pig, Species::Pig, Species::Horse, Species::Horse][self.mob_rng.below(10) as usize];
                            for _ in 0..2 + self.mob_rng.below(3) {
                                if self.mobs.len() >= MAX_MOBS {
                                    break;
                                }
                                self.add_mob(species, (x as f32 + 0.5, y as f32, z as f32 + 0.5));
                            }
                        }
                    }
                    // Un chat errant près d'un lit, de temps en temps
                    if !beds.is_empty() && self.mobs.len() < MAX_MOBS && self.mob_rng.below(3) == 0 {
                        let bed = beds[self.mob_rng.below(beds.len() as u32) as usize];
                        if let Some((bx, y, bz)) = self.floor_near(bed, 3) {
                            let variant = self.cat_variant();
                            let id = self.add_mob(Species::Cat, (bx as f32 + 0.5, y as f32, bz as f32 + 0.5));
                            if let Some(m) = self.mobs.get_mut(&id) {
                                m.variant = variant;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Un enclos dans ce chunk : un sol de terre ou d'herbe à ciel ouvert, fermé par une
    /// barrière dans les quatre directions à sept blocs au plus. Rend une place en son milieu.
    fn find_pen(&self, cx: i32, cz: i32) -> Option<(i32, i32, i32)> {
        let d = self.data.as_ref()?;
        let air = self.world.air;
        let name = |x: i32, y: i32, z: i32| if crate::world::World::in_bounds(y) && self.world.is_ready(x >> 4, z >> 4) { d.block_name(self.world.get(x, y, z)).unwrap_or("").trim_start_matches("minecraft:") } else { "" };
        let fence = |n: &str| n.contains("fence") || n.ends_with("_wall");
        let mut found: Vec<(i32, i32, i32)> = Vec::new();
        for lx in 0..16 {
            for lz in 0..16 {
                let (x, z) = (cx * 16 + lx, cz * 16 + lz);
                let y = self.world.highest(x, z, &|state: u32| state != air);
                if !matches!(name(x, y, z), "grass_block" | "dirt" | "coarse_dirt" | "grass_path" | "sand" | "podzol") || !crate::world::World::in_bounds(y + 2) {
                    continue;
                }
                // Dans chaque direction : de l'air, puis une barrière
                let closed = [(1, 0), (-1, 0), (0, 1), (0, -1)].into_iter().all(|(dx, dz)| {
                    (1..=7).find_map(|i| {
                        let n = name(x + dx * i, y + 1, z + dz * i);
                        if fence(n) {
                            Some(true)
                        } else if n != "air" {
                            Some(false)
                        } else {
                            None
                        }
                    }) == Some(true)
                });
                if closed {
                    found.push((x, y + 1, z));
                }
            }
        }
        // (le milieu de l'enclos : la place médiane parmi celles trouvées)
        found.sort_unstable();
        found.get(found.len() / 2).copied()
    }

    /// Une place où se tenir près de ce bloc, à sa hauteur (un étage près) : deux blocs libres
    /// sur un sol solide, au plus près d'abord.
    fn floor_near(&self, at: (i32, i32, i32), radius: i32) -> Option<(i32, i32, i32)> {
        let mut best: Option<((i32, i32, i32), i32)> = None;
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                for dy in [0, -1, 1] {
                    let (x, y, z) = (at.0 + dx, at.1 + dy, at.2 + dz);
                    if self.mob_free(x, y, z) && self.mob_free(x, y + 1, z) && !self.mob_free(x, y - 1, z) && !self.mob_water(x as f32 + 0.5, y as f32 - 0.5, z as f32 + 0.5) {
                        let d = dx * dx + dz * dz + dy.abs() * 4;
                        if best.map(|(_, b)| d < b).unwrap_or(true) {
                            best = Some(((x, y, z), d));
                        }
                    }
                }
            }
        }
        best.map(|(p, _)| p)
    }

    /// Métier d'un villageois (numéros du jeu : 1 fermier, 2 pêcheur, 3 berger, 4 fléchier,
    /// 5 bibliothécaire, 6 cartographe, 7 clerc, 8 armurier, 9 forgeron d'armes, 10 forgeron
    /// d'outils, 11 boucher, 12 tanneur, 13 maçon ; 14 « nigaud », un sur dix)
    fn villager_job(&mut self) -> i32 {
        if self.mob_rng.below(10) == 0 {
            14
        } else {
            1 + self.mob_rng.below(13) as i32
        }
    }

    /// Pelage d'un chat errant : l'un des onze du jeu (le noir est rare hors des marais)
    fn cat_variant(&mut self) -> i32 {
        match self.mob_rng.below(31) {
            30 => 9,
            n => [0, 1, 2, 3, 4, 5, 6, 7, 8, 10][(n % 10) as usize],
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
                B::Grove => match roll {
                    0..=2 => Species::Wolf,
                    3..=5 => Species::Fox,
                    _ => Species::Rabbit,
                },
                // Ours polaires sur la neige et la glace
                B::SnowyPlains | B::IceSpikes if roll == 0 => Species::PolarBear,
                B::Desert | B::SnowyPlains | B::IceSpikes => Species::Rabbit,
                // Tortues sur les plages, grenouilles dans les marais
                B::Beach if roll < 4 => Species::Turtle,
                B::Swamp | B::MangroveSwamp if roll < 5 => Species::Frog,
                // Jungle : ocelots, perroquets ; pandas dans la jungle de bambous
                B::BambooJungle if roll < 4 => Species::Panda,
                B::Jungle | B::SparseJungle | B::BambooJungle if roll < 2 => Species::Ocelot,
                B::Jungle | B::SparseJungle | B::BambooJungle if roll < 5 => Species::Parrot,
                // Tatous dans la savane et les badlands
                B::Savanna | B::SavannaPlateau | B::Badlands | B::WoodedBadlands | B::ErodedBadlands if roll >= 8 => Species::Armadillo,
                B::Badlands | B::WoodedBadlands | B::ErodedBadlands => return,
                // Abeilles là où il y a des fleurs (dans le jeu elles sortent des ruches, que la
                // génération ne pose pas encore)
                B::FlowerForest | B::Meadow | B::CherryGrove | B::SunflowerPlains if roll >= 7 => Species::Bee,
                B::Plains if roll == 9 => Species::Bee,
                // Renards dans les taïgas
                B::Taiga | B::SnowyTaiga | B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga if roll < 5 && roll >= 2 => Species::Fox,
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
                // Meutes de loups : forêts (rares), taïgas
                B::Forest if roll == 0 => Species::Wolf,
                B::Taiga | B::SnowyTaiga | B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga if roll < 2 => Species::Wolf,
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
            Species::Wolf => (4, 4, 10),
            // (tailles de groupe des règles d'apparition du jeu, de mémoire)
            Species::Fox => (2, 4, 10),
            Species::PolarBear => (1, 2, 30),
            Species::Turtle => (2, 5, 10),
            Species::Frog => (2, 5, 0),
            Species::Panda => (1, 2, 20),
            Species::Ocelot => (1, 3, 0),
            Species::Parrot => (1, 2, 0),
            Species::Armadillo => (1, 2, 10),
            Species::Bee => (2, 3, 0),
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
            // Renard roux, ou arctique dans la neige ; grenouille tempérée, ou chaude dans la
            // mangrove ; cinq couleurs de perroquet
            Species::Fox => matches!(biome, Some(B::SnowyTaiga | B::Grove)) as i32,
            Species::Frog => if biome == Some(B::MangroveSwamp) { 2 } else { 0 },
            Species::Parrot => self.mob_rng.below(5) as i32,
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
                Species::Goat | Species::Llama | Species::Mooshroom | Species::Wolf | Species::Fox | Species::PolarBear | Species::Frog | Species::Armadillo | Species::Parrot | Species::Panda | Species::Ocelot | Species::Bee => true,
                Species::Turtle => below == sand,
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
            // (le feuillage et les troncs ne sont pas un sol : on regarde dessous)
            if !self.mob_free(x, y - 1, z) && self.canopy(self.mob_get(x, y - 1, z)) {
                y -= 1;
                continue;
            }
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

    /// Feuillage, tronc ou liane : là où une créature ne doit pas apparaître.
    fn canopy(&self, state: u32) -> bool {
        self.block_name_of(state).map(|n| n.contains("leaves") || n.ends_with("_log") || n.ends_with("_wood") || n.ends_with("_stem") || n.contains("vine") || n.contains("mangrove_roots")).unwrap_or(false)
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

    /// Ce qu'un bloc fait à la lumière (voir `world/light.rs`).
    pub(super) fn light_cell(&self, state: u32) -> crate::world::light::Cell {
        use crate::world::light::Cell;
        let gen = self.world.generator();
        if state == gen.blocks.air {
            return Cell::CLEAR;
        }
        if state == gen.blocks.water {
            return Cell { emit: 0, damp: 2 };
        }
        let Some(d) = self.data.as_ref() else { return Cell::OPAQUE };
        let Some(name) = d.block_name(state) else { return Cell::OPAQUE };
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        let props = d.block_props(name);
        let emit = props.map(|p| p.light).unwrap_or(0);
        // Les feuilles, la toile et la glace retiennent un peu ; le reste laisse passer ou arrête
        let damp = if short.contains("leaves") || short == "web" || short == "ice" || short == "frosted_ice" {
            1
        } else if short.contains("water") {
            2
        } else {
            match props {
                Some(p) if p.opacity >= 1.0 => 15,
                Some(_) => 0,
                // Bloc absent de la table du jeu : ce que l'on traverse laisse passer
                None if self.npc_passable(state) => 0,
                None => 15,
            }
        };
        Cell { emit, damp }
    }

    /// Lumière reçue à un endroit : celle des blocs lumineux, et celle du ciel en plein jour.
    pub(super) fn light_at(&self, x: i32, y: i32, z: i32) -> crate::world::light::Light {
        // Les mêmes blocs reviennent sans cesse : leur fiche n'est lue qu'une fois par question
        let known: std::cell::RefCell<HashMap<u32, crate::world::light::Cell>> = std::cell::RefCell::new(HashMap::new());
        let of = |s: u32| *known.borrow_mut().entry(s).or_insert_with(|| self.light_cell(s));
        let air = self.world.generator().blocks.air;
        let cell = |x: i32, y: i32, z: i32| of(self.mob_get(x, y, z));
        let top = |x: i32, z: i32| {
            if !self.world.is_ready(x >> 4, z >> 4) {
                return crate::world::MAX_Y;
            }
            self.world.highest(x, z, &|s| s != air && of(s).damp > 0)
        };
        crate::world::light::light_at((x, y, z), &cell, &top)
    }

    /// Un monstre peut-il apparaître ici ? Aucune lumière de bloc (une torche suffit à
    /// l'empêcher), et peu de lumière du ciel : la nuit partout, le jour à l'abri seulement.
    fn dark_for_monsters(&self, x: i32, y: i32, z: i32, night: bool) -> bool {
        // (en plein jour et à ciel ouvert : inutile de chercher des torches)
        let air = self.world.generator().blocks.air;
        if !night && y > self.world.highest(x, z, &|s| s != air && self.light_cell(s).damp > 0) {
            return false;
        }
        let light = self.light_at(x, y, z);
        light.block == 0 && light.sky_now(night) <= 7
    }

    /// Premier sol sous une hauteur donnée (12 blocs au plus), avec deux blocs libres
    /// au-dessus : un endroit où tenir debout dans une grotte.
    fn floor_below(&self, x: i32, from: i32, z: i32) -> Option<i32> {
        let water = self.world.generator().blocks.water;
        let top = from.min(crate::world::MAX_Y - 2);
        let bottom = (from - 12).max(crate::world::MIN_Y + 1);
        // (parti de la roche : il n'y a pas de sol à chercher)
        if !self.mob_free(x, top, z) || !self.world.is_ready(x >> 4, z >> 4) {
            return None;
        }
        let mut y = top;
        while y > bottom {
            if !self.mob_free(x, y - 1, z) {
                let dry = self.mob_get(x, y, z) != water && self.mob_get(x, y - 1, z) != water;
                return (dry && self.mob_free(x, y + 1, z)).then_some(y);
            }
            y -= 1;
        }
        None
    }

    /// Un bloc que l'on traverse (air, herbe haute, fleur, eau…).
    pub(super) fn mob_free(&self, x: i32, y: i32, z: i32) -> bool {
        let s = self.mob_get(x, y, z);
        let p = &self.world.generator().blocks;
        // (herbes marines et laminaires se traversent comme l'eau qui les contient ; un lit ou
        // une dalle basse ne remplit pas son bloc : on y entre, et on se tient dessus)
        s == p.water || p.aquatic.contains(&s) || self.npc_passable(s) || self.low_top(s).is_some()
    }

    /// Hauteur du dessus d'un bloc bas (lit, dalle du bas), en fraction de bloc.
    fn low_top(&self, state: u32) -> Option<f32> {
        let d = self.data.as_ref()?;
        let name = d.block_name(state)?;
        if name == "minecraft:bed" {
            return Some(0.5625);
        }
        if name.ends_with("_slab") && !name.contains("double") && d.prop(state, "minecraft:vertical_half") == Some(crate::data::Prop::Str("bottom".into())) {
            return Some(0.5);
        }
        None
    }

    /// Bloc sur lequel une créature ne grimpe pas d'un saut : barrières et murets (trop hauts
    /// dans le jeu), portillons, trappes, portes, vitres et barreaux (trop minces pour s'y
    /// tenir). Sans cela, elle se retrouvait perchée dessus, comme en l'air.
    fn no_step(&self, x: i32, y: i32, z: i32) -> bool {
        let s = self.mob_get(x, y, z);
        self.data.as_ref().and_then(|d| d.block_name(s)).map(|n| n.contains("fence") || n.ends_with("_wall") || n.contains("trapdoor") || n.contains("door") || n.contains("_pane") || n.contains("iron_bars")).unwrap_or(false)
    }
    /// Comme `mob_free`, mais une porte en bois laisse aussi passer (`doors` : un villageois).
    fn mob_pass(&self, doors: bool, x: i32, y: i32, z: i32) -> bool {
        self.mob_free(x, y, z) || (doors && self.wood_door(x, y, z))
    }
    /// Une porte en bois (pas en fer) à cet endroit ?
    fn wood_door(&self, x: i32, y: i32, z: i32) -> bool {
        let s = self.mob_get(x, y, z);
        self.data.as_ref().map(|d| crate::block::placement::is_door(d, s) && !d.block_name(s).map(|n| n.contains("iron")).unwrap_or(true)).unwrap_or(false)
    }
    /// Ouvre ou ferme une porte (ses deux moitiés). Vrai si elle a changé.
    fn set_door(&mut self, pos: (i32, i32, i32), open: bool) -> bool {
        use crate::data::Prop;
        let mut changed = false;
        for y in [pos.1 - 1, pos.1, pos.1 + 1] {
            let s = self.world.get(pos.0, y, pos.2);
            let next = self.data.as_ref().and_then(|d| {
                if !crate::block::placement::is_door(d, s) || d.prop(s, "open_bit") == Some(Prop::Int(open as i32)) {
                    return None;
                }
                d.with_props(s, &[("open_bit", Prop::Int(open as i32))])
            });
            if let Some(n) = next {
                self.change_block((pos.0, y, pos.2), n);
                changed = true;
            }
        }
        // Le bruit de la porte, pour les joueurs proches
        if changed && !self.refused_extras.contains(&bedrock::ID_LEVEL_EVENT) {
            let at = (pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32 + 0.5);
            let packet = bedrock::level_event(bedrock::LEVEL_EVENT_SOUND_DOOR, at, 0);
            let near: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined && (c.pos.0 - at.0).powi(2) + (c.pos.2 - at.2).powi(2) < 16.0 * 16.0).map(|(a, _)| *a).collect();
            for a in near {
                self.send(&a, bedrock::ID_LEVEL_EVENT, &packet);
            }
        }
        changed
    }
    /// Villageois : la porte en bois la plus proche (8 blocs, un étage près) et le point où
    /// aller une fois passée, de l'autre côté.
    fn door_errand(&self, m: &Mob) -> Option<((f32, f32), (f32, f32))> {
        self.door_near(m.pos, m.pos)
    }
    /// La porte en bois la plus proche de `center`, et le point de l'autre côté pour qui
    /// vient de `from`.
    fn door_near(&self, center: (f32, f32, f32), from: (f32, f32, f32)) -> Option<((f32, f32), (f32, f32))> {
        let (px, py, pz) = (center.0.floor() as i32, center.1.floor() as i32, center.2.floor() as i32);
        let mut best: Option<((i32, i32, i32), i32)> = None;
        for dx in -8..=8 {
            for dz in -8..=8 {
                for dy in -1..=1 {
                    let (x, y, z) = (px + dx, py + dy, pz + dz);
                    // (la moitié basse : le sol est dessous)
                    if self.wood_door(x, y, z) && !self.wood_door(x, y - 1, z) {
                        let d = dx * dx + dz * dz;
                        if best.map(|(_, b)| d < b).unwrap_or(true) {
                            best = Some(((x, y, z), d));
                        }
                    }
                }
            }
        }
        let ((x, y, z), _) = best?;
        let door = (x as f32 + 0.5, z as f32 + 0.5);
        // La porte se traverse dans l'axe où ses deux côtés sont libres ; on vise le côté
        // opposé à celui où se tient le villageois
        let along_x = self.mob_free(x - 1, y, z) && self.mob_free(x + 1, y, z);
        let along_z = self.mob_free(x, y, z - 1) && self.mob_free(x, y, z + 1);
        let beyond = if along_x {
            (door.0 + if from.0 < door.0 { 3.0 } else { -3.0 }, door.1)
        } else if along_z {
            (door.0, door.1 + if from.2 < door.1 { 3.0 } else { -3.0 })
        } else {
            return None;
        };
        Some((door, beyond))
    }
    /// Portes des villageois : celui qui arrive devant une porte fermée l'ouvre ; elle se
    /// referme une seconde après son passage.
    fn villager_doors(&mut self) {
        let tick = self.tick;
        let walkers: Vec<((f32, f32, f32), (f32, f32))> = self.mobs.values().filter(|m| m.species == Species::Villager && m.dead.is_none()).map(|m| (m.pos, (m.vel.0, m.vel.2))).collect();
        for (pos, vel) in &walkers {
            let speed = (vel.0 * vel.0 + vel.1 * vel.1).sqrt();
            let ahead = if speed > 0.01 { (pos.0 + vel.0 / speed * 0.8, pos.2 + vel.1 / speed * 0.8) } else { (pos.0, pos.2) };
            for (x, z) in [(pos.0, pos.2), ahead] {
                // (debout sur un chemin de terre ou une dalle, ses pieds sont dans la case
                // du dessous : la porte est alors une case plus haut)
                let base = (x.floor() as i32, pos.1.floor() as i32, z.floor() as i32);
                let cell = if self.wood_door(base.0, base.1, base.2) { base } else { (base.0, base.1 + 1, base.2) };
                if self.wood_door(cell.0, cell.1, cell.2) {
                    self.set_door(cell, true);
                    match self.open_doors.iter_mut().find(|(p, _)| *p == cell) {
                        Some(d) => d.1 = tick + 20,
                        None => self.open_doors.push((cell, tick + 20)),
                    }
                }
            }
        }
        let due: Vec<(i32, i32, i32)> = self.open_doors.iter().filter(|(_, t)| *t <= tick).map(|(p, _)| *p).collect();
        self.open_doors.retain(|(_, t)| *t > tick);
        for p in due {
            self.set_door(p, false);
        }
    }

    fn mob_water(&self, x: f32, y: f32, z: f32) -> bool {
        let s = self.mob_get(x.floor() as i32, y.floor() as i32, z.floor() as i32);
        let p = &self.world.generator().blocks;
        s == p.water || p.aquatic.contains(&s)
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
        // (fiche du jeu : 25 noyés sur 400 naissent armés d'un trident)
        let trident = species == Species::Drowned && self.mob_rng.below(400) < 25;
        let egg = 6000 + self.mob_rng.below(6000);
        // Montures : vie, vitesse, saut et force tirés au sort comme dans le jeu
        let r = |rng: &mut Rng, lo: f32, hi: f32| lo + rng.unit() as f32 * (hi - lo);
        let max_health = match species {
            s if s.mount() && !s.born_tame() => (15 + self.mob_rng.below(16)) as f32,
            Species::Wolf => WOLF_WILD_HEALTH,
            s => s.health(),
        };
        let (move_speed, jump, strength) = match species {
            Species::Horse => (r(&mut self.mob_rng, 0.1125, 0.3375), r(&mut self.mob_rng, 0.4, 1.0), 0),
            Species::Donkey | Species::Mule => (0.175, 0.5, 0),
            Species::Llama => (0.25, 0.0, 1 + self.mob_rng.below(5) as u8),
            // (le saut que le jeu prête aux chevaux squelettes et zombies, et au dromadaire)
            Species::SkeletonHorse | Species::ZombieHorse => (0.0, 0.7, 0),
            Species::Camel => (0.0, 0.42, 0),
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
            // (le têtard naît petit : il deviendra grenouille au bout de vingt minutes)
            age: if species == Species::Tadpole { -GROW_TICKS } else { 0 },
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
            custom: None,
            custom_speed: 0.0,
            custom_scale: 1.0,
            custom_box: None,
            ident: None,
            custom_hostile: None,
            custom_ranged: trident.then(|| ("minecraft:thrown_trident".to_string(), 8.0, 40, 10.0)),
            custom_explode: None,
            fuse: 0,
            burst: 0,
            fire_until: 0,
            looting: 0,
            trident,
            air: species.air_supply(),
            soak: 0,
            dry: 0,
            barter: 0,
            tilt: 0.0,
            sent_tilt: 0.0,
            act: WardenAct::None,
            act_ticks: 0,
            suspects: Vec::new(),
            focus: None,
            investigate: None,
            beat_at: 0,
            sniff_at: 0,
            best_far: f32::MAX,
            stuck: 0,
            aim: 0,
            facing: false,
            anchor_y: 0.0,
            dive: 0,
            custom_potion: None,
            natural: false,
            burning: false,
            label: String::new(),
            label_always: false,
            mark: 0,
            tamed: species.born_tame(),
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
            owner: None,
            owner_id: -1,
            sitting: false,
            following: false,
            attack: None,
            bite_in: 0,
            refresh: false,
            home: matches!(species, Species::Villager | Species::IronGolem).then_some(pos),
            late: 0,
            meet: None,
            errand: None,
            trade_xp: 0,
            trade_uses: Vec::new(),
            trade_restock: -1,
            asleep: None,
            station: None,
            job_wait: 0,
            path: Vec::new(),
            door: 0,
            path_wait: 0,
            path_ticks: 0,
        };
        self.mobs.insert(id, mob);
        self.mobs_dirty = true;
        id
    }

    /// Données d'entité d'une créature (taille, couleur, bébé, tondu…).
    fn mob_meta(m: &Mob, holder: i64) -> Vec<(u32, bedrock::Meta)> {
        let (w, h) = m.hitbox();
        let mut meta = bedrock::mob_metadata(w, h, m.variant, m.color, m.flags());
        let scale = m.shown_scale();
        // Nom d'une créature créée par un plugin (données 4 : le nom ; 81 : toujours visible)
        if !m.label.is_empty() {
            for (k, v) in [(4u32, bedrock::Meta::Text(m.label.clone())), (81, bedrock::Meta::Byte(m.label_always as u8))] {
                match meta.iter_mut().find(|(key, _)| *key == k) {
                    Some(cell) => cell.1 = v,
                    None => meta.push((k, v)),
                }
            }
        }
        for (k, v) in meta.iter_mut() {
            match *k {
                38 => *v = bedrock::Meta::Float(scale),
                // Motif de la robe du cheval
                43 => *v = bedrock::Meta::Int(m.mark),
                // Qui tient sa laisse
                37 => *v = bedrock::Meta::Long(holder),
                // Son maître (loup apprivoisé)
                bedrock::META_OWNER => *v = bedrock::Meta::Long(m.owner_id),
                92 => *v = bedrock::Meta::Long(m.flags2()),
                // Sa cible : c'est elle qui fait lever l'arc au squelette
                6 => *v = bedrock::Meta::Long(m.aim),
                _ => {}
            }
        }
        for (k, v) in Self::container_meta(m) {
            if let Some(cell) = meta.iter_mut().find(|(key, _)| *key == k) {
                cell.1 = v;
            }
        }
        // Villageois : son niveau (l'insigne à sa ceinture), le niveau le plus haut, son expérience
        if m.species == Species::Villager {
            for (k, v) in Self::trade_meta(m) {
                match meta.iter_mut().find(|(key, _)| *key == k) {
                    Some(cell) => cell.1 = v,
                    None => meta.push((k, v)),
                }
            }
        }
        meta
    }

    /// Inventaire d'une monture, tel que le jeu officiel l'annonce dans ses données d'entité :
    /// type d'écran (44, une fois apprivoisée), nombre de cases (45 : 2 pour le cheval, 16
    /// pour l'âne, la mule et le lama), cases de coffre par point de force (46 : 3 pour le
    /// lama), force (75) et force maximale (76). Laissé à zéro, le jeu plantait dès qu'on
    /// prenait un objet dans l'écran.
    fn container_meta(m: &Mob) -> [(u32, bedrock::Meta); 5] {
        let mount = m.species.mount();
        let llama = m.species == Species::Llama;
        let kind = if mount && m.tamed { bedrock::SCREEN_HORSE } else { 0 };
        let size = match m.species {
            _ if !mount => 0,
            Species::Horse => 2,
            _ => 16,
        };
        let strength = if llama { m.strength.max(1) as i32 } else { 0 };
        [
            (44, bedrock::Meta::Byte(kind)),
            (45, bedrock::Meta::Int(size)),
            (46, bedrock::Meta::Int(if llama { 3 } else { 0 })),
            (75, bedrock::Meta::Int(strength)),
            (76, bedrock::Meta::Int(if llama { 5 } else { 0 })),
        ]
    }

    /// Paquet d'apparition d'une créature.
    fn mob_packet(m: &Mob, rider: Option<i64>, holder: i64) -> Vec<u8> {
        bedrock::add_actor(m.id, m.ident.as_deref().unwrap_or(m.species.id()), m.pos, m.vel, m.yaw, (m.health.max(0.5), m.max_health), m.species.mount().then_some((m.move_speed, m.jump)), rider, &Self::mob_meta(m, holder))
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
    pub(super) fn mob_changed(&mut self, id: i64) {
        let Some(m) = self.mobs.get(&id) else { return };
        let scale = m.shown_scale();
        let (w, h) = m.hitbox();
        let holder = self.holder_id(m);
        let packet = bedrock::set_actor_data(
            id,
            &[
                (0u32, bedrock::Meta::Long(m.flags())),
                (2, bedrock::Meta::Int(m.variant)),
                (3, bedrock::Meta::Byte(m.color)),
                (bedrock::META_LEAD_HOLDER, bedrock::Meta::Long(holder)),
                (bedrock::META_OWNER, bedrock::Meta::Long(m.owner_id)),
                (38, bedrock::Meta::Float(scale)),
                (53, bedrock::Meta::Float(w)),
                (54, bedrock::Meta::Float(h)),
                (92, bedrock::Meta::Long(m.flags2())),
                (6, bedrock::Meta::Long(m.aim)),
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
        // Ce que les monstres tiennent en main (arc, épée, hache)
        let held_ids: HashMap<&'static str, i16> = Species::ALL.into_iter().filter_map(|s| s.held()).chain(std::iter::once("minecraft:trident")).filter_map(|n| self.item_id(n).map(|id| (n, id))).collect();
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
                        if let Some(held) = (if m.trident { Some("minecraft:trident") } else { m.species.held() }).and_then(|n| held_ids.get(n)) {
                            out.push((*addr, bedrock::ID_MOB_EQUIPMENT, bedrock::mob_equipment(m.id, (*held, 1, 0), 0)));
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
    /// Un villageois dort-il dans ce bloc de lit ?
    pub(super) fn mob_sleeps_in(&self, (x, y, z): (i32, i32, i32)) -> bool {
        let centre = (x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
        self.mobs.values().filter_map(|m| m.asleep).any(|p| (p.0 - centre.0).abs() < 1.2 && (p.1 - centre.1).abs() < 1.2 && (p.2 - centre.2).abs() < 1.2)
    }

    pub(super) fn remove_mob(&mut self, id: i64) {
        self.mobs.remove(&id);
        self.mobs_dirty = true;
        let viewers: Vec<SocketAddr> = self.clients.iter_mut().filter_map(|(a, c)| c.mobs_seen.remove(&id).then_some(*a)).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id));
        }
    }

    /// Envoie un paquet aux joueurs qui voient cette créature.
    pub(super) fn send_to_viewers(&mut self, id: i64, packet_id: u32, packet: &[u8]) {
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
        self.villager_doors();
        let ids: Vec<i64> = self.mobs.keys().copied().collect();
        let crowd = self.crowd();
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
            // (son chunk pas en mémoire : elle attend, plutôt que de le faire générer sur place)
            if !near {
                continue;
            }
            // Villageois endormi : il ne bouge pas de son lit jusqu'au matin
            if let Some(bed) = m.asleep {
                if self.world_time().rem_euclid(24_000) < 12_000 {
                    // Le jour se lève : debout, au pied du lit
                    m.asleep = None;
                    m.pos = m.home.unwrap_or(m.pos);
                    m.vel = (0.0, 0.0, 0.0);
                    m.sent = (f32::MAX, 0.0, 0.0);
                    m.refresh = true;
                } else {
                    self.mob_age(&mut m);
                    if m.pos != bed {
                        m.pos = bed;
                        m.vel = (0.0, 0.0, 0.0);
                        let packet = bedrock::move_actor_absolute(id, bed, 0.0, m.yaw, m.yaw, true);
                        self.send_to_viewers(id, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
                    }
                    self.mobs.insert(id, m);
                    continue;
                }
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
                let moves = if m.species.stick().is_some() && m.saddled {
                    if Some(held.as_str()) == m.species.stick() {
                        m.yaw = rider_yaw.rem_euclid(360.0);
                        let (dx, dz) = (-m.yaw.to_radians().sin(), m.yaw.to_radians().cos());
                        let base = if m.species == Species::Strider { 0.1 } else { 0.115 };
                        let speed = if m.boost > 0 { base * 2.0 } else { base };
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
                self.lava_walk(&mut m);
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
            let before = m.pos;
            if let Some(speed) = m.species.flies().or(m.species.swims()) {
                // Créature volante : elle se déplace dans les trois dimensions, sans pesanteur
                self.fly_think(&mut m, speed, &players);
                self.fly_physics(&mut m);
            } else {
                let hunting = m.attack.is_some();
                self.mob_think(&mut m, &players, leash_to);
                if m.species == Species::Vindicator && hunting != m.attack.is_some() {
                    m.refresh = true;
                }
                // Les corps ne se traversent pas : ceux qui se chevauchent s'écartent (sauf deux
                // amoureux, qui doivent se rejoindre)
                if !m.sitting && m.love == 0 {
                    let (px, pz) = Self::push_from(&crowd, &m);
                    m.vel.0 += px;
                    m.vel.2 += pz;
                }
                self.mob_physics(&mut m);
                self.lava_walk(&mut m);
            }
            // Chèvre arrêtée net par un bloc en pleine charge : une corne peut tomber
            if let Some((dx, dz, _)) = m.ram {
                if (m.pos.0 - before.0).abs() + (m.pos.2 - before.2).abs() < 0.01 {
                    m.ram = None;
                    self.goat_impact(&mut m, dx, dz);
                }
            }
            // Poulpe : son corps s'incline dans le sens de sa nage (comme le « xBodyRot » de
            // Java : droit à l'arrêt ou en montée, couché quand il file à l'horizontale),
            // transmis au jeu comme l'inclinaison de l'entité
            if matches!(m.species, Species::Squid | Species::GlowSquid) {
                let (dx, dy, dz) = (m.pos.0 - before.0, m.pos.1 - before.1, m.pos.2 - before.2);
                let flat = (dx * dx + dz * dz).sqrt();
                // (négatif, comme le « xBodyRot » de Java : le haut du corps part en avant, les
                // tentacules traînent derrière)
                let want = if flat + dy.abs() < 0.001 { 0.0 } else { -(flat.atan2(dy)).to_degrees() };
                m.tilt += (want - m.tilt) * 0.1;
            }
            let moved = (m.pos.0 - m.sent.0).abs() + (m.pos.1 - m.sent.1).abs() + (m.pos.2 - m.sent.2).abs() > 0.01 || (m.yaw - m.sent_yaw).abs() > 3.0 || (m.tilt - m.sent_tilt).abs() > 3.0;
            if moved {
                m.sent = m.pos;
                m.sent_yaw = m.yaw;
                m.sent_tilt = m.tilt;
                let packet = bedrock::move_actor_absolute(m.id, m.pos, m.tilt, m.yaw, m.yaw, m.on_ground);
                self.send_to_viewers(m.id, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
                self.mobs_dirty = true;
            }
            // Monture quittée en plein saut, retombée : son état est renvoyé à tous (le jeu de
            // l'ancien cavalier la laissait dans la pose du saut)
            let landed = m.airborne && m.on_ground;
            if landed {
                m.airborne = false;
                m.fall_peak = None;
            }
            let refresh = std::mem::take(&mut m.refresh);
            let love = m.love;
            self.mobs.insert(id, m);
            if grew || landed || refresh {
                self.mob_changed(id);
            }
            if landed {
                if let Some(m) = self.mobs.get(&id) {
                    let packet = bedrock::move_actor_absolute(id, m.pos, 0.0, m.yaw, m.yaw, true);
                    self.send_to_viewers(id, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
                }
                self.send_to_viewers(id, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(id, (0.0, 0.0, 0.0)));
            }
            if love > 0 && love % 10 == 0 {
                self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_LOVE_PARTICLES, 0));
            }
            if had_love && love == 0 {
                self.mob_changed(id); // l'envie passe
            }
            self.mob_habits(id);
        }
        // Les creepers arrivés au bout de leur mèche
        for (id, at, power, cause) in std::mem::take(&mut self.booms) {
            if self.mobs.contains_key(&id) {
                self.remove_mob(id);
                self.explode(at, power, Some(id), cause);
            }
        }
        self.breed();
        if self.tick % 40 == 7 {
            self.villager_jobs();
        }
        if self.tick % 1200 == 611 {
            self.villager_births();
        }
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
        if self.mobs.get(&id).map(|m| matches!(m.species, Species::SnowGolem | Species::Pufferfish | Species::Axolotl | Species::Allay | Species::Hoglin | Species::Warden) && m.dead.is_none()).unwrap_or(false) {
            self.special_habits(id);
        }
        let Some(m) = self.mobs.get_mut(&id) else { return };
        // Têtard grandi : une grenouille, froide dans la neige, chaude sous les tropiques
        if m.species == Species::Tadpole && m.age == 0 {
            let (x, z) = (m.pos.0.floor() as i32, m.pos.2.floor() as i32);
            let variant = match self.world.generator().vanilla().map(|v| v.surface_biome_at(x, z)) {
                Some(B::SnowyPlains | B::IceSpikes | B::SnowyTaiga | B::SnowySlopes | B::Grove | B::FrozenPeaks | B::JaggedPeaks | B::FrozenRiver | B::FrozenOcean | B::DeepFrozenOcean | B::SnowyBeach) => 1,
                Some(B::Desert | B::Jungle | B::SparseJungle | B::BambooJungle | B::MangroveSwamp | B::Savanna | B::SavannaPlateau | B::WindsweptSavanna | B::Badlands | B::WoodedBadlands | B::ErodedBadlands | B::WarmOcean) => 2,
                _ => 0,
            };
            self.transform_mob(id, Species::Frog);
            if let Some(f) = self.mobs.get_mut(&id) {
                f.variant = variant;
                f.max_health = Species::Frog.health();
                f.health = f.max_health;
            }
            return;
        }
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
            // Tatou : une écaille tombe toutes les cinq à dix minutes (« spawn_entity » du jeu)
            Species::Armadillo if !m.baby() => {
                m.egg = m.egg.saturating_sub(1);
                if m.egg == 0 {
                    m.egg = 6000 + self.mob_rng.below(6000);
                    let pos = m.pos;
                    if let Some(scute) = self.item_id("armadillo_scute") {
                        self.drop_item((pos.0, pos.1 + 0.3, pos.2), (0.0, 0.1, 0.0), scute, 1, 10);
                    }
                }
            }
            // Renifleur : de temps en temps il creuse la terre, la mousse ou la boue sous lui et
            // en sort une graine de torchefleur ou une cosse de sarracénie (table du jeu)
            Species::Sniffer if !m.baby() && m.on_ground => {
                m.egg = m.egg.saturating_sub(1);
                if m.egg == 0 {
                    m.egg = 4800 + self.mob_rng.below(4800);
                    let pos = m.pos;
                    let below = self.mob_get(pos.0.floor() as i32, pos.1.floor() as i32 - 1, pos.2.floor() as i32);
                    let diggable = self.block_name_of(below).map(|n| matches!(n.trim_start_matches("minecraft:"), "dirt" | "coarse_dirt" | "grass_block" | "grass" | "podzol" | "dirt_with_roots" | "moss_block" | "pale_moss_block" | "mud" | "muddy_mangrove_roots")).unwrap_or(false);
                    let seed = if self.mob_rng.below(2) == 0 { "torchflower_seeds" } else { "pitcher_pod" };
                    if diggable {
                        if let Some(item) = self.item_id(seed) {
                            self.drop_item((pos.0, pos.1 + 0.3, pos.2), (0.0, 0.15, 0.0), item, 1, 10);
                        }
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
    fn mob_think(&mut self, m: &mut Mob, players: &[((f32, f32, f32), String)], leash_to: Option<(f32, f32, f32)>) {
        // Warden occupé (il sort de terre, rugit, charge son onde, s'enfouit) : immobile,
        // tourné vers sa cible (voir `warden_habits`)
        if m.species == Species::Warden && m.act != WardenAct::None {
            m.vel.0 = 0.0;
            m.vel.2 = 0.0;
            m.target = None;
            return;
        }
        // Piglin qui admire son lingot : immobile, puis il jette son troc devant lui
        if m.species == Species::Piglin && m.barter > 0 {
            m.barter -= 1;
            m.vel.0 = 0.0;
            m.vel.2 = 0.0;
            m.target = None;
            if m.barter == 0 {
                self.piglin_barter(m);
            }
            return;
        }
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
        // Loup : assis, à la poursuite d'une proie, ou sur les talons de son maître
        let wolf = if m.species == Species::Warden {
            self.warden_think(m)
        } else if let Some((damage, sight, name)) = m.hostile() {
            self.hostile_think(m, damage, sight, name)
        } else if m.species.pet() || !m.species.hunts().0.is_empty() {
            self.wolf_think(m)
        } else if m.species == Species::IronGolem {
            self.golem_think(m)
        } else {
            None
        };
        // Villageois : ce que l'heure lui dicte (sauf en pleine fuite)
        let villager = if m.species == Species::Villager && m.panic == 0 { self.villager_think(m) } else { None };
        let near_player = |p: &(f32, f32, f32), r: f32| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2) < r * r && (p.1 - m.pos.1).abs() < 4.0;
        // Lapin : fuit le joueur qui ne lui tend pas sa nourriture (à moins de 8 blocs)
        let scary = if m.species == Species::Rabbit || (m.species == Species::Cat && !m.tamed) {
            // (jamais un joueur en créatif ou spectateur, comme dans le jeu)
            let harmless: Vec<(f32, f32, f32)> = self.clients.values().filter(|c| c.joined && !c.mode.takes_damage()).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2)).collect();
            players.iter().filter(|(p, held)| near_player(p, 8.0) && !tempting.contains(&held.as_str()) && !harmless.contains(p)).map(|(p, _)| *p).min_by(|a, b| {
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
        } else if let Some((s, stop)) = wolf {
            speed = s;
            stop_at = stop;
        } else if m.panic > 0 {
            m.panic -= 1;
            speed = ground_speed(attr * panic);
            m.errand = None;
            m.path.clear();
            // En fuite : une nouvelle direction de temps en temps
            if m.target.is_none() || self.mob_rng.below(20) == 0 {
                let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
                m.target = Some((m.pos.0 + a.cos() * 8.0, m.pos.2 + a.sin() * 8.0));
            }
        } else if let Some((s, stop)) = villager {
            speed = s;
            stop_at = stop;
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
            let mut t = (m.pos.0 + a.cos() * d, m.pos.2 + a.sin() * d);
            // Villageois : une fois sur trois, il va passer la porte la plus proche (pour
            // sortir de chez lui, ou y rentrer)
            m.errand = None;
            if m.species == Species::Villager && self.mob_rng.below(3) == 0 {
                if let Some((_, beyond)) = self.door_errand(m) {
                    // (par un chemin calculé : droit vers la porte, il butait contre le mur
                    // d'à côté et y restait)
                    if let Some(mut path) = self.villager_path(m.pos, (beyond.0, None, beyond.1)) {
                        path.reverse();
                        if let Some(first) = path.pop() {
                            t = first;
                            m.path = path;
                            m.path_ticks = 0;
                        }
                    }
                }
            }
            // Villageois : il reste dans son village (retour vers chez lui au-delà de 24 blocs)
            if let Some(h) = m.home.filter(|h| (t.0 - h.0).powi(2) + (t.1 - h.2).powi(2) > 24.0 * 24.0) {
                let (dx, dz) = (h.0 - m.pos.0, h.2 - m.pos.2);
                let len = (dx * dx + dz * dz).sqrt().max(0.1);
                t = (m.pos.0 + dx / len * d.min(len), m.pos.2 + dz / len * d.min(len));
            }
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
                        // (porte atteinte : il continue de l'autre côté)
                        m.target = m.errand.take();
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

    /// Emploi du temps d'un villageois, comme dans le jeu : la journée il vaque dans le
    /// village, l'après-midi (9000 à 11000) les adultes se retrouvent au centre, la nuit
    /// (12000 à 24000) chacun rentre chez lui et n'en bouge plus. Rend (vitesse, distance
    /// d'arrêt) quand l'heure lui dicte où aller.
    fn villager_think(&mut self, m: &mut Mob) -> Option<(f32, f32)> {
        // Ses « hmm » : de temps en temps le jour (toutes les seize secondes en moyenne)
        if self.mob_rng.below(320) == 0 && self.world_time().rem_euclid(24_000) < 12_500 && !self.refused_extras.contains(&bedrock::ID_LEVEL_SOUND_EVENT) {
            let packet = bedrock::level_sound(bedrock::SOUND_AMBIENT, m.pos, m.species.id(), m.baby(), m.id);
            let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.mobs_seen.contains(&m.id)).map(|(a, _)| *a).collect();
            for v in viewers {
                self.send(&v, bedrock::ID_LEVEL_SOUND_EVENT, &packet);
            }
        }
        // En plein échange : il ne bouge pas
        if self.clients.values().any(|c| c.inv.trade_mob == Some(m.id)) {
            m.target = None;
            return Some((0.0, f32::MAX));
        }
        let home = m.home?;
        let time = self.world_time().rem_euclid(24_000);
        let attr = m.movement();
        let flat2 = |p: (f32, f32)| (p.0 - m.pos.0).powi(2) + (p.1 - m.pos.2).powi(2);
        if time >= 12_000 {
            m.meet = None;
            // La nuit : chez lui, près de son lit
            if flat2((home.0, home.2)) < 1.5 * 1.5 && (home.1 - m.pos.1).abs() < 2.0 {
                m.target = None;
                m.errand = None;
                m.late = 0;
                // Un lit libre tout près : il s'y couche
                if time >= 12_500 && self.mob_rng.below(20) == 0 {
                    if let Some((bed, yaw)) = self.free_bed(home, m.id) {
                        m.asleep = Some(bed);
                        m.pos = bed;
                        m.yaw = yaw;
                        m.vel = (0.0, 0.0, 0.0);
                        m.sent = (f32::MAX, 0.0, 0.0);
                        m.refresh = true;
                    }
                }
                return Some((0.0, f32::MAX));
            }
            m.late += 1;
            if m.late > 1200 {
                // Une minute sans y arriver (il marche droit, sans chercher son chemin) : le
                // voilà rentré
                m.pos = home;
                m.vel = (0.0, 0.0, 0.0);
                m.sent = (f32::MAX, 0.0, 0.0);
                m.target = None;
                m.errand = None;
                m.late = 0;
                return Some((0.0, f32::MAX));
            }
            // Jusqu'à son lit, par un vrai chemin (portes, marches, détours)
            self.villager_go(m, (home.0, Some(home.1), home.2));
            return Some((ground_speed(attr * 0.6), PATH_STOP));
        }
        m.late = 0;
        if (9_000..11_000).contains(&time) && !m.baby() && m.variant != 14 {
            // L'après-midi : rendez-vous au centre du village (le milieu des maisons voisines)
            let center = match m.meet {
                Some(c) => c,
                None => {
                    let homes: Vec<(f32, f32)> = self.mobs.values().filter_map(|v| v.home).filter(|h| (h.0 - home.0).powi(2) + (h.2 - home.2).powi(2) < 64.0 * 64.0).map(|h| (h.0, h.2)).chain([(home.0, home.2)]).collect();
                    let n = homes.len() as f32;
                    let c = (homes.iter().map(|h| h.0).sum::<f32>() / n, homes.iter().map(|h| h.1).sum::<f32>() / n);
                    m.meet = Some(c);
                    c
                }
            };
            if flat2(center) > 5.0 * 5.0 {
                self.villager_go(m, (center.0, None, center.1));
                return Some((ground_speed(attr * 0.6), PATH_STOP));
            }
            return None;
        }
        m.meet = None;
        if (11_000..12_000).contains(&time) || time < 2_000 {
            // (plus de rendez-vous ni de travail : un reste de chemin ne le mène nulle part)
            m.path.clear();
        }
        // Le matin et en début d'après-midi : de temps en temps, un tour à son poste de travail
        if let Some(st) = m.station.filter(|_| (2_000..9_000).contains(&time) && !m.baby()) {
            let at = (st.0 as f32 + 0.5, st.2 as f32 + 0.5);
            if flat2(at) > 2.0 * 2.0 {
                // (en route tant qu'il lui reste du chemin ; sinon il se décide de temps en temps)
                if !m.path.is_empty() || (m.target.is_none() && self.mob_rng.below(120) == 0) {
                    self.villager_go(m, (at.0, Some(st.1 as f32), at.1));
                }
                if !m.path.is_empty() {
                    return Some((ground_speed(attr * 0.6), PATH_STOP));
                }
            }
        }
        // Un chemin de promenade en cours (sortir de chez lui par la porte, y rentrer)
        if !m.path.is_empty() {
            if m.target.is_none() {
                m.errand = None;
                m.path_ticks = 0;
                m.target = m.path.pop();
            } else {
                m.path_ticks += 1;
                if m.path_ticks > 60 {
                    m.path.clear();
                    m.target = None;
                    m.path_ticks = 0;
                }
            }
            return Some((ground_speed(attr * 0.6), PATH_STOP));
        }
        None
    }

    /// Mène un villageois vers une destination par un chemin calculé : il vise le point
    /// suivant du chemin, recalcule s'il reste bloqué trois secondes sur le même, et à défaut
    /// de chemin marche droit devant lui comme avant.
    fn villager_go(&mut self, m: &mut Mob, dest: (f32, Option<f32>, f32)) {
        let tick = self.tick;
        if m.target.is_some() {
            m.path_ticks += 1;
            if m.path_ticks > 60 {
                // Bloqué : on oublie ce chemin, un autre sera cherché d'ici une seconde
                m.target = None;
                m.path.clear();
                m.path_ticks = 0;
                m.path_wait = tick + 20;
            }
            return;
        }
        m.errand = None;
        m.path_ticks = 0;
        if let Some(next) = m.path.pop() {
            m.target = Some(next);
            return;
        }
        if tick < m.path_wait {
            return;
        }
        match self.villager_path(m.pos, dest) {
            Some(mut path) => {
                path.reverse();
                m.target = path.pop();
                m.path = path;
                m.path_wait = tick + 20;
            }
            None => {
                // Pas de chemin trouvé : tout droit, et une nouvelle recherche dans cinq secondes
                m.target = Some((dest.0, dest.2));
                m.path_wait = tick + 100;
            }
        }
    }

    /// Une créature haute de `tall` blocs peut-elle se tenir dans cette case (les pieds en
    /// `y`) ? Il faut un sol ferme dessous (ni eau, ni barrière ou trappe) et `tall` cases
    /// libres ; une porte en bois compte comme libre.
    fn stands(&self, x: i32, y: i32, z: i32, tall: i32) -> bool {
        (0..tall).all(|h| self.mob_pass(true, x, y + h, z)) && !self.mob_pass(true, x, y - 1, z) && !self.no_step(x, y - 1, z)
    }

    /// Chemin à pied (recherche A*) de `from` jusqu'à un bloc de `dest` : pas de côté d'un
    /// bloc, marches d'un bloc à monter, jusqu'à trois à descendre, portes en bois franchies.
    /// Points de passage au centre des cases, du premier au dernier. None : pas de chemin
    /// dans les limites (40 blocs, 1 500 cases explorées) ou destination inaccessible.
    fn villager_path(&self, from: (f32, f32, f32), dest: (f32, Option<f32>, f32)) -> Option<Vec<(f32, f32)>> {
        self.walk_path(from, dest, 2)
    }

    /// Le même chemin pour une créature haute de `tall` blocs (le warden : 3).
    fn walk_path(&self, from: (f32, f32, f32), dest: (f32, Option<f32>, f32), tall: i32) -> Option<Vec<(f32, f32)>> {
        use std::cmp::Reverse;
        use std::collections::{BinaryHeap, HashMap};
        let (sx, sz) = (from.0.floor() as i32, from.2.floor() as i32);
        // (debout sur une dalle ou un chemin de terre : la case du dessus)
        let sy = [from.1.floor() as i32, from.1.floor() as i32 + 1, from.1.floor() as i32 - 1].into_iter().find(|y| self.stands(sx, *y, sz, tall))?;
        let (gx, gz) = (dest.0.floor() as i32, dest.2.floor() as i32);
        let gy = dest.1.map(|y| y.floor() as i32);
        let arrived = |x: i32, y: i32, z: i32| (x - gx).abs() <= 1 && (z - gz).abs() <= 1 && gy.map(|g| (y - g).abs() <= 1).unwrap_or(true);
        let guess = |x: i32, y: i32, z: i32| (x - gx).abs() + (z - gz).abs() + gy.map(|g| (y - g).abs()).unwrap_or(0);
        let start = (sx, sy, sz);
        let mut open = BinaryHeap::new();
        let mut cost: HashMap<(i32, i32, i32), i32> = HashMap::new();
        let mut came: HashMap<(i32, i32, i32), (i32, i32, i32)> = HashMap::new();
        open.push(Reverse((guess(sx, sy, sz), 0, start)));
        cost.insert(start, 0);
        let mut explored = 0;
        while let Some(Reverse((_, g, at))) = open.pop() {
            if cost.get(&at).map(|c| g > *c).unwrap_or(false) {
                continue;
            }
            if arrived(at.0, at.1, at.2) {
                let mut cells = vec![at];
                let mut cur = at;
                while let Some(prev) = came.get(&cur) {
                    cells.push(*prev);
                    cur = *prev;
                }
                cells.reverse();
                // (la case de départ n'est pas un point à viser)
                return Some(cells.into_iter().skip(1).map(|c| (c.0 as f32 + 0.5, c.2 as f32 + 0.5)).collect());
            }
            explored += 1;
            if explored > 1500 {
                return None;
            }
            let (x, y, z) = at;
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if (nx - sx).abs() > 40 || (nz - sz).abs() > 40 {
                    continue;
                }
                // De plain-pied, une marche à monter (la tête doit passer), ou une descente
                let step = [0, 1, -1, -2, -3].into_iter().find(|dy| {
                    let ny = y + dy;
                    self.stands(nx, ny, nz, tall)
                        && match *dy {
                            1 => self.mob_pass(true, x, y + tall, z),
                            d if d < 0 => (ny + tall..=y + tall - 1).all(|h| self.mob_pass(true, nx, h, nz)),
                            _ => true,
                        }
                });
                let Some(dy) = step else { continue };
                let next = (nx, y + dy, nz);
                // (une porte coûte un peu plus : il préfère un passage ouvert)
                let g2 = g + 1 + dy.abs() + self.wood_door(nx, y + dy, nz) as i32;
                if cost.get(&next).map(|c| g2 < *c).unwrap_or(true) {
                    cost.insert(next, g2);
                    came.insert(next, at);
                    open.push(Reverse((g2 + guess(next.0, next.1, next.2), g2, next)));
                }
            }
        }
        None
    }

    /// Lit libre à moins de trois blocs de `home` : où s'y coucher (au milieu de l'oreiller,
    /// sur le matelas) et dans quel sens. Un lit où dort déjà un autre villageois est pris.
    fn free_bed(&self, home: (f32, f32, f32), me: i64) -> Option<((f32, f32, f32), f32)> {
        let d = self.data.as_ref()?;
        let (hx, hy, hz) = (home.0.floor() as i32, home.1.floor() as i32, home.2.floor() as i32);
        let taken: Vec<(f32, f32, f32)> = self.mobs.values().filter(|v| v.id != me).filter_map(|v| v.asleep).collect();
        let mut best: Option<(((f32, f32, f32), f32), i32)> = None;
        for dx in -3..=3 {
            for dz in -3..=3 {
                for dy in -1..=1 {
                    let (x, y, z) = (hx + dx, hy + dy, hz + dz);
                    let s = self.world.get(x, y, z);
                    if d.block_name(s) != Some("minecraft:bed") || d.prop(s, "head_piece_bit") != Some(Prop::Int(1)) {
                        continue;
                    }
                    // Sens du lit dans la numérotation du jeu (sud, ouest, nord, est) : du pied
                    // vers l'oreiller. Le jeu couche le dormeur sur le dos en le faisant pivoter
                    // par le milieu du corps, la tête à l'opposé de son regard : il regarde
                    // donc vers le pied du lit, posé un peu avant le milieu du lit (réglage
                    // fait sur captures : la tête sur l'oreiller, les pieds au bout).
                    let n = match d.prop(s, "direction") {
                        Some(Prop::Int(n)) => n.rem_euclid(4),
                        _ => 0,
                    };
                    let (hx2, hz2) = [(0.0, 1.0), (-1.0, 0.0), (0.0, -1.0), (1.0, 0.0)][n as usize];
                    let at = (x as f32 + 0.5 - hx2 * 0.7, y as f32 + 0.5625, z as f32 + 0.5 - hz2 * 0.7);
                    if taken.iter().any(|t| (t.0 - at.0).abs() < 0.6 && (t.2 - at.2).abs() < 0.6 && (t.1 - at.1).abs() < 1.0) {
                        continue;
                    }
                    let yaw = ((n * 90 + 180) % 360) as f32;
                    let dist = dx * dx + dz * dz + dy * dy;
                    if best.map(|b| dist < b.1).unwrap_or(true) {
                        best = Some(((at, yaw), dist));
                    }
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Métiers et postes de travail. Un villageois sans métier prend le poste libre le plus
    /// proche de chez lui (16 blocs) et le métier qui va avec ; celui dont le poste a disparu
    /// perd son métier, sauf s'il a déjà échangé. Peu de recherches à la fois : elles coûtent.
    fn villager_jobs(&mut self) {
        let Some(d) = self.data.as_ref() else { return };
        let tick = self.tick;
        let mut claimed: HashSet<(i32, i32, i32)> = self.mobs.values().filter_map(|m| m.station).collect();
        let ids: Vec<i64> = self.mobs.values().filter(|m| m.species == Species::Villager && !m.baby() && m.variant != 14 && m.dead.is_none() && m.home.is_some()).map(|m| m.id).collect();
        let mut scans = 0;
        let mut changes: Vec<(i64, Option<(i32, i32, i32)>, i32, u64)> = Vec::new();
        for id in ids {
            let Some(m) = self.mobs.get(&id) else { continue };
            let Some(home) = m.home else { continue };
            if let Some(st) = m.station {
                if !self.world.is_ready(st.0 >> 4, st.2 >> 4) {
                    continue;
                }
                let still = d.block_name(self.world.get(st.0, st.1, st.2)).and_then(station_job) == Some(m.variant);
                if !still {
                    // Poste cassé : plus de métier, à moins d'avoir déjà une clientèle
                    changes.push((id, None, if m.trade_xp == 0 { 0 } else { m.variant }, tick + 100));
                }
                continue;
            }
            if m.job_wait > tick || scans >= 2 {
                continue;
            }
            scans += 1;
            let (hx, hy, hz) = (home.0.floor() as i32, home.1.floor() as i32, home.2.floor() as i32);
            let mut best: Option<((i32, i32, i32), i32, i32)> = None;
            for dx in -16..=16 {
                for dz in -16..=16 {
                    let (x, z) = (hx + dx, hz + dz);
                    if !self.world.is_ready(x >> 4, z >> 4) {
                        continue;
                    }
                    for dy in -4..=4 {
                        let y = hy + dy;
                        let Some(job) = d.block_name(self.world.get(x, y, z)).and_then(station_job) else { continue };
                        // (un villageois qui a déjà un métier ne prend qu'un poste de ce métier)
                        if (m.variant != 0 && job != m.variant) || claimed.contains(&(x, y, z)) {
                            continue;
                        }
                        let dist = dx * dx + dz * dz + dy * dy * 4;
                        if best.map(|b| dist < b.2).unwrap_or(true) {
                            best = Some(((x, y, z), job, dist));
                        }
                    }
                }
            }
            match best {
                Some((at, job, _)) => {
                    claimed.insert(at);
                    changes.push((id, Some(at), job, tick));
                }
                None => changes.push((id, None, m.variant, tick + 600)),
            }
        }
        for (id, station, job, wait) in changes {
            let mut changed = false;
            if let Some(m) = self.mobs.get_mut(&id) {
                changed = m.variant != job;
                m.station = station;
                m.variant = job;
                m.job_wait = wait;
            }
            if changed {
                self.mob_changed(id);
            }
            self.mobs_dirty = true;
        }
    }

    /// Naissances : deux villageois adultes côte à côte, de jour, font un petit s'il reste
    /// un lit libre dans le village. Une naissance au plus par passage (une fois par minute).
    fn villager_births(&mut self) {
        if self.world_time().rem_euclid(24_000) >= 12_000 || self.mobs.len() >= MAX_MOBS {
            return;
        }
        let adults: Vec<(i64, (f32, f32, f32), (f32, f32, f32), i32)> = self
            .mobs
            .values()
            .filter(|m| m.species == Species::Villager && m.age == 0 && m.dead.is_none() && m.asleep.is_none())
            .filter_map(|m| m.home.map(|h| (m.id, m.pos, h, m.mark)))
            .collect();
        for (i, a) in adults.iter().enumerate() {
            let Some(b) = adults[i + 1..].iter().find(|b| (a.1 .0 - b.1 .0).powi(2) + (a.1 .2 - b.1 .2).powi(2) < 4.0 * 4.0 && (a.1 .1 - b.1 .1).abs() < 2.0) else { continue };
            // Lits du village (deux blocs par lit) contre habitants des environs
            let (cx, cz) = ((a.2 .0.floor() as i32) >> 4, (a.2 .2.floor() as i32) >> 4);
            let mut beds = 0;
            for x in cx - 3..=cx + 3 {
                for z in cz - 3..=cz + 3 {
                    beds += self.world.structure_beds(x, z).len();
                }
            }
            let people = self.mobs.values().filter(|m| m.species == Species::Villager && m.dead.is_none()).filter_map(|m| m.home).filter(|h| (h.0 - a.2 .0).powi(2) + (h.2 - a.2 .2).powi(2) < 56.0 * 56.0).count();
            if people >= beds / 2 {
                continue;
            }
            let at = ((a.1 .0 + b.1 .0) / 2.0, a.1 .1.max(b.1 .1), (a.1 .2 + b.1 .2) / 2.0);
            let baby = self.add_mob(Species::Villager, at);
            if let Some(m) = self.mobs.get_mut(&baby) {
                m.variant = 0;
                m.mark = a.3;
                m.age = -GROW_TICKS;
                m.home = Some(a.2);
            }
            for parent in [a.0, b.0] {
                if let Some(m) = self.mobs.get_mut(&parent) {
                    m.age = BREED_COOLDOWN;
                }
                self.send_to_viewers(parent, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(parent, bedrock::EVENT_LOVE_PARTICLES, 0));
            }
            self.mobs_dirty = true;
            return;
        }
    }

    /// Les monstres et la nuit, une fois par seconde :
    ///
    /// - la nuit, quelques monstres apparaissent à la surface autour des joueurs en survie, à
    ///   bonne distance (de 24 à 48 blocs), jusqu'à `MONSTERS_PER_PLAYER` chacun ;
    /// - ceux qui sont apparus ainsi disparaissent loin de tout joueur (toujours au-delà de
    ///   128 blocs, peu à peu au-delà de 48) ;
    /// - le jour, un zombie à ciel ouvert prend feu et perd un point de vie par seconde.
    pub(super) fn monsters_tick(&mut self) {
        let day_time = self.world_time().rem_euclid(24000);
        let night = (13000..23000).contains(&day_time);
        let players: Vec<(f32, f32, f32, bool)> =
            self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame && c.dead.is_none()).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2, c.mode.takes_damage())).collect();
        let nearest2 = |p: (f32, f32, f32)| players.iter().map(|q| (q.0 - p.0).powi(2) + (q.2 - p.2).powi(2)).fold(f32::MAX, f32::min);
        // Disparition, et feu au soleil
        let mut gone = Vec::new();
        let mut scorched = Vec::new();
        let mut changed = Vec::new();
        let ids: Vec<i64> = self.mobs.iter().filter(|(_, m)| m.dead.is_none() && (m.natural || m.species.burns())).map(|(id, _)| *id).collect();
        for id in ids {
            let Some(m) = self.mobs.get(&id) else { continue };
            let (pos, natural, burns, was, aflame) = (m.pos, m.natural, m.species.burns() && !m.baby(), m.burning, m.fire_until > self.tick);
            if natural && m.leash.is_none() && m.label.is_empty() {
                let d2 = nearest2(pos);
                if d2 > 128.0 * 128.0 || (d2 > 48.0 * 48.0 && self.mob_rng.below(40) == 0) {
                    gone.push(id);
                    continue;
                }
            }
            let (bx, by, bz) = (pos.0.floor() as i32, pos.1.floor() as i32, pos.2.floor() as i32);
            let sunlit = burns && self.cfg.vanilla.daylight_burn && !night && !self.weather.sky.rains() && !self.mob_water(pos.0, pos.1, pos.2) && (by + 2..(by + 66).min(crate::world::MAX_Y)).all(|y| self.mob_free(bx, y, bz) && self.mob_get(bx, y, bz) != self.world.generator().blocks.water);
            if sunlit {
                scorched.push((id, pos));
            }
            if (sunlit || aflame) != was {
                changed.push((id, sunlit || aflame));
            }
        }
        for id in gone {
            self.remove_mob(id);
        }
        for (id, burning) in changed {
            if let Some(m) = self.mobs.get_mut(&id) {
                m.burning = burning;
            }
            self.mob_changed(id);
        }
        for (id, pos) in scorched {
            self.hit_mob(id, pos, 1.0, Attacker::Mob(id));
        }
        // Phantoms : la nuit, ils viennent chercher les joueurs restés trois jours sans dormir
        // (les phantoms : à la surface seulement)
        let surface = !self.other_dimension();
        if night && self.cfg.spawn_monsters && surface {
            let now = self.world_time();
            let sleepless: Vec<(f32, f32, f32)> =
                self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame && c.dead.is_none() && c.mode.takes_damage() && now - c.rested_at >= 72_000).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2)).collect();
            for (px, py, pz) in sleepless {
                // (une fois par minute environ, à ciel ouvert, quatre phantoms au plus)
                if self.mob_rng.below(60) != 0 || self.mobs.len() >= MAX_MOBS {
                    continue;
                }
                let around = self.mobs.values().filter(|m| m.species == Species::Phantom && m.dead.is_none() && (m.pos.0 - px).powi(2) + (m.pos.2 - pz).powi(2) < 64.0 * 64.0).count();
                let (bx, by, bz) = (px.floor() as i32, py.floor() as i32, pz.floor() as i32);
                if around >= 4 || self.light_at(bx, by + 1, bz).sky < 15 {
                    continue;
                }
                for _ in 0..1 + self.mob_rng.below(3) {
                    let spot = (px + self.mob_rng.below(21) as f32 - 10.0, py + 20.0 + self.mob_rng.below(15) as f32, pz + self.mob_rng.below(21) as f32 - 10.0);
                    if spot.1 >= crate::world::MAX_Y as f32 - 2.0 || !self.mob_free(spot.0.floor() as i32, spot.1.floor() as i32, spot.2.floor() as i32) {
                        continue;
                    }
                    let id = self.add_mob(Species::Phantom, spot);
                    if let Some(m) = self.mobs.get_mut(&id) {
                        m.natural = true;
                    }
                    if self.plugins.listens("CreatureSpawnEvent") {
                        let out = self.fire("CreatureSpawnEvent", vec![("entity", self.mob_json(id))]);
                        if super::cancelled(&out) {
                            self.remove_mob(id);
                        }
                    }
                }
            }
        }
        // Poissons, calamars, dauphins, axolotls et chauves-souris autour des joueurs
        // (dans les mondes au relief du jeu seulement : un monde plat n'a ni océan ni grotte)
        if self.cfg.spawn_animals && surface && self.world.generator().vanilla().is_some() {
            self.ambient_spawn();
        }
        // Apparition : là où il fait noir, autour des joueurs en survie ou en aventure.
        // La nuit, c'est toute la surface sans torche ; le jour, les grottes.
        if !self.cfg.spawn_monsters {
            return;
        }
        // (au Nether et dans l'End : leurs propres créatures, quelle que soit la lumière)
        let elsewhere = self.other_dimension();
        // Avec beaucoup de joueurs, le calcul de lumière ne doit pas peser sur un seul tick :
        // quarante calculs au plus par seconde, et les joueurs servis à tour de rôle
        let mut light_left = 40u32;
        let turn = if players.is_empty() { 0 } else { (self.tick / 20) as usize % players.len() };
        for (px, py, pz, fair) in players.iter().cycle().skip(turn).take(players.len()).copied() {
            if light_left == 0 {
                break;
            }
            if !fair || self.mobs.len() >= MAX_MOBS {
                continue;
            }
            let around = self.mobs.values().filter(|m| m.natural && !m.species.ambient() && m.dead.is_none() && (m.pos.0 - px).powi(2) + (m.pos.2 - pz).powi(2) < 96.0 * 96.0).count();
            if around >= MONSTERS_PER_PLAYER {
                continue;
            }
            // Douze essais par seconde et par joueur, un seul monstre au plus : en surface
            // d'abord (la nuit), puis à des hauteurs prises au hasard, plutôt vers le bas (les
            // grottes). La plupart tombent dans la roche et ne coûtent qu'une lecture de bloc.
            let mut spot = None;
            for attempt in 0..12 {
                let angle = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
                let far = 24.0 + self.mob_rng.unit() as f32 * 24.0;
                let (x, z) = ((px + angle.cos() * far).floor() as i32, (pz + angle.sin() * far).floor() as i32);
                // (près d'une forteresse du Nether : un essai sur ses ponts, où vivent blazes et
                // wither squelettes)
                let bridge = if elsewhere && attempt == 0 { self.world.generator().fortress_near(px.floor() as i32, pz.floor() as i32) } else { None };
                if let Some((fx, fz)) = bridge {
                    use crate::world::generator::{FORTRESS_ARM, FORTRESS_Y};
                    let along = self.mob_rng.below(2 * FORTRESS_ARM as u32 + 1) as i32 - FORTRESS_ARM;
                    let (bx, bz) = if self.mob_rng.below(2) == 0 { (fx + along, fz) } else { (fx, fz + along) };
                    let d2 = nearest2((bx as f32, FORTRESS_Y as f32, bz as f32));
                    if d2 >= 20.0 * 20.0 && d2 <= 72.0 * 72.0 && self.mob_free(bx, FORTRESS_Y + 1, bz) && self.mob_free(bx, FORTRESS_Y + 2, bz) && !self.mob_free(bx, FORTRESS_Y, bz) {
                        spot = Some((bx, FORTRESS_Y + 1, bz));
                        break;
                    }
                }
                let found = if attempt == 0 && night && !elsewhere {
                    self.surface_y(x, z, py.floor() as i32).filter(|y| (*y as f32 - py).abs() < 24.0)
                } else {
                    let start = py.floor() as i32 - 32 + self.mob_rng.below(44) as i32;
                    self.floor_below(x, start, z)
                };
                let Some(y) = found else { continue };
                // (pas perché sur un arbre)
                if self.block_name_of(self.mob_get(x, y - 1, z)).map(|n| n.contains("leaves")).unwrap_or(false) {
                    continue;
                }
                // (pas au nez d'un autre joueur)
                if nearest2((x as f32, y as f32, z as f32)) < 20.0 * 20.0 {
                    continue;
                }
                // (pas à la lumière : une torche à portée, ou le jour qui entre, l'en empêche)
                light_left = light_left.saturating_sub(1);
                if light_left == 0 || (!elsewhere && !self.dark_for_monsters(x, y, z, night)) {
                    continue;
                }
                spot = Some((x, y, z));
                break;
            }
            let Some((x, y, z)) = spot else { continue };
            // Comme dans le jeu : surtout des zombies, puis squelettes, creepers et araignées
            let species = if elsewhere {
                match self.other_species(x, y, z) {
                    Some(s) => s,
                    None => continue,
                }
            } else {
                let picked = match self.mob_rng.below(100) {
                    0..=34 => Species::Zombie,
                    35..=37 => Species::ZombieVillager,
                    38..=62 => Species::Skeleton,
                    63..=80 => Species::Creeper,
                    81..=96 => Species::Spider,
                    97..=98 => Species::Enderman,
                    _ => Species::Witch,
                };
                // Selon le biome, comme dans le jeu : zombies momifiés au désert, vagabonds dans
                // le froid, squelettes des marais et slimes dans les marécages (à la surface)
                let open_sky = self.light_at(x, y, z).sky > 0;
                let biome = if open_sky { self.world.generator().vanilla().map(|v| v.surface_biome_at(x, z)) } else { None };
                let swap = self.mob_rng.below(5) != 0;
                match (picked, biome) {
                    (Species::Zombie, Some(B::Desert)) if swap => Species::Husk,
                    (Species::Skeleton, Some(B::SnowyPlains | B::IceSpikes | B::FrozenRiver | B::FrozenOcean | B::DeepFrozenOcean)) if swap => Species::Stray,
                    (Species::Skeleton, Some(B::Swamp | B::MangroveSwamp)) if swap => Species::Bogged,
                    (Species::Creeper, Some(B::Swamp)) if self.mob_rng.below(2) == 0 => Species::Slime,
                    (s, _) => s,
                }
            };
            let id = self.add_mob(species, (x as f32 + 0.5, y as f32, z as f32 + 0.5));
            // Slime et cube de magma : petit, moyen ou gros
            if species.cube() {
                let size = [1, 2, 4][self.mob_rng.below(3) as usize];
                if let Some(m) = self.mobs.get_mut(&id) {
                    m.variant = size;
                    m.max_health = (size * size) as f32;
                    m.health = m.max_health;
                }
            }
            // (un zombie sur vingt est un bébé, et le reste : il ne grandit pas)
            let baby = matches!(species, Species::Zombie | Species::ZombieVillager) && self.mob_rng.below(20) == 0;
            if let Some(m) = self.mobs.get_mut(&id) {
                m.natural = true;
                if baby {
                    m.age = -(1i64 << 40);
                    m.refresh = true;
                }
            }
            if self.plugins.listens("CreatureSpawnEvent") {
                let out = self.fire("CreatureSpawnEvent", vec![("entity", self.mob_json(id))]);
                if super::cancelled(&out) {
                    self.remove_mob(id);
                }
            }
        }
    }

    /// Monstre : il repère le joueur le plus proche (en survie ou en aventure), le poursuit
    /// et le frappe au contact, une fois par seconde. Sans joueur en vue, il se promène.
    fn hostile_think(&mut self, m: &mut Mob, damage: f32, sight: f32, name: &'static str) -> Option<(f32, f32)> {
        m.bite_in = m.bite_in.saturating_sub(1);
        // Un archer annonce sa cible au jeu, qui lui fait alors lever son arc
        if matches!(m.species.held(), Some("minecraft:bow") | Some("minecraft:crossbow")) {
            let aim = match m.attack {
                Some(Prey::Player(a)) => self.clients.get(&a).map(|c| c.id).unwrap_or(0),
                _ => 0,
            };
            if aim != m.aim || (aim == 0 && m.facing) {
                m.aim = aim;
                m.facing &= aim != 0;
                m.refresh = true;
            }
        }
        let attr = m.movement();
        let flat2 = |p: (f32, f32, f32)| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2);
        let fair = |c: &Client| c.joined && c.dead.is_none() && c.mode.takes_damage();
        let feet = |c: &Client| (c.pos.0, c.pos.1 - EYE, c.pos.2);
        // Sa cible est-elle toujours là ? (il la suit un peu au-delà de ce qu'il voit)
        let mut prey = match m.attack {
            Some(Prey::Player(a)) => self.clients.get(&a).filter(|c| fair(c)).map(|c| (a, c.id, feet(c))),
            _ => None,
        }
        .filter(|(_, _, p)| flat2(*p) < (sight * 1.5).powi(2));
        if m.attack.is_some() && prey.is_none() {
            m.attack = None;
            m.target = None;
        }
        // Slime : ses dégâts suivent sa taille (le plus petit ne blesse pas)
        let damage = match m.species {
            Species::Slime => [0.0, 0.0, 2.0, 3.0, 4.0][m.variant.clamp(1, 4) as usize],
            // (le cube de magma blesse même petit : 3, 4 et 6)
            Species::MagmaCube => [0.0, 3.0, 4.0, 4.0, 6.0][m.variant.clamp(1, 4) as usize],
            _ => damage,
        };
        // Sans cible : un coup d'œil alentour de temps en temps (toutes les demi-secondes
        // environ, sans que tous les monstres regardent au même instant). Une créature
        // neutre ne cherche personne : elle ne répond qu'aux coups.
        if prey.is_none() && !m.species.neutral() && self.mob_rng.below(8) == 0 {
            prey = self
                .clients
                .iter()
                .filter(|(_, c)| fair(c) && flat2(feet(c)) < sight * sight && (c.pos.1 - EYE - m.pos.1).abs() < 8.0)
                // (le piglin laisse en paix qui porte de l'or ; le petit ne s'en prend à personne)
                .filter(|(_, c)| m.species != Species::Piglin || (!m.baby() && !self.wears_gold(c)))
                .min_by(|a, b| flat2(feet(a.1)).total_cmp(&flat2(feet(b.1))))
                .map(|(a, c)| (*a, c.id, feet(c)));
            if let Some((a, _, _)) = prey {
                m.attack = Some(Prey::Player(a));
                m.panic = 0;
            }
        }
        let (a, pid, p) = prey?;
        m.target = Some((p.0, p.2));
        let (width, height) = m.size();
        let far2 = flat2(p);
        // Évocateur : il garde ses distances et, toutes les cinq secondes, fait jaillir du sol
        // une ligne de crocs vers sa proie
        if m.species == Species::Evoker {
            m.barter = m.barter.saturating_sub(1);
            if m.barter == 0 && far2 < 14.0 * 14.0 {
                m.barter = 100;
                self.cast_fangs(m, p);
            }
            return Some((ground_speed(m.movement() * 0.6), 8.0));
        }
        // Il ne lâche pas sa proie : si la ligne droite est barrée (mur, fossé, étage), il
        // cherche un chemin pour la contourner, refait une fois par seconde, et le suit de
        // case en case. (Chemin à pied : marches, descentes, portes en bois — qu'un zombie
        // défonce, voir `mob_physics`.)
        let chest = |q: (f32, f32, f32)| (q.0, q.1 + 0.6, q.2);
        let direct = far2 < 2.0 * 2.0 || ((p.1 - m.pos.1).abs() < 1.2 && self.clear_shot(chest(m.pos), chest(p)));
        let mut via = false;
        if direct || m.species.flies().is_some() {
            m.path.clear();
        } else {
            if self.tick >= m.path_wait {
                m.path = match self.villager_path(m.pos, (p.0, Some(p.1), p.2)) {
                    Some(mut path) => {
                        path.reverse();
                        path
                    }
                    None => Vec::new(),
                };
                m.path_wait = self.tick + 20;
            }
            while let Some(&(wx, wz)) = m.path.last() {
                if (wx - m.pos.0).powi(2) + (wz - m.pos.2).powi(2) < 0.35 * 0.35 {
                    m.path.pop();
                } else {
                    break;
                }
            }
            if let Some(&next) = m.path.last() {
                m.target = Some(next);
                via = true;
            }
        }
        // (en suivant un chemin, il va jusqu'au point de passage au lieu de s'arrêter à portée)
        let stop = |at: f32| if via { 0.15 } else { at };
        // Creeper : au contact il allume sa mèche, et explose une seconde et demie plus tard
        // si sa proie ne s'est pas éloignée
        if let Some(power) = m.explodes() {
            if m.fuse == 0 && far2 < 3.0 * 3.0 && (p.1 - m.pos.1).abs() < 3.0 {
                m.fuse = 1;
                m.refresh = true;
                self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("fuse", m.pos, "", false, -1));
            } else if m.fuse > 0 {
                if far2 > 7.0 * 7.0 {
                    m.fuse = 0;
                    m.refresh = true;
                } else {
                    m.fuse += 1;
                    if m.fuse >= 30 {
                        self.booms.push((m.id, (m.pos.0, m.pos.1 + height / 2.0, m.pos.2), power, name));
                    }
                }
            }
            if m.fuse > 0 {
                m.yaw = (-(p.0 - m.pos.0)).atan2(p.2 - m.pos.2).to_degrees().rem_euclid(360.0);
                return Some((0.0, 64.0));
            }
            return Some((ground_speed(attr * m.species.chase()), stop(1.0)));
        }
        // Tireur : à portée et la vue dégagée, il s'arrête et tire ; sinon il s'approche
        if let Some((kind, shot_damage, interval, range)) = m.ranged() {
            let eye = (m.pos.0, m.pos.1 + height * 0.85, m.pos.2);
            let aim = (p.0, p.1 + 1.2, p.2);
            let ready = far2 < range * range && self.clear_shot(eye, aim);
            if ready != m.facing && m.aim != 0 {
                m.facing = ready;
                m.refresh = true;
            }
            if ready {
                m.yaw = (-(p.0 - m.pos.0)).atan2(p.2 - m.pos.2).to_degrees().rem_euclid(360.0);
                if m.bite_in == 0 {
                    m.bite_in = (interval + self.mob_rng.below(20) as u32).min(60_000) as _;
                    self.mob_throws(m, &kind, aim, shot_damage, name);
                }
                return Some((ground_speed(attr), range * 0.6));
            }
            return Some((ground_speed(attr * m.species.chase()), stop(1.0)));
        }
        let reach = 1.2 + width / 2.0;
        // (jamais à travers un mur : il lui faut voir sa proie)
        let in_view = self.clear_shot((m.pos.0, m.pos.1 + height * 0.85, m.pos.2), (p.0, p.1 + 1.0, p.2)) || self.clear_shot((m.pos.0, m.pos.1 + height * 0.5, m.pos.2), (p.0, p.1 + 0.4, p.2));
        if damage > 0.0 && flat2(p) < reach * reach && (p.1 - m.pos.1).abs() < height.max(1.8) && m.bite_in == 0 && in_view {
            m.bite_in = 20;
            self.send_to_viewers(m.id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(m.id, bedrock::EVENT_ARM_SWING, 0));
            self.hurt(&a, damage, Cause::Mob(name));
            // Armure à épines : la créature se blesse en frappant (appliqué après sa réflexion)
            if let Some(back) = self.thorns_roll(&a) {
                self.thorned.push((m.id, back));
            }
            // Araignée venimeuse : sept secondes de poison
            if m.species == Species::CaveSpider {
                self.give_effect(&a, super::effects::POISON, 0, 140, true);
            }
            // Le coup repousse un peu sa victime
            if !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) {
                let d = flat2(p).sqrt().max(0.01);
                self.send(&a, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(pid, ((p.0 - m.pos.0) / d * 0.4, 0.36, (p.2 - m.pos.2) / d * 0.4)));
            }
        }
        Some((ground_speed(attr * m.species.chase()), stop(reach * 0.6)))
    }

    /// Une créature vivante répond-elle à ce test (pieds, largeur, hauteur) ?
    pub(super) fn mobs_in_block(&self, hits: &dyn Fn((f32, f32, f32), f32, f32) -> bool) -> bool {
        self.mobs.values().any(|m| {
            let (w, h) = m.size();
            m.dead.is_none() && hits(m.pos, w, h)
        })
    }

    /// Créature volante : où elle veut aller, et sur qui elle tire ou fond.
    ///
    /// - le **blaze** reste à hauteur d'homme, à quelques blocs de sa proie, et tire ;
    /// - le **ghast** se tient haut et loin, et crache des boules de feu qui explosent ;
    /// - le **phantom** tourne au-dessus de sa proie, fond sur elle, mord et remonte.
    fn fly_think(&mut self, m: &mut Mob, speed: f32, players: &[((f32, f32, f32), String)]) {
        m.bite_in = m.bite_in.saturating_sub(1);
        // Frappée : elle garde un instant l'élan du coup
        if m.knock > 0 {
            m.knock -= 1;
            m.vel = (m.vel.0 * 0.8, m.vel.1 * 0.8, m.vel.2 * 0.8);
            return;
        }
        // Abeille qui a piqué : elle a perdu son dard, et meurt peu après
        if m.species == Species::Bee && m.barter > 0 {
            m.barter -= 1;
            m.attack = None;
            if m.barter == 0 {
                self.thorned.push((m.id, 1.0e6));
            }
        }
        let Some((damage, sight, name)) = m.hostile() else {
            self.drift(m, speed, players);
            return;
        };
        let (_, height) = m.size();
        let centre = (m.pos.0, m.pos.1 + height / 2.0, m.pos.2);
        let dist2 = |p: (f32, f32, f32)| (p.0 - centre.0).powi(2) + (p.1 - centre.1).powi(2) + (p.2 - centre.2).powi(2);
        let fair = |c: &Client| c.joined && c.dead.is_none() && c.mode.takes_damage();
        let chest = |c: &Client| (c.pos.0, c.pos.1 - EYE + 0.9, c.pos.2);
        let mut prey = match m.attack {
            Some(Prey::Player(a)) => self.clients.get(&a).filter(|c| fair(c)).map(|c| (a, c.id, chest(c))),
            _ => None,
        }
        .filter(|(_, _, p)| dist2(*p) < (sight * 1.5).powi(2));
        if m.attack.is_some() && prey.is_none() {
            m.attack = None;
        }
        if prey.is_none() && !m.species.neutral() && self.mob_rng.below(8) == 0 {
            prey = self.clients.iter().filter(|(_, c)| fair(c) && dist2(chest(c)) < sight * sight).min_by(|a, b| dist2(chest(a.1)).total_cmp(&dist2(chest(b.1)))).map(|(a, c)| (*a, c.id, chest(c)));
            if let Some((a, _, _)) = prey {
                m.attack = Some(Prey::Player(a));
            }
        }
        // Neutre sans adversaire (abeille, dauphin) : il vit sa vie de créature paisible
        if prey.is_none() && m.species.neutral() {
            self.drift(m, speed, players);
            return;
        }
        let swimmer = m.species.swims().is_some();
        // Où aller, et à quelle allure
        let (goal, pace) = match prey {
            None => {
                // Sans proie : elle flâne, à sa hauteur de croisière au-dessus du sol
                (m.anchor_y, m.dive) = (0.0, 0);
                if m.target.is_none() || self.mob_rng.below(100) == 0 {
                    let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
                    m.target = Some((m.pos.0 + a.cos() * 12.0, m.pos.2 + a.sin() * 12.0));
                }
                let (tx, tz) = m.target.unwrap_or((m.pos.0, m.pos.2));
                let cruise = match m.species {
                    Species::Ghast => 14.0,
                    Species::Phantom => 18.0,
                    _ => 1.5,
                };
                ((tx, self.ground_under(m.pos) + cruise, tz), 0.5)
            }
            Some((a, pid, p)) => {
                let flat = ((p.0 - centre.0).powi(2) + (p.2 - centre.2).powi(2)).sqrt().max(0.01);
                let away = ((centre.0 - p.0) / flat, (centre.2 - p.2) / flat);
                m.yaw = (-(p.0 - m.pos.0)).atan2(p.2 - m.pos.2).to_degrees().rem_euclid(360.0);
                if m.species == Species::Phantom {
                    // Comme dans le jeu : il tourne autour d'un point fixé au-dessus de l'endroit
                    // où était sa proie, pas autour d'elle. Ce point n'est repris qu'après chaque
                    // piqué : qui court sort du cercle.
                    let pause = |s: &mut Self| (200 + s.mob_rng.below(200)) as u32;
                    if m.anchor_y == 0.0 {
                        m.target = Some((p.0, p.2));
                        m.anchor_y = p.1 + 20.0 + self.mob_rng.below(21) as f32;
                        m.bite_in = pause(self) as _;
                    }
                    if m.bite_in == 0 {
                        // Il fond sur sa proie ; au contact il mord, et s'il la manque trop
                        // longtemps il renonce. Dans les deux cas il remonte tourner.
                        m.dive += 1;
                        let bitten = dist2(p) < 1.3 * 1.3;
                        if bitten {
                            self.hurt(&a, damage, Cause::Mob(name));
                            if !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) {
                                self.send(&a, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(pid, (-away.0 * 0.4, 0.3, -away.1 * 0.4)));
                            }
                        }
                        if bitten || m.dive > 160 {
                            m.dive = 0;
                            m.bite_in = pause(self) as _;
                            m.target = Some((p.0, p.2));
                            m.anchor_y = p.1 + 20.0 + self.mob_rng.below(21) as f32;
                        }
                        ((p.0, p.1 - height / 2.0, p.2), 1.6)
                    } else {
                        // Sur un cercle de 5 à 15 blocs, le nez dans le sens du vol
                        let (ax, az) = m.target.unwrap_or((p.0, p.2));
                        let radius = 5.0 + (m.id.rem_euclid(11)) as f32;
                        let turn = self.tick as f32 * (0.4 / radius) + m.id as f32;
                        m.yaw = turn.sin().atan2(turn.cos()).to_degrees().rem_euclid(360.0);
                        ((ax + turn.cos() * radius, m.anchor_y, az + turn.sin() * radius), 1.0)
                    }
                } else {
                    // Au contact, le blaze brûle aussi sa proie de près (trois cœurs par seconde)
                    if m.species == Species::Blaze && dist2(p) < 2.0 * 2.0 && self.tick % 20 == m.id.rem_euclid(20) as u64 {
                        self.hurt(&a, 6.0, Cause::Mob(name));
                    }
                    // Sans projectile (abeille, dauphin) : il fonce au contact et mord
                    if m.ranged().is_none() && m.species != Species::Blaze {
                        if dist2(p) < 1.6 * 1.6 && m.bite_in == 0 && damage > 0.0 {
                            m.bite_in = 20;
                            self.hurt(&a, damage, Cause::Mob(name));
                            if m.species == Species::Bee {
                                // Le dard : dix secondes de poison, puis l'abeille meurt
                                self.give_effect(&a, super::effects::POISON, 0, 200, true);
                                m.barter = 600 + self.mob_rng.below(600) as u32;
                                m.attack = None;
                            }
                        }
                        ((p.0, p.1 - height / 2.0, p.2), 1.4)
                    } else {
                    // Blaze et ghast : à bonne distance, la vue dégagée, ils tirent
                    let (keep, above) = if m.species == Species::Ghast { (16.0, 8.0) } else { (7.0, 0.6) };
                    if let Some((kind, shot_damage, interval, range)) = m.ranged() {
                        if dist2(p) < range * range && m.bite_in == 0 && self.clear_shot(centre, p) {
                            // Le blaze tire par rafales de trois, puis reprend son souffle
                            m.bite_in = if m.species == Species::Blaze {
                                m.burst = (m.burst + 1) % 3;
                                if m.burst == 0 { 80 } else { 6 }
                            } else {
                                interval + self.mob_rng.below(20)
                            };
                            self.mob_throws(m, &kind, p, shot_damage, name);
                        }
                    }
                    ((p.0 + away.0 * keep, p.1 + above - height / 2.0, p.2 + away.1 * keep), 1.0)
                    }
                }
            }
        };
        let (dx, dy, dz) = (goal.0 - m.pos.0, goal.1 - m.pos.1, goal.2 - m.pos.2);
        let d = (dx * dx + dy * dy + dz * dz).sqrt();
        if d < 0.3 {
            m.vel = (0.0, 0.0, 0.0);
            if prey.is_none() {
                m.target = None;
            }
            return;
        }
        let step = (speed * pace).min(d);
        m.vel = (dx / d * step, dy / d * step, dz / d * step);
        // (un dauphin en chasse ne sort pas de l'eau)
        if swimmer && !self.mob_water(m.pos.0 + m.vel.0 * 4.0, m.pos.1 + m.vel.1 * 4.0 + 0.1, m.pos.2 + m.vel.2 * 4.0) {
            m.vel = (0.0, m.vel.1.min(0.0), 0.0);
        }
        if prey.is_none() {
            m.yaw = (-dx).atan2(dz).to_degrees().rem_euclid(360.0);
        }
    }

    /// Créature paisible qui vole ou qui nage : elle flâne. Celle qui vole reste à quelques
    /// blocs du sol ; celle qui nage ne quitte pas l'eau, et tombe si elle en est sortie.
    fn drift(&mut self, m: &mut Mob, speed: f32, players: &[((f32, f32, f32), String)]) {
        let swimmer = m.species.swims().is_some();
        if swimmer && !self.mob_water(m.pos.0, m.pos.1 + 0.1, m.pos.2) {
            // Hors de l'eau : il tombe, et sur le sol il frétille par petits bonds
            let grounded = !self.mob_free(m.pos.0.floor() as i32, (m.pos.1 - 0.05).floor() as i32, m.pos.2.floor() as i32);
            if grounded && m.vel.1 <= 0.0 && self.mob_rng.below(8) == 0 {
                let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
                m.vel = (a.cos() * 0.08, 0.3, a.sin() * 0.08);
            } else if grounded && m.vel.1 <= 0.0 {
                m.vel = (0.0, 0.0, 0.0);
            } else {
                m.vel = (m.vel.0 * 0.9, (m.vel.1 - 0.04).max(-0.4), m.vel.2 * 0.9);
            }
            m.target = None;
            return;
        }
        // Perroquet apprivoisé : assis, il ne bouge plus ; sinon il rejoint son maître
        if m.species.pet() && m.tamed {
            if m.sitting {
                m.vel = (0.0, (m.vel.1 - 0.04).max(-0.4), 0.0);
                if !self.mob_free(m.pos.0.floor() as i32, (m.pos.1 - 0.05).floor() as i32, m.pos.2.floor() as i32) {
                    m.vel.1 = 0.0;
                }
                return;
            }
            let master = m.owner.as_deref().and_then(|o| self.clients.values().find(|c| c.joined && c.dead.is_none() && c.name.as_deref() == Some(o))).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2));
            if let Some(p) = master {
                let d2 = (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2);
                if d2 > 3.0 * 3.0 {
                    m.target = Some((p.0, p.2));
                    m.anchor_y = p.1 + 1.5;
                }
            }
        }
        // Attiré par ce qu'il aime (l'abeille par une fleur, l'axolotl par un seau de poisson)
        let tempt = m.species.tempt();
        if !tempt.is_empty() {
            let lure = players.iter().filter(|(_, held)| tempt.contains(&held.as_str())).map(|(p, _)| *p).find(|p| (p.0 - m.pos.0).powi(2) + (p.1 - m.pos.1).powi(2) + (p.2 - m.pos.2).powi(2) < 8.0 * 8.0);
            if let Some(p) = lure {
                m.target = Some((p.0, p.2));
                m.anchor_y = p.1 + 1.0;
            }
        }
        if m.target.is_none() || self.mob_rng.below(120) == 0 {
            let a = self.mob_rng.unit() as f32 * std::f32::consts::TAU;
            let far = 3.0 + self.mob_rng.unit() as f32 * 7.0;
            m.target = Some((m.pos.0 + a.cos() * far, m.pos.2 + a.sin() * far));
            // (pour qui vole, le « sol » est aussi la surface de l'eau : il ne plonge pas)
            let air = self.world.air;
            let (bx, bz, top) = (m.pos.0.floor() as i32, m.pos.2.floor() as i32, m.pos.1.floor() as i32);
            let surface = (0..32).map(|d| top - d).find(|y| self.mob_get(bx, *y - 1, bz) != air).map(|y| y as f32).unwrap_or(m.pos.1);
            m.anchor_y = if swimmer {
                // (un nageur reste sous la surface et au-dessus du fond)
                let water = self.world.generator().blocks.water;
                let roof = (0..32).map(|d| top + d).find(|y| self.mob_get(bx, *y, bz) != water).unwrap_or(top + 1) as f32;
                let floor = (0..32).map(|d| top - d).find(|y| self.mob_get(bx, *y, bz) != water).unwrap_or(top - 1) as f32 + 1.0;
                let (lo, hi) = (floor + 0.3, (roof - 1.0).max(floor + 0.3));
                (m.pos.1 + self.mob_rng.unit() as f32 * 4.0 - 2.0).clamp(lo, hi)
            } else {
                surface + 1.5 + self.mob_rng.unit() as f32 * 3.0
            };
        }
        let (tx, tz) = m.target.unwrap_or((m.pos.0, m.pos.2));
        let (dx, dy, dz) = (tx - m.pos.0, m.anchor_y - m.pos.1, tz - m.pos.2);
        let d = (dx * dx + dy * dy + dz * dz).sqrt();
        if d < 0.4 {
            m.vel = (0.0, 0.0, 0.0);
            m.target = None;
            return;
        }
        let step = (speed * 0.6).min(d);
        let vel = (dx / d * step, dy / d * step, dz / d * step);
        // (un nageur ne sort pas de l'eau : il fait demi-tour avant)
        if swimmer && !self.mob_water(m.pos.0 + vel.0 * 6.0, m.pos.1 + vel.1 * 6.0 + 0.1, m.pos.2 + vel.2 * 6.0) {
            m.vel = (0.0, 0.0, 0.0);
            m.target = None;
            return;
        }
        m.vel = vel;
        // (il tourne peu à peu vers sa route, 10° par tick au plus, au lieu de pivoter d'un coup)
        let want = (-dx).atan2(dz).to_degrees();
        let turn = ((want - m.yaw + 540.0).rem_euclid(360.0) - 180.0).clamp(-10.0, 10.0);
        m.yaw = (m.yaw + turn).rem_euclid(360.0);
    }

    /// L'arpenteur marche sur la lave comme sur un sol (« walk_on_lava » des fiches du jeu).
    fn lava_walk(&self, m: &mut Mob) {
        if m.species != Species::Strider {
            return;
        }
        let lava = |s: &Self, y: i32| s.block_name_of(s.mob_get(m.pos.0.floor() as i32, y, m.pos.2.floor() as i32)).map(|n| n.contains("lava")).unwrap_or(false);
        let mut y = (m.pos.1 - 0.01).floor() as i32;
        if !lava(self, y) {
            return;
        }
        for _ in 0..3 {
            if lava(self, y + 1) {
                y += 1;
            }
        }
        m.pos.1 = (y + 1) as f32;
        m.vel.1 = m.vel.1.max(0.0);
        m.on_ground = true;
    }

    /// Hauteur du sol sous un point (32 blocs au plus bas ; sinon le point lui-même).
    fn ground_under(&self, p: (f32, f32, f32)) -> f32 {
        let (x, z) = (p.0.floor() as i32, p.2.floor() as i32);
        let top = p.1.floor() as i32;
        (0..32).map(|d| top - d).find(|y| !self.mob_free(x, *y - 1, z)).map(|y| y as f32).unwrap_or(p.1)
    }

    /// Déplacement d'une créature volante : pas de pesanteur, et chaque axe s'arrête
    /// devant un bloc plein (elle glisse le long des murs et des plafonds).
    fn fly_physics(&mut self, m: &mut Mob) {
        let (w, h) = m.size();
        let half = w / 2.0;
        let free = |s: &Self, p: (f32, f32, f32)| -> bool {
            let mut y = p.1;
            loop {
                let top = y.min(p.1 + h - 0.01);
                for (ox, oz) in [(0.0, 0.0), (-half, -half), (half, -half), (-half, half), (half, half)] {
                    if !s.mob_free((p.0 + ox).floor() as i32, top.floor() as i32, (p.2 + oz).floor() as i32) {
                        return false;
                    }
                }
                if y >= p.1 + h - 0.01 {
                    return true;
                }
                y += 1.0;
            }
        };
        let mut blocked = false;
        for axis in 0..3 {
            let mut next = m.pos;
            match axis {
                0 => next.0 += m.vel.0,
                1 => next.1 += m.vel.1,
                _ => next.2 += m.vel.2,
            }
            if next.1 < crate::world::MAX_Y as f32 && free(self, next) {
                m.pos = next;
            } else {
                blocked = true;
            }
        }
        // Coincée contre un mur : elle choisira une autre direction
        if blocked && m.attack.is_none() {
            m.target = None;
        }
        m.on_ground = false;
    }

    /// Une créature tire son projectile ; si c'est une potion, elle la remplit (celle de sa
    /// fiche, ou l'une de celles de la sorcière). La potion vise les pieds, pour se briser au sol.
    fn mob_throws(&mut self, m: &Mob, kind: &str, at: (f32, f32, f32), damage: f32, name: &'static str) {
        let roll = self.mob_rng.below(100);
        if kind.contains("thrown_trident") {
            self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("item.trident.throw", m.pos, "", false, -1));
        }
        let potion = if kind.contains("potion") { m.potion(roll).or(Some(0)) } else { None };
        let at = if potion.is_some() { (at.0, at.1 - 1.0, at.2) } else { at };
        if let (Some(id), Some(variant)) = (self.mob_shoots(m, kind, at, damage, name), potion) {
            if let Some(s) = self.shots.get_mut(&id) {
                s.potion = Some((variant, 0.75));
            }
        }
    }

    /// La vue est-elle dégagée entre deux points ? (aucun bloc plein sur la ligne)
    /// Un corps de cette largeur et de cette hauteur passe-t-il en ligne droite d'ici à là ?
    fn body_clear(&self, from: (f32, f32, f32), to: (f32, f32, f32), width: f32, height: f32) -> bool {
        let (dx, dz) = (to.0 - from.0, to.2 - from.2);
        let d = (dx * dx + dz * dz).sqrt().max(0.01);
        // (de part et d'autre, perpendiculairement à la route)
        let (sx, sz) = (-dz / d * width / 2.0, dx / d * width / 2.0);
        [0.3, 1.2, height * 0.6, height - 0.1].iter().all(|&h| {
            [-1.0f32, 0.0, 1.0].iter().all(|&side| {
                let a = (from.0 + sx * side, from.1 + h, from.2 + sz * side);
                let b = (to.0 + sx * side, to.1 + h, to.2 + sz * side);
                self.clear_shot(a, b)
            })
        })
    }

    fn clear_shot(&self, from: (f32, f32, f32), to: (f32, f32, f32)) -> bool {
        let (dx, dy, dz) = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
        let steps = ((dx * dx + dy * dy + dz * dz).sqrt() * 5.0).ceil().clamp(1.0, 200.0) as i32;
        (1..steps).all(|i| {
            let t = i as f32 / steps as f32;
            self.mob_free((from.0 + dx * t).floor() as i32, (from.1 + dy * t).floor() as i32, (from.2 + dz * t).floor() as i32)
        })
    }

    /// Les créatures et les joueurs, rangés par cases de deux blocs : qui pousse qui.
    pub(super) fn crowd(&self) -> HashMap<(i32, i32), Vec<(i64, (f32, f32, f32), f32, f32)>> {
        let mut grid: HashMap<(i32, i32), Vec<(i64, (f32, f32, f32), f32, f32)>> = HashMap::new();
        let cell = |p: (f32, f32, f32)| ((p.0 / 2.0).floor() as i32, (p.2 / 2.0).floor() as i32);
        for m in self.mobs.values().filter(|m| m.dead.is_none() && m.asleep.is_none()) {
            let (w, h) = m.size();
            grid.entry(cell(m.pos)).or_default().push((m.id, m.pos, w, h));
        }
        for c in self.clients.values().filter(|c| c.joined && c.stage == Stage::InGame && c.dead.is_none() && !c.mode.is_spectator()) {
            let feet = (c.pos.0, c.pos.1 - EYE, c.pos.2);
            grid.entry(cell(feet)).or_default().push((c.id, feet, 0.6, 1.8));
        }
        grid
    }

    /// Poussée reçue des corps qui chevauchent celui-ci : ils s'écartent doucement, comme
    /// dans le jeu, au lieu de se traverser.
    fn push_from(grid: &HashMap<(i32, i32), Vec<(i64, (f32, f32, f32), f32, f32)>>, m: &Mob) -> (f32, f32) {
        let (w, h) = m.size();
        let (cx, cz) = ((m.pos.0 / 2.0).floor() as i32, (m.pos.2 / 2.0).floor() as i32);
        let mut push = (0.0f32, 0.0f32);
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(list) = grid.get(&(cx + dx, cz + dz)) else { continue };
                for (id, p, ow, oh) in list {
                    if *id == m.id || p.1 >= m.pos.1 + h || m.pos.1 >= p.1 + oh {
                        continue;
                    }
                    let room = (w + ow) / 2.0 * 0.9;
                    let (ax, az) = (m.pos.0 - p.0, m.pos.2 - p.2);
                    let d = (ax * ax + az * az).sqrt();
                    if d >= room {
                        continue;
                    }
                    // (exactement au même endroit : chacun part de son côté, d'après son numéro)
                    let (ux, uz) = if d < 0.01 { if m.id > *id { (1.0, 0.3) } else { (-1.0, -0.3) } } else { (ax / d, az / d) };
                    let force = (1.0 - d / room) * 0.06;
                    push.0 += ux * force;
                    push.1 += uz * force;
                }
            }
        }
        (push.0.clamp(-0.1, 0.1), push.1.clamp(-0.1, 0.1))
    }

    /// Golem de fer : il fonce sur le joueur qui l'a frappé, lui ou un villageois, et l'envoie
    /// en l'air d'un coup de bras ; sinon il reste près du cœur du village.
    fn golem_think(&mut self, m: &mut Mob) -> Option<(f32, f32)> {
        m.bite_in = m.bite_in.saturating_sub(1);
        let attr = m.movement();
        let flat2 = |p: (f32, f32, f32)| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2);
        let prey = match m.attack {
            Some(Prey::Player(a)) => self.clients.get(&a).filter(|c| c.joined && c.dead.is_none() && c.mode.takes_damage()).map(|c| (a, c.id, (c.pos.0, c.pos.1 - EYE, c.pos.2))),
            _ => None,
        }
        .filter(|(_, _, p)| flat2(*p) < 24.0 * 24.0);
        if m.attack.is_some() && prey.is_none() {
            m.attack = None;
            m.target = None;
        }
        if let Some((a, pid, p)) = prey {
            m.target = Some((p.0, p.2));
            if flat2(p) < 2.4 * 2.4 && (p.1 - m.pos.1).abs() < 3.0 && m.bite_in == 0 {
                m.bite_in = 20;
                // Entre 7,5 et 21,5 points de dégâts, comme dans le jeu
                let damage = 7.5 + self.mob_rng.unit() as f32 * 14.0;
                self.send_to_viewers(m.id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(m.id, bedrock::EVENT_ARM_SWING, 0));
                self.hurt(&a, damage, Cause::Mob("un golem de fer"));
                if !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) {
                    let d = flat2(p).sqrt().max(0.01);
                    self.send(&a, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(pid, ((p.0 - m.pos.0) / d * 0.3, 0.8, (p.2 - m.pos.2) / d * 0.3)));
                }
            }
            return Some((ground_speed(attr * 1.3), 1.6));
        }
        if let Some(h) = m.home {
            // Coincé sur un puits ou perché sur un toit (là où d'anciennes versions le
            // faisaient naître) : il est reposé sur la place, près de la cloche
            if self.mob_rng.below(40) == 0 {
                let feet = (m.pos.0.floor() as i32, m.pos.1.round() as i32, m.pos.2.floor() as i32);
                if self.golem_trapped(feet) {
                    if let Some((x, y, z)) = self.golem_spot((h.0.floor() as i32, h.1.floor() as i32, h.2.floor() as i32)) {
                        m.pos = (x as f32 + 0.5, y as f32, z as f32 + 0.5);
                        m.vel = (0.0, 0.0, 0.0);
                        m.target = None;
                        return None;
                    }
                }
            }
            // Trop loin de la cloche : il revient
            if flat2(h) > 24.0 * 24.0 {
                m.target = Some((h.0, h.2));
                return Some((ground_speed(attr * 0.6), 4.0));
            }
        }
        None
    }

    /// Un endroit dégagé pour un golem de fer (large de 1,4 bloc, haut de 2,9) : un sol plein
    /// de 3 × 3, sans eau, trois blocs libres au-dessus, pas de toit, et de la vraie terre
    /// dessous (pas un toit ni une margelle).
    fn golem_open(&self, (x, y, z): (i32, i32, i32)) -> bool {
        let water = self.world.generator().blocks.water;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let floor = self.mob_get(x + dx, y - 1, z + dz);
                if floor == water || self.mob_free(x + dx, y - 1, z + dz) || self.no_step(x + dx, y - 1, z + dz) {
                    return false;
                }
                if (0..3).any(|h| !self.mob_free(x + dx, y + h, z + dz) || self.mob_get(x + dx, y + h, z + dz) == water) {
                    return false;
                }
            }
        }
        (3..10).all(|h| self.mob_free(x, y + h, z)) && (2..=3).all(|d| !self.mob_free(x, y - d, z))
    }

    /// L'endroit dégagé le plus proche de la cloche, à sa hauteur à deux blocs près.
    fn golem_spot(&self, bell: (i32, i32, i32)) -> Option<(i32, i32, i32)> {
        for r in 2i32..=8 {
            for dx in -r..=r {
                for dz in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let (x, z) = (bell.0 + dx, bell.2 + dz);
                    let found = self.surface_y(x, z, bell.1 + 2).filter(|y| (*y - bell.1).abs() <= 2).filter(|y| self.golem_open((x, *y, z)));
                    if let Some(y) = found {
                        return Some((x, y, z));
                    }
                }
            }
        }
        None
    }

    /// Le golem est-il là où il ne peut pas vivre ? Perché (du vide sous son sol), ou au bord
    /// d'une eau sous un toit ou sur une margelle (un puits).
    fn golem_trapped(&self, (x, y, z): (i32, i32, i32)) -> bool {
        if !self.world.is_ready(x >> 4, z >> 4) || self.mob_free(x, y - 1, z) {
            return false; // (terrain pas prêt, ou en pleine chute)
        }
        let water = self.world.generator().blocks.water;
        let perched = (2..=3).any(|d| self.mob_free(x, y - d, z) && self.mob_get(x, y - d, z) != water);
        let wet = (-1..=1).any(|dx| (-1..=1).any(|dz| (-2..=0).any(|dy| self.mob_get(x + dx, y + dy, z + dz) == water)));
        let roof = (3..8).any(|h| !self.mob_free(x, y + h, z));
        let rim = self.block_name_of(self.mob_get(x, y - 1, z)).map(|n| n.contains("slab") || n.contains("wall") || n.contains("stairs") || n.contains("fence")).unwrap_or(false);
        perched || (wet && (roof || rim))
    }

    /// Citrouille sculptée (ou citrouille-lanterne) posée au sommet d'un T de quatre blocs de
    /// fer : les cinq blocs disparaissent et un golem de fer prend leur place. Bâti par un
    /// joueur, il ne s'en prend jamais aux joueurs. Vrai si un golem est né.
    pub(super) fn try_build_golem(&mut self, (x, y, z): (i32, i32, i32)) -> bool {
        let name = |s: &Self, p: (i32, i32, i32)| s.data.as_ref().and_then(|d| d.block_name(s.world.get(p.0, p.1, p.2))).unwrap_or("").to_string();
        let head = name(self, (x, y, z));
        if head != "minecraft:carved_pumpkin" && head != "minecraft:lit_pumpkin" {
            return false;
        }
        let iron = |s: &Self, p: (i32, i32, i32)| name(s, p) == "minecraft:iron_block";
        if !iron(self, (x, y - 1, z)) || !iron(self, (x, y - 2, z)) {
            return false;
        }
        // Les bras : d'est en ouest ou du nord au sud
        let arms = [[(x - 1, y - 1, z), (x + 1, y - 1, z)], [(x, y - 1, z - 1), (x, y - 1, z + 1)]].into_iter().find(|a| iron(self, a[0]) && iron(self, a[1]));
        let Some(arms) = arms else { return false };
        if self.mobs.len() >= MAX_MOBS {
            return false;
        }
        let air = self.world.generator().blocks.air;
        for p in [(x, y, z), (x, y - 1, z), (x, y - 2, z), arms[0], arms[1]] {
            self.change_block(p, air);
        }
        let id = self.add_mob(Species::IronGolem, (x as f32 + 0.5, (y - 2) as f32, z as f32 + 0.5));
        if let Some(m) = self.mobs.get_mut(&id) {
            m.age = 0;
            m.tamed = true;
            m.home = None;
        }
        self.mobs_dirty = true;
        true
    }

    /// Les golems de fer des environs prennent ce joueur pour cible (il a frappé un
    /// villageois ou l'un d'eux). Jamais un joueur en créatif.
    fn rally_golems(&mut self, culprit: SocketAddr, near: (f32, f32, f32)) {
        if !self.clients.get(&culprit).map(|c| c.mode.takes_damage()).unwrap_or(false) {
            return;
        }
        for g in self.mobs.values_mut() {
            // (un golem bâti par un joueur ne s'en prend jamais aux joueurs)
            if g.species == Species::IronGolem && !g.tamed && g.dead.is_none() && (g.pos.0 - near.0).powi(2) + (g.pos.2 - near.2).powi(2) < 24.0 * 24.0 {
                g.attack = Some(Prey::Player(culprit));
                g.panic = 0;
            }
        }
    }

    /// Ce que fait un loup avant tout le reste : (vitesse, distance d'arrêt) s'il a mieux à
    /// faire que se promener. Assis, il ne bouge pas ; avec une cible, il la poursuit et la
    /// mord ; sauvage, il chasse moutons et lapins ; apprivoisé, il rejoint son maître.
    fn wolf_think(&mut self, m: &mut Mob) -> Option<(f32, f32)> {
        m.bite_in = m.bite_in.saturating_sub(1);
        if m.sitting {
            m.target = None;
            if m.attack.take().is_some() {
                m.refresh = true;
            }
            return Some((0.0, f32::MAX));
        }
        let attr = m.movement();
        let flat2 = |p: (f32, f32, f32)| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2);
        // Sa cible existe-t-elle encore, à portée ?
        let prey_pos = match m.attack {
            Some(Prey::Mob(id)) => self.mobs.get(&id).filter(|t| t.dead.is_none()).map(|t| t.pos),
            Some(Prey::Player(a)) => self.clients.get(&a).filter(|c| c.joined && c.dead.is_none() && c.mode.takes_damage()).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2)),
            None => None,
        }
        .filter(|p| flat2(*p) < 24.0 * 24.0);
        if m.attack.is_some() && prey_pos.is_none() {
            m.attack = None;
            m.target = None;
            m.refresh = true;
        }
        if let (Some(prey), Some(p)) = (m.attack, prey_pos) {
            m.target = Some((p.0, p.2));
            if flat2(p) < 1.7 * 1.7 && (p.1 - m.pos.1).abs() < 1.8 && m.bite_in == 0 {
                m.bite_in = 20;
                let damage = if m.tamed && m.species == Species::Wolf { 4.0 } else { m.species.hunts().1 };
                match prey {
                    Prey::Mob(id) => {
                        self.hit_mob(id, m.pos, damage, Attacker::Mob(m.id));
                    }
                    Prey::Player(a) => {
                        self.hurt(&a, damage, Cause::Mob(if m.species == Species::Cat { "un chat" } else { "un loup" }));
                        // Les loups de la victime la défendent
                        if let Some(name) = self.clients.get(&a).and_then(|c| c.name.clone()) {
                            self.rally_wolves(&name, Prey::Mob(m.id), m.pos);
                        }
                    }
                }
            }
            return Some((ground_speed(attr * 1.25), 1.2));
        }
        // Sauvage et adulte : il chasse ses proies des environs (voir `Species::hunts` ; la
        // grenouille ne gobe que les plus petits cubes)
        if !m.tamed && !m.baby() && self.mob_rng.below(60) == 0 {
            let (kinds, _) = m.species.hunts();
            let prey = self
                .mobs
                .values()
                .filter(|t| kinds.contains(&t.species) && (!t.species.cube() || t.variant <= 1) && t.dead.is_none() && flat2(t.pos) < 12.0 * 12.0 && (t.pos.1 - m.pos.1).abs() < 4.0)
                // (une proie dans l'eau est hors d'atteinte d'un chasseur à pattes)
                .filter(|t| !self.mob_water(t.pos.0, t.pos.1 + 0.1, t.pos.2))
                .min_by(|a, b| flat2(a.pos).total_cmp(&flat2(b.pos)))
                .map(|t| t.id);
            if let Some(id) = prey {
                m.attack = Some(Prey::Mob(id));
                return Some((ground_speed(attr * 1.25), 1.2));
            }
        }
        // Apprivoisé : il rejoint son maître au-delà de 10 blocs (jusqu'à 2), et réapparaît
        // près de lui s'il est distancé
        if let Some(owner) = m.owner.as_deref().filter(|_| m.tamed && m.leash.is_none()) {
            let master = self.clients.values().find(|c| c.joined && c.dead.is_none() && c.name.as_deref() == Some(owner)).map(|c| (c.id, (c.pos.0, c.pos.1 - EYE, c.pos.2)));
            let known = master.map(|(id, _)| id).unwrap_or(-1);
            if known != m.owner_id {
                m.owner_id = known;
                m.refresh = true;
            }
            if let Some((_, p)) = master {
                let d2 = flat2(p);
                if d2 > 10.0 * 10.0 {
                    m.following = true;
                } else if d2 < 2.5 * 2.5 && m.following {
                    m.following = false;
                    m.target = None;
                }
                if m.following && (d2 > 20.0 * 20.0 || (p.1 - m.pos.1).abs() > 8.0) && d2 < 96.0 * 96.0 {
                    // Distancé : il réapparaît à côté de lui, sur un sol ferme
                    let (bx, bz) = (p.0.floor() as i32 + 1, p.2.floor() as i32 + 1);
                    if let Some(y) = self.surface_y(bx, bz, p.1.floor() as i32).filter(|y| (*y as f32 - p.1).abs() < 4.0) {
                        m.pos = (bx as f32 + 0.5, y as f32, bz as f32 + 0.5);
                        m.vel = (0.0, 0.0, 0.0);
                        m.sent = (f32::MAX, 0.0, 0.0);
                        m.target = None;
                        m.following = false;
                        return Some((0.0, f32::MAX));
                    }
                }
                if m.following {
                    m.target = Some((p.0, p.2));
                    return Some((ground_speed(attr * 1.0), 2.5));
                }
            } else {
                m.following = false;
            }
        }
        None
    }

    /// Les loups apprivoisés de ce joueur, debout et à moins de 20 blocs de `near`, prennent
    /// cette cible (jamais un autre de ses loups, ni lui-même).
    fn rally_wolves(&mut self, owner: &str, prey: Prey, near: (f32, f32, f32)) {
        if let Prey::Mob(id) = prey {
            if self.mobs.get(&id).map(|t| t.species == Species::Wolf && t.owner.as_deref() == Some(owner)).unwrap_or(false) {
                return;
            }
        }
        if let Prey::Player(a) = prey {
            if self.clients.get(&a).and_then(|c| c.name.as_deref()) == Some(owner) {
                return;
            }
        }
        let mut changed = Vec::new();
        for w in self.mobs.values_mut() {
            let close = (w.pos.0 - near.0).powi(2) + (w.pos.2 - near.2).powi(2) < 20.0 * 20.0;
            if w.species == Species::Wolf && w.tamed && !w.sitting && !w.baby() && w.dead.is_none() && close && w.owner.as_deref() == Some(owner) && Prey::Mob(w.id) != prey && w.attack != Some(prey) {
                w.attack = Some(prey);
                changed.push(w.id);
            }
        }
        for id in changed {
            self.mob_changed(id);
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
                // (les grenouilles pondent : ce sont des têtards qui naissent, dans l'eau voisine)
                let kind = if kind == Species::Frog { Species::Tadpole } else { kind };
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
                // L'éleveur (le joueur le plus proche, à 10 blocs au plus) y gagne de l'expérience
                let near = |c: &Client| (c.pos.0 - at.0).powi(2) + (c.pos.1 - at.1).powi(2) + (c.pos.2 - at.2).powi(2);
                let breeder = self.clients.iter().filter(|(_, c)| c.joined && c.dead.is_none() && near(c) < 100.0).min_by(|x, y| near(x.1).total_cmp(&near(y.1))).map(|(a, _)| *a);
                if breeder.is_some() {
                    let xp = self.xp_between(1, 7);
                    self.drop_xp((at.0, at.1 + 0.5, at.2), xp);
                }
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
                let wolf_owner = self.mobs.get(&a.0).and_then(|m| m.owner.clone());
                // Tortues : la mère pond de un à quatre œufs sur le sable voisin (8 blocs), qui
                // écloront plus tard ; pas de sable, pas de ponte
                if kind == Species::Turtle {
                    self.lay_turtle_eggs(at);
                    self.send_to_viewers(a.0, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(a.0, bedrock::EVENT_LOVE_PARTICLES, 0));
                    continue;
                }
                let baby = self.add_mob(kind, at);
                if let Some(m) = self.mobs.get_mut(&baby) {
                    m.age = -GROW_TICKS;
                    m.color = color;
                    m.variant = variant;
                    m.mark = if kind == Species::Horse { mark } else { 0 };
                    if kind.pet() {
                        // Petit de compagnons apprivoisés : apprivoisé, au même maître
                        m.tamed = true;
                        m.owner = wolf_owner.clone();
                        if kind == Species::Wolf {
                            m.max_health = WOLF_TAMED_HEALTH;
                            m.health = m.max_health;
                        }
                    }
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
                if kind == Species::Tadpole {
                    // Le frai : deux à six têtards en tout, dans l'eau la plus proche (4 blocs)
                    let (bx, by, bz) = (at.0.floor() as i32, at.1.floor() as i32, at.2.floor() as i32);
                    let pond = (-4..=4).flat_map(|dx| (-2..=1).flat_map(move |dy| (-4..=4).map(move |dz| (dx, dy, dz)))).filter(|(dx, dy, dz)| self.mob_water((bx + dx) as f32 + 0.5, (by + dy) as f32 + 0.5, (bz + dz) as f32 + 0.5)).min_by_key(|(dx, dy, dz)| dx * dx + dy * dy + dz * dz);
                    let spot = pond.map(|(dx, dy, dz)| ((bx + dx) as f32 + 0.5, (by + dy) as f32 + 0.2, (bz + dz) as f32 + 0.5)).unwrap_or(at);
                    if let Some(m) = self.mobs.get_mut(&baby) {
                        m.pos = spot;
                        m.sent = (f32::MAX, 0.0, 0.0);
                    }
                    for _ in 0..1 + self.mob_rng.below(5) {
                        if self.mobs.len() < MAX_MOBS {
                            let (ox, oz) = ((self.mob_rng.unit() as f32 - 0.5) * 0.6, (self.mob_rng.unit() as f32 - 0.5) * 0.6);
                            self.add_mob(Species::Tadpole, (spot.0 + ox, spot.1, spot.2 + oz));
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
        // Un villageois passe les portes en bois (il les ouvre devant lui)
        let doors = m.species == Species::Villager;
        let (w, h) = m.size();
        let half = w / 2.0;
        let in_water = self.mob_water(m.pos.0, m.pos.1 + 0.2, m.pos.2);
        // Au sol en début de tick (le bond du lapin ne compte pas : il peut aussi gravir une marche)
        let grounded = m.on_ground;
        if m.attack.is_none() {
            m.door = 0;
        }
        // Vertical
        // Flottaison : le corps reste dans l'eau, la tête dehors (un peu plus de la moitié de la
        // hauteur sous la surface). Avant, seules les pattes trempaient : la créature semblait
        // marcher sur l'eau.
        let sinks = m.species.sinks();
        let deep = in_water && self.mob_water(m.pos.0, m.pos.1 + (h * 0.55).max(0.25), m.pos.2);
        // Le noyé vit au fond, mais il nage : il remonte vers un joueur qu'il poursuit plus haut
        let rises = m.species == Species::Drowned
            && match m.attack {
                Some(Prey::Player(a)) => self.clients.get(&a).map(|c| c.pos.1 - crate::player::EYE_HEIGHT > m.pos.1 + 0.5).unwrap_or(false),
                _ => false,
            };
        if in_water && sinks && rises {
            m.vel.1 = if deep { (m.vel.1 + 0.03).min(0.08) } else { m.vel.1.min(0.0) };
        } else if in_water && sinks {
            // (zombies et squelettes ne nagent pas : ils coulent et marchent au fond ; un saut
            // sous l'eau y retombe vite)
            m.vel.1 = (m.vel.1 - 0.06).max(-0.12);
        } else if deep {
            m.vel.1 = (m.vel.1 + 0.02).min(0.06);
        } else if in_water {
            m.vel.1 = (m.vel.1 - 0.02).max(-0.03);
        } else {
            m.vel.1 = (m.vel.1 - 0.08) * 0.98;
            if m.species == Species::Chicken && !m.on_ground {
                m.vel.1 = m.vel.1.max(-0.1); // les poules battent des ailes
            }
        }
        // Le slime n'avance qu'en sautant : au sol il attend, puis bondit vers là où il va
        // (toutes les demi-secondes en poursuite, toutes les secondes et demie sinon)
        if m.species.cube() && m.on_ground && !in_water {
            let every = if m.attack.is_some() { 10 } else { 30 };
            if (m.vel.0 != 0.0 || m.vel.2 != 0.0) && (self.tick + m.id as u64) % every == 0 {
                // (le cube de magma bondit plus haut)
                m.vel.1 = if m.species == Species::MagmaCube { 0.42 + 0.1 * m.variant.clamp(1, 4) as f32 } else { 0.42 };
                m.on_ground = false;
                // (son bruit mouillé, grave pour un gros, aigu pour un petit)
                let sound = if m.variant > 1 { "squish.big" } else { "squish.small" };
                self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound(sound, m.pos, m.species.id(), false, m.id));
            } else {
                m.vel.0 = 0.0;
                m.vel.2 = 0.0;
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
                if !s.mob_pass(doors, x.floor() as i32, yy.floor() as i32, z.floor() as i32) {
                    return false;
                }
                yy += 1.0;
            }
            s.mob_pass(doors, x.floor() as i32, (y + h - 0.01).floor() as i32, z.floor() as i32)
        };
        // Le corps a une largeur, comme dans le jeu : son bord avant est testé sur toute sa
        // largeur (les deux coins et le milieu), pas seulement en son centre — sinon la moitié
        // du corps passait dans le bloc d'à côté.
        let reach = (half - 0.02).max(0.0);
        let side = [-reach, 0.0, reach];
        let front_free = |s: &Self, axis: usize, ex: f32, y: f32, ez: f32, wide: bool| -> bool {
            side.iter().filter(|o| wide || **o == 0.0).all(|o| if axis == 0 { body_free(s, ex, y, ez + o) } else { body_free(s, ex + o, y, ez) })
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
            // (déjà à moitié dans un bloc — ancienne sauvegarde, bloc posé contre elle : seul le
            // centre compte, le temps qu'elle s'en dégage)
            let wide = side.iter().all(|o| if axis == 0 { body_free(self, m.pos.0, m.pos.1 + 0.01, m.pos.2 + o) } else { body_free(self, m.pos.0 + o, m.pos.1 + 0.01, m.pos.2) });
            let ok = front_free(self, axis, ex, m.pos.1 + 0.01, ez, wide);
            if ok {
                // Pas de chute de plus de 3 blocs (sauf en fuite ou dans l'eau)
                let ground_below = (1..=4).any(|d| !self.mob_pass(doors, ex.floor() as i32, m.pos.1.floor() as i32 - d, ez.floor() as i32));
                if !ground_below && m.panic == 0 && m.knock == 0 && !in_water {
                    m.target = None;
                    continue;
                }
                if axis == 0 {
                    m.pos.0 = nx;
                } else {
                    m.pos.2 = nz;
                }
            } else if m.attack.is_some() && m.species.breaks_doors() && self.cfg.mob_griefing && (0..2).any(|dy| self.wood_door(ex.floor() as i32, m.pos.1.floor() as i32 + dy, ez.floor() as i32)) {
                // Une porte en bois entre lui et sa proie : il la frappe, et elle cède au bout de
                // douze secondes (comme dans le jeu en difficile)
                m.door += 1;
                let door = (ex.floor() as i32, m.pos.1.floor() as i32, ez.floor() as i32);
                let centre = (door.0 as f32 + 0.5, door.1 as f32 + 1.0, door.2 as f32 + 0.5);
                if m.door % 20 == 1 {
                    self.send_to_viewers(m.id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(m.id, bedrock::EVENT_ARM_SWING, 0));
                }
                if m.door >= 240 {
                    m.door = 0;
                    for y in door.1 - 1..=door.1 + 2 {
                        if self.wood_door(door.0, y, door.2) {
                            let state = self.world.get(door.0, y, door.2);
                            self.send_near(centre, bedrock::ID_LEVEL_EVENT, &bedrock::level_event(bedrock::LEVEL_EVENT_DESTROY, (door.0 as f32 + 0.5, y as f32 + 0.5, door.2 as f32 + 0.5), state as i32));
                            self.change_block((door.0, y, door.2), self.world.air);
                        }
                    }
                }
            } else if m.species.climbs() && m.attack.is_some() && !in_water {
                // Araignée à la poursuite d'un joueur : le mur ne l'arrête pas, elle l'escalade
                m.vel.1 = 0.2;
                m.on_ground = false;
            } else if grounded && !self.no_step(ex.floor() as i32, m.pos.1.floor() as i32, ez.floor() as i32) && front_free(self, axis, ex, m.pos.1.floor() + 1.01, ez, wide) {
                // Marche d'un bloc : saute
                m.vel.1 = m.vel.1.max(0.42);
                m.on_ground = false;
            } else if !sinks && (in_water || self.mob_water(m.pos.0, m.pos.1, m.pos.2)) && body_free(self, ex, m.pos.1.floor() + 1.01, ez) {
                // Contre la berge, les pieds dans l'eau : un élan pour se hisser dessus. En
                // flottant seulement, ses pieds restaient juste sous le bord et elle y
                // butait sans fin.
                m.vel.1 = m.vel.1.max(0.3);
                m.on_ground = false;
            } else if !grounded && m.vel.1 > -0.3 && body_free(self, ex, m.pos.1.floor() + 1.01, ez) {
                // En plein saut vers la marche : il continue de monter, sans changer d'idée
            } else if in_water && !sinks {
                m.vel.1 = 0.1; // sort de l'eau en grimpant au bord
            } else {
                m.target = None;
            }
        }
        // Vertical : atterrissage et plafond
        let ny = m.pos.1 + m.vel.1;
        if m.vel.1 <= 0.0 {
            let feet = ny.floor() as i32;
            // Elle se pose dès qu'une partie de son corps a un bloc dessous (le bord d'une
            // marche la porte, comme dans le jeu). Un coin déjà pris dans un mur ne compte pas.
            let body_y = m.pos.1.floor() as i32;
            let held = side.iter().any(|ox| {
                side.iter().any(|oz| {
                    let (cx, cz) = ((m.pos.0 + ox).floor() as i32, (m.pos.2 + oz).floor() as i32);
                    (feet == body_y || self.mob_pass(doors, cx, body_y, cz)) && !self.mob_pass(doors, cx, feet, cz)
                })
            });
            let centre = !self.mob_pass(doors, m.pos.0.floor() as i32, feet, m.pos.2.floor() as i32);
            // (dans l'eau aussi, le fond porte : celui qui coule y marche, celui qui flotte en eau
            // peu profonde y a pied)
            if centre || (held && feet < body_y && !in_water) {
                m.pos.1 = (feet + 1) as f32;
                m.vel.1 = 0.0;
                m.on_ground = true;
            } else {
                m.pos.1 = ny;
                m.on_ground = false;
            }
        } else if self.mob_pass(doors, m.pos.0.floor() as i32, (ny + h).floor() as i32, m.pos.2.floor() as i32) {
            m.pos.1 = ny;
            m.on_ground = false;
        } else {
            m.vel.1 = 0.0;
        }
        // Sur un lit ou une dalle basse : les pieds sont posés à sa hauteur, pas dans le bloc ni
        // un demi-bloc au-dessus
        {
            let (cx, cy, cz) = (m.pos.0.floor() as i32, (m.pos.1 + 0.001).floor() as i32, m.pos.2.floor() as i32);
            if let Some(top) = self.low_top(self.mob_get(cx, cy, cz)) {
                if m.pos.1 < cy as f32 + top && m.vel.1 <= 0.0 {
                    m.pos.1 = cy as f32 + top;
                    m.vel.1 = 0.0;
                    m.on_ground = true;
                }
            }
        }
        // Coincé dans un bloc (bloc posé dessus) : remonte
        if !self.mob_pass(doors, m.pos.0.floor() as i32, m.pos.1.floor() as i32, m.pos.2.floor() as i32) {
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
            self.mob_died(id, None);
            self.mob_loot(id);
        }
    }

    /// Créature telle que les écouteurs des plugins la voient : identifiant, sorte (le nom
    /// donné par le plugin pour une créature créée, celui du jeu sinon), position, vie, nom.
    pub(super) fn mob_json(&self, id: i64) -> Json {
        let Some(m) = self.mobs.get(&id) else { return Json::Null };
        let custom = m.custom.and_then(crate::entity::custom::get);
        let n = |v: f32| Json::Num(v as f64);
        Json::Obj(vec![
            ("id".into(), Json::Num(id as f64)),
            ("type".into(), Json::Str(custom.as_ref().map(|def| def.name.clone()).unwrap_or_else(|| m.species.id().to_string()))),
            ("looks".into(), Json::Str(m.species.id().into())),
            ("custom".into(), Json::Bool(custom.is_some())),
            ("x".into(), n(m.pos.0)),
            ("y".into(), n(m.pos.1)),
            ("z".into(), n(m.pos.2)),
            ("health".into(), n(m.health.max(0.0))),
            ("maxHealth".into(), n(m.max_health)),
            ("name".into(), Json::Str(m.label.clone())),
            ("baby".into(), Json::Bool(m.baby())),
        ])
    }

    /// Écouteurs : une créature vient de mourir (son butin tombe juste après).
    fn mob_died(&mut self, id: i64, killer: Option<SocketAddr>) {
        if !self.plugins.listens("CreatureDeathEvent") {
            return;
        }
        let mut fields = vec![("entity", self.mob_json(id))];
        if let Some(a) = killer {
            fields.push(("killer", self.name_json(&a)));
        }
        self.fire("CreatureDeathEvent", fields);
    }

    /// Ce qu'un plugin demande pour une créature : la retirer, la tuer, changer sa vie ou
    /// le nom affiché au-dessus d'elle.
    pub(super) fn mob_order(&mut self, id: i64, verb: &str, value: &str) -> Result<(), String> {
        if !self.mobs.contains_key(&id) {
            return Err("cette créature n'existe plus".into());
        }
        match verb {
            "remove" => self.remove_mob(id),
            "kill" => {
                let at = self.mobs.get(&id).map(|m| m.pos).unwrap_or_default();
                if let Some(m) = self.mobs.get_mut(&id) {
                    m.hurt_tick = 0;
                }
                self.hit_mob(id, at, 1.0e6, Attacker::Mob(id));
            }
            "health" => {
                let health = value.parse::<f32>().ok().filter(|v| v.is_finite() && *v > 0.0).ok_or("setHealth attend un nombre positif")?;
                if let Some(m) = self.mobs.get_mut(&id) {
                    m.health = health.min(m.max_health);
                    let (now, max) = (m.health, m.max_health);
                    self.send_to_viewers(id, bedrock::ID_UPDATE_ATTRIBUTES, &bedrock::health_attribute(id, now, max));
                }
            }
            "name" => {
                let label: String = value.chars().filter(|c| !c.is_control()).take(64).collect();
                if let Some(m) = self.mobs.get_mut(&id) {
                    m.label = label.clone();
                    m.label_always = true;
                }
                // Ceux qui la voient déjà reçoivent le nouveau nom (données 4 et 81) ; les
                // autres l'auront avec la créature
                let packet = bedrock::set_actor_data(id, &[(4u32, bedrock::Meta::Text(label)), (81, bedrock::Meta::Byte(1))]);
                self.send_to_viewers(id, bedrock::ID_SET_ACTOR_DATA, &packet);
                self.mobs_dirty = true;
            }
            _ => return Err(format!("ordre inconnu « {} »", verb)),
        }
        Ok(())
    }

    pub(super) fn hit_mob(&mut self, id: i64, from: (f32, f32, f32), damage: f32, by: Attacker) -> bool {
        let tick = self.tick;
        // Écouteurs : le coup peut être annulé, ou ses dégâts changés
        let mut damage = damage;
        if self.plugins.listens("CreatureDamageEvent") {
            let lands = self.mobs.get(&id).map(|m| m.dead.is_none() && tick.saturating_sub(m.hurt_tick) >= 10 && (m.pos.0 - from.0).powi(2) + (m.pos.2 - from.2).powi(2) <= 36.0).unwrap_or(false);
            if lands {
                let mut fields = vec![("entity", self.mob_json(id)), ("damage", Json::Num(damage as f64)), ("cause", Json::Str(if matches!(by, Attacker::Player(_)) { "PLAYER_ATTACK" } else { "CREATURE_ATTACK" }.into()))];
                if let Attacker::Player(a) = by {
                    fields.push(("damager", self.name_json(&a)));
                }
                let out = self.fire("CreatureDamageEvent", fields);
                if super::cancelled(&out) {
                    return true;
                }
                if let Some(n) = out.as_ref().and_then(|v| v.get("damage")).and_then(|n| n.as_f64()) {
                    damage = n.clamp(0.0, 1.0e6) as f32;
                }
            }
        }
        // Enderman : une fois sur deux, le coup le fait se téléporter à quelques blocs
        let mut blink = None;
        if self.mobs.get(&id).map(|m| m.species == Species::Enderman).unwrap_or(false) && self.mob_rng.below(2) == 0 {
            let at = self.mobs.get(&id).map(|m| m.pos).unwrap_or(from);
            for _ in 0..12 {
                let (x, z) = (at.0.floor() as i32 + self.mob_rng.below(17) as i32 - 8, at.2.floor() as i32 + self.mob_rng.below(17) as i32 - 8);
                let far = (x as f32 - at.0).abs() + (z as f32 - at.2).abs() >= 4.0;
                if let Some(y) = self.surface_y(x, z, at.1.floor() as i32).filter(|y| far && self.mob_free(x, y + 2, z)) {
                    blink = Some((x as f32 + 0.5, y as f32, z as f32 + 0.5));
                    break;
                }
            }
        }
        let Some(m) = self.mobs.get_mut(&id) else { return false };
        if m.dead.is_some() || tick.saturating_sub(m.hurt_tick) < 10 {
            return true;
        }
        let (dx, dz) = (m.pos.0 - from.0, m.pos.2 - from.2);
        if dx * dx + dz * dz > 6.0 * 6.0 {
            return true;
        }
        m.hurt_tick = tick;
        // (le butin ne compte que si c'est le coup d'un joueur qui la tue)
        if !matches!(by, Attacker::Player(_)) {
            m.looting = 0;
        }
        // (son armure naturelle absorbe une part du coup, comme celle d'un joueur)
        m.health -= damage * crate::item::armor::armor_factor(m.species.armor());
        // Le recul, moins la résistance de l'espèce (« knockback_resistance » des fiches du
        // jeu : le golem de fer et le warden ne bougent pas)
        let resist = m.species.knockback_resistance();
        if resist < 1.0 {
            let flat = (dx * dx + dz * dz).sqrt().max(0.01);
            let k = 1.0 - resist;
            m.vel = (dx / flat * 0.4 * k, 0.36 * k, dz / flat * 0.4 * k);
            m.knock = 8;
            m.on_ground = false;
        }
        if let Some(p) = blink.filter(|_| m.health > 0.0) {
            m.pos = p;
            m.vel = (0.0, 0.0, 0.0);
            m.knock = 0;
            m.sent = (f32::MAX, 0.0, 0.0);
        }
        // (un monstre ne fuit pas : il se retourne contre le joueur qui l'a frappé)
        let monster = m.hostile().is_some();
        m.panic = if monster || matches!(m.species, Species::Wolf | Species::IronGolem) { 0 } else { 100 };
        if m.species == Species::Warden {
            // (frappé, le warden se fâche aussitôt contre qui l'a frappé)
            let who = match by {
                Attacker::Player(a) => Some(Prey::Player(a)),
                Attacker::Mob(o) if o != id => Some(Prey::Mob(o)),
                _ => None,
            };
            if let Some(who) = who {
                warden_anger(m, who, 100);
            }
        } else if let (true, Attacker::Player(a)) = (monster, by) {
            m.attack = Some(Prey::Player(a));
            m.refresh |= m.species == Species::Vindicator;
        }
        // Un loup assis qu'on frappe se relève, un villageois endormi se réveille
        m.sitting = false;
        if m.asleep.take().is_some() {
            m.refresh = true;
        }
        m.target = None;
        m.love = 0;
        let dead = m.health <= 0.0;
        let (species, wolf_owner, health, max_health) = (m.species, m.owner.clone(), m.health, m.max_health);
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
            // (jamais sur un joueur en créatif ou spectateur, comme dans le jeu)
            let attacker = attacker.filter(|(_, c)| matches!(c.mode, GameMode::Survival | GameMode::Adventure));
            if let (Some((a, _)), Some(m)) = (attacker.map(|(a, c)| (*a, c.id)), self.mobs.get_mut(&id)) {
                m.angry = Some((a, 10));
                m.panic = 0;
            }
        }
        // Loups : celui qu'on frappe se retourne contre l'agresseur (pas contre son maître),
        // les loups sauvages voisins avec lui ; et les loups d'un joueur attaquent ce qu'il frappe
        let attacker_name = match by {
            Attacker::Player(a) => self.clients.get(&a).and_then(|c| c.name.clone()),
            _ => None,
        };
        if species == Species::Wolf && !dead {
            let own = attacker_name.is_some() && attacker_name == wolf_owner;
            let prey = match by {
                Attacker::Player(a) if !own && self.clients.get(&a).map(|c| c.mode.takes_damage()).unwrap_or(false) => Some(Prey::Player(a)),
                Attacker::Mob(other) if other != id => Some(Prey::Mob(other)),
                _ => None,
            };
            if let Some(prey) = prey {
                let wild = wolf_owner.is_none();
                let at = self.mobs.get(&id).map(|m| m.pos).unwrap_or(from);
                let mut angry = Vec::new();
                for w in self.mobs.values_mut() {
                    let pack = wild && w.species == Species::Wolf && !w.tamed && !w.baby() && w.dead.is_none() && (w.pos.0 - at.0).powi(2) + (w.pos.2 - at.2).powi(2) < 10.0 * 10.0;
                    if (w.id == id && !w.sitting) || (pack && w.id != id) {
                        if Prey::Mob(w.id) != prey && w.attack != Some(prey) {
                            w.attack = Some(prey);
                            angry.push(w.id);
                        }
                    }
                }
                for w in angry {
                    self.mob_changed(w);
                }
            }
            self.send_to_viewers(id, bedrock::ID_UPDATE_ATTRIBUTES, &bedrock::health_attribute(id, health, max_health));
            self.mob_changed(id);
        }
        if let Some(name) = attacker_name {
            self.rally_wolves(&name, Prey::Mob(id), from);
        }
        // Golem blessé : ses fissures apparaissent chez les joueurs
        if species == Species::IronGolem && !dead {
            self.send_to_viewers(id, bedrock::ID_UPDATE_ATTRIBUTES, &bedrock::health_attribute(id, health, max_health));
        }
        // Villageois ou golem frappé par un joueur : les golems du village s'en mêlent
        if let (Attacker::Player(a), Species::Villager | Species::IronGolem) = (by, species) {
            self.rally_golems(a, from);
        }
        // Piglins, piglins zombifiés, hoglins, ours, abeilles, dauphins : ses semblables
        // voisins prennent sa défense (« broadcast_anger » des fiches du jeu)
        if let (Attacker::Player(a), Some(range)) = (by, species.anger_range()) {
            let at = self.mobs.get(&id).map(|m| m.pos).unwrap_or(from);
            let fair = self.clients.get(&a).map(|c| c.mode.takes_damage()).unwrap_or(false);
            if fair {
                for o in self.mobs.values_mut() {
                    let kin = o.species == species || (species == Species::Piglin && o.species == Species::PiglinBrute);
                    if kin && o.id != id && o.dead.is_none() && o.attack.is_none() && (o.pos.0 - at.0).powi(2) + (o.pos.2 - at.2).powi(2) < range * range && (o.species != Species::Bee || o.barter == 0) {
                        o.attack = Some(Prey::Player(a));
                        o.panic = 0;
                    }
                }
            }
        }
        self.mobs_dirty = true;
        self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_HURT, 0));
        if dead {
            if let Some(r) = rider {
                self.dismount(&r);
            }
            self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_DEATH, 0));
            self.mob_died(id, match by {
                Attacker::Player(a) => Some(a),
                _ => None,
            });
            // Slime : un gros se divise en deux à quatre plus petits ; seuls les petits laissent du butin
            let split = self.mobs.get(&id).filter(|m| m.species.cube() && m.variant > 1 && m.custom.is_none()).map(|m| (m.pos, m.variant / 2, m.species));
            if let Some((at, size, kind)) = split {
                for _ in 0..2 + self.mob_rng.below(3) {
                    if self.mobs.len() >= MAX_MOBS {
                        break;
                    }
                    let spot = (at.0 + (self.mob_rng.unit() as f32 - 0.5), at.1 + 0.2, at.2 + (self.mob_rng.unit() as f32 - 0.5));
                    let child = self.add_mob(kind, spot);
                    if let Some(c) = self.mobs.get_mut(&child) {
                        c.variant = size;
                        c.max_health = (size * size) as f32;
                        c.health = c.max_health;
                        // (ils reprennent le combat de leur parent, sans attendre de repérer le joueur)
                        if let Attacker::Player(a) = by {
                            c.attack = Some(Prey::Player(a));
                        }
                    }
                }
                // (le cube de magma laisse sa crème quand il est gros, pas une fois tout petit)
                if kind == Species::MagmaCube {
                    self.mob_loot(id);
                }
            } else if self.mobs.get(&id).map(|m| m.species != Species::MagmaCube || m.custom.is_some()).unwrap_or(true) {
                self.mob_loot(id);
            }
            // Tuée par un joueur : un peu d'expérience (rien pour un petit)
            if let Attacker::Player(a) = by {
                if !self.mobs.get(&id).map(|m| m.baby()).unwrap_or(true) {
                    // (celle de sa fiche pour une créature créée par un plugin)
                    let (lo, hi) = self.mobs.get(&id).and_then(|m| m.custom).and_then(crate::entity::custom::get).and_then(|def| def.xp).unwrap_or((1, 3));
                    let xp = self.xp_between(lo, hi);
                    let _ = a;
                    if let Some(at) = self.mobs.get(&id).map(|m| m.pos) {
                        self.drop_xp((at.0, at.1 + 0.5, at.2), xp);
                    }
                }
            }
        }
        // Aucun son envoyé : le jeu joue lui-même les cris, les bruits de coup et de mort
        true
    }

    /// Niveau (101), niveau le plus haut (102) et expérience (103) d'un villageois, tels que
    /// le jeu les lit dans ses données d'entité : c'est d'eux qu'il tire le titre de l'écran,
    /// la barre d'expérience et les offres encore verrouillées.
    fn trade_meta(m: &Mob) -> [(u32, bedrock::Meta); 3] {
        [(101, bedrock::Meta::Int(trade_tier(m.trade_xp))), (102, bedrock::Meta::Int(4)), (103, bedrock::Meta::Int(m.trade_xp))]
    }

    /// Toutes les offres d'un villageois (le jeu verrouille lui-même celles des niveaux qu'il
    /// n'a pas encore), avec leur usure et l'expérience que chacune
    /// lui rapporte. Tirées une fois pour toutes d'après sa maison et son métier : les mêmes
    /// à chaque ouverture, et après un redémarrage.
    fn villager_offers(&self, id: i64) -> Vec<(bedrock::TradeOffer, i32)> {
        let Some(m) = self.mobs.get(&id) else { return Vec::new() };
        let home = m.home.unwrap_or(m.pos);
        let seed = (home.0.floor() as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (home.2.floor() as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (home.1.floor() as i64 as u64).rotate_left(17) ^ m.variant as u64;
        job_offers(m.variant, seed)
            .into_iter()
            // (un objet inconnu de cette version du jeu : l'offre est laissée de côté)
            .filter(|o| self.item_id(o.buy_a.0).is_some() && self.item_id(o.sell.0).is_some() && o.buy_b.map(|b| self.item_id(b.0).is_some()).unwrap_or(true))
            .enumerate()
            .map(|(i, o)| {
                let offer = bedrock::TradeOffer {
                    buy_a: (o.buy_a.0.to_string(), o.buy_a.1),
                    buy_b: o.buy_b.map(|b| (b.0.to_string(), b.1)),
                    sell: (o.sell.0.to_string(), o.sell.1),
                    max_uses: o.max_uses,
                    uses: m.trade_uses.get(i).copied().unwrap_or(0),
                    net_id: TRADE_NET_BASE + i as i32,
                    tier: o.tier,
                    trader_exp: o.xp(),
                };
                (offer, o.xp())
            })
            .collect()
    }

    /// Ouvre l'écran d'échange d'un villageois qui a un métier. Vrai si l'écran s'ouvre.
    fn open_trade(&mut self, addr: &SocketAddr, id: i64) -> bool {
        if !self.cfg.vanilla.trading {
            return false;
        }
        let Some(title) = self.mobs.get(&id).filter(|m| m.species == Species::Villager && !m.baby() && m.dead.is_none() && m.asleep.is_none()).and_then(|m| villager_title(m.variant)) else { return false };
        if self.refused_extras.contains(&bedrock::ID_UPDATE_TRADE) || self.clients.get(addr).map(|c| c.inv.open_window != 0).unwrap_or(true) {
            return false;
        }
        // Un seul joueur à la fois devant un même villageois
        if self.clients.values().any(|c| c.inv.trade_mob == Some(id)) {
            return false;
        }
        // Avant d'ouvrir : le réassort s'il a eu lieu depuis la dernière fois (deux fois par
        // jour, aux heures de travail)
        let restocks = restocks_before(self.world_time());
        if let Some(m) = self.mobs.get_mut(&id) {
            // (un compte différent, pas seulement plus grand : si l'heure du monde a reculé —
            // commande time, monde d'avant l'enregistrement de l'heure — le villageois se
            // réassortit plutôt que d'attendre de la rattraper)
            if m.trade_restock >= 0 && restocks != m.trade_restock {
                m.trade_uses.clear();
            }
            m.trade_restock = restocks;
        }
        let offers: Vec<bedrock::TradeOffer> = self.villager_offers(id).into_iter().map(|(o, _)| o).collect();
        if offers.is_empty() {
            return false;
        }
        const WINDOW: u8 = 3;
        let player = match self.clients.get_mut(addr) {
            Some(c) => {
                c.inv.open_window = WINDOW;
                c.inv.trade_mob = Some(id);
                c.id
            }
            None => return false,
        };
        let (tier, meta) = match self.mobs.get_mut(&id) {
            Some(m) => {
                m.target = None;
                (trade_tier(m.trade_xp), Self::trade_meta(m))
            }
            None => return false,
        };
        if self.cfg.debug {
            if let Some(m) = self.mobs.get(&id) {
                info!("{} : échange avec le villageois {} — métier {}, expérience {}, niveau {}, usure {:?}, {} offres", addr, id, m.variant, m.trade_xp, tier, m.trade_uses, offers.len());
            }
        }
        // Niveau du villageois, niveau le plus haut, expérience : l'en-tête de l'écran, et
        // l'insigne à sa ceinture pour tous ceux qui le voient
        self.send_to_viewers(id, bedrock::ID_SET_ACTOR_DATA, &bedrock::set_actor_data(id, &meta));
        // Avec qui il échange (donnée 68)
        self.send(addr, bedrock::ID_SET_ACTOR_DATA, &bedrock::set_actor_data(id, &[(68, bedrock::Meta::Long(player))]));
        self.send(addr, bedrock::ID_UPDATE_TRADE, &bedrock::update_trade(WINDOW, tier, id, player, title, &offers));
        self.mob_sound(id, bedrock::SOUND_HAGGLE, None);
        true
    }

    /// Échange demandé par le jeu : l'offre du villageois ouvert qui porte ce numéro, si
    /// elle n'est pas épuisée. Les quantités sont celles de tous les échanges d'affilée.
    pub(super) fn check_trade(&self, addr: &SocketAddr, request: &bedrock::StackRequest) -> Option<TradeDeal> {
        let mob = self.clients.get(addr)?.inv.trade_mob?;
        let (recipe, times) = request.actions.iter().find_map(|a| match a {
            bedrock::StackAction::CraftRecipe { recipe, times } => Some((*recipe as i64, (*times).max(1) as i32)),
            _ => None,
        })?;
        let index = usize::try_from(recipe - TRADE_NET_BASE as i64).ok()?;
        let offers = self.villager_offers(mob);
        let (o, xp) = offers.get(index)?;
        // Épuisée, ou d'un niveau que le villageois n'a pas encore : refus
        let tier = self.mobs.get(&mob).map(|m| trade_tier(m.trade_xp))?;
        if o.uses + times > o.max_uses || o.tier > tier {
            return None;
        }
        let total = |count: u8| u8::try_from(count as i32 * times).ok().filter(|t| *t <= 64);
        Some(TradeDeal {
            mob,
            index,
            times,
            xp: xp * times,
            a: (self.item_id(&o.buy_a.0)?, total(o.buy_a.1)?),
            b: match &o.buy_b {
                Some(b) => Some((self.item_id(&b.0)?, total(b.1)?)),
                None => None,
            },
            sell: (self.item_id(&o.sell.0)?, total(o.sell.1)?),
        })
    }

    /// Échange conclu : l'offre s'use, le villageois gagne de l'expérience. Sa barre avance
    /// et, au niveau suivant, ses nouvelles offres se déverrouillent dans l'écran ouvert ;
    /// son insigne change pour tous ceux qui le voient.
    pub(super) fn trade_done(&mut self, deal: &TradeDeal) {
        let meta = match self.mobs.get_mut(&deal.mob) {
            Some(m) => {
                if m.trade_uses.len() <= deal.index {
                    m.trade_uses.resize(deal.index + 1, 0);
                }
                m.trade_uses[deal.index] += deal.times;
                m.trade_xp = (m.trade_xp + deal.xp).min(TIER_XP[4]);
                Self::trade_meta(m)
            }
            None => return,
        };
        self.mobs_dirty = true;
        self.send_to_viewers(deal.mob, bedrock::ID_SET_ACTOR_DATA, &bedrock::set_actor_data(deal.mob, &meta));
        self.mob_sound(deal.mob, bedrock::SOUND_HAGGLE_YES, None);
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
        // Villageois qui a un métier : l'écran d'échange
        if m.species == Species::Villager && !riding {
            self.open_trade(addr, id);
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
        // Perroquet : le cookie l'empoisonne, comme dans le jeu
        if m.species == Species::Parrot && held == "cookie" {
            if !creative {
                if let Some(c) = self.clients.get_mut(addr) {
                    c.inv.consume_held();
                }
                self.send_inventory(addr);
                self.show_held(addr);
            }
            self.thorned.push((id, 1.0e6));
            return true;
        }
        // Seau d'eau tendu à un poisson, un axolotl ou un têtard : il entre dans le seau
        if held == "water_bucket" && m.rider.is_none() {
            if let Some(bucket) = m.species.bucket() {
                self.remove_mob(id);
                if creative {
                    // (en créatif, le seau rempli s'ajoute à l'inventaire)
                    if let Some(item) = self.item_id(bucket) {
                        if let Some(c) = self.clients.get_mut(addr) {
                            c.inv.add(item, 1);
                        }
                        self.send_inventory(addr);
                    }
                } else {
                    swap(self, bucket);
                }
                self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("bucket.fill.fish", m.pos, "", false, -1));
                return true;
            }
        }
        // Brosse sur un tatou : une écaille (la brosse s'use de 16, comme dans le jeu)
        if m.species == Species::Armadillo && held == "brush" && !m.baby() {
            if let Some(scute) = self.item_id("armadillo_scute") {
                self.drop_item((m.pos.0, m.pos.1 + 0.5, m.pos.2), (0.0, 0.2, 0.0), scute, 1, 10);
            }
            if !creative {
                let max = self.data.as_ref().and_then(|d| d.item_by_id(self.clients.get(addr).map(|c| c.inv.held().id).unwrap_or(0))).map(|def| crate::item::VanillaItems::get(&def.name).max_durability().min(u16::MAX as u32) as u16).unwrap_or(0);
                if let Some(c) = self.clients.get_mut(addr) {
                    let slot = (c.inv.selected as usize).min(8);
                    let stack = &mut c.inv.main.slots[slot];
                    stack.damage = stack.damage.saturating_add(16);
                    if max > 0 && stack.damage >= max {
                        *stack = crate::inventory::ItemStack::EMPTY;
                    }
                    c.inv_touched = true;
                }
                self.send_inventory(addr);
                self.show_held(addr);
            }
            return true;
        }
        // Allay : on lui confie l'objet tenu ; il rapportera les pareils (main vide : il le rend)
        if m.species == Species::Allay {
            let player = self.clients.get(addr).and_then(|c| c.name.clone());
            if held.is_empty() {
                if let Some((name, n)) = m.items.first().cloned().flatten() {
                    if let Some(it) = self.item_id(&name) {
                        self.drop_item(m.pos, (0.0, 0.2, 0.0), it, n.max(1), 10);
                    }
                    if let Some(a) = self.mobs.get_mut(&id) {
                        a.items.clear();
                        a.owner = None;
                    }
                    return true;
                }
                return false;
            }
            if m.items.iter().all(|i| i.is_none()) {
                if let Some(a) = self.mobs.get_mut(&id) {
                    a.items = vec![Some((held.clone(), 1))];
                    a.owner = player;
                }
                if let Some(it) = self.item_id(&held) {
                    self.send_to_viewers(id, bedrock::ID_MOB_EQUIPMENT, &bedrock::mob_equipment(id, (it, 1, 0), 0));
                }
                if !creative {
                    if let Some(c) = self.clients.get_mut(addr) {
                        c.inv.consume_held();
                    }
                    self.send_inventory(addr);
                    self.show_held(addr);
                }
                return true;
            }
        }
        // Piglin adulte : il prend le lingot d'or, l'admire six secondes, puis rend son troc
        if m.species == Species::Piglin && held == "gold_ingot" && !m.baby() && m.barter == 0 && m.attack.is_none() {
            if let Some(p) = self.mobs.get_mut(&id) {
                p.barter = BARTER_TICKS;
                p.target = None;
            }
            if let Some(gold) = self.item_id("gold_ingot") {
                self.send_to_viewers(id, bedrock::ID_MOB_EQUIPMENT, &bedrock::mob_equipment(id, (gold, 1, 0), 0));
            }
            if !creative {
                if let Some(c) = self.clients.get_mut(addr) {
                    c.inv.consume_held();
                }
                self.send_inventory(addr);
                self.show_held(addr);
            }
            return true;
        }
        let grow = m.species.grow_items().iter().find(|(name, _)| *name == held).map(|(_, g)| *g);
        let heal = m.species.heal_items().iter().find(|(name, _)| *name == held).map(|(_, h)| *h);
        let breeds = m.species.breed_items().contains(&held.as_str()) && (!m.species.tameable() || m.tamed) && (m.species != Species::Wolf || m.health >= m.max_health);
        if m.species.pet() {
            let name = self.clients.get(addr).and_then(|c| c.name.clone());
            let mine = m.tamed && name.is_some() && m.owner == name;
            if !m.tamed && m.species.taming_items().contains(&held.as_str()) && !matches!(m.attack, Some(Prey::Player(_))) {
                // Un os : une chance sur trois de l'apprivoiser ; il s'assoit, collier rouge
                let tamed = self.mob_rng.below(3) == 0;
                let player = self.clients.get(addr).map(|c| c.id).unwrap_or(-1);
                if tamed {
                    if let Some(w) = self.mobs.get_mut(&id) {
                        w.tamed = true;
                        w.owner = name;
                        w.owner_id = player;
                        w.sitting = true;
                        w.attack = None;
                        w.target = None;
                        w.color = 14;
                        if w.species == Species::Wolf {
                            w.max_health = WOLF_TAMED_HEALTH;
                            w.health = WOLF_TAMED_HEALTH;
                        }
                    }
                    if m.species == Species::Wolf {
                        self.send_to_viewers(id, bedrock::ID_UPDATE_ATTRIBUTES, &bedrock::health_attribute(id, WOLF_TAMED_HEALTH, WOLF_TAMED_HEALTH));
                    }
                    self.mob_changed(id);
                }
                self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, if tamed { bedrock::EVENT_TAME_SUCCESS } else { bedrock::EVENT_TAME_FAIL }, 0));
                if !creative {
                    if let Some(c) = self.clients.get_mut(addr) {
                        c.inv.consume_held();
                    }
                    self.send_inventory(addr);
                    self.show_held(addr);
                }
                return true;
            }
            if mine {
                if let Some(color) = dye_color(&held) {
                    // Teinture : la couleur du collier
                    if color != m.color {
                        if let Some(w) = self.mobs.get_mut(&id) {
                            w.color = color;
                        }
                        self.mob_changed(id);
                        if !creative {
                            if let Some(c) = self.clients.get_mut(addr) {
                                c.inv.consume_held();
                            }
                            self.send_inventory(addr);
                            self.show_held(addr);
                        }
                    }
                    return true;
                }
                let meal = heal.is_some() && (m.health < m.max_health || m.baby() || (breeds && m.age == 0 && m.love == 0));
                if !meal {
                    // Main vide ou autre objet : assis, debout
                    if let Some(w) = self.mobs.get_mut(&id) {
                        w.sitting = !w.sitting;
                        w.attack = None;
                        w.target = None;
                        w.following = false;
                        w.vel.0 = 0.0;
                        w.vel.2 = 0.0;
                    }
                    self.mob_changed(id);
                    return true;
                }
            } else if !m.tamed || !mine {
                // Loup sauvage ou celui d'un autre : rien d'autre à faire qu'un os
                if held != "lead" {
                    return true;
                }
            }
        }
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
        } else if held == "saddle" && !m.saddled && !m.baby() && ((m.tamed && m.species.steerable()) || m.species.stick().is_some()) {
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
            // Golem réparé : ses fissures se referment chez les joueurs
            if m.species == Species::IronGolem {
                if let Some((h, max)) = self.mobs.get(&id).map(|s| (s.health, s.max_health)) {
                    self.send_to_viewers(id, bedrock::ID_UPDATE_ATTRIBUTES, &bedrock::health_attribute(id, h, max));
                }
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
        } else if (m.species.mount() || (m.species.stick().is_some() && m.saddled)) && !m.baby() && m.rider.is_none() && !riding && !m.species.breed_items().contains(&held.as_str()) {
            // Main vide (ou un objet sans usage ici) : en selle
            self.mount_mob(addr, id);
        }
        // Un repas donné : elle mange
        let food = grow.is_some() || heal.is_some() || breeds || temper.is_some() || FLOWERS.contains(&held.as_str());
        if used && food {
            self.mob_sound(id, bedrock::SOUND_EAT, None);
            // La queue du loup remonte avec sa vie
            if let Some(w) = self.mobs.get(&id).filter(|w| w.species == Species::Wolf) {
                let packet = bedrock::health_attribute(id, w.health, w.max_health);
                self.send_to_viewers(id, bedrock::ID_UPDATE_ATTRIBUTES, &packet);
            }
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

    /// Habitudes propres à quelques espèces, chaque tick (le plus lourd une fois par seconde,
    /// décalé d'une créature à l'autre) :
    ///
    /// - golem de neige : il lance des boules de neige aux monstres (10 blocs), laisse une
    ///   traînée de neige là où il fait froid, et fond dans les biomes chauds ;
    /// - poisson-globe : il empoisonne qui le frôle ;
    /// - axolotl : il chasse poissons, calamars et noyés dans l'eau ;
    /// - allay : il rapporte à son joueur les objets pareils à celui qu'on lui a confié ;
    /// - hoglin : il fuit le champignon biscornu.
    fn special_habits(&mut self, id: i64) {
        let Some(m) = self.mobs.get(&id).cloned() else { return };
        let second = (self.tick + id.unsigned_abs()) % 20 == 0;
        let near2 = |p: (f32, f32, f32)| (p.0 - m.pos.0).powi(2) + (p.1 - m.pos.1).powi(2) + (p.2 - m.pos.2).powi(2);
        match m.species {
            Species::SnowGolem => {
                let (x, y, z) = (m.pos.0.floor() as i32, m.pos.1.floor() as i32, m.pos.2.floor() as i32);
                let biome = self.world.generator().vanilla().map(|v| v.surface_biome_at(x, z));
                let hot = matches!(biome, Some(B::Desert | B::Savanna | B::SavannaPlateau | B::WindsweptSavanna | B::Badlands | B::WoodedBadlands | B::ErodedBadlands | B::Jungle | B::SparseJungle | B::BambooJungle | B::MangroveSwamp)) || self.other_dimension();
                if second && hot {
                    self.hit_mob(id, m.pos, 1.0, Attacker::Mob(id));
                    return;
                }
                // La traînée de neige (une couche, sur un sol plein, là où il fait froid)
                if !hot && m.on_ground && self.mob_get(x, y, z) == self.world.air && !self.mob_free(x, y - 1, z) {
                    let below = self.block_name_of(self.mob_get(x, y - 1, z)).unwrap_or_default();
                    if !below.contains("snow") && !below.contains("water") && !below.contains("leaves") {
                        if let Some(snow) = self.data.as_ref().and_then(|d| d.default_state("minecraft:snow_layer")) {
                            self.change_block((x, y, z), snow);
                        }
                    }
                }
                if second {
                    let foe = self.mobs.values().filter(|t| t.dead.is_none() && t.species.hostile().is_some() && !t.species.neutral() && t.species != Species::Creeper && near2(t.pos) < 10.0 * 10.0).min_by(|a, b| near2(a.pos).total_cmp(&near2(b.pos))).map(|t| (t.pos, t.size().1, t.species));
                    if let Some((p, h, kind)) = foe {
                        // (seul le blaze en souffre : 3 points)
                        let damage = if kind == Species::Blaze { 3.0 } else { 0.0 };
                        self.mob_shoots(&m, "minecraft:snowball", (p.0, p.1 + h * 0.6, p.2), damage, "un golem de neige");
                        if let Some(g) = self.mobs.get_mut(&id) {
                            g.yaw = (-(p.0 - m.pos.0)).atan2(p.2 - m.pos.2).to_degrees().rem_euclid(360.0);
                        }
                    }
                }
            }
            Species::Pufferfish if second => {
                let victims: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined && c.dead.is_none() && c.mode.takes_damage() && near2((c.pos.0, c.pos.1 - 1.0, c.pos.2)) < 2.5 * 2.5).map(|(a, _)| *a).collect();
                for a in victims {
                    self.hurt(&a, 1.0, Cause::Mob("un poisson-globe"));
                    self.give_effect(&a, super::effects::POISON, 0, 120, true);
                }
            }
            Species::Axolotl => {
                let water = self.world.generator().blocks.water;
                if self.mob_get(m.pos.0.floor() as i32, m.pos.1.floor() as i32, m.pos.2.floor() as i32) != water {
                    return;
                }
                let prey = self.mobs.values().filter(|t| t.dead.is_none() && matches!(t.species, Species::Cod | Species::Salmon | Species::TropicalFish | Species::Pufferfish | Species::Squid | Species::GlowSquid | Species::Drowned | Species::Tadpole) && near2(t.pos) < 8.0 * 8.0).min_by(|a, b| near2(a.pos).total_cmp(&near2(b.pos))).map(|t| (t.id, t.pos));
                if let Some((pid, p)) = prey {
                    if let Some(a) = self.mobs.get_mut(&id) {
                        a.target = Some((p.0, p.2));
                        a.anchor_y = p.1;
                    }
                    if near2(p) < 1.5 * 1.5 && m.bite_in == 0 {
                        if let Some(a) = self.mobs.get_mut(&id) {
                            a.bite_in = 20;
                        }
                        self.hit_mob(pid, p, 2.0, Attacker::Mob(id));
                    }
                }
                if let Some(a) = self.mobs.get_mut(&id) {
                    a.bite_in = a.bite_in.saturating_sub(1);
                }
            }
            Species::Allay => {
                let Some(owner) = m.owner.clone() else { return };
                let Some((wanted, carried)) = m.items.first().cloned().flatten() else { return };
                let master = self.clients.iter().find(|(_, c)| c.joined && c.dead.is_none() && c.name.as_deref() == Some(owner.as_str())).map(|(a, c)| (*a, (c.pos.0, c.pos.1 - EYE, c.pos.2)));
                let item = self.item_id(&wanted);
                // Il porte déjà ce qu'il a trouvé : il le rapporte et le lâche près du joueur
                if carried > 1 {
                    if let Some((_, p)) = master {
                        if near2(p) < 3.0 * 3.0 {
                            if let Some(it) = item {
                                let d = near2(p).sqrt().max(0.1);
                                self.drop_item(m.pos, ((p.0 - m.pos.0) / d * 0.2, 0.2, (p.2 - m.pos.2) / d * 0.2), it, carried - 1, 40);
                            }
                            if let Some(a) = self.mobs.get_mut(&id) {
                                a.items[0] = Some((wanted.clone(), 1));
                            }
                        } else if let Some(a) = self.mobs.get_mut(&id) {
                            a.target = Some((p.0, p.2));
                            a.anchor_y = p.1 + 1.5;
                        }
                    }
                    return;
                }
                // Sinon il cherche, à 32 blocs, un objet pareil au sol
                let Some(it) = item else { return };
                let found = self.ground.iter().filter(|(_, g)| g.item == it && g.pickup_at <= self.tick && near2(g.body.pos) < 32.0 * 32.0).min_by(|a, b| near2(a.1.body.pos).total_cmp(&near2(b.1.body.pos))).map(|(gid, g)| (*gid, g.body.pos, g.count));
                match found {
                    Some((gid, p, count)) if near2(p) < 1.5 * 1.5 => {
                        self.remove_ground_item(gid, Some(id));
                        if let Some(a) = self.mobs.get_mut(&id) {
                            a.items[0] = Some((wanted.clone(), (carried as u32 + count as u32).min(65) as u8));
                        }
                    }
                    Some((_, p, _)) => {
                        if let Some(a) = self.mobs.get_mut(&id) {
                            a.target = Some((p.0, p.2));
                            a.anchor_y = p.1 + 0.5;
                        }
                    }
                    // (rien à chercher : il reste près de son joueur)
                    None => {
                        if let (Some((_, p)), Some(a)) = (master, self.mobs.get_mut(&id)) {
                            if near2(p) > 6.0 * 6.0 {
                                a.target = Some((p.0, p.2));
                                a.anchor_y = p.1 + 1.5;
                            }
                        }
                    }
                }
            }
            Species::Warden => self.warden_habits(id),
            Species::Hoglin if second => {
                // Le champignon biscornu (et les portails, les ancres de réapparition) à 7
                // blocs : il fuit
                let (x, y, z) = (m.pos.0.floor() as i32, m.pos.1.floor() as i32, m.pos.2.floor() as i32);
                let scary = (-7..=7).any(|dx| (-2..=2).any(|dy| (-7..=7).any(|dz| {
                    let s = self.mob_get(x + dx, y + dy, z + dz);
                    s != self.world.air && self.block_name_of(s).map(|n| n == "minecraft:warped_fungus" || n == "minecraft:portal" || n == "minecraft:respawn_anchor").unwrap_or(false)
                })));
                if scary {
                    if let Some(h) = self.mobs.get_mut(&id) {
                        h.attack = None;
                        h.panic = 60;
                    }
                }
            }
            _ => {}
        }
    }

    /// Ponte d'une tortue : le sable le plus proche (8 blocs, de l'air au-dessus) reçoit de un à
    /// quatre œufs.
    fn lay_turtle_eggs(&mut self, at: (f32, f32, f32)) {
        let Some(sand) = self.data.as_ref().and_then(|d| d.default_state("minecraft:sand")) else { return };
        let air = self.world.air;
        let (bx, by, bz) = (at.0.floor() as i32, at.1.floor() as i32, at.2.floor() as i32);
        let spot = (-8..=8).flat_map(|dx| (-3..=3).flat_map(move |dy| (-8..=8).map(move |dz| (bx + dx, by + dy, bz + dz)))).filter(|&(x, y, z)| self.world.is_ready(x >> 4, z >> 4) && self.mob_get(x, y, z) == sand && self.mob_get(x, y + 1, z) == air && self.mob_get(x, y + 2, z) == air).min_by_key(|&(x, y, z)| (x - bx).pow(2) + (y - by).pow(2) + (z - bz).pow(2));
        let Some((x, y, z)) = spot else { return };
        let count = ["one_egg", "two_egg", "three_egg", "four_egg"][self.mob_rng.below(4) as usize];
        let state = self.data.as_ref().and_then(|d| d.default_state("minecraft:turtle_egg").and_then(|e| d.with_props(e, &[("turtle_egg_count", crate::data::Prop::Str(count.to_string()))])));
        if let Some(egg) = state {
            self.change_block((x, y + 1, z), egg);
        }
    }

    /// Seize crocs alignés de l'évocateur vers sa cible, un tous les 1,25 bloc, chacun un
    /// tick après le précédent (comme dans le jeu).
    fn cast_fangs(&mut self, m: &Mob, to: (f32, f32, f32)) {
        let (dx, dz) = (to.0 - m.pos.0, to.2 - m.pos.2);
        let d = (dx * dx + dz * dz).sqrt().max(0.01);
        self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("cast.spell", m.pos, "minecraft:evocation_illager", false, m.id));
        for i in 0..16u64 {
            let step = 1.25 * (i + 1) as f32;
            let (x, z) = (m.pos.0 + dx / d * step, m.pos.2 + dz / d * step);
            // (posé sur le sol, à deux blocs près de la hauteur de l'évocateur)
            let base = m.pos.1.floor() as i32;
            let Some(y) = (-2..=2).rev().map(|o| base + o).find(|y| self.mob_free(x.floor() as i32, *y, z.floor() as i32) && !self.mob_free(x.floor() as i32, *y - 1, z.floor() as i32)) else { continue };
            let id = self.next_id;
            self.next_id += 1;
            self.fangs.push((self.tick + 1 + i, id, (x, y as f32, z), m.id));
        }
    }

    /// Les crocs : ils apparaissent, mordent au bout d'une seconde (6 points à qui est dessus,
    /// joueur ou créature autre qu'un illageois) et disparaissent.
    pub(super) fn fangs_tick(&mut self) {
        if self.fangs.is_empty() {
            return;
        }
        let tick = self.tick;
        for (at, id, pos, caster) in self.fangs.clone() {
            if tick == at {
                let meta = bedrock::mob_metadata(0.5, 0.8, 0, 0, 0);
                self.send_near(pos, bedrock::ID_ADD_ACTOR, &bedrock::add_actor(id, "minecraft:evocation_fang", pos, (0.0, 0.0, 0.0), 0.0, (1.0, 1.0), None, None, &meta));
            } else if tick == at + 14 {
                self.send_near(pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("fang", pos, "minecraft:evocation_fang", false, -1));
                let near = |p: (f32, f32, f32)| (p.0 - pos.0).powi(2) + (p.2 - pos.2).powi(2) < 0.9 * 0.9 && p.1 > pos.1 - 0.5 && p.1 < pos.1 + 1.0;
                let players: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined && c.dead.is_none() && c.mode.takes_damage() && near((c.pos.0, c.pos.1 - EYE, c.pos.2))).map(|(a, _)| *a).collect();
                for a in players {
                    self.hurt(&a, 6.0, Cause::Magic);
                }
                let mobs: Vec<(i64, (f32, f32, f32))> = self.mobs.values().filter(|t| t.dead.is_none() && !matches!(t.species, Species::Evoker | Species::Vindicator | Species::Pillager | Species::Ravager | Species::Witch) && near(t.pos)).map(|t| (t.id, t.pos)).collect();
                for (t, p) in mobs {
                    self.hit_mob(t, p, 6.0, Attacker::Mob(caster));
                }
            } else if tick >= at + 22 {
                self.send_near(pos, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id));
            }
        }
        self.fangs.retain(|(at, ..)| tick < at + 22);
    }

    /// Un bébé tortue sort de son œuf.
    pub(super) fn hatch_turtle(&mut self, at: (f32, f32, f32)) {
        if self.mobs.len() >= MAX_MOBS {
            return;
        }
        let id = self.add_mob(Species::Turtle, at);
        if let Some(m) = self.mobs.get_mut(&id) {
            m.age = -GROW_TICKS;
        }
    }

    /// Le warden, comme dans le jeu (fiche « warden » de Mojang) :
    ///
    /// - aveugle, il entend : les pas de ce qui bouge à 16 blocs (pas qui avance accroupi),
    ///   les blocs cassés et posés, les projectiles qui retombent ; chaque bruit fâche contre
    ///   son auteur (35 points, 10 pour un projectile) et l'attire vers l'endroit ;
    /// - il flaire toutes les 5 à 10 secondes ce qui vit à 24 blocs (35 points au plus proche) ;
    /// - frappé, 100 points contre l'agresseur ; la colère retombe d'un point par seconde ;
    /// - à 80, il rugit et poursuit le plus haï, joueur ou créature, sans le lâcher ;
    /// - son cœur bat toutes les 2 secondes, jusqu'à 0,5 seconde quand il est furieux.
    fn warden_think(&mut self, m: &mut Mob) -> Option<(f32, f32)> {
        m.bite_in = m.bite_in.saturating_sub(1);
        m.barter = m.barter.saturating_sub(1);
        let attr = m.movement();
        let target = self.warden_target(m);
        let Some((who, p)) = target else {
            m.attack = None;
            m.focus = None;
            // Un bruit à aller voir
            if let Some(spot) = m.investigate {
                if (spot.0 - m.pos.0).powi(2) + (spot.2 - m.pos.2).powi(2) < 2.0 * 2.0 {
                    m.investigate = None;
                    return None;
                }
                m.target = Some((spot.0, spot.2));
                return Some((ground_speed(attr * 0.7), 1.0));
            }
            return None;
        };
        m.attack = Some(who);
        m.investigate = None;
        // Une nouvelle cible : il rugit d'abord
        if m.focus != Some(who) {
            m.focus = Some(who);
            m.best_far = f32::MAX;
            m.stuck = 0;
            m.act = WardenAct::Roaring;
            m.act_ticks = 0;
            m.refresh = true;
            self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("roar", m.pos, "minecraft:warden", false, m.id));
            return Some((0.0, f32::MAX));
        }
        m.target = Some((p.0, p.2));
        let far2 = (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2);
        // Le chemin à pied jusqu'à sa cible (il fait trois blocs de haut), refait chaque
        // seconde ; pas de chemin : il ne peut pas l'atteindre
        let mut unreachable = false;
        // (tout droit seulement si son corps entier passe : 0,9 de large, 2,9 de haut — un
        // tronc ou des feuilles à hauteur de tête le forcent à contourner)
        let direct = far2 < 2.0 * 2.0 || ((p.1 - m.pos.1).abs() < 1.2 && self.body_clear(m.pos, p, 0.9, 2.9));
        if direct {
            m.path.clear();
        } else {
            if self.tick >= m.path_wait {
                m.path = match self.walk_path(m.pos, (p.0, Some(p.1), p.2), 3) {
                    Some(mut path) => {
                        path.reverse();
                        path
                    }
                    None => Vec::new(),
                };
                m.path_wait = self.tick + 20;
            }
            while let Some(&(wx, wz)) = m.path.last() {
                if (wx - m.pos.0).powi(2) + (wz - m.pos.2).powi(2) < 0.35 * 0.35 {
                    m.path.pop();
                } else {
                    break;
                }
            }
            match m.path.last() {
                Some(&next) => m.target = Some(next),
                None => unreachable = true,
            }
        }
        // Au contact : 30 points (« melee_box_attack »)
        if far2 < 2.4 * 2.4 && (p.1 - m.pos.1).abs() < 3.0 && m.bite_in == 0 {
            m.bite_in = 18;
            self.send_to_viewers(m.id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(m.id, bedrock::EVENT_ARM_SWING, 0));
            self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("attack", m.pos, "minecraft:warden", false, m.id));
            match who {
                Prey::Player(a) => self.hurt(&a, 30.0, Cause::Mob("un warden")),
                Prey::Mob(t) => {
                    self.hit_mob(t, m.pos, 30.0, Attacker::Mob(m.id));
                }
            }
        }
        // Se rapproche-t-il encore ? (d'un bloc au moins en trois secondes)
        if far2.sqrt() < m.best_far.sqrt() - 1.0 {
            m.best_far = far2;
            m.stuck = 0;
        } else {
            m.stuck += 1;
        }
        // Coincé (mur, fossé, cible perchée hors d'atteinte), à moins de 15 blocs (20 en
        // hauteur) : l'onde sonique, qui traverse les murs ; deux secondes de recharge après.
        // Tant qu'il gagne du terrain, il marche sur sa cible pour la frapper.
        let perched = (p.1 - m.pos.1).abs() > 3.0;
        if m.barter == 0 && (m.stuck >= 60 || perched || unreachable) && far2 > 3.0 * 3.0 && far2 < 15.0 * 15.0 && (p.1 - m.pos.1).abs() < 20.0 {
            m.stuck = 0;
            m.best_far = far2;
            m.act = WardenAct::SonicBoom;
            m.act_ticks = 0;
            m.refresh = true;
            m.vel.0 = 0.0;
            m.vel.2 = 0.0;
            self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("sonic_charge", m.pos, "", false, -1));
            return Some((0.0, f32::MAX));
        }
        Some((ground_speed(attr * 1.2), if m.path.is_empty() { 1.6 } else { 0.15 }))
    }

    /// Le suspect le plus haï du warden, s'il l'est assez (80) et qu'il est encore là (à 64
    /// blocs, vivant, attaquable) : qui, et où.
    fn warden_target(&self, m: &mut Mob) -> Option<(Prey, (f32, f32, f32))> {
        let mut best: Option<(Prey, (f32, f32, f32), u32)> = None;
        let mut gone = Vec::new();
        for (who, anger) in &m.suspects {
            let at = match who {
                Prey::Player(a) => self.clients.get(a).filter(|c| c.joined && c.dead.is_none() && c.mode.takes_damage()).map(|c| (c.pos.0, c.pos.1 - EYE, c.pos.2)),
                Prey::Mob(t) => self.mobs.get(t).filter(|t| t.dead.is_none()).map(|t| t.pos),
            }
            .filter(|p| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2) < 64.0 * 64.0);
            match at {
                None => gone.push(*who),
                // (à 80 et plus : un joueur passe avant une créature, puis le plus haï)
                Some(p) if *anger >= 80 && best.map(|b| (matches!(who, Prey::Player(_)), *anger) > (matches!(b.0, Prey::Player(_)), b.2)).unwrap_or(true) => best = Some((*who, p, *anger)),
                _ => {}
            }
        }
        m.suspects.retain(|(w, _)| !gone.contains(w));
        best.map(|(w, p, _)| (w, p))
    }

    /// Chaque tick : les bruits du monde parviennent aux wardens à 16 blocs.
    pub(super) fn vibrations_tick(&mut self) {
        let wardens: Vec<(i64, (f32, f32, f32))> = self.mobs.values().filter(|m| m.species == Species::Warden && m.dead.is_none() && !matches!(m.act, WardenAct::Emerging | WardenAct::Digging)).map(|m| (m.id, m.pos)).collect();
        let mut heard = std::mem::take(&mut self.vibrations);
        if wardens.is_empty() {
            return;
        }
        // Une fois par seconde : les pas de ce qui bouge (joueur qui ne se tient pas accroupi,
        // créature qui marche)
        if self.tick % 20 == 0 {
            let tick = self.tick;
            for (_, at) in &wardens {
                let near = |p: (f32, f32, f32)| (p.0 - at.0).powi(2) + (p.1 - at.1).powi(2) + (p.2 - at.2).powi(2) < 16.0 * 16.0;
                for (a, c) in &self.clients {
                    let feet = (c.pos.0, c.pos.1 - EYE, c.pos.2);
                    // (marcher sur la laine ou un tapis ne fait pas de bruit)
                    let under = self.block_name_of(self.mob_get(feet.0.floor() as i32, (feet.1 - 0.05).floor() as i32, feet.2.floor() as i32)).unwrap_or_default();
                    let muffled = under.contains("wool") || under.contains("carpet") || self.block_name_of(self.mob_get(feet.0.floor() as i32, feet.1.floor() as i32, feet.2.floor() as i32)).map(|n| n.contains("carpet")).unwrap_or(false);
                    if c.joined && c.dead.is_none() && c.mode.takes_damage() && !c.sneaking && !muffled && tick.saturating_sub(c.moved_tick) < 20 && near(feet) {
                        heard.push((feet, Some(Prey::Player(*a)), 35));
                    }
                }
                for t in self.mobs.values() {
                    if t.species != Species::Warden && t.dead.is_none() && (t.vel.0.abs() + t.vel.2.abs() > 0.02 || t.vel.1.abs() > 0.1) && near(t.pos) {
                        heard.push((t.pos, Some(Prey::Mob(t.id)), 35));
                    }
                }
            }
        }
        // Le bruit voyage (un bloc par tick) ; la laine l'arrête en chemin
        for (id, at) in &wardens {
            for (pos, who, annoy) in &heard {
                let d2 = (pos.0 - at.0).powi(2) + (pos.1 - at.1).powi(2) + (pos.2 - at.2).powi(2);
                if d2 > 16.0 * 16.0 || self.wool_between(*pos, (at.0, at.1 + 2.0, at.2)) {
                    continue;
                }
                self.heard_later.push((self.tick + d2.sqrt().ceil() as u64, *id, *pos, *who, *annoy));
            }
        }
        let now = self.tick;
        let (arrived, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.heard_later).into_iter().partition(|h| h.0 <= now);
        self.heard_later = waiting;
        for (_, id, pos, who, annoy) in arrived {
            {
                let (pos, who, annoy) = (&pos, &who, &annoy);
                let Some(w) = self.mobs.get_mut(&id) else { continue };
                if matches!(w.act, WardenAct::Emerging | WardenAct::Digging) {
                    continue;
                }
                let before = w.suspects.iter().map(|s| s.1).max().unwrap_or(0);
                if let Some(who) = who {
                    warden_anger(w, *who, *annoy);
                }
                if w.attack.is_none() {
                    w.investigate = Some(*pos);
                }
                let after = w.suspects.iter().map(|s| s.1).max().unwrap_or(0);
                // (le bruit qu'il fait en entendant : « listening », fâché à partir de 40)
                if after > before || who.is_none() {
                    let sound = if after >= 40 { "listening_angry" } else { "listening" };
                    let wp = w.pos;
                    self.send_near(wp, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound(sound, wp, "minecraft:warden", false, id));
                }
            }
        }
        heard.clear();
    }

    /// La laine se trouve-t-elle sur la ligne droite entre ces deux points ? (elle étouffe les
    /// bruits, comme dans le jeu)
    fn wool_between(&self, a: (f32, f32, f32), b: (f32, f32, f32)) -> bool {
        let d = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2) + (b.2 - a.2).powi(2)).sqrt();
        let steps = (d * 2.0).ceil() as i32;
        let mut last = (i32::MIN, 0, 0);
        for i in 1..steps {
            let t = i as f32 / steps as f32;
            let c = ((a.0 + (b.0 - a.0) * t).floor() as i32, (a.1 + (b.1 - a.1) * t).floor() as i32, (a.2 + (b.2 - a.2) * t).floor() as i32);
            if c == last {
                continue;
            }
            last = c;
            let s = self.mob_get(c.0, c.1, c.2);
            if s != self.world.air && self.block_name_of(s).map(|n| n.ends_with("_wool") || n == "minecraft:wool").unwrap_or(false) {
                return true;
            }
        }
        false
    }

    /// Ce que fait le warden chaque tick, en plus de penser : battements de cœur, colère qui
    /// retombe, flair, obscurité, onde sonique, s'enfouir au bout d'une minute d'ennui.
    fn warden_habits(&mut self, id: i64) {
        let Some(m) = self.mobs.get(&id).cloned() else { return };
        let tick = self.tick;
        let prey = match m.attack {
            Some(Prey::Player(a)) => self.clients.get(&a).filter(|c| c.joined && c.dead.is_none() && c.mode.takes_damage()).map(|c| (Prey::Player(a), c.id, (c.pos.0, c.pos.1 - EYE, c.pos.2))),
            Some(Prey::Mob(t)) => self.mobs.get(&t).filter(|t| t.dead.is_none()).map(|t| (Prey::Mob(t.id), t.id, t.pos)),
            None => None,
        };
        let anger = m.suspects.iter().map(|s| s.1).max().unwrap_or(0);
        // Le cœur : toutes les 2 secondes, plus vite avec la colère (0,5 seconde à 80)
        if tick >= m.beat_at && m.act != WardenAct::Emerging {
            let every = 2.0 - (anger as f32 / 80.0 * 1.5).clamp(0.0, 1.5);
            if let Some(w) = self.mobs.get_mut(&id) {
                w.beat_at = tick + (every * 20.0) as u64;
            }
            self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("heartbeat", m.pos, "minecraft:warden", false, id));
        }
        if (tick + id.unsigned_abs()) % 20 == 0 {
            // La colère retombe d'un point par seconde
            if let Some(w) = self.mobs.get_mut(&id) {
                for s in w.suspects.iter_mut() {
                    s.1 = s.1.saturating_sub(1);
                }
                w.suspects.retain(|s| s.1 > 0);
            }
        }
        // Ses grognements, selon sa colère : calme, agité (40), furieux (80)
        if m.act == WardenAct::None && self.mob_rng.below(160) == 0 {
            let sound = match anger {
                0..=39 => "ambient",
                40..=79 => "agitated",
                _ => "angry",
            };
            self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound(sound, m.pos, "minecraft:warden", false, id));
        }
        if (tick + id.unsigned_abs()) % 120 == 0 {
            let near: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.joined && c.dead.is_none() && c.mode.takes_damage() && (c.pos.0 - m.pos.0).powi(2) + (c.pos.1 - m.pos.1).powi(2) + (c.pos.2 - m.pos.2).powi(2) < 20.0 * 20.0).map(|(a, _)| *a).collect();
            for a in near {
                self.give_effect(&a, 30, 0, 260, false);
            }
        }
        let sniff_cooldown = 100 + self.mob_rng.below(100) as u64;
        let mut end = false;
        if let Some(w) = self.mobs.get_mut(&id) {
            if let Some((_, _, p)) = prey {
                if w.act != WardenAct::None {
                    w.yaw = (-(p.0 - w.pos.0)).atan2(p.2 - w.pos.2).to_degrees().rem_euclid(360.0);
                }
            }
            if w.act == WardenAct::None {
                // Sans cible ni soupçon, il s'ennuie ; au bout d'une minute il s'enfouit
                w.dry = if prey.is_some() || !w.suspects.is_empty() || w.investigate.is_some() { 0 } else { w.dry.saturating_add(1) };
                if w.dry >= 1200 && w.custom.is_none() {
                    w.act = WardenAct::Digging;
                    w.act_ticks = 0;
                    w.refresh = true;
                } else if prey.is_none() && tick >= w.sniff_at {
                    // Le flair
                    if w.sniff_at == 0 {
                        w.sniff_at = tick + sniff_cooldown;
                    } else {
                        w.act = WardenAct::Sniffing;
                        w.act_ticks = 0;
                        w.refresh = true;
                    }
                }
            } else {
                w.act_ticks += 1;
                end = w.act_ticks >= w.act.ticks();
            }
        }
        let act = self.mobs.get(&id).map(|w| (w.act, w.act_ticks)).unwrap_or((WardenAct::None, 0));
        if act == (WardenAct::SonicBoom, 34) {
            if let Some((who, pid, p)) = prey {
                let (dx, dz) = (p.0 - m.pos.0, p.2 - m.pos.2);
                let flat = (dx * dx + dz * dz).sqrt();
                if flat < 15.0 && (p.1 - m.pos.1).abs() < 20.0 {
                    self.send_near(m.pos, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound("sonic_boom", m.pos, "", false, -1));
                    // L'onde visible : des anneaux de la poitrine du warden jusqu'à sa cible
                    let from = (m.pos.0, m.pos.1 + 1.6, m.pos.2);
                    let to = (p.0, p.1 + 1.0, p.2);
                    let len = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2) + (to.2 - from.2).powi(2)).sqrt();
                    let rings = (len.ceil() as i32).clamp(1, 20);
                    let dim = self.world.generator().kind.dimension() as u8;
                    for i in 1..=rings {
                        let t = i as f32 / rings as f32;
                        let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t, from.2 + (to.2 - from.2) * t);
                        if !self.refused_extras.contains(&bedrock::ID_SPAWN_PARTICLE_EFFECT) {
                            self.send_near(at, bedrock::ID_SPAWN_PARTICLE_EFFECT, &bedrock::spawn_particle_effect(dim, at, "minecraft:sonic_explosion"));
                        }
                    }
                    let d = flat.max(0.01);
                    match who {
                        Prey::Player(a) => {
                            self.hurt(&a, 10.0, Cause::Magic);
                            if !self.refused_extras.contains(&bedrock::ID_SET_ACTOR_MOTION) {
                                self.send(&a, bedrock::ID_SET_ACTOR_MOTION, &bedrock::set_actor_motion(pid, (dx / d * 2.5, 0.5, dz / d * 2.5)));
                            }
                        }
                        Prey::Mob(t) => {
                            self.hit_mob(t, m.pos, 10.0, Attacker::Mob(id));
                            if let Some(v) = self.mobs.get_mut(&t) {
                                v.vel = (dx / d * 2.5, 0.5, dz / d * 2.5);
                            }
                        }
                    }
                }
            }
        }
        if end {
            if act.0 == WardenAct::Sniffing {
                // Ce qu'il a flairé : le plus proche à 24 blocs (pas un autre warden)
                let near2 = |p: (f32, f32, f32)| (p.0 - m.pos.0).powi(2) + (p.1 - m.pos.1).powi(2) + (p.2 - m.pos.2).powi(2);
                let player = self.clients.iter().filter(|(_, c)| c.joined && c.dead.is_none() && c.mode.takes_damage()).map(|(a, c)| (Prey::Player(*a), near2((c.pos.0, c.pos.1 - EYE, c.pos.2)), (c.pos.0, c.pos.1 - EYE, c.pos.2)));
                let mobs = self.mobs.values().filter(|t| t.dead.is_none() && t.species != Species::Warden && !t.species.ambient()).map(|t| (Prey::Mob(t.id), near2(t.pos), t.pos));
                let smelled = player.chain(mobs).filter(|s| s.1 < 24.0 * 24.0).min_by(|a, b| a.1.total_cmp(&b.1));
                if let (Some((who, _, at)), Some(w)) = (smelled, self.mobs.get_mut(&id)) {
                    warden_anger(w, who, 35);
                    w.investigate = Some(at);
                }
                if let Some(w) = self.mobs.get_mut(&id) {
                    w.sniff_at = tick + sniff_cooldown;
                }
            }
            if let Some(w) = self.mobs.get_mut(&id) {
                if w.act == WardenAct::SonicBoom {
                    w.barter = 40;
                }
                w.act = WardenAct::None;
                w.act_ticks = 0;
            }
            self.mob_changed(id);
        } else if self.mobs.get(&id).map(|w| w.refresh).unwrap_or(false) {
            if let Some(w) = self.mobs.get_mut(&id) {
                w.refresh = false;
            }
            self.mob_changed(id);
        }
        if end && act.0 == WardenAct::Digging {
            self.remove_mob(id);
        }
    }

    /// Fin du troc : un lot tiré dans la table du jeu tombe devant le piglin, qui reprend son épée.
    fn piglin_barter(&mut self, m: &Mob) {
        let total: u32 = BARTER.iter().map(|b| b.0).sum();
        let mut roll = self.mob_rng.below(total);
        let (_, item, lo, hi) = *BARTER.iter().find(|b| {
            if roll < b.0 {
                true
            } else {
                roll -= b.0;
                false
            }
        }).unwrap_or(&BARTER[0]);
        let count = lo + self.mob_rng.below((hi - lo) as u32 + 1) as u8;
        if let Some(id) = self.item_id(item) {
            let (dx, dz) = (-m.yaw.to_radians().sin(), m.yaw.to_radians().cos());
            self.drop_item((m.pos.0 + dx * 0.5, m.pos.1 + 1.2, m.pos.2 + dz * 0.5), (dx * 0.2, 0.2, dz * 0.2), id, count, 20);
        }
        if let Some(sword) = self.item_id("minecraft:golden_sword") {
            self.send_to_viewers(m.id, bedrock::ID_MOB_EQUIPMENT, &bedrock::mob_equipment(m.id, (sword, 1, 0), 0));
        }
    }

    /// Ce joueur porte-t-il au moins une pièce d'armure en or ? (les piglins le laissent en paix)
    pub(super) fn wears_gold(&self, c: &Client) -> bool {
        let Some(d) = self.data.as_ref() else { return false };
        c.inv.armor.slots.iter().any(|s| s.count > 0 && d.item_by_id(s.id).map(|def| def.name.starts_with("minecraft:golden_")).unwrap_or(false))
    }

    /// Un seau de poisson, d'axolotl ou de têtard vidé : la créature retrouve l'eau.
    pub(super) fn bucket_release(&mut self, bucket: &str, at: (i32, i32, i32)) {
        let Some(species) = Species::ALL.into_iter().find(|s| s.bucket() == Some(bucket)) else { return };
        if self.mobs.len() >= MAX_MOBS {
            return;
        }
        let id = self.add_mob(species, (at.0 as f32 + 0.5, at.1 as f32 + 0.1, at.2 as f32 + 0.5));
        let variant = if species == Species::Axolotl { axolotl_color(&mut self.mob_rng) } else { 0 };
        if let Some(m) = self.mobs.get_mut(&id) {
            m.variant = variant;
        }
    }

    /// `/summon` : fait apparaître un animal (pelage, robe, qualités tirés au sort comme à
    /// l'apparition naturelle). `baby` : un petit (événement « minecraft:entity_born »).
    pub(super) fn summon_mob(&mut self, name: &str, pos: (f32, f32, f32), baby: bool, tamed: bool) -> Result<(), String> {
        // Créature créée par un plugin : celle du jeu dont elle part, avec sa fiche
        let custom = crate::entity::custom::find(&name.to_ascii_lowercase());
        let base = custom.as_ref().map(|(_, def)| def.looks.clone()).unwrap_or_else(|| name.to_string());
        let short = base.trim_start_matches("minecraft:").to_ascii_lowercase();
        let Some(species) = Species::from_id(&format!("minecraft:{}", short)) else {
            return Err(format!("Basalt ne sait pas encore faire apparaître : {}", base));
        };
        if !pos.1.is_finite() || pos.1 < crate::world::MIN_Y as f32 || pos.1 > crate::world::MAX_Y as f32 {
            return Err("Position hors du monde".into());
        }
        let variant = match species {
            Species::Horse => self.mob_rng.below(7) as i32,
            Species::Llama => self.mob_rng.below(4) as i32,
            Species::Goat => (self.mob_rng.below(100) < 2) as i32,
            Species::Cat => self.cat_variant(),
            Species::Villager => self.villager_job(),
            // Slime : petit, moyen ou gros
            Species::Slime | Species::MagmaCube => [1, 2, 4][self.mob_rng.below(3) as usize],
            Species::Fox => self.mob_rng.below(2) as i32,
            Species::Frog => self.mob_rng.below(3) as i32,
            Species::Parrot => self.mob_rng.below(5) as i32,
            Species::Axolotl => axolotl_color(&mut self.mob_rng),
            _ => 0,
        };
        // (un warden invoqué sort de terre, comme celui qu'appelle un hurleur sculk)
        let emerging = species == Species::Warden;
        let mark = if species == Species::Horse { self.mob_rng.below(5) as i32 } else { 0 };
        let id = self.add_mob(species, pos);
        if let Some(m) = self.mobs.get_mut(&id) {
            if emerging {
                m.act = WardenAct::Emerging;
            }
            m.variant = variant;
            m.mark = mark;
            if species.cube() {
                m.max_health = (variant * variant) as f32;
                m.health = m.max_health;
            }
            if baby {
                m.age = -GROW_TICKS;
            }
            // Événement « minecraft:on_tame » du jeu : une monture déjà apprivoisée
            m.tamed = tamed && species.mount();
            if let Some((rank, def)) = &custom {
                m.take_custom(*rank, def, true);
            }
        }
        // Écouteurs : une créature apparaît (annulé, elle n'apparaît pas)
        if self.plugins.listens("CreatureSpawnEvent") {
            let out = self.fire("CreatureSpawnEvent", vec![("entity", self.mob_json(id))]);
            if super::cancelled(&out) {
                self.remove_mob(id);
                return Ok(());
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
        let held = self.held_name(c);
        match self.mobs.get_mut(&id) {
            Some(m) if m.species.stick() == Some(held.as_str()) && m.saddled && m.boost == 0 => {
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
        let llama = self.mobs.get(&id).map(|m| m.species == Species::Llama).unwrap_or(false);
        const WINDOW: u8 = 2;
        if let Some(c) = self.clients.get_mut(addr) {
            c.inv.open_window = WINDOW;
            c.inv.block_mob = Some(id);
            c.inv.block_is_furnace = false;
            c.inv.block_mob_carpet = llama;
            c.inv.block.slots = slots.clone();
        }
        // Les cases d'équipement et ce qu'elles acceptent (selle ; armure du cheval, tapis du lama)
        let species = self.mobs.get(&id).map(|m| m.species);
        const COLORS: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
        let carpets: Vec<String> = COLORS.iter().map(|c| format!("{}_carpet", c)).collect();
        // Icônes des cases vides
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
        // Taille annoncée, comptée comme le jeu : l'équipement, puis le coffre à partir de 1
        let size = if slots.len() > 2 { slots.len() as i32 - 1 } else { 2 };
        // Comme le jeu officiel : ce paquet seul ouvre l'écran de monture. Accompagné d'une
        // ouverture de conteneur, le jeu recevait deux ouvertures : cases sans icône à la
        // première, puis refus (« fermeture de la fenêtre -1 ») et écran figé.
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
        self.mobs.get(&id).map(|m| (m.species.seat(), m.pos)).or_else(|| self.boat_seat(id))
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
        let stand = self.clients.get(addr).and_then(|c| c.riding).and_then(|id| self.mobs.get(&id).map(|m| m.pos).or_else(|| self.boat_seat(id).map(|(_, p)| p))).map(|p| (p.0, p.1 + crate::player::EYE_HEIGHT, p.2));
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
        if self.release_boat(mob, player) {
            let link = bedrock::set_actor_link(mob, player, bedrock::LINK_REMOVE, false);
            let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.visible.contains(&player)).map(|(a, _)| *a).collect();
            for v in viewers {
                self.send(&v, bedrock::ID_SET_ACTOR_LINK, &link);
            }
            return;
        }
        if let Some(m) = self.mobs.get_mut(&mob) {
            m.rider = None;
            m.tame_in = 0;
            m.sent = (f32::MAX, 0.0, 0.0); // position renvoyée à tous au prochain tick
            // En l'air : c'est le serveur qui la fait retomber, à partir d'ici
            if m.airborne {
                m.on_ground = false;
                m.vel = (0.0, 0.0, 0.0);
            }
        }
        let link = bedrock::set_actor_link(mob, player, bedrock::LINK_REMOVE, false);
        self.send_to_viewers(mob, bedrock::ID_SET_ACTOR_LINK, &link);
        // Son état est renvoyé à tous : le jeu du cavalier, qui la dirigeait, garde sinon la
        // pose du saut
        self.mob_changed(mob);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.visible.contains(&player)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_SET_ACTOR_LINK, &link);
        }
    }

    /// Données du joueur cavalier (ou redevenu piéton, `seat` à None), chez lui et chez ceux
    /// qui le voient : indicateur « en selle » et position du siège.
    pub(super) fn rider_data(&mut self, addr: &SocketAddr, seat: Option<(f32, f32, f32)>) {
        let Some(c) = self.clients.get(addr) else { return };
        let (id, sneaking, sprinting) = (c.id, c.sneaking, c.sprinting);
        let riding = if seat.is_some() { bedrock::FLAG_RIDING } else { 0 };
        // Dans un bateau, le serveur officiel verrouille aussi le regard du passager : un
        // quart de tour de chaque côté, siège tourné de -90° (relevé à la montée). À la
        // descente, tout est remis à zéro.
        let aboard = seat.is_some() && c.riding.map(|r| self.boats.contains_key(&r)).unwrap_or(false);
        let mut extra: Vec<(u32, bedrock::Meta)> = Vec::new();
        if aboard || seat.is_none() {
            let on = aboard as u8;
            extra = vec![(57, bedrock::Meta::Byte(on)), (58, bedrock::Meta::Float(90.0 * on as f32)), (59, bedrock::Meta::Byte(on)), (60, bedrock::Meta::Float(-90.0 * on as f32))];
        }
        let seat = seat.unwrap_or((0.0, 0.0, 0.0));
        let own = bedrock::OWN_PLAYER_FLAGS | riding | Self::posture_flags(c);
        let mut entries = vec![(0, bedrock::Meta::Long(own)), (bedrock::META_SEAT_POSITION, bedrock::Meta::Vec3(seat.0, seat.1, seat.2))];
        entries.extend(extra);
        let packet = bedrock::set_actor_data(id, &entries);
        self.send(addr, bedrock::ID_SET_ACTOR_DATA, &packet);
        let others = Self::remote_flags(sneaking, sprinting) | riding;
        entries[0] = (0, bedrock::Meta::Long(others));
        let packet = bedrock::set_actor_data(id, &entries);
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
        // (un bateau : seul celui qui rame le dirige)
        if self.boat_seat(runtime as i64).is_some() {
            if self.rows(addr, runtime as i64) {
                self.steer_boat(runtime as i64, pos, yaw, Some(player));
            }
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
        if self.boat_follows(addr, id, pos, yaw) {
            return;
        }
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
        if !self.cfg.vanilla.mob_drops {
            return;
        }
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
        // Créature créée par un plugin : le butin de sa fiche, s'il en a fixé un
        if let Some(drops) = m.custom.and_then(crate::entity::custom::get).and_then(|def| def.drops) {
            for (name, lo, hi) in drops {
                let count = lo + self.mob_rng.below((hi - lo) as u32 + 1) as u8;
                let Some(item) = self.item_id(&name).filter(|_| count > 0) else { continue };
                let vel = ((self.mob_rng.unit() as f32 - 0.5) * 0.2, 0.2, (self.mob_rng.unit() as f32 - 0.5) * 0.2);
                self.drop_item((m.pos.0, m.pos.1 + 0.5, m.pos.2), vel, item, count, 10);
            }
            return;
        }
        // Le noyé armé lâche parfois son trident (8,5 %, un point de plus par niveau de butin)
        if m.trident && self.mob_rng.below(1000) < 85 + 10 * m.looting as u32 {
            if let Some(item) = self.item_id("trident") {
                self.drop_item((m.pos.0, m.pos.1 + 0.5, m.pos.2), (0.0, 0.2, 0.0), item, 1, 10);
            }
        }
        for &(name, lo, hi) in m.species.drops() {
            // (butin : de 0 à niveau unités de plus par objet)
            let count = (lo + self.mob_rng.below((hi - lo) as u32 + 1) as u8).saturating_add(self.mob_rng.below(m.looting as u32 + 1) as u8);
            if count == 0 {
                continue;
            }
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
            // Morte en flammes (aura de feu, lave, soleil) : sa viande tombe cuite
            let name = match (m.burning || m.fire_until > self.tick, name.trim_start_matches("minecraft:")) {
                (true, raw @ ("beef" | "porkchop" | "chicken" | "mutton" | "rabbit" | "cod" | "salmon")) => format!("minecraft:cooked_{}", raw),
                _ => name,
            };
            let Some(item) = self.item_id(&name) else { continue };
            let vel = ((self.mob_rng.unit() as f32 - 0.5) * 0.2, 0.2, (self.mob_rng.unit() as f32 - 0.5) * 0.2);
            self.drop_item((m.pos.0, m.pos.1 + 0.5, m.pos.2), vel, item, count, 10);
        }
    }

    /// Renforce le recul du coup que cette créature vient de recevoir (recul, frappe).
    pub(super) fn boost_knock(&mut self, id: i64, extra: f32) {
        let tick = self.tick;
        if let Some(m) = self.mobs.get_mut(&id).filter(|m| extra > 0.0 && m.hurt_tick == tick && m.knock > 0) {
            let flat = (m.vel.0 * m.vel.0 + m.vel.2 * m.vel.2).sqrt();
            if flat > 0.01 {
                let k = (flat + extra) / flat;
                m.vel = (m.vel.0 * k, m.vel.1, m.vel.2 * k);
            }
        }
    }

    /// Les épines de l'armure de ce joueur : 15 % de chances par niveau (toutes pièces
    /// confondues) de rendre 1 à 4 points de dégâts à qui le frappe.
    pub(super) fn thorns_roll(&mut self, victim: &SocketAddr) -> Option<f32> {
        let levels: u32 = self.clients.get(victim).map(|c| c.inv.armor.slots.iter().filter(|s| !s.is_empty()).map(|s| s.ench.level(crate::item::enchant::THORNS) as u32).sum()).unwrap_or(0);
        if levels == 0 || self.mob_rng.below(100) >= levels * 15 {
            return None;
        }
        // (les épines qui blessent coûtent deux points d'usure à la pièce qui les porte)
        let d = self.data.clone();
        if let Some(c) = self.clients.get_mut(victim) {
            if let Some(stack) = c.inv.armor.slots.iter_mut().find(|s| !s.is_empty() && s.ench.level(crate::item::enchant::THORNS) > 0) {
                let max = d.as_ref().and_then(|d| d.item_by_id(stack.id)).map(|def| crate::item::VanillaItems::get(&def.name).max_durability().min(u16::MAX as u32) as u16).unwrap_or(0);
                if max > 0 {
                    stack.damage = stack.damage.saturating_add(2);
                    if stack.damage >= max {
                        *stack = crate::inventory::ItemStack::EMPTY;
                    }
                    c.inv_touched = true;
                }
            }
        }
        Some(1.0 + self.mob_rng.below(4) as f32)
    }

    /// Une fois par seconde : les créatures qui ont la tête sous l'eau perdent leur air, puis
    /// deux points de vie par seconde. Le zombie resté sous l'eau devient un noyé, le zombie
    /// momifié un zombie.
    pub(super) fn mob_air_tick(&mut self) {
        if !self.cfg.vanilla.drowning {
            return;
        }
        let water = self.world.generator().blocks.water;
        let mut drowning: Vec<(i64, (f32, f32, f32))> = Vec::new();
        let mut turning: Vec<(i64, Species)> = Vec::new();
        let heads: Vec<(i64, bool)> = self
            .mobs
            .iter()
            .map(|(id, m)| {
                let head = (m.pos.0.floor() as i32, (m.pos.1 + m.size().1 * 0.85).floor() as i32, m.pos.2.floor() as i32);
                (*id, crate::world::World::in_bounds(head.1) && self.world.is_ready(head.0 >> 4, head.2 >> 4) && self.world.get(head.0, head.1, head.2) == water)
            })
            .collect();
        let mut gasping: Vec<(i64, (f32, f32, f32))> = Vec::new();
        for (id, under) in heads {
            let Some(m) = self.mobs.get_mut(&id) else { continue };
            // Poissons, calamars, têtard : hors de l'eau ils s'étouffent (un point par seconde
            // une fois leur air épuisé) ; dans l'eau ils respirent
            if m.species.gills() {
                if under {
                    m.air = m.species.air_supply();
                } else if m.air > 0 {
                    m.air -= 1;
                } else {
                    gasping.push((id, m.pos));
                }
                continue;
            }
            // Axolotl (cinq minutes) et dauphin (deux) : hors de l'eau ils se dessèchent
            let dries = match m.species {
                Species::Axolotl => Some(300u16),
                Species::Dolphin => Some(120u16),
                _ => None,
            };
            if let Some(limit) = dries {
                let wet = self.world.is_ready(m.pos.0.floor() as i32 >> 4, m.pos.2.floor() as i32 >> 4) && self.world.get(m.pos.0.floor() as i32, m.pos.1.floor() as i32, m.pos.2.floor() as i32) == water;
                if wet || under {
                    m.dry = 0;
                } else if m.dry < limit {
                    m.dry += 1;
                } else {
                    gasping.push((id, m.pos));
                }
            }
            if !under {
                m.air = m.species.air_supply();
                m.soak = 0;
                continue;
            }
            m.soak = m.soak.saturating_add(1);
            match m.species {
                Species::Zombie if m.soak >= SOAKED => turning.push((id, Species::Drowned)),
                Species::Husk if m.soak >= SOAKED => turning.push((id, Species::Zombie)),
                s if s.breathes_water() => {}
                _ if m.air > 0 => m.air -= 1,
                _ => drowning.push((id, m.pos)),
            }
        }
        for (id, into) in turning {
            if let Some(m) = self.mobs.get_mut(&id) {
                m.soak = 0;
            }
            self.transform_mob(id, into);
        }
        for (id, pos) in drowning {
            self.hit_mob(id, pos, 2.0, Attacker::Mob(id));
        }
        for (id, pos) in gasping {
            self.hit_mob(id, pos, 1.0, Attacker::Mob(id));
        }
    }

    /// Semelles givrantes : l'eau autour d'un joueur qui marche gèle sous ses pas, dans un
    /// rayon de 2 + niveau blocs. La glace fond quelques secondes plus tard.
    pub(super) fn frost_tick(&mut self) {
        let tick = self.tick;
        let (Some(ice), water) = (self.data.as_ref().and_then(|d| d.default_state("minecraft:frosted_ice")), self.world.generator().blocks.water) else { return };
        let walkers: Vec<((f32, f32, f32), i32)> = self
            .clients
            .values()
            .filter(|c| c.joined && c.stage == Stage::InGame && c.dead.is_none() && !c.mode.is_spectator())
            .filter_map(|c| {
                let level = c.inv.armor.slots.iter().map(|s| s.ench.level(crate::item::enchant::FROST_WALKER)).max().unwrap_or(0);
                (level > 0).then(|| ((c.pos.0, c.pos.1 - EYE, c.pos.2), 2 + level as i32))
            })
            .collect();
        for (feet, radius) in walkers {
            let (cx, cy, cz) = (feet.0.floor() as i32, (feet.1 + 0.05).floor() as i32, feet.2.floor() as i32);
            // (en train de nager, ou en l'air : rien ne gèle)
            if !crate::world::World::in_bounds(cy - 1) || !self.world.is_ready(cx >> 4, cz >> 4) || self.world.get(cx, cy, cz) == water {
                continue;
            }
            let below = self.world.get(cx, cy - 1, cz);
            if below == self.world.air {
                continue;
            }
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let (x, z) = (cx + dx, cz + dz);
                    if dx * dx + dz * dz > radius * radius || self.frost.len() >= MAX_FROST || !self.world.is_ready(x >> 4, z >> 4) {
                        continue;
                    }
                    if self.world.get(x, cy - 1, z) == water && self.world.get(x, cy, z) == self.world.air {
                        self.change_block((x, cy - 1, z), ice);
                        let life = 60 + self.mob_rng.below(60) as u64;
                        self.frost.insert((x, cy - 1, z), tick + life);
                    } else if let Some(until) = self.frost.get_mut(&(x, cy - 1, z)) {
                        // (tant qu'on marche dessus, elle tient)
                        *until = (*until).max(tick + 40);
                    }
                }
            }
        }
        // La glace arrivée à son terme redevient de l'eau
        if tick % 10 == 0 && !self.frost.is_empty() {
            let melted: Vec<(i32, i32, i32)> = self.frost.iter().filter(|(_, until)| **until <= tick).map(|(p, _)| *p).collect();
            for p in melted {
                self.frost.remove(&p);
                if self.world.is_ready(p.0 >> 4, p.2 >> 4) && self.world.get(p.0, p.1, p.2) == ice {
                    self.change_block(p, water);
                }
            }
        }
    }

    /// Fait fondre toute la glace des semelles givrantes (à l'arrêt du serveur).
    pub(super) fn melt_frost(&mut self) {
        let (Some(ice), water) = (self.data.as_ref().and_then(|d| d.default_state("minecraft:frosted_ice")), self.world.generator().blocks.water) else { return };
        for (p, _) in std::mem::take(&mut self.frost) {
            if self.world.get(p.0, p.1, p.2) == ice {
                self.world.set(p.0, p.1, p.2, water);
            }
        }
    }

    /// Les créatures qui se sont blessées sur une armure à épines.
    pub(super) fn thorns_tick(&mut self) {
        for (id, damage) in std::mem::take(&mut self.thorned) {
            if let Some(pos) = self.mobs.get(&id).map(|m| m.pos) {
                self.hit_mob(id, pos, damage, Attacker::Mob(id));
            }
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
            // Créature d'un plugin : elle retrouve sa fiche (sans le plugin, c'est une créature
            // ordinaire de son espèce, à la vie de celle-ci)
            m.natural = r.natural;
            match crate::entity::custom::find(&r.custom) {
                Some((rank, def)) => m.take_custom(rank, &def, false),
                None => {
                    m.max_health = m.max_health.min(m.species.health().max(30.0));
                    m.health = m.health.min(m.max_health);
                }
            }
            m.mark = r.mark;
            m.tamed = r.tamed;
            m.saddled = r.saddled;
            m.temper = r.temper;
            m.chested = r.chested;
            m.horns = r.horns;
            m.owner = r.owner;
            m.sitting = r.sitting;
            if r.home.is_some() {
                m.home = r.home;
            }
            m.station = r.station;
            m.trade_xp = r.trade_xp;
            m.trade_uses = r.trade_uses;
            m.trade_restock = r.trade_restock;
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
        // La copie au format du jeu, dans la base du monde
        self.export_actors(&live);
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
    /// Monde venu du jeu, ouvert pour la première fois : ses créatures sont reprises (espèce,
    /// position, bébé, apprivoisée, variante des chevaux et des lamas, couleur des moutons).
    /// Celles que Basalt ne connaît pas sont laissées de côté ; la base du jeu n'est pas modifiée.
    pub(super) fn import_actors(&mut self) {
        let Some(store) = self.world.db_store().cloned().filter(|s| !s.native) else { return };
        let dir = std::path::PathBuf::from(&self.cfg.world_dir).join("entities");
        let mark = dir.join("importe.txt");
        if mark.exists() {
            return;
        }
        let (mut taken, mut unknown, mut elsewhere) = (0usize, std::collections::BTreeMap::<String, usize>::new(), 0usize);
        for actor in store.actors() {
            let Some(kind) = actor.str_of("identifier") else { continue };
            // (objets au sol, flèches, cadres… et tout ce qui n'est pas dans le monde normal)
            if matches!(actor.get("LastDimensionId"), Some(crate::nbt::Tag::Int(d)) if *d != 0) {
                elsewhere += 1;
                continue;
            }
            let pos = match actor.get("Pos") {
                Some(crate::nbt::Tag::List(_, p)) => match (p.first(), p.get(1), p.get(2)) {
                    (Some(crate::nbt::Tag::Float(x)), Some(crate::nbt::Tag::Float(y)), Some(crate::nbt::Tag::Float(z))) => (*x, *y, *z),
                    _ => continue,
                },
                _ => continue,
            };
            let Some(species) = Species::from_id(kind) else {
                *unknown.entry(kind.trim_start_matches("minecraft:").to_string()).or_default() += 1;
                continue;
            };
            if taken >= 4000 || !pos.0.is_finite() || !pos.2.is_finite() {
                continue;
            }
            let flag = |key: &str| matches!(actor.get(key), Some(crate::nbt::Tag::Byte(b)) if *b != 0);
            let id = self.next_id;
            if self.summon_mob(kind, pos, flag("IsBaby"), flag("IsTamed")).is_err() {
                continue;
            }
            if let Some(m) = self.mobs.get_mut(&id) {
                if let (true, Some(crate::nbt::Tag::Int(v))) = (matches!(species, Species::Horse | Species::Llama), actor.get("Variant")) {
                    m.variant = *v;
                }
                if let (Species::Sheep, Some(crate::nbt::Tag::Byte(c))) = (species, actor.get("Color")) {
                    m.color = (*c as u8).min(15);
                }
                if let Some(crate::nbt::Tag::List(_, r)) = actor.get("Rotation") {
                    if let Some(crate::nbt::Tag::Float(yaw)) = r.first() {
                        m.yaw = yaw.rem_euclid(360.0);
                    }
                }
                taken += 1;
            }
        }
        self.mobs_dirty = true;
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(&mark, "Les créatures du monde d'origine ont été reprises une fois.\n");
        if taken + unknown.len() + elsewhere > 0 {
            let missed: Vec<String> = unknown.iter().map(|(k, n)| format!("{} × {}", n, k)).collect();
            info!("Créatures du monde d'origine reprises : {} (non reprises : {}{})", taken, if missed.is_empty() { "aucune".to_string() } else { missed.join(", ") }, if elsewhere > 0 { format!(", et {} du Nether ou de l'End", elsewhere) } else { String::new() });
        }
    }

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

/// Durée pendant laquelle un piglin admire le lingot reçu (« admire_item » : 8 secondes dans
/// la fiche du jeu, troc rendu au bout de 6)
const BARTER_TICKS: u32 = 120;

/// Troc d'un piglin : (poids, objet, quantité min, max). Table du jeu de mémoire (le fichier de
/// butin n'est pas dans les échantillons Mojang) ; les objets enchantés et les potions de la
/// table manquent, les autres gardent leur poids (la flèche spectrale de Java n'existe pas
/// dans Bedrock : des flèches).
const BARTER: [(u32, &str, u8, u8); 15] = [
    (8, "iron_boots", 1, 1),
    (10, "potion", 1, 1),
    (10, "iron_nugget", 10, 36),
    (10, "ender_pearl", 2, 4),
    (20, "string", 3, 9),
    (20, "quartz", 5, 12),
    (40, "obsidian", 1, 1),
    (40, "crying_obsidian", 1, 3),
    (40, "fire_charge", 1, 1),
    (40, "leather", 2, 4),
    (40, "soul_sand", 2, 8),
    (40, "nether_brick", 2, 8),
    (40, "arrow", 6, 12),
    (40, "gravel", 8, 16),
    (40, "blackstone", 8, 16),
];

/// La colère d'un warden contre un suspect monte (150 au plus).
fn warden_anger(w: &mut Mob, who: Prey, by: u32) {
    match w.suspects.iter_mut().find(|s| s.0 == who) {
        Some(s) => s.1 = (s.1 + by).min(150),
        None => w.suspects.push((who, by.min(150))),
    }
}

/// Couleur d'un axolotl : rose, cyan, or ou brun, et le bleu une fois sur 1200 (comme dans le jeu).
fn axolotl_color(rng: &mut Rng) -> i32 {
    if rng.below(1200) == 0 {
        4
    } else {
        [0, 1, 2, 3][rng.below(4) as usize]
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
    fn smite_and_bane_know_their_prey() {
        assert!(Species::Zombie.undead() && Species::Skeleton.undead() && Species::Phantom.undead() && Species::Drowned.undead());
        assert!(Species::Spider.arthropod() && Species::CaveSpider.arthropod() && Species::Silverfish.arthropod());
        // Aucune espèce n'est les deux ; les animaux, le creeper et l'enderman ne sont ni l'un ni l'autre
        assert!(Species::ALL.iter().all(|s| !(s.undead() && s.arthropod())));
        assert!([Species::Cow, Species::Creeper, Species::Enderman, Species::Witch].iter().all(|s| !s.undead() && !s.arthropod()));
    }

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
            // (le villageois ne se nourrit pas à la main)
            // (ni le reste du bestiaire, au comportement générique : il ne se reproduit pas encore)
            assert!(matches!(s, Species::Villager | Species::IronGolem) || s.hostile().is_some() || s.breed_items().is_empty() || !s.grow_items().is_empty(), "{:?} : un petit doit pouvoir grandir plus vite", s);
        }
    }

    /// Chaque espèce a de quoi vivre (vie, taille) et son butin existe dans le jeu ; celles qui
    /// nagent ou volent ne marchent pas, et inversement.
    #[test]
    fn every_species_is_complete() {
        let data = crate::data::GameData::load(std::path::Path::new("data")).unwrap();
        assert_eq!(Species::ALL.len(), 73);
        for s in Species::ALL {
            assert!(s.health() > 0.0 && s.size().0 > 0.0 && s.size().1 > 0.0, "{:?}", s);
            assert!(s.id().starts_with("minecraft:"), "{:?}", s);
            assert!(!(s.flies().is_some() && s.swims().is_some()), "{:?}", s);
            for (item, min, max) in s.drops() {
                assert!(data.item(item).is_some(), "{:?} : objet inconnu {}", s, item);
                assert!(min <= max, "{:?}", s);
            }
        }
        assert!(Species::Cod.swims().is_some() && Species::Bat.flies().is_some() && Species::Fox.swims().is_none());
        assert!(Species::Piglin.hostile().is_some() && Species::ZombifiedPiglin.neutral() && Species::Fox.hostile().is_none());
        assert_eq!(Species::from_id("minecraft:zombie_pigman"), Some(Species::ZombifiedPiglin));
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

    /// Ce que mangent, aiment ou donnent les créatures existe dans le jeu (sinon le joueur ne
    /// pourrait jamais le leur tendre), et le troc des piglins aussi.
    #[test]
    fn every_creature_item_exists() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = crate::data::GameData::load(&dir).unwrap();
        let mut missing = Vec::new();
        for s in Species::ALL {
            let names = s.tempt().iter().chain(s.breed_items()).chain(s.taming_items()).copied().chain(s.grow_items().iter().map(|g| g.0)).chain(s.heal_items().iter().map(|h| h.0)).chain(s.bucket()).chain(s.stick());
            for name in names {
                if data.item(name).is_none() && !missing.contains(&name) {
                    missing.push(name);
                }
            }
        }
        for (_, name, lo, hi) in BARTER {
            assert!(lo <= hi && lo > 0);
            if data.item(name).is_none() {
                missing.push(name);
            }
        }
        for name in ["armadillo_scute", "torchflower_seeds", "pitcher_pod", "brush", "cookie"] {
            if data.item(name).is_none() {
                missing.push(name);
            }
        }
        assert!(missing.is_empty(), "objets inconnus : {:?}", missing);
    }

    /// Toutes les créatures sont proposées par l'aide à la saisie de /summon.
    #[test]
    fn summon_offers_every_species() {
        let summon = crate::command::VanillaCommands::all().iter().find(|c| c.name == "summon").unwrap();
        let list = summon.usage.split("entityType:").nth(1).unwrap().split('>').next().unwrap();
        let names: Vec<&str> = list.split('|').collect();
        // (et toutes passent dans ce qui est envoyé au jeu, pas seulement dans le texte)
        let forms = crate::bedrock::command_overloads(summon.usage);
        let sent = match &forms[0][0].kind {
            crate::bedrock::ArgKind::Choice(words) => words.clone(),
            other => panic!("{:?}", other),
        };
        for s in Species::ALL {
            let id = s.id().trim_start_matches("minecraft:");
            assert!(names.contains(&id), "/summon ne propose pas {}", id);
            assert!(sent.iter().any(|w| w == id), "/summon n'envoie pas {}", id);
        }
    }

    /// L'œuf de tortue a bien les états que la ponte et l'éclosion lui donnent.
    #[test]
    fn turtle_eggs_have_their_states() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = crate::data::GameData::load(&dir).unwrap();
        let egg = data.default_state("minecraft:turtle_egg").expect("œuf de tortue");
        let four = data.with_props(egg, &[("turtle_egg_count", crate::data::Prop::Str("four_egg".into()))]).expect("quatre œufs");
        let cracked = data.with_props(four, &[("cracked_state", crate::data::Prop::Str("max_cracked".into()))]).expect("fêlé");
        assert_ne!(egg, four);
        assert_eq!(data.prop(cracked, "turtle_egg_count"), Some(crate::data::Prop::Str("four_egg".into())));
        assert!(data.default_state("minecraft:snow_layer").is_some());
    }

    /// Comportements du jeu : qui est neutre, qui respire sous l'eau, qui se divise, qui se
    /// laisse monter d'emblée.
    #[test]
    fn creatures_behave_like_the_game() {
        assert!(Species::Cod.gills() && Species::Squid.gills() && !Species::Dolphin.gills() && !Species::Axolotl.gills());
        assert_eq!(Species::Dolphin.air_supply(), 240);
        // (tailles des petits d'après les fiches du jeu : le dessin du jeu les compense)
        assert_eq!(Species::Turtle.scale(true), 0.16);
        // (« knockback_resistance » des fiches du jeu)
        assert_eq!(Species::Warden.knockback_resistance(), 1.0);
        assert_eq!(Species::IronGolem.knockback_resistance(), 1.0);
        assert_eq!(Species::Ravager.knockback_resistance(), 0.75);
        assert_eq!(Species::Zombie.knockback_resistance(), 0.0);
        assert_eq!(Species::Panda.scale(true), 0.4);
        assert_eq!(Species::Cow.scale(true), 0.5);
        assert!(Species::MagmaCube.cube() && Species::Slime.cube() && !Species::Frog.cube());
        assert!(Species::PolarBear.neutral() && Species::Bee.neutral() && Species::Dolphin.neutral() && !Species::Hoglin.neutral());
        assert!(Species::Camel.born_tame() && Species::Camel.mount() && Species::Camel.steerable() && !Species::Horse.born_tame());
        assert_eq!(Species::Strider.stick(), Some("warped_fungus_on_a_stick"));
        assert_eq!(Species::Pig.stick(), Some("carrot_on_a_stick"));
        assert!(Species::Fox.hunts().0.contains(&Species::Chicken) && Species::Frog.hunts().0.contains(&Species::MagmaCube));
        assert!(Species::Parrot.pet() && Species::Parrot.taming_items().contains(&"wheat_seeds"));
        assert!(Species::ZombieHorse.undead() && Species::Bogged.burns() && Species::Bee.arthropod());
        assert_eq!(Species::Hoglin.breed_items(), &["crimson_fungus"]);
        assert_eq!(Species::Strider.breed_items(), &["warped_fungus"]);
        assert!(Species::Tadpole.breed_items().is_empty() && Species::Tadpole.grow_items().iter().any(|g| g.0 == "slime_ball"));
        assert!(Species::Bee.tempt().contains(&"poppy") && Species::Fox.tempt().contains(&"sweet_berries"));
        // (chaque espèce qui se reproduit a de quoi faire grandir ses petits)
        for s in Species::ALL {
            if !s.breed_items().is_empty() && !matches!(s, Species::Wolf) {
                assert!(!s.grow_items().is_empty(), "{:?}", s);
            }
        }
        assert!(Species::ALL.iter().filter(|s| s.ambient()).all(|s| s.hostile().is_none() || s.neutral()));
        let mut rng = Rng::new(7);
        let colors: Vec<i32> = (0..5000).map(|_| axolotl_color(&mut rng)).collect();
        assert!(colors.iter().all(|c| (0..=4).contains(c)) && colors.iter().filter(|c| **c == 4).count() < 20);
    }

    /// Tous les objets des offres existent dans les données du jeu (sinon l'offre est
    /// laissée de côté sans rien dire).
    #[test]
    fn every_traded_item_exists() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let data = crate::data::GameData::load(&dir).unwrap();
        let mut missing = Vec::new();
        for job in 1..=13 {
            for seed in 0..200u64 {
                for o in job_offers(job, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)) {
                    for name in [Some(o.buy_a.0), o.buy_b.map(|b| b.0), Some(o.sell.0)].into_iter().flatten() {
                        if data.item(name).is_none() && !missing.contains(&name) {
                            missing.push(name);
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "objets inconnus : {:?}", missing);
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
            custom: String::new(),
            natural: false,
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
            owner: None,
            sitting: false,
            home: Some((10.5, 70.0, -3.5)),
            trade_xp: 75,
            trade_uses: vec![3, 0, 12],
            trade_restock: 7,
            station: Some((12, 70, -5)),
        };
        assert_eq!(MobRecord::from_json(&r.to_json()), Some(r.clone()));
        // Loup apprivoisé : son maître, assis, collier, vie d'un loup apprivoisé
        let wolf = MobRecord { species: Species::Wolf, health: 33.0, max_health: 40.0, color: 14, variant: 0, mark: 0, saddled: false, chested: false, knot: None, items: Vec::new(), owner: Some("Steve".into()), sitting: true, ..r };
        assert_eq!(MobRecord::from_json(&wolf.to_json()), Some(wolf));
        assert_eq!(Species::from_id("minecraft:wolf"), Some(Species::Wolf));
        // Échanges : chaque métier a ses offres à tous les niveaux, toujours les mêmes pour un
        // même villageois ; un novice en a deux, dont une qui achète contre une émeraude
        for job in 1..=13 {
            let offers = job_offers(job, 42);
            assert_eq!(offers, job_offers(job, 42));
            let novice: Vec<&Offer> = offers.iter().filter(|o| o.tier == 0).collect();
            assert_eq!(novice.len(), 2, "métier {}", job);
            assert!(novice[0].sell == ("emerald", 1) && novice[1].buy_a.0 == "emerald" && villager_title(job).is_some());
            for tier in 0..5 {
                assert!(offers.iter().any(|o| o.tier == tier), "métier {} : rien au niveau {}", job, tier);
            }
            // Les offres sont rangées par niveau : en monter n'en déplace aucune
            assert!(offers.windows(2).all(|w| w[0].tier <= w[1].tier));
        }
        assert!(job_offers(0, 1).is_empty() && job_offers(14, 1).is_empty() && villager_title(14).is_none());
        // Postes de travail : chaque métier a le sien, et un seul
        for job in 1..=13 {
            let blocks = ["composter", "barrel", "loom", "fletching_table", "lectern", "cartography_table", "brewing_stand", "blast_furnace", "grindstone", "smithing_table", "smoker", "cauldron", "stonecutter_block"];
            assert_eq!(station_job(&format!("minecraft:{}", blocks[job as usize - 1])), Some(job));
        }
        assert_eq!(station_job("minecraft:chest"), None);
        // Niveaux : 10, 70, 150 et 250 points d'expérience ; un achat de novice en rapporte 2
        assert_eq!([trade_tier(0), trade_tier(9), trade_tier(10), trade_tier(69), trade_tier(150), trade_tier(9999)], [0, 0, 1, 1, 3, 4]);
        assert_eq!(job_offers(1, 42)[0].xp(), 2);
        // Réassort : deux par jour, à 2000 et 8000
        assert_eq!([restocks_before(0), restocks_before(2_000), restocks_before(7_999), restocks_before(8_000), restocks_before(24_000 + 2_500)], [0, 1, 1, 2, 3]);
        // Le golem de fer : 100 points de vie, du fer et des coquelicots, ni élevage ni petit
        assert_eq!(Species::from_id("minecraft:iron_golem"), Some(Species::IronGolem));
        assert_eq!(Species::IronGolem.health(), 100.0);
        assert!(Species::IronGolem.breed_items().is_empty() && Species::IronGolem.drops().iter().any(|d| d.0 == "minecraft:iron_ingot"));
        assert_eq!(Species::from_id("minecraft:cat"), Some(Species::Cat));
        assert_eq!(Species::from_id("minecraft:villager"), Some(Species::Villager));
        assert_eq!(Species::from_id("minecraft:villager_v2"), Some(Species::Villager));
        assert!(Species::Cat.pet() && Species::Cat.taming_items().contains(&"salmon") && Species::Cat.breed_items().contains(&"cod"));
        assert!(Species::Wolf.breed_items().contains(&"cooked_beef") && Species::Wolf.tameable() && !Species::Wolf.mount());
        // Ancien enregistrement (sans les nouveaux champs) : valeurs du jeu par défaut
        let old = crate::json::parse(r#"{"kind":"minecraft:cow","x":0,"y":0,"z":0}"#).unwrap();
        let cow = MobRecord::from_json(&old).unwrap();
        assert_eq!((cow.max_health, cow.health, cow.tamed, cow.mark), (10.0, 10.0, false, 0));
    }
}
