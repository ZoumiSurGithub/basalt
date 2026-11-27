//! Les bateaux : posés sur l'eau avec leur objet, ils flottent, se montent d'un clic droit et
//! se cassent à coups de poing. C'est le jeu du passager qui dirige le bateau (comme pour une
//! monture) ; le serveur reprend la position qu'il annonce et la montre aux autres.

use super::*;

/// Les bois, dans l'ordre des variantes du jeu.
pub(super) const WOODS: [&str; 10] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "bamboo", "cherry", "pale_oak"];

/// L'entité est annoncée 0,375 bloc au-dessus de sa coque (comme dans le jeu).
const ORIGIN: f32 = 0.375;
/// Les sièges (fiche du jeu, « seats ») : seul à bord, puis le rameur et son passager.
/// La fiche donne −0,2 ; pour un joueur, dont la position est comptée à ses yeux, le siège
/// est 1,22 plus haut (même écart que pour les montures : 0,63 dans la fiche du cochon, 1,83 ici).
const SEAT: (f32, f32, f32) = (0.0, 1.02, 0.0);
const SEAT_FRONT: (f32, f32, f32) = (0.2, 1.02, 0.0);
const SEAT_BACK: (f32, f32, f32) = (-0.6, 1.02, 0.0);
/// Lien d'un passager qui ne dirige pas.
const LINK_PASSENGER: u8 = 2;
/// Données d'un bateau : le temps de rame de chaque côté (les rames bougent quand il avance).
const META_PADDLE_LEFT: u32 = 13;
const META_PADDLE_RIGHT: u32 = 14;
/// Vitesse (blocs par tick) à laquelle le courant emporte un bateau vide.
const DRIFT: f32 = 0.04;
/// La boîte du bateau (fiche du jeu).
#[allow(dead_code)]
const SIZE: (f32, f32) = (1.4, 0.455);
/// Hauteur de l'eau dans un bloc plein, et part de la coque sous l'eau.
const WATER_TOP: f32 = 0.9;
const DRAUGHT: f32 = 0.15;
const MAX_BOATS: usize = 512;
/// Coups de poing pour casser un bateau en survie, et temps (ticks) après lequel ils s'oublient.
const HITS: u8 = 4;
const FORGET: u64 = 40;

#[derive(Clone)]
pub(super) struct Boat {
    pub pos: (f32, f32, f32),
    pub yaw: f32,
    pub wood: u8,
    /// Celui qui rame, et son passager
    pub rider: Option<SocketAddr>,
    pub mate: Option<SocketAddr>,
    paddle: f32,
    vy: f32,
    sent: (f32, f32, f32),
    sent_yaw: f32,
    hits: u8,
    hit_tick: u64,
    /// Dernier tick où le jeu du passager a donné sa position
    driven: u64,
}

/// Le bois d'un objet bateau (nom sans `minecraft:`) ; les bateaux à coffre n'en sont pas.
pub(super) fn boat_wood(item: &str) -> Option<u8> {
    if item.ends_with("_chest_boat") || item.ends_with("_chest_raft") {
        return None;
    }
    let wood = if item == "bamboo_raft" { "bamboo" } else { item.strip_suffix("_boat")? };
    WOODS.iter().position(|w| *w == wood).map(|i| i as u8)
}

/// L'objet que rend un bateau de ce bois.
pub(super) fn boat_item(wood: u8) -> String {
    match WOODS.get(wood as usize).copied().unwrap_or("oak") {
        "bamboo" => "bamboo_raft".into(),
        w => format!("{}_boat", w),
    }
}

impl Server {
    fn boat_packet(id: i64, b: &Boat, rider: Option<i64>) -> Vec<u8> {
        let meta = bedrock::boat_metadata(b.wood as i32);
        bedrock::add_actor(id, "minecraft:boat", b.pos, (0.0, 0.0, 0.0), b.yaw, (40.0, 40.0), None, rider, &meta)
    }

