//! Les blocs qui s'ouvrent : le son des portes, portillons, trappes et boutons, le couvercle
//! et le son des coffres, et les coffres doubles (deux coffres voisins, un seul inventaire
//! de 54 cases).

use super::*;

type Pos = (i32, i32, i32);

/// Le son d'un bloc qui s'ouvre ou se ferme (nom sans `minecraft:`). Le jeu choisit lui-même
/// la variante selon la matière du bloc (bois, fer, cuivre…).
pub(super) fn toggle_sound(short: &str, open: bool) -> Option<&'static str> {
    let pick = |a: &'static str, b: &'static str| Some(if open { a } else { b });
    if short.ends_with("trapdoor") {
        pick("trapdoor.open", "trapdoor.close")
    } else if short.ends_with("fence_gate") {
        pick("fence_gate.open", "fence_gate.close")
    } else if short.ends_with("door") {
        pick("door.open", "door.close")
    } else if short.ends_with("button") {
        pick("button.click_on", "button.click_off")
    } else {
        None
    }
}

/// Le son d'un conteneur qui s'ouvre ou se ferme, et s'il a un couvercle qui s'anime.
pub(super) fn lid_sound(short: &str, open: bool) -> Option<(&'static str, bool)> {
    let pick = |a: &'static str, b: &'static str, lid: bool| Some((if open { a } else { b }, lid));
    if short == "ender_chest" {
        pick("enderchest.open", "enderchest.closed", true)
    } else if short.ends_with("chest") {
        pick("chest.open", "chest.closed", true)
    } else if short.ends_with("shulker_box") {
        pick("shulkerbox.open", "shulkerbox.closed", true)
    } else if short == "barrel" {
        pick("block.barrel.open", "block.barrel.close", false)
    } else {
        None
    }
}

/// Deux coffres voisins : celui dont les 27 premières cases s'affichent en haut vient en
/// premier (le plus petit z, puis le plus petit x, comme dans le jeu).
pub(super) fn ordered(a: Pos, b: Pos) -> [Pos; 2] {
    if (b.2, b.0) < (a.2, a.0) { [b, a] } else { [a, b] }
}

impl Server {
    fn short_name(&self, state: u32) -> String {
        self.data.as_ref().and_then(|d| d.block_name(state)).unwrap_or("").trim_start_matches("minecraft:").to_string()
    }

    fn block_sound(&mut self, pos: Pos, sound: &str, state: u32) {
        let centre = (pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32 + 0.5);
        self.send_near(centre, bedrock::ID_LEVEL_SOUND_EVENT, &bedrock::level_sound_with(sound, centre, state as i32, "", false, -1));
    }

    /// Porte, portillon, trappe ou bouton qui vient de changer d'état : son bruit.
    pub(super) fn toggled_sound(&mut self, pos: Pos, state: u32) {
        let Some(d) = self.data.clone() else { return };
        let open = d.prop(state, "open_bit") == Some(Prop::Int(1)) || d.prop(state, "button_pressed_bit") == Some(Prop::Int(1));
        if let Some(sound) = toggle_sound(&self.short_name(state), open) {
            self.block_sound(pos, sound, state);
        }
    }

    /// Les moitiés d'un conteneur, dans l'ordre de leurs cases : une seule, ou les deux
    /// coffres d'un coffre double.
    pub(super) fn halves(&self, pos: Pos) -> Vec<Pos> {
        let partner = self.world.tiles.get(pos).and_then(|t| t.pair()).map(|(x, z)| (x, pos.1, z));
        match partner {
            // (l'autre doit exister et le désigner en retour)
            Some(p) if self.world.tiles.get(p).and_then(|t| t.pair()) == Some((pos.0, pos.2)) && self.world.tiles.get(p).map(|t| t.inventory().is_some()).unwrap_or(false) => ordered(pos, p).to_vec(),
            _ => vec![pos],
        }
    }

    /// Le contenu d'un conteneur, coffre double compris (54 cases).
    pub(super) fn container_view(&self, pos: Pos) -> Option<Vec<inventory::ItemStack>> {
        let mut all = Vec::new();
        for half in self.halves(pos) {
            all.extend_from_slice(self.world.tiles.get(half)?.inventory()?.slots());
        }
        Some(all)
    }

    /// Recopie une vue dans le conteneur (les cases qui ont changé seulement).
    pub(super) fn container_store(&mut self, pos: Pos, slots: &[inventory::ItemStack]) {
        let mut offset = 0;
        for half in self.halves(pos) {
            let Some(inv) = self.world.tiles.get_mut(half).and_then(|t| t.inventory_mut()) else { return };
            let size = inv.slots().len();
            for i in 0..size {
                if let Some(s) = slots.get(offset + i) {
                    if inv.slots().get(i) != Some(s) {
                        inv.set_item(i, *s);
                    }
                }
            }
            offset += size;
        }
    }

