//! Les inventaires créés par les plugins (`server.inventory`) : un écran de coffre, de 27 ou
//! 54 cases, que le plugin remplit et ouvre chez un joueur. Verrouillé (par défaut), rien
//! n'en sort : chaque clic est rapporté au plugin, qui en fait un menu. Libre, c'est un vrai
//! coffre sans bloc, dont le plugin reçoit le contenu à la fermeture.
//!
//! Le jeu n'ouvre un écran de coffre que sur un coffre : le serveur lui en montre un faux,
//! au-dessus du joueur et pour lui seul, le temps que l'écran est ouvert. Le monde n'est
//! pas touché.

use super::*;

type Pos = (i32, i32, i32);

const MAX_MENUS: usize = 512;
const MAX_TITLE: usize = 64;
/// Ticks entre l'apparition du faux coffre et l'ouverture de son écran : le jeu doit avoir
/// reçu le bloc et son nom (et, pour un grand coffre, uni ses deux moitiés).
const SETTLE: u64 = 5;
/// Conteneurs du jeu qui désignent le bloc ouvert dans une demande d'inventaire.
const BLOCK_CONTAINERS: [u8; 3] = [7, 30, 58];

pub(super) struct Menu {
    title: String,
    slots: Vec<inventory::ItemStack>,
    /// Les joueurs peuvent y prendre et y poser des objets
    free: bool,
    /// Cases demandées avant que le serveur ne connaisse les objets du jeu (un plugin remplit
    /// souvent son inventaire dès son chargement) : (case, objet, quantité)
    waiting: Vec<(usize, String, u8)>,
}

/// La fiche d'un inventaire : « taille<TAB>libre ou verrouillé<TAB>titre ».
pub(super) fn parse_sheet(sheet: &str) -> Option<(usize, bool, String)> {
    let mut parts = sheet.splitn(3, '\t');
    let size = match parts.next()?.parse::<usize>().ok()? {
        27 => 27,
        54 => 54,
        _ => return None,
    };
    let free = parts.next()? == "free";
    let title: String = parts.next().unwrap_or("").chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect();
    Some((size, free, title))
}

/// Une case à remplir : « case<TAB>objet<TAB>quantité » (cases à partir de 1).
pub(super) fn parse_slot(sheet: &str) -> Option<(usize, String, u8)> {
    let mut parts = sheet.splitn(3, '\t');
    let slot = parts.next()?.parse::<f64>().ok().filter(|v| *v >= 1.0 && *v <= 54.0)? as usize - 1;
    let item = parts.next()?.to_string();
    let count = parts.next().and_then(|c| c.parse::<f64>().ok()).unwrap_or(1.0).clamp(0.0, 64.0) as u8;
    Some((slot, item, count))
}