    /// Le joueur clique sur un bloc avec un bateau en main : il est posé là, sur l'eau ou
    /// sur le sol. Vrai si l'objet tenu était un bateau.
    pub(super) fn place_boat(&mut self, addr: &SocketAddr, (x, y, z): (i32, i32, i32)) -> bool {
        let Some(c) = self.clients.get(addr) else { return false };
        let Some(wood) = boat_wood(&self.held_name(c)).filter(|_| self.cfg.vanilla.boats) else { return false };
        let (survival, yaw) = (c.mode.takes_damage(), c.yaw);
        let (air, clicked) = (self.world.air, self.world.get(x, y, z));
        // Sur l'eau cliquée : à sa surface ; sur une herbe ou une fleur : à sa place, sur le sol
        // qui la porte ; sinon sur le bloc, dans l'air, l'herbe ou l'eau au-dessus
        let soft = |s: &Self, state: u32| state != air && !s.is_water(state) && s.npc_passable(state);
        let bottom = if self.is_water(clicked) {
            y as f32 + WATER_TOP - DRAUGHT
        } else if soft(self, clicked) {
            y as f32
        } else if crate::world::World::in_bounds(y + 1) && self.is_water(self.world.get(x, y + 1, z)) {
            y as f32 + 1.0 + WATER_TOP - DRAUGHT
        } else if crate::world::World::in_bounds(y + 1) && (self.world.get(x, y + 1, z) == air || soft(self, self.world.get(x, y + 1, z))) {
            y as f32 + 1.0
        } else {
            return true;
        };
        if self.boats.len() >= MAX_BOATS {
            return true;
        }
        let id = self.next_id;
        self.next_id += 1;
        let pos = (x as f32 + 0.5, bottom + ORIGIN, z as f32 + 0.5);
        self.boats.insert(id, Boat { pos, yaw: (yaw + 90.0).rem_euclid(360.0), wood, rider: None, mate: None, paddle: 0.0, vy: 0.0, sent: pos, sent_yaw: yaw, hits: 0, hit_tick: 0, driven: 0 });
        self.boats_dirty = true;
        if survival {
            if let Some(c) = self.clients.get_mut(addr) {
                c.inv.consume_held();
                c.inv_touched = true;
            }
            self.send_inventory(addr);
            self.show_held(addr);
        }
        self.show_boats();
        true
    }

    /// Montre à chaque joueur les bateaux proches, retire ceux dont il s'est éloigné.
    pub(super) fn show_boats(&mut self) {
        if self.boats.is_empty() {
            return;
        }
        const VIEW: f32 = 64.0;
        let ids: HashMap<SocketAddr, i64> = self.clients.iter().map(|(a, c)| (*a, c.id)).collect();
        let mut out: Vec<(SocketAddr, u32, Vec<u8>)> = Vec::new();
        for (addr, c) in self.clients.iter_mut().filter(|(_, c)| c.joined && c.stage == Stage::InGame) {
            for (id, b) in self.boats.iter() {
                let far = (b.pos.0 - c.pos.0).powi(2) + (b.pos.2 - c.pos.2).powi(2);
                if far < VIEW * VIEW {
                    if c.mobs_seen.insert(*id) {
                        out.push((*addr, bedrock::ID_ADD_ACTOR, Self::boat_packet(*id, b, b.rider.and_then(|r| ids.get(&r).copied()))));
                        if let Some(mate) = b.mate.and_then(|m| ids.get(&m).copied()) {
                            out.push((*addr, bedrock::ID_SET_ACTOR_LINK, bedrock::set_actor_link(*id, mate, LINK_PASSENGER, true)));
                        }
                    }
                } else if far > (VIEW + 16.0) * (VIEW + 16.0) && c.mobs_seen.remove(id) {
                    out.push((*addr, bedrock::ID_REMOVE_ACTOR, bedrock::remove_actor(*id)));
                }
            }
        }
        for (addr, packet, body) in out {
            self.send(&addr, packet, &body);
        }
    }

