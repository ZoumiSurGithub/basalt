//! Créatures, objets au sol et bateaux écrits dans la base du monde comme le fait le jeu.
//!
//! Basalt garde ses propres fichiers (`entities/`, objets au sol, bateaux) : ce sont eux qu'il
//! relit, avec tout ce qu'il sait de chaque créature. Ce module en tient une copie au format
//! du jeu dans la base du monde, pour qu'un monde de Basalt ouvert ailleurs garde ses
//! entités. Seuls les mondes créés par Basalt sont concernés : la base d'un monde venu du jeu
//! n'est jamais modifiée de ce côté.

use super::mob_store::{chunk_of, region_of};
use super::mobs::MobRecord;
use super::*;
use crate::nbt::Tag;
use crate::world::mojang::actor_unique_id;

fn floats(values: &[f32]) -> Tag {
    Tag::List(5, values.iter().map(|v| Tag::Float(*v)).collect())
}

/// Les champs communs à toute entité, tels que le jeu les écrit.
fn actor(kind: &str, pos: (f32, f32, f32), yaw: f32, number: u32) -> Vec<(&'static str, Tag)> {
    vec![
        ("identifier", Tag::string(kind)),
        ("definitions", Tag::List(8, vec![Tag::string(&format!("+{}", kind))])),
        ("UniqueID", Tag::Long(actor_unique_id(number))),
        ("Pos", floats(&[pos.0, pos.1, pos.2])),
        ("Rotation", floats(&[yaw, 0.0])),
        ("OnGround", Tag::Byte(1)),
        ("FallDistance", Tag::Float(0.0)),
        ("Invulnerable", Tag::Byte(0)),
        ("PortalCooldown", Tag::Int(0)),
        ("LastDimensionId", Tag::Int(0)),
        ("Tags", Tag::List(0, Vec::new())),
    ]
}

/// Une créature, avec ce que le jeu sait lire de ce que Basalt en connaît.
pub(super) fn mob_nbt(r: &MobRecord, number: u32) -> Tag {
    let mut t = actor(r.species.id(), r.pos, r.yaw, number);
    let bit = |b: bool| Tag::Byte(b as i8);
    let health = Tag::compound(vec![
        ("Base", Tag::Float(r.max_health)),
        ("Current", Tag::Float(r.health)),
        ("DefaultMax", Tag::Float(r.max_health)),
        ("DefaultMin", Tag::Float(0.0)),
        ("Max", Tag::Float(r.max_health)),
        ("Min", Tag::Float(0.0)),
        ("Name", Tag::string("minecraft:health")),
    ]);
    t.extend([
        ("Air", Tag::Short(300)),
        ("Dead", Tag::Byte(0)),
        ("DeathTime", Tag::Short(0)),
        ("HurtTime", Tag::Short(0)),
        ("Attributes", Tag::List(10, vec![health])),
        ("IsBaby", bit(r.age < 0)),
        ("IsTamed", bit(r.tamed)),
        ("Sheared", bit(r.sheared)),
        ("Saddled", bit(r.saddled)),
        ("Chested", bit(r.chested)),
        ("Sitting", bit(r.sitting)),
        ("Color", Tag::Byte(r.color as i8)),
        ("Color2", Tag::Byte(0)),
        ("Variant", Tag::Int(r.variant)),
        ("MarkVariant", Tag::Int(r.mark)),
        ("Strength", Tag::Int(r.strength as i32)),
        ("NaturalSpawn", bit(r.natural)),
        ("LeasherID", Tag::Long(-1)),
        ("OwnerNew", Tag::Long(-1)),
        ("TargetID", Tag::Long(-1)),
        ("canPickupItems", Tag::Byte(0)),
        ("expDropEnabled", Tag::Byte(1)),
    ]);
    Tag::compound(t)
}

/// Un objet posé au sol.
pub(super) fn item_nbt(name: &str, count: u8, damage: u16, pos: (f32, f32, f32), number: u32) -> Tag {
    let mut t = actor("minecraft:item", pos, 0.0, number);
    let stack = Tag::compound(vec![("Name", Tag::string(name)), ("Count", Tag::Byte(count as i8)), ("Damage", Tag::Short(damage as i16)), ("WasPickedUp", Tag::Byte(0))]);
    t.extend([("Item", stack), ("Age", Tag::Short(0)), ("Health", Tag::Short(5))]);
    Tag::compound(t)
}

/// Un bateau (sa variante est son bois).
pub(super) fn boat_nbt(wood: u8, pos: (f32, f32, f32), yaw: f32, number: u32) -> Tag {
    let mut t = actor("minecraft:boat", pos, yaw, number);
    t.push(("Variant", Tag::Int(wood as i32)));
    Tag::compound(t)
}

