//! Créatures : les animaux paisibles (vache, cochon, mouton, poule, lapin).
//!
//! Comme dans le jeu, les animaux apparaissent quand une terre est découverte : à la
//! première visite d'un chunk, un petit troupeau s'y installe parfois, selon le biome. Ils
//! sont enregistrés avec le monde (`mobs.json`) et ne vivent (se déplacent) que près des
//! joueurs. Ils se promènent, sautent les marches, évitent les chutes, nagent, suivent le
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
}

impl Species {
    const FARM: [Species; 4] = [Species::Cow, Species::Pig, Species::Sheep, Species::Chicken];
    const ALL: [Species; 5] = [Species::Cow, Species::Pig, Species::Sheep, Species::Chicken, Species::Rabbit];

    fn id(self) -> &'static str {
        match self {
            Species::Cow => "minecraft:cow",
            Species::Pig => "minecraft:pig",
            Species::Sheep => "minecraft:sheep",
            Species::Chicken => "minecraft:chicken",
            Species::Rabbit => "minecraft:rabbit",
        }
    }
    fn from_id(s: &str) -> Option<Species> {
        Species::ALL.into_iter().find(|k| k.id() == s)
    }
    fn health(self) -> f32 {
        match self {
            Species::Cow | Species::Pig => 10.0,
            Species::Sheep => 8.0,
            Species::Chicken => 4.0,
            Species::Rabbit => 3.0,
        }
    }
    /// Largeur et hauteur de la boîte de collision d'un adulte (celles du jeu)
    fn size(self) -> (f32, f32) {
        match self {
            Species::Cow => (0.9, 1.3),
            Species::Pig => (0.9, 0.9),
            Species::Sheep => (0.9, 1.3),
            Species::Chicken => (0.4, 0.7),
            Species::Rabbit => (0.4, 0.5),
        }
    }
    /// Vitesse de marche (blocs par tick)
    fn speed(self) -> f32 {
        match self {
            Species::Chicken => 0.07,
            Species::Rabbit => 0.12,
            _ => 0.09,
        }
    }
    /// Nourriture : ce qui l'attire, le fait grandir ou le rend amoureux
    fn food(self) -> &'static [&'static str] {
        match self {
            Species::Cow | Species::Sheep => &["wheat"],
            Species::Pig => &["carrot", "potato", "beetroot"],
            Species::Chicken => &["wheat_seeds", "pumpkin_seeds", "melon_seeds", "beetroot_seeds", "torchflower_seeds", "pitcher_pod"],
            Species::Rabbit => &["carrot", "golden_carrot", "dandelion"],
        }
    }
    /// Butin : (objet, quantité minimale, maximale)
    fn drops(self) -> &'static [(&'static str, u8, u8)] {
        match self {
            Species::Cow => &[("minecraft:beef", 1, 3), ("minecraft:leather", 0, 2)],
            Species::Pig => &[("minecraft:porkchop", 1, 3)],
            Species::Sheep => &[("minecraft:mutton", 1, 2), ("minecraft:white_wool", 1, 1)],
            Species::Chicken => &[("minecraft:chicken", 1, 1), ("minecraft:feather", 0, 2)],
            Species::Rabbit => &[("minecraft:rabbit", 0, 1), ("minecraft:rabbit_hide", 0, 1)],
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
        f
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

/// Au plus tant de créatures dans le monde (les nouvelles terres n'en reçoivent plus)
const MAX_MOBS: usize = 4000;
/// Rayon d'affichage
const VIEW: f32 = 80.0;
/// Au-delà, une créature ne bouge plus (personne ne la voit)
const ACTIVE: f32 = 96.0;
/// Chunks examinés autour de chaque joueur pour y installer des animaux
const POPULATE_RADIUS: i32 = 4;

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
                    if self.populated.contains(&(cx, cz)) || !self.world.is_ready(cx, cz) {
                        continue;
                    }
                    if budget == 0 {
                        return;
                    }
                    budget -= 1;
                    self.populated.insert((cx, cz));
                    self.mobs_dirty = true;
                    if self.mobs.len() < MAX_MOBS && self.mob_rng.below(10) == 0 {
                        self.spawn_herd(cx, cz, p.1 as i32);
                    }
                }
            }
        }
    }

    /// Un troupeau dans ce chunk, de l'espèce qui convient au biome.
    fn spawn_herd(&mut self, cx: i32, cz: i32, near_y: i32) {
        let (x, z) = (cx * 16 + self.mob_rng.below(16) as i32, cz * 16 + self.mob_rng.below(16) as i32);
        let biome = self.world.generator().vanilla().map(|v| v.surface_biome_at(x, z));
        let species = match biome {
            None => Species::FARM[self.mob_rng.below(4) as usize],
            Some(b) => match b {
                B::Desert | B::SnowyPlains | B::IceSpikes | B::SnowySlopes | B::Grove => Species::Rabbit,
                B::Plains | B::SunflowerPlains | B::Meadow | B::Forest | B::BirchForest | B::OldGrowthBirchForest | B::DarkForest | B::Savanna | B::SavannaPlateau | B::WindsweptSavanna | B::WindsweptForest | B::WindsweptHills | B::Jungle | B::SparseJungle | B::BambooJungle | B::CherryGrove => Species::FARM[self.mob_rng.below(4) as usize],
                B::FlowerForest | B::Taiga | B::SnowyTaiga | B::OldGrowthPineTaiga | B::OldGrowthSpruceTaiga => {
                    if self.mob_rng.below(4) == 0 {
                        Species::Rabbit
                    } else {
                        Species::FARM[self.mob_rng.below(4) as usize]
                    }
                }
                _ => return,
            },
        };
        let variant = if species == Species::Rabbit {
            match biome {
                Some(B::Desert) => 4,
                Some(B::SnowyPlains | B::IceSpikes | B::SnowySlopes | B::Grove | B::SnowyTaiga) => {
                    if self.mob_rng.below(5) == 0 {
                        3
                    } else {
                        1
                    }
                }
                _ => [0, 0, 2, 5][self.mob_rng.below(4) as usize],
            }
        } else {
            0
        };
        let count = if species == Species::Rabbit { 2 + self.mob_rng.below(2) } else { 2 + self.mob_rng.below(3) };
        let g = &self.world.generator().blocks;
        let (grass, sand, snow) = (g.grass, g.sand, g.snow);
        for _ in 0..count {
            let (dx, dz) = (self.mob_rng.below(5) as i32 - 2, self.mob_rng.below(5) as i32 - 2);
            let (bx, bz) = (x + dx, z + dz);
            let Some(y) = self.surface_y(bx, bz, near_y) else { continue };
            let below = self.mob_get(bx, y - 1, bz);
            let ok = below == grass || (species == Species::Rabbit && (below == sand || below == snow));
            if ok {
                let id = self.add_mob(species, (bx as f32 + 0.5, y as f32, bz as f32 + 0.5));
                if let Some(m) = self.mobs.get_mut(&id) {
                    m.variant = variant;
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
        let mob = Mob {
            id,
            species,
            pos,
            vel: (0.0, 0.0, 0.0),
            yaw,
            on_ground: true,
            health: species.health(),
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
        };
        self.mobs.insert(id, mob);
        self.mobs_dirty = true;
        id
    }

    /// Données d'entité d'une créature (taille, couleur, bébé, tondu…).
    fn mob_meta(m: &Mob) -> Vec<(u32, bedrock::Meta)> {
        let (w, h) = m.size();
        let mut meta = bedrock::mob_metadata(w, h, m.variant, m.color, m.flags());
        if m.baby() {
            for (k, v) in meta.iter_mut() {
                if *k == 38 {
                    *v = bedrock::Meta::Float(0.5);
                }
            }
        }
        meta
    }

    /// Paquet d'apparition d'une créature.
    fn mob_packet(m: &Mob) -> Vec<u8> {
        bedrock::add_actor(m.id, m.species.id(), m.pos, m.vel, m.yaw, m.species.health(), &Self::mob_meta(m))
    }

    /// Envoie aux joueurs qui la voient l'état changé d'une créature (bébé devenu adulte,
    /// tondu, amoureux).
    fn mob_changed(&mut self, id: i64) {
        let Some(m) = self.mobs.get(&id) else { return };
        let scale = if m.baby() { 0.5 } else { 1.0 };
        let (w, h) = m.size();
        let packet = bedrock::set_actor_data(id, &[(0, bedrock::Meta::Long(m.flags())), (38, bedrock::Meta::Float(scale)), (53, bedrock::Meta::Float(w)), (54, bedrock::Meta::Float(h))]);
        self.send_to_viewers(id, bedrock::ID_SET_ACTOR_DATA, &packet);
        self.mobs_dirty = true;
    }

    /// Montre à chaque joueur les créatures proches de lui et retire les autres.
    pub(super) fn show_mobs(&mut self) {
        let mut out: Vec<(SocketAddr, u32, Vec<u8>)> = Vec::new();
        for (addr, c) in self.clients.iter_mut() {
            if !c.joined || c.stage != Stage::InGame {
                continue;
            }
            for m in self.mobs.values() {
                let far = (m.pos.0 - c.pos.0).powi(2) + (m.pos.2 - c.pos.2).powi(2);
                if far < VIEW * VIEW && m.dead.is_none() {
                    if c.mobs_seen.insert(m.id) {
                        out.push((*addr, bedrock::ID_ADD_ACTOR, Self::mob_packet(m)));
                    }
                } else if far > (VIEW + 16.0) * (VIEW + 16.0) && c.mobs_seen.remove(&m.id) {
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
        for id in ids {
            let Some(mut m) = self.mobs.get(&id).cloned() else { continue };
            if let Some(t) = m.dead {
                if tick.saturating_sub(t) >= 20 {
                    self.remove_mob(id);
                }
                continue;
            }
            let near = players.iter().any(|(p, _)| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2) < ACTIVE * ACTIVE);
            if !near {
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
        } else if let Some(p) = players.iter().filter(|(_, held)| m.species.food().contains(&held.as_str())).map(|(p, _)| *p).find(|p| (p.0 - m.pos.0).powi(2) + (p.2 - m.pos.2).powi(2) < 100.0 && (p.1 - m.pos.1).abs() < 4.0) {
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
            .filter(|o| o.id != m.id && o.species == m.species && o.love > 0 && !o.baby() && o.dead.is_none())
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
                if done.contains(&a.0) || done.contains(&b.0) || a.1 != b.1 {
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
                // Le petit tient de l'un de ses parents (couleur de laine, pelage)
                let (color, variant) = self.mobs.get(&if self.mob_rng.below(2) == 0 { a.0 } else { b.0 }).map(|m| (m.color, m.variant)).unwrap_or((0, 0));
                let baby = self.add_mob(a.1, at);
                if let Some(m) = self.mobs.get_mut(&baby) {
                    m.age = -GROW_TICKS;
                    m.color = color;
                    m.variant = variant;
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
        if dead {
            m.dead = Some(tick);
        }
        self.mobs_dirty = true;
        self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_HURT, 0));
        if dead {
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
        let (held, creative, near) = match self.clients.get(addr) {
            Some(c) => {
                let h = c.inv.held();
                let name = if h.is_empty() { String::new() } else { self.data.as_ref().and_then(|d| d.item_by_id(h.id)).map(|d| d.name.trim_start_matches("minecraft:").to_string()).unwrap_or_default() };
                let near = (c.pos.0 - m.pos.0).powi(2) + (c.pos.2 - m.pos.2).powi(2) < 8.0 * 8.0;
                (name, c.mode == GameMode::Creative, near)
            }
            None => return true,
        };
        if !near {
            return true;
        }
        let mut changed = false;
        let mut used = false;
        if held == "shears" && m.species == Species::Sheep && !m.baby() && !m.sheared {
            // Tonte : 1 à 3 laines de sa couleur
            const COLORS: [&str; 16] = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
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
        } else if held == "bucket" && m.species == Species::Cow && !m.baby() {
            // Traite : le seau vide devient un seau de lait
            if !creative {
                if let Some(milk) = self.item_id("milk_bucket") {
                    if let Some(c) = self.clients.get_mut(addr) {
                        c.inv.replace_held(milk);
                    }
                }
                self.send_inventory(addr);
                self.show_held(addr);
            }
        } else if m.species.food().contains(&held.as_str()) {
            if m.baby() {
                // Un petit nourri grandit plus vite (un dixième du temps restant)
                if let Some(s) = self.mobs.get_mut(&id) {
                    s.age = (s.age - s.age / 10).min(-1);
                }
                used = true;
            } else if m.age == 0 && m.love == 0 {
                if let Some(s) = self.mobs.get_mut(&id) {
                    s.love = LOVE_TICKS;
                }
                self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_LOVE_PARTICLES, 0));
                changed = true;
                used = true;
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

    /// Butin d'une créature morte, lâché sur place (rien pour un petit).
    fn mob_loot(&mut self, id: i64) {
        let Some(m) = self.mobs.get(&id).cloned() else { return };
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

    /// Créatures et chunks peuplés, à enregistrer avec le monde (`mobs.json`).
    pub(super) fn mobs_json(&self) -> String {
        let n = |v: f32| Json::Num((v as f64 * 100.0).round() / 100.0);
        let list: Vec<Json> = self
            .mobs
            .values()
            .filter(|m| m.dead.is_none())
            .map(|m| {
                Json::Obj(vec![
                    ("kind".into(), Json::Str(m.species.id().into())),
                    ("x".into(), n(m.pos.0)),
                    ("y".into(), n(m.pos.1)),
                    ("z".into(), n(m.pos.2)),
                    ("yaw".into(), n(m.yaw)),
                    ("health".into(), n(m.health)),
                    ("age".into(), Json::Num(m.age as f64)),
                    ("color".into(), Json::Num(m.color as f64)),
                    ("variant".into(), Json::Num(m.variant as f64)),
                    ("sheared".into(), Json::Bool(m.sheared)),
                ])
            })
            .collect();
        let chunks: Vec<Json> = self.populated.iter().map(|(x, z)| Json::Str(format!("{},{}", x, z))).collect();
        Json::Obj(vec![("mobs".into(), Json::Arr(list)), ("chunks".into(), Json::Arr(chunks))]).to_json()
    }

    /// Recharge les créatures enregistrées.
    pub(super) fn load_mobs(&mut self) {
        let path = std::path::Path::new(&self.cfg.world_dir).join("mobs.json");
        let Ok(text) = std::fs::read_to_string(&path) else { return };
        let Ok(doc) = crate::json::parse(&text) else {
            warn!("{} illisible : animaux non rechargés", path.display());
            return;
        };
        for c in doc.get("chunks").and_then(|c| c.as_arr()).unwrap_or(&[]) {
            if let Some((x, z)) = c.as_str().and_then(|s| s.split_once(',')) {
                if let (Ok(x), Ok(z)) = (x.parse(), z.parse()) {
                    self.populated.insert((x, z));
                }
            }
        }
        let mut loaded = 0;
        for v in doc.get("mobs").and_then(|m| m.as_arr()).unwrap_or(&[]).iter().take(MAX_MOBS) {
            let Some(species) = v.get("kind").and_then(|k| k.as_str()).and_then(Species::from_id) else { continue };
            let num = |k: &str| v.get(k).and_then(|n| n.as_f64()).filter(|n| n.abs() < 3.0e7);
            let (Some(x), Some(y), Some(z)) = (num("x"), num("y"), num("z")) else { continue };
            let id = self.add_mob(species, (x as f32, y as f32, z as f32));
            if let Some(m) = self.mobs.get_mut(&id) {
                m.yaw = num("yaw").unwrap_or(0.0) as f32;
                m.sent_yaw = m.yaw;
                m.health = (num("health").unwrap_or(species.health() as f64) as f32).clamp(0.5, species.health());
                m.age = num("age").unwrap_or(0.0) as i64;
                m.color = num("color").unwrap_or(0.0) as u8 & 15;
                m.variant = num("variant").unwrap_or(0.0) as i32;
                m.sheared = v.get("sheared").and_then(|s| s.as_bool()).unwrap_or(false);
            }
            loaded += 1;
        }
        self.mobs_dirty = false;
        if loaded > 0 {
            info!("Animaux rechargés : {}", loaded);
        }
    }
}

/// Générateur de hasard des créatures, d'après l'heure de démarrage.
pub(super) fn mob_rng() -> Rng {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    Rng::new(seed)
}