    /// Le couvercle d'un conteneur s'ouvre ou se referme chez ceux qui le voient, avec son bruit.
    pub(super) fn lid(&mut self, pos: Pos, open: bool) {
        let state = self.world.get(pos.0, pos.1, pos.2);
        let short = self.short_name(state);
        let Some((sound, lid)) = lid_sound(&short, open) else { return };
        if open {
            self.lids.insert(pos);
        } else {
            self.lids.remove(&pos);
        }
        self.block_sound(pos, sound, state);
        for half in self.halves(pos) {
            let centre = (half.0 as f32 + 0.5, half.1 as f32 + 0.5, half.2 as f32 + 0.5);
            if lid {
                self.send_near(centre, bedrock::ID_BLOCK_EVENT, &bedrock::block_event(half, 1, open as i32));
            } else if let Some(next) = self.data.as_ref().and_then(|d| d.with_props(state, &[("open_bit", Prop::Int(open as i32))])) {
                // (le tonneau n'a pas de couvercle : c'est son bloc qui s'ouvre)
                if next != state && self.world.set(half.0, half.1, half.2, next) {
                    self.broadcast(bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(half.0, half.1, half.2, next), None);
                }
            }
        }
    }

    /// Referme les couvercles que plus personne ne regarde (écran fermé, joueur parti).
    pub(super) fn lids_tick(&mut self) {
        if self.lids.is_empty() {
            return;
        }
        let watched: HashSet<Pos> = self.clients.values().filter_map(|c| c.inv.block_pos).collect();
        let idle: Vec<Pos> = self.lids.iter().filter(|p| !watched.contains(p)).copied().collect();
        for pos in idle {
            self.lid(pos, false);
        }
    }

    /// Un coffre vient d'être posé : il s'unit au coffre voisin de même sorte et de même
    /// orientation, s'il est seul. Pas en étant accroupi, comme dans le jeu.
    pub(super) fn pair_chest(&mut self, addr: &SocketAddr, pos: Pos) {
        if self.clients.get(addr).map(|c| c.sneaking).unwrap_or(false) {
            // (accroupi : il restera simple, même à la première ouverture)
            if let Some(t) = self.world.tiles.get_mut(pos) {
                t.set_alone(true);
            }
            return;
        }
        self.pair_chest_at(pos);
    }

    /// Unit ce coffre à un voisin libre, s'il y en a un. Sert à la pose, et à la première
    /// ouverture pour les coffres arrivés autrement (`/setblock`, anciens mondes) ; les
    /// coffres posés accroupi restent simples.
    pub(super) fn pair_chest_at(&mut self, pos: Pos) {
        let Some(d) = self.data.clone() else { return };
        let state = self.world.get(pos.0, pos.1, pos.2);
        let name = d.block_name(state).unwrap_or("").to_string();
        let ready = self.cfg.vanilla.double_chests && self.world.tiles.get(pos).map(|t| t.pair().is_none() && !t.alone() && t.inventory().is_some()).unwrap_or(false);
        if !matches!(name.as_str(), "minecraft:chest" | "minecraft:trapped_chest") || !ready {
            return;
        }
        let facing = d.prop(state, "minecraft:cardinal_direction");
        // (les deux coffres sont côte à côte, face au même côté)
        let sides: [(i32, i32); 2] = match &facing {
            Some(Prop::Str(dir)) if dir == "north" || dir == "south" => [(1, 0), (-1, 0)],
            _ => [(0, 1), (0, -1)],
        };
        for (dx, dz) in sides {
            let other = (pos.0 + dx, pos.1, pos.2 + dz);
            let there = self.world.get(other.0, other.1, other.2);
            let free = self.world.tiles.get(other).map(|t| t.pair().is_none() && !t.alone() && t.inventory().is_some()).unwrap_or(false);
            if free && d.block_name(there) == Some(name.as_str()) && d.prop(there, "minecraft:cardinal_direction") == facing {
                if let Some(t) = self.world.tiles.get_mut(pos) {
                    t.set_pair(Some((other.0, other.2)));
                }
                if let Some(t) = self.world.tiles.get_mut(other) {
                    t.set_pair(Some((pos.0, pos.2)));
                }
                self.world.tiles.dirty = true;
                self.pairs.insert(pos, (other.0, other.2));
                self.pairs.insert(other, (pos.0, pos.2));
                self.pairs_changed(&[pos, other]);
                return;
            }
        }
    }