    /// Clic droit sur un bateau : le joueur s'y assoit, aux rames s'il est vide, sinon en
    /// passager. Vrai si c'était un bateau.
    pub(super) fn mount_boat(&mut self, addr: &SocketAddr, id: i64) -> bool {
        let Some(b) = self.boats.get(&id) else { return false };
        let Some((player, near, free)) = self.clients.get(addr).map(|c| (c.id, (c.pos.0 - b.pos.0).powi(2) + (c.pos.2 - b.pos.2).powi(2) < 8.0 * 8.0, c.riding.is_none() && c.dead.is_none())) else { return true };
        let (driver, full) = (b.rider, b.rider.is_some() && b.mate.is_some());
        if full || !near || !free {
            return true;
        }
        let tick = self.tick;
        if let Some(b) = self.boats.get_mut(&id) {
            match driver {
                None => {
                    b.rider = Some(*addr);
                    b.vy = 0.0;
                    b.driven = tick;
                }
                Some(_) => b.mate = Some(*addr),
            }
        }
        if let Some(c) = self.clients.get_mut(addr) {
            c.riding = Some(id);
            c.fall_peak = None;
        }
        let link = bedrock::set_actor_link(id, player, if driver.is_some() { LINK_PASSENGER } else { bedrock::LINK_RIDER }, false);
        self.send_to_viewers(id, bedrock::ID_SET_ACTOR_LINK, &link);
        match driver {
            // (à deux, le rameur avance d'un cran et le passager s'assoit derrière)
            Some(d) => {
                self.rider_data(&d, Some(SEAT_FRONT));
                self.rider_data(addr, Some(SEAT_BACK));
            }
            None => self.rider_data(addr, Some(SEAT)),
        }
        true
    }

    /// Ce joueur est-il aux rames de ce bateau ?
    pub(super) fn rows(&self, addr: &SocketAddr, id: i64) -> bool {
        self.boats.get(&id).map(|b| b.rider == Some(*addr)).unwrap_or(false)
    }

    /// L'eau, immobile ou courante.
    pub(super) fn is_water(&self, state: u32) -> bool {
        state == self.world.generator().blocks.water || (state != self.world.air && self.data.as_ref().and_then(|d| d.block_name(state)).map(|n| n.ends_with("water")).unwrap_or(false))
    }

    /// Le siège d'un bateau et sa position (pour montrer son passager aux autres).
    pub(super) fn boat_seat(&self, id: i64) -> Option<((f32, f32, f32), (f32, f32, f32))> {
        self.boats.get(&id).map(|b| (SEAT, b.pos))
    }

    /// Un occupant descend : son lien est retiré chez tous. Si c'était le rameur, son passager
    /// prend les rames. Vrai si c'était un bateau.
    pub(super) fn release_boat(&mut self, id: i64, player: i64) -> bool {
        if !self.boats.contains_key(&id) {
            return false;
        }
        let who = self.clients.iter().find(|(_, c)| c.id == player).map(|(a, _)| *a);
        let ids: HashMap<SocketAddr, i64> = self.clients.iter().map(|(a, c)| (*a, c.id)).collect();
        let Some(b) = self.boats.get_mut(&id) else { return false };
        let mut promoted = None;
        if b.mate.is_some() && b.mate == who {
            b.mate = None;
        } else {
            b.rider = b.mate.take();
            promoted = b.rider;
        }
        let stays = b.rider;
        b.sent = (f32::MAX, 0.0, 0.0);
        self.boats_dirty = true;
        let link = bedrock::set_actor_link(id, player, bedrock::LINK_REMOVE, false);
        self.send_to_viewers(id, bedrock::ID_SET_ACTOR_LINK, &link);
        if let Some(pid) = promoted.and_then(|p| ids.get(&p).copied()) {
            let link = bedrock::set_actor_link(id, pid, bedrock::LINK_RIDER, false);
            self.send_to_viewers(id, bedrock::ID_SET_ACTOR_LINK, &link);
        }
        // (celui qui reste est de nouveau seul à bord)
        if let Some(s) = stays {
            self.rider_data(&s, Some(SEAT));
        }
        true
    }

    /// Le jeu du passager déplace son bateau. Vrai si c'était un bateau.
    pub(super) fn steer_boat(&mut self, id: i64, pos: (f32, f32, f32), yaw: f32, player: Option<(f32, f32, f32)>) -> bool {
        let tick = self.tick;
        // Un bateau ne traverse pas les blocs : là où il y a un bloc plein, il n'avance pas,
        // et celui qui le mène est ramené à bord
        let free = |s: &Self, q: (f32, f32, f32)| s.mob_free(q.0.floor() as i32, (q.1 + 0.1).floor() as i32, q.2.floor() as i32) && s.mob_free(q.0.floor() as i32, (q.1 + 0.9).floor() as i32, q.2.floor() as i32);
        let old = self.boats.get(&id).map(|b| b.pos);
        if let Some(old) = old.filter(|_| pos.0.is_finite() && pos.1.is_finite() && pos.2.is_finite()) {
            if !free(self, pos) && free(self, old) {
                let rider = self.boats.get(&id).and_then(|b| b.rider);
                if let (Some(addr), true) = (rider, tick % 4 == 0) {
                    self.teleport(&addr, (old.0 + SEAT.0, old.1 + SEAT.1 + crate::player::EYE_HEIGHT - 1.02, old.2 + SEAT.2));
                }
                return true;
            }
        }
        let Some(b) = self.boats.get_mut(&id) else { return false };
        if let Some(p) = player {
            if (p.0 - pos.0).powi(2) + (p.2 - pos.2).powi(2) > 8.0 * 8.0 || !pos.0.is_finite() || !pos.1.is_finite() || !pos.2.is_finite() {
                return true;
            }
            b.driven = tick;
        }
        b.pos = pos;
        b.yaw = yaw.rem_euclid(360.0);
        true
    }