impl Server {
    /// Une demande d'un plugin sur l'un de ses inventaires.
    pub(super) fn menu_action(&mut self, plugin: &str, id: &str, op: &str, sheet: &str) {
        let key = format!("{}\t{}", plugin, id);
        match op {
            "create" => match parse_sheet(sheet) {
                Some((size, free, title)) if self.menus.len() < MAX_MENUS || self.menus.contains_key(&key) => {
                    self.menus.insert(key, Menu { title, slots: vec![inventory::ItemStack::EMPTY; size], free, waiting: Vec::new() });
                }
                Some(_) => warn!("[{}] server.inventory : trop d'inventaires ouverts ({} au plus) ; détruisez ceux qui ne servent plus avec :destroy()", plugin, MAX_MENUS),
                None => warn!("[{}] server.inventory(titre, taille) : la taille est 27 ou 54", plugin),
            },
            "set" => {
                let Some((slot, item, count)) = parse_slot(sheet) else { return warn!("[{}] inventaire:setItem(case, objet, quantité) : la case va de 1 à la taille de l'inventaire", plugin) };
                // (les objets du jeu ne sont pas encore connus au chargement des plugins : la
                // case attend alors la première ouverture)
                if self.data.is_none() {
                    if let Some(m) = self.menus.get_mut(&key).filter(|m| slot < m.slots.len() && m.waiting.len() < 4096) {
                        m.waiting.push((slot, item, count));
                    }
                    return;
                }
                if !self.menu_fill(&key, slot, &item, count) {
                    return warn!("[{}] inventaire:setItem : objet inconnu « {} »", plugin, crate::log::clean(&item, 64));
                }
                self.menu_changed(&key);
            }
            "clear" => {
                if let Some(m) = self.menus.get_mut(&key) {
                    m.slots.iter_mut().for_each(|s| *s = inventory::ItemStack::EMPTY);
                    m.waiting.clear();
                }
                self.menu_changed(&key);
            }
            "destroy" => {
                let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.inv.menu.as_deref() == Some(key.as_str())).map(|(a, _)| *a).collect();
                for v in viewers {
                    self.force_close(&v);
                }
                self.menus.remove(&key);
            }
            _ => {}
        }
    }

    /// Met un objet dans une case (quantité 0 ou nom vide : la vide). Faux si l'objet est inconnu.
    fn menu_fill(&mut self, key: &str, slot: usize, item: &str, count: u8) -> bool {
        let stack = if count == 0 || item.is_empty() {
            inventory::ItemStack::EMPTY
        } else {
            let Some(item_id) = self.item_id(item) else { return false };
            self.tile_net_id = if self.tile_net_id >= i32::MAX - 1 { 1_000_000_000 } else { self.tile_net_id + 1 };
            inventory::ItemStack::new(item_id, count, self.tile_net_id)
        };
        if let Some(cell) = self.menus.get_mut(key).and_then(|m| m.slots.get_mut(slot)) {
            *cell = stack;
        }
        true
    }

    /// Remplit les cases qui attendaient que les objets du jeu soient connus.
    fn menu_settle(&mut self, key: &str) {
        let waiting = self.menus.get_mut(key).map(|m| std::mem::take(&mut m.waiting)).unwrap_or_default();
        for (slot, item, count) in waiting {
            if !self.menu_fill(key, slot, &item, count) {
                warn!("[{}] inventaire:setItem : objet inconnu « {} »", key.split('\t').next().unwrap_or(""), crate::log::clean(&item, 64));
            }
        }
    }

    /// Le contenu d'un inventaire a changé : ceux qui le regardent le reçoivent.
    fn menu_changed(&mut self, key: &str) {
        let Some(slots) = self.menus.get(key).map(|m| m.slots.clone()) else { return };
        let viewers: Vec<SocketAddr> = self.clients.iter().filter(|(_, c)| c.inv.menu.as_deref() == Some(key)).map(|(a, _)| *a).collect();
        for v in viewers {
            if let Some(c) = self.clients.get_mut(&v) {
                c.inv.block.slots = slots.clone();
            }
            self.send_block_content(&v, &slots);
        }
    }

    /// `inventaire:open(joueur)` (ou `joueur:closeInventory()`, identifiant vide) : l'écran
    /// ouvert se ferme d'abord, puis le faux coffre apparaît et l'écran s'ouvre un instant après.
    pub(super) fn open_menu(&mut self, addr: &SocketAddr, id: &str) {
        let had_window = self.clients.get(addr).map(|c| c.inv.open_window != 0).unwrap_or(false);
        if had_window {
            self.force_close(addr);
        }
        if id.is_empty() {
            return;
        }
        // (l'identifiant vient du plugin sans son nom : on retrouve l'inventaire qui le porte)
        let Some(key) = self.menus.keys().find(|k| k.rsplit('\t').next() == Some(id)).cloned() else { return };
        self.menu_settle(&key);
        let Some((title, size, free)) = self.menus.get(&key).map(|m| (m.title.clone(), m.slots.len(), m.free)) else { return };
        if free && self.clients.values().any(|c| c.inv.menu.as_deref() == Some(key.as_str())) {
            self.send(addr, bedrock::ID_TEXT, &bedrock::text_raw("§7Quelqu'un d'autre utilise déjà cet inventaire."));
            return;
        }
        let Some(chest) = self.data.as_ref().and_then(|d| d.default_state("minecraft:chest")) else { return };
        let Some(c) = self.clients.get(addr).filter(|c| c.joined && c.stage == Stage::InGame && c.dead.is_none()) else { return };
        // Au-dessus de sa tête, hors de sa vue ; sous ses pieds s'il est tout en haut du monde
        let (x, z) = (c.pos.0.floor() as i32, c.pos.2.floor() as i32);
        let y = if crate::world::World::in_bounds(c.pos.1.floor() as i32 + 3) { c.pos.1.floor() as i32 + 3 } else { c.pos.1.floor() as i32 - 4 };
        let blocks: Vec<Pos> = if size > 27 { vec![(x, y, z), (x + 1, y, z)] } else { vec![(x, y, z)] };
        use crate::nbt::Tag;
        for (i, p) in blocks.iter().enumerate() {
            let mut fields = vec![("id", Tag::string("Chest")), ("x", Tag::Int(p.0)), ("y", Tag::Int(p.1)), ("z", Tag::Int(p.2)), ("CustomName", Tag::string(&title))];
            if blocks.len() == 2 {
                let other = blocks[1 - i];
                fields.push(("pairx", Tag::Int(other.0)));
                fields.push(("pairz", Tag::Int(other.2)));
                fields.push(("pairlead", Tag::Byte((i == 0) as i8)));
            }
            self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(p.0, p.1, p.2, chest));
            self.send(addr, bedrock::ID_BLOCK_ACTOR_DATA, &bedrock::block_actor_data(*p, &Tag::compound(fields)));
        }
        let due = self.tick + SETTLE + if had_window { 4 } else { 0 };
        if let Some(c) = self.clients.get_mut(addr) {
            c.inv.menu_blocks = blocks;
            c.inv.menu_due = Some((key, due));
        }
    }

    /// Ouvre les écrans dont le faux coffre a eu le temps d'apparaître.
    pub(super) fn menus_tick(&mut self) {
        let tick = self.tick;
        let due: Vec<(SocketAddr, String)> = self
            .clients
            .iter_mut()
            .filter(|(_, c)| c.inv.menu_due.as_ref().map(|d| d.1 <= tick).unwrap_or(false))
            .filter_map(|(a, c)| c.inv.menu_due.take().map(|d| (*a, d.0)))
            .collect();
        for (addr, key) in due {
            let slots = self.menus.get(&key).map(|m| m.slots.clone());
            let ready = self.clients.get(&addr).map(|c| c.inv.open_window == 0 && !c.inv.menu_blocks.is_empty() && c.dead.is_none()).unwrap_or(false);
            let (Some(slots), true) = (slots, ready) else {
                self.hide_menu_blocks(&addr);
                continue;
            };
            const WINDOW: u8 = 2;
            let mut at = (0, 0, 0);
            if let Some(c) = self.clients.get_mut(&addr) {
                c.inv.open_window = WINDOW;
                c.inv.block_is_furnace = false;
                c.inv.block.slots = slots.clone();
                c.inv.menu = Some(key);
                at = c.inv.menu_blocks[0];
            }
            self.send(&addr, bedrock::ID_CONTAINER_OPEN, &bedrock::container_open_block(WINDOW, 0, at));
            self.send_block_content(&addr, &slots);
        }
    }

    /// Le vrai monde reparaît à la place des faux coffres.
    fn hide_menu_blocks(&mut self, addr: &SocketAddr) {
        let blocks = self.clients.get_mut(addr).map(|c| std::mem::take(&mut c.inv.menu_blocks)).unwrap_or_default();
        for p in blocks {
            let real = if crate::world::World::in_bounds(p.1) { self.world.get(p.0, p.1, p.2) } else { self.world.air };
            self.send(addr, bedrock::ID_UPDATE_BLOCK, &bedrock::update_block(p.0, p.1, p.2, real));
        }
    }

    /// L'écran d'un inventaire de plugin se ferme : les faux coffres disparaissent, et le
    /// plugin est prévenu (avec le contenu, pour un inventaire libre).
    pub(super) fn menu_closed(&mut self, addr: &SocketAddr) {
        let Some(key) = self.clients.get_mut(addr).and_then(|c| c.inv.menu.take()) else { return };
        self.hide_menu_blocks(addr);
        let Some(name) = self.clients.get(addr).and_then(|c| c.name.clone()) else { return };
        let id = key.rsplit('\t').next().unwrap_or("").to_string();
        let items: Vec<LVal> = self
            .menus
            .get(&key)
            .map(|m| m.slots.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, s)| LVal::Map(vec![("slot", LVal::Int(i as i64 + 1)), ("name", LVal::Str(self.item_name(s.id))), ("count", LVal::Int(s.count as i64))])).collect())
            .unwrap_or_default();
        self.emit("menu_close", &[LVal::Player(name), LVal::Str(id), LVal::List(items)]);
    }

    fn item_name(&self, id: i16) -> String {
        self.data.as_ref().and_then(|d| d.item_by_id(id)).map(|def| def.name.clone()).unwrap_or_default()
    }

    /// Une demande d'inventaire pendant qu'un inventaire de plugin est ouvert : le clic est
    /// rapporté au plugin. Verrouillé, la demande est refusée et l'écran remis en état : vrai,
    /// elle est traitée. Libre, faux : elle suit son cours normal.
    pub(super) fn menu_request(&mut self, addr: &SocketAddr, request: &bedrock::StackRequest) -> bool {
        let Some((key, name)) = self.clients.get(addr).and_then(|c| Some((c.inv.menu.clone()?, c.name.clone()?))) else { return false };
        let Some(free) = self.menus.get(&key).map(|m| m.free) else { return false };
        // La première case de l'inventaire que la demande touche
        let touched = request.actions.iter().find_map(|a| {
            let refs: Vec<bedrock::SlotRef> = match a {
                bedrock::StackAction::Take { from, to, .. } | bedrock::StackAction::Place { from, to, .. } => vec![*from, *to],
                bedrock::StackAction::Swap { a, b } => vec![*a, *b],
                bedrock::StackAction::Drop { from, .. } | bedrock::StackAction::Destroy { from, .. } => vec![*from],
                _ => Vec::new(),
            };
            refs.into_iter().find(|r| BLOCK_CONTAINERS.contains(&r.container)).map(|r| r.slot as usize)
        });
        let Some(slot) = touched else { return false };
        let stack = self.menus.get(&key).and_then(|m| m.slots.get(slot).copied()).unwrap_or(inventory::ItemStack::EMPTY);
        let id = key.rsplit('\t').next().unwrap_or("").to_string();
        let item = if stack.is_empty() { String::new() } else { self.item_name(stack.id) };
        if !free {
            // Refus d'abord : le plugin peut ouvrir un autre inventaire en réponse au clic
            self.send(addr, bedrock::ID_ITEM_STACK_RESPONSE, &bedrock::item_stack_response(request.id, None));
            self.send_inventory_with(addr, true);
            let view = self.clients.get(addr).map(|c| c.inv.block.slots.clone()).unwrap_or_default();
            self.send_block_content(addr, &view);
        }
        self.emit("menu_click", &[LVal::Player(name), LVal::Str(id), LVal::Int(slot as i64 + 1), LVal::Str(item), LVal::Int(stack.count as i64)]);
        !free
    }

    /// Avant une demande : la vue du joueur reprend le contenu de l'inventaire (les piles
    /// inchangées gardent le numéro que son jeu connaît).
    pub(super) fn load_menu_view(&mut self, addr: &SocketAddr) -> bool {
        let Some(key) = self.clients.get(addr).and_then(|c| c.inv.menu.clone()) else { return false };
        match (self.menus.get(&key).map(|m| m.slots.clone()), self.clients.get_mut(addr)) {
            (Some(slots), Some(c)) => {
                c.inv.block.slots = slots;
                true
            }
            _ => {
                self.force_close(addr);
                false
            }
        }
    }

    /// Après une demande acceptée : la vue du joueur est recopiée dans l'inventaire. Vrai si
    /// un inventaire de plugin était ouvert.
    pub(super) fn store_menu_view(&mut self, addr: &SocketAddr) -> bool {
        let Some((key, slots)) = self.clients.get(addr).and_then(|c| c.inv.menu.clone().map(|k| (k, c.inv.block.slots.clone()))) else { return false };
        if let Some(m) = self.menus.get_mut(&key).filter(|m| m.free && m.slots.len() == slots.len()) {
            m.slots = slots;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_sheets_are_checked() {
        assert_eq!(parse_sheet("27\tlocked\tBoutique"), Some((27, false, "Boutique".to_string())));
        assert_eq!(parse_sheet("54\tfree\tCoffre de \tguilde"), Some((54, true, "Coffre de guilde".to_string())));
        // Ni autre taille, ni fiche tronquée ; un titre trop long est coupé
        assert_eq!((parse_sheet("9\tlocked\tx"), parse_sheet("27"), parse_sheet("abc\tfree\tx")), (None, None, None));
        assert_eq!(parse_sheet(&format!("27\tlocked\t{}", "a".repeat(500))).unwrap().2.len(), MAX_TITLE);
        // Les cases se comptent à partir de 1
        assert_eq!(parse_slot("1\tminecraft:diamond\t3"), Some((0, "minecraft:diamond".to_string(), 3)));
        assert_eq!(parse_slot("54\tstone\t999"), Some((53, "stone".to_string(), 64)));
        assert_eq!(parse_slot("5\tapple"), Some((4, "apple".to_string(), 1)));
        assert_eq!((parse_slot("0\tstone\t1"), parse_slot("55\tstone\t1"), parse_slot("x\tstone\t1")), (None, None, None));
    }
}
