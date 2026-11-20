//! Objets posés au sol : apparition et ramassage.
//!
//! Dispositions d'après la documentation du protocole, pas relevées auprès du serveur
//! officiel 1.26 ; l'objet est écrit comme dans les inventaires (format vérifié avec le jeu).
//! Si le jeu les refuse, le serveur revient à l'ancien comportement (butin dans l'inventaire).

#[allow(unused_imports)]
use super::*;

pub const ID_ADD_ITEM_ACTOR: u32 = 15;
pub const ID_TAKE_ITEM_ACTOR: u32 = 17;

/// AddItemActor : un objet apparaît au sol.
pub fn add_item_actor(entity_id: i64, item: i16, count: u8, block: u32, pos: (f32, f32, f32), motion: (f32, f32, f32)) -> Vec<u8> {
    let mut w = W::default();
    w.vari64(entity_id);
    w.varu64(entity_id as u64);
    item_stack(&mut w, item, count, 0, block);
    for v in [pos.0, pos.1, pos.2, motion.0, motion.1, motion.2] {
        w.f32(v);
    }
    w.varu32(0); // données de l'entité : aucune
    w.bool(false); // pêché : non
    w.0
}

/// TakeItemActor : un joueur ramasse l'objet (animation de l'objet qui vole vers lui).
pub fn take_item_actor(item_entity: i64, taker: i64) -> Vec<u8> {
    let mut w = W::default();
    w.varu64(item_entity as u64);
    w.varu64(taker as u64);
    w.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_items_have_the_documented_layout() {
        let p = add_item_actor(5, 1, 3, 0, (0.5, -60.0, 0.5), (0.0, 0.0, 0.0));
        // identifiants (zigzag puis non signé), objet (8 octets : identifiant, quantité, variante,
        // numéro de pile absent, bloc, données), 6 nombres, données vides, indicateur de pêche
        assert_eq!(&p[..2], &[10, 5]);
        assert_eq!(p.len(), 2 + 8 + 24 + 2);
        assert_eq!(take_item_actor(5, 300), vec![5, 0xac, 0x02]);
    }
}