    /// Le passager a bougé sans que son jeu donne la position du bateau : il est sous lui.
    pub(super) fn boat_follows(&mut self, addr: &SocketAddr, id: i64, player: (f32, f32, f32), yaw: f32) -> bool {
        let tick = self.tick;
        let Some(b) = self.boats.get(&id) else { return false };
        // (le passager ne dirige rien)
        if b.rider == Some(*addr) && tick.saturating_sub(b.driven) >= 10 {
            let pos = (player.0, b.pos.1, player.2);
            let driven = b.driven;
            self.steer_boat(id, pos, yaw + 90.0, None);
            if let Some(b) = self.boats.get_mut(&id) {
                b.driven = driven;
            }
        }
        true
    }

    /// Un joueur frappe un bateau : cassé d'un coup en créatif, en quelques coups sinon, et il
    /// rend alors son objet. Vrai si c'était un bateau.
    pub(super) fn hit_boat(&mut self, addr: &SocketAddr, id: i64) -> bool {
        let tick = self.tick;
        let Some(creative) = self.clients.get(addr).map(|c| c.mode == GameMode::Creative) else { return false };
        let Some(b) = self.boats.get_mut(&id) else { return false };
        if tick.saturating_sub(b.hit_tick) > FORGET {
            b.hits = 0;
        }
        b.hits += 1;
        b.hit_tick = tick;
        if !creative && b.hits < HITS {
            self.send_to_viewers(id, bedrock::ID_ACTOR_EVENT, &bedrock::actor_event(id, bedrock::EVENT_HURT, 0));
            return true;
        }
        self.remove_boat(id, !creative);
        true
    }

    /// Retire un bateau du monde (son passager descend) ; `drop` : il rend son objet.
    pub(super) fn remove_boat(&mut self, id: i64, drop: bool) {
        let aboard: Vec<SocketAddr> = self.boats.get(&id).map(|b| b.mate.into_iter().chain(b.rider).collect()).unwrap_or_default();
        for a in aboard {
            self.dismount(&a);
        }
        let Some(b) = self.boats.remove(&id) else { return };
        self.boats_dirty = true;
        let viewers: Vec<SocketAddr> = self.clients.iter_mut().filter_map(|(a, c)| c.mobs_seen.remove(&id).then_some(*a)).collect();
        for v in viewers {
            self.send(&v, bedrock::ID_REMOVE_ACTOR, &bedrock::remove_actor(id));
        }
        if drop {
            if let Some(item) = self.item_id(&boat_item(b.wood)) {
                self.drop_stack((b.pos.0, b.pos.1, b.pos.2), (0.0, 0.2, 0.0), inventory::ItemStack::new(item, 1, 0), 10);
            }
        }
    }