impl Server {
    /// Tient à jour, dans la base du monde, la copie au format du jeu des entités des régions
    /// en mémoire. Un chunk n'est réécrit que si ses entités ont changé.
    pub(super) fn export_actors(&mut self, live: &HashMap<(i32, i32), Vec<MobRecord>>) {
        let Some(store) = self.world.db_store().cloned().filter(|s| s.native) else { return };
        let mut next = match self.mob_store.next_actor {
            Some(n) => n,
            None => store.next_actor(),
        };
        // Tout ce qui se trouve dans chaque chunk : (description pour comparer, de quoi l'écrire)
        enum What {
            Mob(MobRecord),
            Item(String, u8, u16, (f32, f32, f32)),
            Boat(u8, (f32, f32, f32), f32),
        }
        let mut found: HashMap<(i32, i32), Vec<(String, What)>> = HashMap::new();
        let regions: Vec<(i32, i32)> = self.mob_store.regions.keys().copied().collect();
        for r in &regions {
            let stored = self.mob_store.regions.get(r).into_iter().flat_map(|region| region.chunks.values().flatten());
            for record in stored.chain(live.get(r).into_iter().flatten()) {
                found.entry(chunk_of(record.pos.0, record.pos.2)).or_default().push((record.to_json().to_json(), What::Mob(record.clone())));
            }
        }
        let loaded = |pos: (f32, f32, f32)| regions.contains(&region_of(chunk_of(pos.0, pos.2)));
        if let Some(d) = &self.data {
            for g in self.ground.values().filter(|g| loaded(g.body.pos)) {
                let Some(def) = d.item_by_id(g.item) else { continue };
                // (au dixième de bloc : un objet qui frémit au sol ne fait pas réécrire son chunk)
                let p = g.body.pos;
                let text = format!("objet {} {} {} {:.1} {:.1} {:.1}", def.name, g.count, g.damage, p.0, p.1, p.2);
                found.entry(chunk_of(p.0, p.2)).or_default().push((text, What::Item(def.name.clone(), g.count, g.damage, p)));
            }
        }
        for b in self.boats.values().filter(|b| loaded(b.pos)) {
            let text = format!("bateau {} {:.1} {:.1} {:.1} {:.0}", b.wood, b.pos.0, b.pos.1, b.pos.2, b.yaw);
            found.entry(chunk_of(b.pos.0, b.pos.2)).or_default().push((text, What::Boat(b.wood, b.pos, b.yaw)));
        }
        // Première fois pour une région depuis le démarrage : ses chunks qui portent encore des
        // entités d'un précédent lancement sont à revoir, même s'ils sont vides aujourd'hui.
        // (1 024 lectures de la base par région : faites sur le fil des écritures, pas pendant
        // le tick — en courant vers de nouvelles régions, elles figeaient le serveur)
        let mut stale_regions = Vec::new();
        for r in &regions {
            if self.mob_store.exported_regions.insert(*r) {
                stale_regions.push(*r);
            }
        }
        // Les chunks dont les entités ont disparu depuis le dernier passage
        let gone: Vec<(i32, i32)> = self.mob_store.exported.keys().filter(|c| regions.contains(&region_of(**c)) && !found.contains_key(*c)).copied().collect();
        for chunk in gone {
            found.insert(chunk, Vec::new());
        }
        let mut writes: Vec<((i32, i32), Vec<(u32, Vec<u8>)>)> = Vec::new();
        let busy: std::collections::HashSet<(i32, i32)> = found.keys().copied().collect();
        for (chunk, mut list) in found {
            list.sort_by(|a, b| a.0.cmp(&b.0));
            let mark = list.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, (text, _)| text.bytes().chain([0]).fold(h, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)));
            if self.mob_store.exported.get(&chunk) == Some(&mark) {
                continue;
            }
            let actors: Vec<(u32, Vec<u8>)> = list
                .iter()
                .map(|(_, what)| {
                    let number = next;
                    next = next.wrapping_add(1).max(1);
                    let tag = match what {
                        What::Mob(r) => mob_nbt(r, number),
                        What::Item(name, count, damage, pos) => item_nbt(name, *count, *damage, *pos, number),
                        What::Boat(wood, pos, yaw) => boat_nbt(*wood, *pos, *yaw, number),
                    };
                    (number, crate::nbt::write_le(&tag))
                })
                .collect();
            // (noté tout de suite : l'écriture part sur le fil des écritures, dans l'ordre)
            if list.is_empty() {
                self.mob_store.exported.remove(&chunk);
            } else {
                self.mob_store.exported.insert(chunk, mark);
            }
            writes.push((chunk, actors));
        }
        self.mob_store.next_actor = Some(next);
        if writes.is_empty() && stale_regions.is_empty() {
            return;
        }
        crate::background::DISK.run(move || {
            for r in stale_regions {
                for cx in r.0 << 5..(r.0 + 1) << 5 {
                    for cz in r.1 << 5..(r.1 + 1) << 5 {
                        if !busy.contains(&(cx, cz)) && store.has_actors(cx, cz) {
                            if let Err(e) = store.write_actors(cx, cz, &[]) {
                                warn!("Entités du chunk {} {} non effacées de la base du monde : {}", cx, cz, e);
                            }
                        }
                    }
                }
            }
            for (chunk, actors) in writes {
                if let Err(e) = store.write_actors(chunk.0, chunk.1, &actors) {
                    warn!("Entités du chunk {} {} non écrites dans la base du monde : {}", chunk.0, chunk.1, e);
                }
            }
        });
    }
}