    /// Un coffre va disparaître : son voisin redevient un coffre simple, et l'écran de ceux
    /// qui regardaient le coffre double se ferme.
    pub(super) fn unpair_chest(&mut self, pos: Pos) {
        let Some((x, z)) = self.pairs.remove(&pos) else { return };
        let other = (x, pos.1, z);
        self.pairs.remove(&other);
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.inv.block_pos == Some(pos) || c.inv.block_pos == Some(other)).map(|(a, _)| *a).collect();
        for v in viewers {
            self.force_close(&v);
        }
        for p in [pos, other] {
            if let Some(t) = self.world.tiles.get_mut(p) {
                t.set_pair(None);
            }
        }
        self.world.tiles.dirty = true;
        self.pairs_changed(&[pos, other]);
    }

    /// Ces coffres ont changé de voisin : leurs données repartent chez les joueurs.
    fn pairs_changed(&mut self, list: &[Pos]) {
        for c in self.clients.values_mut() {
            for p in list {
                c.pairs_seen.remove(p);
                c.pairs_stale.insert(*p);
            }
        }
    }

    /// Les coffres doubles relus du monde enregistré.
    pub(super) fn load_pairs(&mut self) {
        self.pairs.clear();
        for pos in self.world.tiles.positions() {
            if let Some(pair) = self.world.tiles.get(pos).and_then(|t| t.pair()) {
                self.pairs.insert(pos, pair);
            }
        }
    }

    fn chest_tag(pos: Pos, pair: Option<(i32, i32)>, lead: bool) -> crate::nbt::Tag {
        use crate::nbt::Tag;
        let mut fields = vec![("id", Tag::string("Chest")), ("x", Tag::Int(pos.0)), ("y", Tag::Int(pos.1)), ("z", Tag::Int(pos.2)), ("isMovable", Tag::Byte(1))];
        if let Some((x, z)) = pair {
            fields.push(("pairx", Tag::Int(x)));
            fields.push(("pairz", Tag::Int(z)));
            fields.push(("pairlead", Tag::Byte(lead as i8)));
        }
        Tag::compound(fields)
    }

    /// Annonce à chaque joueur les coffres doubles des chunks qu'il a reçus (le jeu dessine
    /// deux coffres simples tant qu'il ne sait pas qu'ils sont unis), et ceux qui ne le sont plus.
    pub(super) fn sync_pairs(&mut self) {
        let mut out: Vec<(SocketAddr, Vec<u8>)> = Vec::new();
        for (addr, c) in self.clients.iter_mut() {
            if !(c.joined && c.stage == Stage::InGame) {
                continue;
            }
            for pos in std::mem::take(&mut c.pairs_stale) {
                if !self.pairs.contains_key(&pos) && c.delivered.contains(&(pos.0 >> 4, pos.2 >> 4)) {
                    out.push((*addr, bedrock::block_actor_data(pos, &Self::chest_tag(pos, None, false))));
                }
            }
            if self.pairs.is_empty() {
                continue;
            }
            let delivered = &c.delivered;
            c.pairs_seen.retain(|p| delivered.contains(&(p.0 >> 4, p.2 >> 4)));
            for (pos, pair) in &self.pairs {
                // (les deux moitiés d'un coup, une fois leurs deux chunks reçus)
                let other = (pair.0, pos.1, pair.1);
                if c.delivered.contains(&(pos.0 >> 4, pos.2 >> 4)) && c.delivered.contains(&(other.0 >> 4, other.2 >> 4)) && c.pairs_seen.insert(*pos) {
                    out.push((*addr, bedrock::block_actor_data(*pos, &Self::chest_tag(*pos, Some(*pair), ordered(*pos, other)[0] == *pos))));
                }
            }
        }
        for (addr, packet) in out {
            self.send(&addr, bedrock::ID_BLOCK_ACTOR_DATA, &packet);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_that_open_have_their_sounds() {
        assert_eq!((toggle_sound("oak_door", true), toggle_sound("iron_door", false), toggle_sound("spruce_fence_gate", true), toggle_sound("oak_trapdoor", false)), (Some("door.open"), Some("door.close"), Some("fence_gate.open"), Some("trapdoor.close")));
        assert_eq!((toggle_sound("stone_button", true), toggle_sound("stone_button", false), toggle_sound("stone", true)), (Some("button.click_on"), Some("button.click_off"), None));
        // Le coffre a un couvercle qui s'anime, le tonneau non
        assert_eq!((lid_sound("chest", true), lid_sound("trapped_chest", false), lid_sound("ender_chest", true), lid_sound("barrel", true), lid_sound("furnace", true)), (Some(("chest.open", true)), Some(("chest.closed", true)), Some(("enderchest.open", true)), Some(("block.barrel.open", false)), None));
        assert_eq!(lid_sound("red_shulker_box", false), Some(("shulkerbox.closed", true)));
        // Coffre double : la moitié au plus petit z, puis au plus petit x, vient en premier
        assert_eq!(ordered((5, 64, 3), (4, 64, 3)), [(4, 64, 3), (5, 64, 3)]);
        assert_eq!(ordered((5, 64, 3), (5, 64, 2)), [(5, 64, 2), (5, 64, 3)]);
        assert_eq!(ordered((-1, 64, 0), (0, 64, 0)), [(-1, 64, 0), (0, 64, 0)]);
    }
}