    /// Chaque tick : un bateau sans passager flotte à la surface, ou tombe s'il n'est plus
    /// porté ; la lave le détruit. Les déplacements sont montrés à ceux qui le voient.
    pub(super) fn boats_tick(&mut self) {
        if self.boats.is_empty() {
            return;
        }
        if self.tick % 10 == 3 {
            self.show_boats();
        }
        let air = self.world.air;
        let ids: Vec<i64> = self.boats.keys().copied().collect();
        let mut lost: Vec<i64> = Vec::new();
        for id in ids {
            // (un passager parti sans descendre : le bateau est de nouveau libre)
            let left = |who: Option<SocketAddr>| who.filter(|r| self.clients.get(r).map(|c| c.riding != Some(id)).unwrap_or(true)).is_some();
            let (rider_gone, mate_gone) = self.boats.get(&id).map(|b| (left(b.rider), left(b.mate))).unwrap_or((false, false));
            if rider_gone || mate_gone {
                if let Some(b) = self.boats.get_mut(&id) {
                    if mate_gone {
                        b.mate = None;
                    }
                    if rider_gone {
                        b.rider = b.mate.take();
                    }
                    b.sent = (f32::MAX, 0.0, 0.0);
                }
            }
            let Some(b) = self.boats.get(&id) else { continue };
            let (mut pos, mut vy, ridden) = (b.pos, b.vy, b.rider.is_some());
            let (x, z) = (pos.0.floor() as i32, pos.2.floor() as i32);
            if !self.world.is_ready(x >> 4, z >> 4) {
                continue;
            }
            let bottom = pos.1 - ORIGIN;
            if bottom < crate::world::MIN_Y as f32 - 8.0 {
                lost.push(id);
                continue;
            }
            let cell = |y: f32| {
                let y = y.floor() as i32;
                if crate::world::World::in_bounds(y) { self.world.get(x, y, z) } else { air }
            };
            if self.data.as_ref().and_then(|d| d.block_name(cell(bottom + 0.1))).map(|n| n.contains("lava")).unwrap_or(false) {
                lost.push(id);
                continue;
            }
            if !ridden {
                // (la coque s'enfonce de DRAUGHT sous la surface, pas plus)
                if self.is_water(cell(bottom + DRAUGHT + 0.02)) {
                    vy = (vy + 0.02).min(0.12);
                } else if self.is_water(cell(bottom + DRAUGHT - 0.06)) {
                    vy = 0.0;
                    // Le courant l'emporte, tant que rien ne lui barre la route
                    let (dx, dz) = self.current((x, (bottom + DRAUGHT - 0.06).floor() as i32, z));
                    let (nx, nz) = (pos.0 + dx * DRIFT, pos.2 + dz * DRIFT);
                    let hull = (bottom + 0.3).floor() as i32;
                    if (dx != 0.0 || dz != 0.0) && self.mob_free((nx + dx * 0.7).floor() as i32, hull, (nz + dz * 0.7).floor() as i32) {
                        pos.0 = nx;
                        pos.2 = nz;
                    }
                } else if cell(bottom - 0.05) == air {
                    vy = (vy - 0.04).max(-1.2);
                } else {
                    // posé : sur le dessus du bloc qui le porte
                    if vy < 0.0 {
                        pos.1 = (bottom - 0.05).floor() + 1.0 + ORIGIN;
                    }
                    vy = 0.0;
                }
                pos.1 += vy;
            }
            let rowing = ridden && self.tick % 4 == 0;
            let Some(b) = self.boats.get_mut(&id) else { continue };
            // Les rames : elles bougent chez les autres joueurs tant que le bateau avance
            let advanced = ridden && (b.pos.0 - b.sent.0).abs() + (b.pos.2 - b.sent.2).abs() > 0.02;
            let mut oars = None;
            if advanced {
                b.paddle += 0.05;
                if rowing {
                    oars = Some((b.rider, bedrock::set_actor_data(id, &[(META_PADDLE_LEFT, bedrock::Meta::Float(b.paddle)), (META_PADDLE_RIGHT, bedrock::Meta::Float(b.paddle))])));
                }
            }
            b.pos = pos;
            b.vy = vy;
            if let Some((rider, packet)) = oars {
                let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| Some(**a) != rider && c.mobs_seen.contains(&id)).map(|(a, _)| *a).collect();
                for v in viewers {
                    self.send(&v, bedrock::ID_SET_ACTOR_DATA, &packet);
                }
            }
            let Some(b) = self.boats.get_mut(&id) else { continue };
            let moved = (b.pos.0 - b.sent.0).abs() + (b.pos.1 - b.sent.1).abs() + (b.pos.2 - b.sent.2).abs() > 0.01 || (b.yaw - b.sent_yaw).abs() > 2.0;
            if !moved {
                continue;
            }
            b.sent = b.pos;
            b.sent_yaw = b.yaw;
            let (rider, packet) = (b.rider, bedrock::move_actor_absolute(id, b.pos, 0.0, b.yaw, b.yaw, vy == 0.0));
            self.boats_dirty = true;
            // (le passager le dirige lui-même : on ne le lui renvoie pas)
            let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(a, c)| Some(**a) != rider && c.mobs_seen.contains(&id)).map(|(a, _)| *a).collect();
            for v in viewers {
                self.send(&v, bedrock::ID_MOVE_ACTOR_ABSOLUTE, &packet);
            }
        }
        for id in lost {
            self.remove_boat(id, false);
        }
    }

    /// Le sens du courant dans ce bloc d'eau : vers les voisins où l'eau est plus loin de sa
    /// source. (0, 0) dans une eau immobile.
    fn current(&self, (x, y, z): (i32, i32, i32)) -> (f32, f32) {
        let Some(d) = self.data.as_ref() else { return (0.0, 0.0) };
        let depth = |state: u32| match d.prop(state, "liquid_depth") {
            Some(crate::data::Prop::Int(n)) if n < 8 => n,
            _ => 0,
        };
        if !crate::world::World::in_bounds(y) {
            return (0.0, 0.0);
        }
        let here = depth(self.world.get(x, y, z));
        let (mut fx, mut fz) = (0.0f32, 0.0f32);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            if !self.world.is_ready((x + dx) >> 4, (z + dz) >> 4) {
                continue;
            }
            let next = self.world.get(x + dx, y, z + dz);
            if self.is_water(next) {
                let slope = (depth(next) - here) as f32;
                fx += dx as f32 * slope;
                fz += dz as f32 * slope;
            }
        }
        let length = (fx * fx + fz * fz).sqrt();
        if length < 0.01 { (0.0, 0.0) } else { (fx / length, fz / length) }
    }

    /// Les bateaux du monde, dans `boats.txt` : une ligne « x y z cap bois » par bateau.
    pub(super) fn save_boats(&mut self) {
        if !std::mem::take(&mut self.boats_dirty) {
            return;
        }
        let text: String = self.boats.values().map(|b| format!("{} {} {} {} {}\n", b.pos.0, b.pos.1, b.pos.2, b.yaw, b.wood)).collect();
        let path = std::path::PathBuf::from(&self.cfg.world_dir).join("boats.txt");
        if let Err(e) = std::fs::write(&path, text) {
            warn!("{} : {}", path.display(), e);
        }
    }

    pub(super) fn load_boats(&mut self) {
        let Ok(text) = std::fs::read_to_string(std::path::PathBuf::from(&self.cfg.world_dir).join("boats.txt")) else { return };
        for b in text.lines().filter_map(parse_boat).take(MAX_BOATS) {
            let id = self.next_id;
            self.next_id += 1;
            self.boats.insert(id, b);
        }
    }
}

