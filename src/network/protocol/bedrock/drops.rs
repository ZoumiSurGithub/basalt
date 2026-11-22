//! Objets posés au sol : apparition et ramassage.
//!
//! Dispositions d'après la documentation du protocole, pas relevées auprès du serveur
//! officiel 1.26 ; l'objet est écrit comme dans les inventaires (format vérifié avec le jeu).
//! Si le jeu les refuse, le serveur revient à l'ancien comportement (butin dans l'inventaire).

use super::*;

pub const ID_ADD_ITEM_ACTOR: u32 = 15;
pub const ID_TAKE_ITEM_ACTOR: u32 = 17;

/// Données d'entité d'un objet au sol, relevées telles quelles auprès du serveur officiel
/// 1.26.65 (64 entrées : indicateurs, taille 1, boîte de collision 0,25…). Sans elles, le jeu
/// accepte le paquet mais n'affiche pas l'objet (taille nulle).
const ITEM_METADATA_HEX: &str = concat!(
    "40000707808080808080800301020202020202000300000004040400050707010701012c011600000025070700260303",
    "0000803f270000002a01012c012b0202002c0000002d0202002e0202003503030000803e3603030000803e3702020138",
    "0808000000000000000000000000390000003a0303000000003b0000003c030300000000460000004704040048040400",
    "490000014a0000004b0202004c0202004d0202004e020201510000ff520000005c070700650202006602020067020200",
    "680202006a0202066b0000016c0303000000416d0303000080416e040407616d6269656e746f03030000803f72020200",
    "7302020074020200750202007605050a000077000000780303000000007c03030000803f7d0202287e0202ce09840104",
    "040086010303000000008701030300000000880102020189010202018a010202018b010707008c01030300008042",
);

/// AddItemActor : un objet apparaît au sol. Disposition relevée auprès du serveur officiel.
pub fn add_item_actor(entity_id: i64, item: i16, count: u8, block: u32, pos: (f32, f32, f32), motion: (f32, f32, f32)) -> Vec<u8> {
    let mut w = W::default();
    w.vari64(item_unique_id(entity_id));
    w.varu64(entity_id as u64);
    // L'objet : comme dans les inventaires, mais ses données annexes ne sont pas vides : 10
    // octets à zéro (pas de NBT, aucun bloc « peut être posé sur » ni « peut casser »)
    w.0.extend_from_slice(&item.to_le_bytes());
    w.0.extend_from_slice(&(count as u16).to_le_bytes());
    w.varu32(0); // variante
    w.bool(false); // pas de numéro de pile
    w.varu32(block);
    w.varu32(10);
    w.0.extend_from_slice(&[0u8; 10]);
    for v in [pos.0, pos.1, pos.2, motion.0, motion.1, motion.2] {
        w.f32(v);
    }
    w.0.extend_from_slice(&item_metadata());
    w.bool(false); // pêché : non
    w.0
}

/// Le paquet relevé, à l'identique (identifiants compris : 34 à l'exécution). Pour un essai.
pub fn add_item_actor_recorded() -> Vec<u8> {
    let pos = (0.7581789493560791, -59.28812789916992, 0.8182617425918579);
    let motion = (-0.07429742068052292, 0.20000000298023224, -0.054183147847652435);
    let mut p = add_item_actor(34, 3, 1, 2_186_211_206, pos, motion);
    // Identifiant unique d'origine (-85899345913) à la place du nôtre
    let ours = { let mut w = W::default(); w.vari64(item_unique_id(34)); w.0.len() };
    p.splice(0..ours, [0xf1, 0xff, 0xff, 0xff, 0xff, 0x04]);
    p
}

/// Données d'entité d'un objet au sol, relevées sur un vrai serveur.
fn item_metadata() -> Vec<u8> {
    let h = ITEM_METADATA_HEX.as_bytes();
    (0..h.len() / 2).map(|i| u8::from_str_radix(std::str::from_utf8(&h[2 * i..2 * i + 2]).unwrap_or("00"), 16).unwrap_or(0)).collect()
}

/// Identifiant « unique » d'un objet au sol, distinct de son identifiant d'exécution, comme
/// ceux du serveur officiel (le jeu affiche l'objet relevé, pas le même avec un identifiant
/// unique égal à son identifiant d'exécution). À utiliser aussi pour le retirer (RemoveActor).
pub fn item_unique_id(entity_id: i64) -> i64 {
    -(1i64 << 36) - entity_id
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
    fn dropped_items_have_the_recorded_layout() {
        // Même objet que dans le relevé du serveur officiel : de la terre (objet 3, bloc
        // 0x0122ef6fc6 en entier variable), à la même position
        let pos = (0.7581789493560791, -59.28812789916992, 0.8182617425918579);
        let motion = (-0.07429742068052292, 0.20000000298023224, -0.054183147847652435);
        let p = add_item_actor(34, 3, 1, 2_186_211_206, pos, motion);
        // Après les deux identifiants (ici 2 octets), tout est identique au relevé : objet,
        // position, élan, données d'entité, indicateur de pêche
        let mut recorded = vec![0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x86, 0xdf, 0xbb, 0x92, 0x08, 0x0a];
        recorded.extend_from_slice(&[0; 10]);
        for v in [pos.0, pos.1, pos.2, motion.0, motion.1, motion.2] {
            recorded.extend_from_slice(&v.to_le_bytes());
        }
        recorded.extend_from_slice(&item_metadata());
        recorded.push(0);
        let head = { let mut w = W::default(); w.vari64(item_unique_id(34)); w.varu64(34); w.0 };
        assert_eq!(&p[..head.len()], &head[..]);
        assert_eq!(&p[head.len()..], &recorded[..]);
        assert_eq!(item_metadata().len(), 334);
        assert_eq!(item_metadata()[0], 0x40, "64 entrées");
        let exact = add_item_actor_recorded();
        assert_eq!(&exact[..7], &[0xf1, 0xff, 0xff, 0xff, 0xff, 0x04, 0x22]);
        assert_eq!(exact.len(), 388, "même taille que le relevé");
        assert_eq!(take_item_actor(5, 300), vec![5, 0xac, 0x02]);
    }
}