/// Une ligne de `boats.txt`.
fn parse_boat(line: &str) -> Option<Boat> {
    let n: Vec<f32> = line.split_whitespace().map(|w| w.parse::<f32>().ok().filter(|v| v.is_finite())).collect::<Option<_>>()?;
    let [x, y, z, yaw, wood] = n[..] else { return None };
    ((wood as usize) < WOODS.len() && wood >= 0.0).then(|| Boat { pos: (x, y, z), yaw: yaw.rem_euclid(360.0), wood: wood as u8, rider: None, mate: None, paddle: 0.0, vy: 0.0, sent: (f32::MAX, 0.0, 0.0), sent_yaw: 0.0, hits: 0, hit_tick: 0, driven: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boat_items_and_woods_match() {
        assert_eq!((boat_wood("oak_boat"), boat_wood("dark_oak_boat"), boat_wood("bamboo_raft"), boat_wood("pale_oak_boat")), (Some(0), Some(5), Some(7), Some(9)));
        // Ni les bateaux à coffre, ni autre chose
        assert_eq!((boat_wood("oak_chest_boat"), boat_wood("bamboo_chest_raft"), boat_wood("stick"), boat_wood("stone_boat")), (None, None, None, None));
        // L'objet rendu redonne le même bois
        assert!((0..WOODS.len() as u8).all(|w| boat_wood(&boat_item(w)) == Some(w)));
        // Le fichier : relu tel qu'écrit, les lignes abîmées ignorées
        let b = parse_boat("12.5 63.1 -8.5 270 3").unwrap();
        assert_eq!((b.pos, b.yaw, b.wood), ((12.5, 63.1, -8.5), 270.0, 3));
        assert!(parse_boat("1 2 3 4").is_none() && parse_boat("1 2 3 4 99").is_none() && parse_boat("a b c d e").is_none() && parse_boat("NaN 2 3 4 1").is_none());
    }
}
