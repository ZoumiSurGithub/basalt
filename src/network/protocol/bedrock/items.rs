//! Objets : registre, description d'une pile, inventaire, équipement.

#[allow(unused_imports)]
use super::*;

// ---------------------------------------------------------------- Objets ----

/// Blocs sans propriétés que le serveur sait donner et poser : (nom, identifiant d'objet).
/// L'identifiant d'un bloc en tant qu'objet est son numéro historique (inférieur à 256).
pub const GIVABLE_BLOCKS: [(&str, i16); 7] = [
    ("stone", 1),
    ("grass_block", 2),
    ("dirt", 3),
    ("cobblestone", 4),
    ("oak_planks", 5),
    ("glass", 20),
    ("brick_block", 45),
];

/// Liste d'objets réduite aux blocs que le serveur sait donner. C'est le serveur qui
/// attribue les identifiants : le jeu s'y conforme.
pub fn item_registry_basic() -> Vec<u8> {
    let mut w = W::default();
    w.varu32(GIVABLE_BLOCKS.len() as u32);
    for (name, id) in GIVABLE_BLOCKS.iter() {
        w.string(&format!("minecraft:{}", name));
        w.i16(*id);
        w.bool(false); // pas un objet à composants
        w.vari32(0); // version : format historique
        w.0.extend_from_slice(&[0x0a, 0x00, 0x00]); // composants : aucun
    }
    w.0
}

/// Description d'une pile de blocs. Format 1.26.60 reconstitué à partir de l'objet vide
/// (8 octets nuls) et des trois refus successifs du jeu : identifiant sur 2 octets,
/// quantité sur 2 octets, variante (0 à 32767), numéro de pile absent, bloc, données.
pub fn block_item(w: &mut W, name: &str, id: i16, count: u16) {
    block_item_net(w, name, id, count, 0)
}

/// Même description, avec le numéro de pile attribué par le serveur (0 : aucun).
pub fn block_item_net(w: &mut W, name: &str, id: i16, count: u16, net_id: i32) {
    w.i16(id);
    w.0.extend_from_slice(&count.to_le_bytes());
    w.varu32(0); // variante de l'objet
    if net_id != 0 {
        w.bool(true);
        w.vari32(net_id);
    } else {
        w.bool(false); // numéro de pile : absent
    }
    w.varu32(block_hash(name)); // bloc correspondant (entier non signé, comme l'envoie le jeu)
    w.varu32(0); // données supplémentaires : aucune
}

/// Inventaire complet du joueur : 36 emplacements (identifiant d'objet, quantité).
pub fn inventory_content(slots: &[(i16, u16)], f: crate::versions::Features) -> Vec<u8> {
    let mut w = W::default();
    w.varu32(0); // inventaire du joueur
    w.varu32(slots.len() as u32);
    for (id, count) in slots {
        match GIVABLE_BLOCKS.iter().find(|(_, i)| i == id) {
            Some((name, _)) if *count > 0 => block_item(&mut w, &format!("minecraft:{}", name), *id, *count),
            _ => w.0.extend_from_slice(&[0u8; 8]),
        }
    }
    w.u8(0);
    if f.editor_migration_version {
        w.bool(false);
    } else {
        w.u32(0);
    }
    w.0.extend_from_slice(&[0u8; 8]);
    w.0
}

/// Identifiant d'objet d'un bloc que le serveur sait donner, d'après son identifiant réseau.
pub fn item_for_block(block: u32) -> Option<i16> {
    GIVABLE_BLOCKS.iter().find(|(n, _)| block_hash(&format!("minecraft:{}", n)) == block).map(|(_, id)| *id)
}

/// Objet obtenu en cassant un bloc à la main, comme dans le jeu normal : l'herbe donne de la
/// terre ; la pierre, les pavés et les briques ne donnent rien sans pioche ; le verre se brise.
pub fn drop_for_block(block: u32) -> Option<i16> {
    let name = GIVABLE_BLOCKS.iter().find(|(n, _)| block_hash(&format!("minecraft:{}", n)) == block).map(|(n, _)| *n)?;
    match name {
        "grass_block" | "dirt" => Some(3),
        "oak_planks" => Some(5),
        _ => None,
    }
}

/// Identifiant réseau du bloc posé par un objet.
pub fn block_for_item(id: i16) -> Option<u32> {
    GIVABLE_BLOCKS.iter().find(|(_, i)| *i == id).map(|(n, _)| block_hash(&format!("minecraft:{}", n)))
}

pub const ID_MOB_EQUIPMENT: u32 = 31;
pub const ID_PLAYER_ACTION: u32 = 36;

/// Saute la description d'un objet (format 1.26.60 relevé dans les paquets du jeu).
pub(crate) fn skip_item(d: &[u8], pos: &mut usize) -> Option<()> {
    *pos = pos.checked_add(4)?; // identifiant et quantité
    read_varu32(d, pos)?; // variante
    match *d.get(*pos)? {
        0 => *pos += 1,
        1 => {
            *pos += 1;
            read_vari32(d, pos)?;
        }
        _ => return None,
    }
    read_vari32(d, pos)?; // bloc
    let len = read_varu32(d, pos)? as usize;
    if len > 65_536 {
        return None;
    }
    *pos = pos.checked_add(len)?;
    if *pos > d.len() {
        return None;
    }
    Some(())
}

/// MobEquipment : renvoie l'emplacement de la barre d'objets que le joueur a en main.
pub fn read_selected_slot(body: &[u8]) -> Option<u8> {
    let mut pos = 0;
    read_varu64(body, &mut pos)?;
    skip_item(body, &mut pos)?;
    let slot = *body.get(pos.checked_add(1)?)?;
    if slot < 9 {
        Some(slot)
    } else {
        None
    }
}

/// PlayerAction « début d'utilisation d'un objet sur un bloc » : renvoie la position où
/// le bloc tenu en main doit être posé.
pub fn read_place_target(body: &[u8]) -> Option<(i32, i32, i32)> {
    let mut pos = 0;
    read_varu64(body, &mut pos)?;
    if read_vari32(body, &mut pos)? != 28 {
        return None;
    }
    for _ in 0..3 {
        read_vari32(body, &mut pos)?; // bloc cliqué
    }
    let x = read_vari32(body, &mut pos)?;
    let y = read_vari32(body, &mut pos)?;
    let z = read_vari32(body, &mut pos)?;
    Some((x, y, z))
}

#[cfg(test)]
mod item_tests {
    use super::*;

    #[test]
    fn packets_captured_from_the_game_are_understood() {
        // MobEquipment envoyé par le jeu 1.26.65 avec 64 pierres en main, emplacement 0
        let equip = [
            0x01, 0x01, 0x00, 0x40, 0x00, 0x00, 0x00, 0xa1, 0x9c, 0xc4, 0x81, 0x08, 0x0a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0x00, 0x00, 0x00,
        ];
        assert_eq!(read_selected_slot(&equip), Some(0));
        // PlayerAction : clic sur le bloc (5, -61, 4), pose en (5, -60, 4)
        let action = [0x01, 0x38, 0x0a, 0x79, 0x08, 0x0a, 0x77, 0x08, 0x02];
        assert_eq!(read_place_target(&action), Some((5, -60, 4)));
        // fin d'utilisation : ignorée
        assert_eq!(read_place_target(&[0x01, 0x3a, 0x0a, 0x77, 0x08, 0, 0, 0, 0]), None);
        assert_eq!(read_selected_slot(&[1, 2]), None);
    }

    #[test]
    fn our_item_matches_the_game_encoding() {
        // Les 12 premiers octets de notre pierre sont ceux que le jeu envoie lui-même
        let mut w = W::default();
        block_item(&mut w, "minecraft:stone", 1, 64);
        assert_eq!(&w.0[..11], &[0x01, 0x00, 0x40, 0x00, 0x00, 0x00, 0xa1, 0x9c, 0xc4, 0x81, 0x08]);
        assert_eq!(item_for_block(block_hash("minecraft:stone")), Some(1));
        assert_eq!(block_for_item(3), Some(block_hash("minecraft:dirt")));
        assert_eq!(block_for_item(999), None);
        assert_eq!(drop_for_block(block_hash("minecraft:grass_block")), Some(3));
        assert_eq!(drop_for_block(block_hash("minecraft:stone")), None);
        assert_eq!(drop_for_block(block_hash("minecraft:air")), None);
    }
}

// ------------------------------------------------------------ Inventaires ----

pub const ID_INTERACT: u32 = 33;
pub const ID_CONTAINER_OPEN: u32 = 46;
pub const ID_CONTAINER_CLOSE: u32 = 47;
pub const ID_ITEM_STACK_REQUEST: u32 = 147;
pub const ID_ITEM_STACK_RESPONSE: u32 = 148;

/// Contenu d'un conteneur : (identifiant d'objet, quantité, numéro de pile) par emplacement.
/// `window` : 0 pour l'inventaire du joueur, 119 seconde main, 120 armure, 124 curseur.
pub fn container_content(window: u32, slots: &[(i16, u8, i32)], net_ids: bool, f: crate::versions::Features) -> Vec<u8> {
    let mut w = W::default();
    w.varu32(window);
    w.varu32(slots.len() as u32);
    for (id, count, net_id) in slots {
        match GIVABLE_BLOCKS.iter().find(|(_, i)| i == id) {
            Some((name, _)) if *count > 0 => {
                let net = if net_ids { *net_id } else { 0 };
                block_item_net(&mut w, &format!("minecraft:{}", name), *id, *count as u16, net)
            }
            _ => w.0.extend_from_slice(&[0u8; 8]),
        }
    }
    w.u8(0);
    if f.editor_migration_version {
        w.bool(false);
    } else {
        w.u32(0);
    }
    w.0.extend_from_slice(&[0u8; 8]);
    w.0
}

/// Interact « ouvrir l'inventaire » (action 6).
pub fn read_open_inventory(body: &[u8]) -> bool {
    body.first() == Some(&6)
}

/// Ouvre l'écran d'inventaire du joueur (type de conteneur 0xff).
pub fn container_open_inventory(window: u8, unique_id: i64) -> Vec<u8> {
    let mut w = W::default();
    w.u8(window);
    w.u8(0xff);
    w.vari32(0);
    w.vari32(0);
    w.vari32(0);
    w.vari64(unique_id);
    w.0
}

pub const ID_CONTAINER_SET_DATA: u32 = 51;

/// Ouvre l'écran d'un bloc (coffre, four, établi…) situé en `pos`.
pub fn container_open_block(window: u8, kind: u8, pos: (i32, i32, i32)) -> Vec<u8> {
    let mut w = W::default();
    w.u8(window);
    w.u8(kind);
    w.vari32(pos.0);
    w.vari32(pos.1);
    w.vari32(pos.2);
    w.vari64(-1);
    w.0
}

/// Jauge d'un écran (flamme et flèche du four) : (fenêtre, propriété, valeur).
pub fn container_set_data(window: u8, property: i32, value: i32) -> Vec<u8> {
    let mut w = W::default();
    w.u8(window);
    w.vari32(property);
    w.vari32(value);
    w.0
}

pub fn container_close(window: u8, kind: u8, server_initiated: bool) -> Vec<u8> {
    vec![window, kind, server_initiated as u8]
}

/// Lit ContainerClose envoyé par le jeu : (fenêtre, type).
pub fn read_container_close(body: &[u8]) -> Option<(u8, u8)> {
    Some((*body.first()?, *body.get(1)?))
}

/// Emplacement cité dans une demande : (nom de conteneur, numéro, numéro de pile annoncé).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotRef {
    pub container: u8,
    pub slot: u8,
    pub net_id: i32,
}

/// Action d'une demande de déplacement d'objets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StackAction {
    Take { count: u8, from: SlotRef, to: SlotRef },
    Place { count: u8, from: SlotRef, to: SlotRef },
    Swap { a: SlotRef, b: SlotRef },
    Drop { count: u8, from: SlotRef },
    Destroy { count: u8, from: SlotRef },
    /// Prise d'un objet dans le menu créatif (numéro de l'objet dans ce menu)
    CraftCreative { creative_id: u32 },
    /// Fabrication d'une recette (numéro de la recette dans la liste envoyée au jeu)
    CraftRecipe { recipe: u32, times: u8 },
    /// Résultat annoncé par le jeu pour la création précédente (voir `StackRequest::created`)
    Results,
    /// Action que le serveur ne gère pas encore (fabrication, etc.)
    Unsupported(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StackRequest {
    pub id: i32,
    pub actions: Vec<StackAction>,
    /// Nom de l'objet que le jeu annonce avoir créé
    pub created: Option<String>,
    /// Quantité annoncée pour cet objet
    pub created_count: u16,
}

fn read_slot_ref(d: &[u8], pos: &mut usize) -> Option<SlotRef> {
    let container = *d.get(*pos)?;
    *pos += 1;
    match *d.get(*pos)? {
        0 => *pos += 1,
        1 => *pos = pos.checked_add(5)?, // identifiant dynamique présent
        _ => return None,
    }
    let slot = *d.get(*pos)?;
    *pos += 1;
    // Numéro de pile : entier de 4 octets (relevé dans les demandes du jeu 1.26.65)
    let b = d.get(*pos..pos.checked_add(4)?)?;
    let net_id = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    *pos += 4;
    Some(SlotRef { container, slot, net_id })
}

/// Lit ItemStackRequest. Limites strictes : au plus 16 demandes de 64 actions. Dès qu'une
/// action inconnue est rencontrée, la lecture de la demande s'arrête (sa taille est inconnue).
pub fn read_stack_requests(body: &[u8]) -> Option<Vec<StackRequest>> {
    let mut pos = 0;
    let count = read_varu32(body, &mut pos)? as usize;
    // « Tout ramasser » (double-clic) envoie une demande par pile : jusqu'à 27 pour un coffre,
    // 54 pour un double coffre, plus l'inventaire
    if count > 128 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let id = read_vari32(body, &mut pos)?;
        let n = read_varu32(body, &mut pos)? as usize;
        if n > 64 {
            return None;
        }
        let mut actions = Vec::with_capacity(n);
        let mut complete = true;
        let mut created = None;
        let mut created_count = 0u16;
        for _ in 0..n {
            // Chaque action commence par son rang dans la liste des variantes, puis son type
            read_varu32(body, &mut pos)?;
            let kind = *body.get(pos)?;
            pos += 1;
            let action = match kind {
                0 | 1 => {
                    let count = *body.get(pos)?;
                    pos += 1;
                    let from = read_slot_ref(body, &mut pos)?;
                    let to = read_slot_ref(body, &mut pos)?;
                    if kind == 0 {
                        StackAction::Take { count, from, to }
                    } else {
                        StackAction::Place { count, from, to }
                    }
                }
                2 => {
                    let a = read_slot_ref(body, &mut pos)?;
                    let b = read_slot_ref(body, &mut pos)?;
                    StackAction::Swap { a, b }
                }
                3 => {
                    let count = *body.get(pos)?;
                    pos += 1;
                    let from = read_slot_ref(body, &mut pos)?;
                    pos += 1; // « au hasard »
                    StackAction::Drop { count, from }
                }
                4 | 5 => {
                    let count = *body.get(pos)?;
                    pos += 1;
                    let from = read_slot_ref(body, &mut pos)?;
                    StackAction::Destroy { count, from }
                }
                12 => {
                    let recipe = read_varu32(body, &mut pos)?;
                    let times = *body.get(pos)?; // nombre de fabrications d'affilée
                    pos += 1;
                    StackAction::CraftRecipe { recipe, times }
                }
                14 => {
                    let creative_id = read_varu32(body, &mut pos)?;
                    pos += 1; // nombre de créations
                    StackAction::CraftCreative { creative_id }
                }
                20 => {
                    // Résultats annoncés : un objet désigné par son nom. Relevé du jeu 1.26.65 :
                    // [nombre][2 octets][nom][entier][quantité sur 2 octets][bloc][données][créations]
                    if read_varu32(body, &mut pos)? != 1 {
                        return None;
                    }
                    let mut name = None;
                    for skip in 0..5 {
                        let mut p = pos + skip;
                        let len = match read_varu32(body, &mut p) {
                            Some(l) if (3..=120).contains(&l) => l as usize,
                            _ => continue,
                        };
                        match body.get(p..p + len) {
                            Some(text) if text.contains(&b':') && text.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b':' | b'_' | b'.')) => {
                                name = Some(String::from_utf8_lossy(text).to_string());
                                pos = p + len;
                                break;
                            }
                            _ => {}
                        }
                    }
                    created = Some(name?);
                    read_varu32(body, &mut pos)?;
                    let q = body.get(pos..pos.checked_add(2)?)?;
                    created_count = u16::from_le_bytes([q[0], q[1]]);
                    pos += 2;
                    read_varu32(body, &mut pos)?;
                    let extra = read_varu32(body, &mut pos)? as usize;
                    pos = pos.checked_add(extra)?.checked_add(1)?;
                    if pos > body.len() {
                        return None;
                    }
                    StackAction::Results
                }
                other => {
                    complete = false;
                    StackAction::Unsupported(other)
                }
            };
            actions.push(action);
            if !complete {
                break;
            }
        }
        out.push(StackRequest { id, actions, created, created_count });
        if !complete {
            return Some(out); // la suite du paquet n'est plus lisible
        }
        // textes à filtrer, puis leur origine
        let strings = read_varu32(body, &mut pos)? as usize;
        if strings > 16 {
            return None;
        }
        for _ in 0..strings {
            let len = read_varu32(body, &mut pos)? as usize;
            pos = pos.checked_add(len)?;
        }
        pos = pos.checked_add(4)?;
        if pos > body.len() {
            return None;
        }
    }
    Some(out)
}

/// Réponse à une demande : `changed` liste les emplacements modifiés sous la forme
/// (nom de conteneur, numéro, quantité, numéro de pile). None : demande refusée.
pub fn item_stack_response(request: i32, changed: Option<&[(u8, u8, u8, i32)]>) -> Vec<u8> {
    let mut w = W::default();
    w.varu32(1); // une réponse
    let slots = match changed {
        Some(s) => s,
        None => {
            w.u8(1); // erreur
            w.vari32(request);
            w.bool(false);
            return w.0;
        }
    };
    w.u8(0); // succès
    w.vari32(request);
    w.bool(true);
    // un groupe par conteneur
    let mut names: Vec<u8> = slots.iter().map(|s| s.0).collect();
    names.sort_unstable();
    names.dedup();
    w.varu32(names.len() as u32);
    for name in names {
        w.u8(name);
        w.bool(false); // identifiant dynamique : absent
        let group: Vec<&(u8, u8, u8, i32)> = slots.iter().filter(|s| s.0 == name).collect();
        w.varu32(group.len() as u32);
        for (_, slot, count, net_id) in group {
            w.u8(*slot); // emplacement demandé
            w.u8(*slot);
            w.u8(*count);
            if *count > 0 {
                w.bool(true);
                w.vari32(*net_id);
            } else {
                w.bool(false);
            }
            w.string(""); // nom personnalisé
            w.bool(false); // version filtrée : absente
            w.vari32(0); // correction de durabilité
        }
    }
    w.0
}

#[cfg(test)]
mod inventory_tests {
    use super::*;

    fn slot(w: &mut W, container: u8, slot: u8, net_id: i32) {
        w.u8(container);
        w.bool(false);
        w.u8(slot);
        w.0.extend_from_slice(&net_id.to_le_bytes());
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn requests_captured_from_the_game_are_understood() {
        // Prendre 4 objets de la barre (emplacement 0, pile n° 6) vers le curseur
        let take = read_stack_requests(&hex("0155010000041c0000060000003b00000000000000ffffffff")).unwrap();
        assert_eq!(take[0].id, -43);
        assert_eq!(
            take[0].actions,
            vec![StackAction::Take {
                count: 4,
                from: SlotRef { container: 28, slot: 0, net_id: 6 },
                to: SlotRef { container: 59, slot: 0, net_id: 0 }
            }]
        );
        // Les déposer dans l'emplacement 3 de la barre
        let place = read_stack_requests(&hex("0159010101043b0000d5ffffff1c00030000000000ffffffff")).unwrap();
        assert_eq!(place[0].id, -45);
        assert_eq!(
            place[0].actions,
            vec![StackAction::Place {
                count: 4,
                from: SlotRef { container: 59, slot: 0, net_id: -43 },
                to: SlotRef { container: 28, slot: 3, net_id: 0 }
            }]
        );
    }

    #[test]
    fn stack_requests_are_read_with_limits() {
        let mut w = W::default();
        w.varu32(1);
        w.vari32(-5);
        w.varu32(2);
        w.varu32(0);
        w.u8(0); // prendre 32 de la barre vers le curseur
        w.u8(32);
        slot(&mut w, 28, 0, 7);
        slot(&mut w, 59, 0, 0);
        w.varu32(3);
        w.u8(3); // jeter 1
        w.u8(1);
        slot(&mut w, 59, 0, 8);
        w.bool(false);
        w.varu32(0);
        w.0.extend_from_slice(&0i32.to_le_bytes());
        let reqs = read_stack_requests(&w.0).unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].id, -5);
        assert_eq!(
            reqs[0].actions[0],
            StackAction::Take {
                count: 32,
                from: SlotRef { container: 28, slot: 0, net_id: 7 },
                to: SlotRef { container: 59, slot: 0, net_id: 0 }
            }
        );
        assert!(matches!(reqs[0].actions[1], StackAction::Drop { count: 1, .. }));
        // action inconnue : signalée, sans lire plus loin
        let unknown = [1u8, 2, 1, 8, 17, 9, 9, 9];
        let reqs = read_stack_requests(&unknown).unwrap();
        assert_eq!(reqs[0].actions, vec![StackAction::Unsupported(17)]);
        // trop de demandes, paquet tronqué
        assert_eq!(read_stack_requests(&[200, 1]), None);
        assert_eq!(read_stack_requests(&[1, 2, 1, 0, 0, 5]), None);
        assert_eq!(read_stack_requests(&[]), None);
    }

    #[test]
    fn responses_group_slots_by_container() {
        let ok = item_stack_response(3, Some(&[(28, 0, 32, 9), (59, 0, 32, 10), (28, 4, 0, 0)]));
        // 1 réponse, succès, demande 3 (zigzag 6), conteneurs présents, 2 groupes
        assert_eq!(&ok[..5], &[1, 0, 6, 1, 2]);
        let refused = item_stack_response(3, None);
        assert_eq!(refused, vec![1, 1, 6, 0]);
    }
}

// ------------------------------------------------ Utilisation d'un objet ----

pub const ID_INVENTORY_TRANSACTION: u32 = 30;

fn use_item_at(body: &[u8], mut pos: usize, slots_flag: bool) -> Option<BlockUse> {
    let legacy = read_vari32(body, &mut pos)?;
    let has_slots = if slots_flag {
        let b = *body.get(pos)?;
        pos += 1;
        if b > 1 {
            return None;
        }
        b == 1
    } else {
        legacy != 0
    };
    if has_slots {
        let n = read_varu32(body, &mut pos)? as usize;
        if n > 64 {
            return None;
        }
        for _ in 0..n {
            pos += 1;
            let m = read_varu32(body, &mut pos)? as usize;
            pos = pos.checked_add(m)?;
        }
    }
    if read_varu32(body, &mut pos)? != 2 {
        return None; // autre chose qu'une utilisation d'objet
    }
    let actions = read_varu32(body, &mut pos)? as usize;
    if actions > 16 {
        return None;
    }
    for _ in 0..actions {
        match read_varu32(body, &mut pos)? {
            0 | 99999 => pos += 1,
            2 => {
                read_varu32(body, &mut pos)?;
            }
            1 | 3 => {}
            _ => return None,
        }
        read_varu32(body, &mut pos)?;
        skip_item(body, &mut pos)?;
        skip_item(body, &mut pos)?;
    }
    let action = read_vari32(body, &mut pos)?;
    let trigger = *body.get(pos)?;
    pos += 1;
    let x = read_vari32(body, &mut pos)?;
    let y = read_vari32(body, &mut pos)?;
    let z = read_vari32(body, &mut pos)?;
    let face = *body.get(pos)?;
    if !(0..=3).contains(&action) || trigger > 2 || face > 5 || !(-64..=319).contains(&y) {
        return None;
    }
    // La transaction se termine par : point cliqué (3 nombres), bloc visé (entier variable),
    // puis deux octets. On lit donc le point cliqué en partant de la fin.
    let click = (|| {
        let last = body.len().checked_sub(3)?;
        if last < pos || body[last] & 0x80 != 0 {
            return None;
        }
        let mut start = last;
        while start > pos && last - start < 4 && body[start - 1] & 0x80 != 0 {
            start -= 1;
        }
        let at = start.checked_sub(12).filter(|a| *a > pos)?;
        let f = |p: usize| body.get(p..p + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        clean_click((f(at)?, f(at + 4)?, f(at + 8)?))
    })()
    .unwrap_or((0.5, 0.5, 0.5));
    Some(BlockUse { action, pos: (x, y, z), face, click })
}

/// Point cliqué plausible (entre 0 et 1), sinon rien.
fn clean_click(c: (f32, f32, f32)) -> Option<(f32, f32, f32)> {
    let ok = |v: f32| v.is_finite() && (-0.01..=1.01).contains(&v);
    if ok(c.0) && ok(c.1) && ok(c.2) {
        Some((c.0.clamp(0.0, 1.0), c.1.clamp(0.0, 1.0), c.2.clamp(0.0, 1.0)))
    } else {
        None
    }
}

/// Lit InventoryTransaction « utilisation d'un objet » : bloc visé, face et type de geste.
/// La documentation ne dit pas si un octet de présence précède la liste d'emplacements :
/// les deux lectures sont essayées, et le résultat est contrôlé (geste, face, hauteur).
pub fn read_use_item(body: &[u8]) -> Option<BlockUse> {
    use_item_at(body, 0, false).or_else(|| use_item_at(body, 0, true))
}

/// Lecture de secours : quand le début de la transaction contient des objets dont la
/// disposition n'est pas connue, on cherche la fin du paquet, dont le format est fixe
/// (geste, bloc, face, emplacement, main, objet tenu, deux positions, bloc visé, deux
/// octets). Une position de départ n'est retenue que si la lecture tombe exactement
/// sur la fin du paquet et si la position du joueur annoncée est proche de `player`.
pub fn read_use_item_scan(body: &[u8], player: (f32, f32, f32)) -> Option<BlockUse> {
    scan_use_item(body, player, 0, true)
}

/// Même recherche dans un paquet de mouvement (PlayerAuthInput), où le geste est suivi
/// d'autres données : la lecture n'a pas à tomber sur la fin du paquet. La position du
/// joueur écrite dans le geste reste exigée, ce qui écarte les fausses correspondances.
pub fn read_use_item_in_input(body: &[u8], player: (f32, f32, f32)) -> Option<BlockUse> {
    scan_use_item(body, player, 32, false)
}

fn scan_use_item(body: &[u8], player: (f32, f32, f32), from: usize, must_end: bool) -> Option<BlockUse> {
    let f32_at = |at: usize| -> Option<f32> {
        body.get(at..at + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    for start in from..body.len().saturating_sub(30) {
        let mut pos = start;
        let attempt = (|| -> Option<BlockUse> {
            let action = read_vari32(body, &mut pos)?;
            let trigger = *body.get(pos)?;
            pos += 1;
            let x = read_vari32(body, &mut pos)?;
            let y = read_vari32(body, &mut pos)?;
            let z = read_vari32(body, &mut pos)?;
            let face = *body.get(pos)?;
            pos += 1;
            if !(0..=3).contains(&action) || trigger > 2 || face > 5 || !(-64..=319).contains(&y) {
                return None;
            }
            let slot = read_vari32(body, &mut pos)?;
            let hand = *body.get(pos)?;
            pos += 1;
            if !(0..9).contains(&slot) || hand > 1 {
                return None;
            }
            skip_item(body, &mut pos)?;
            let (px, py, pz) = (f32_at(pos)?, f32_at(pos + 4)?, f32_at(pos + 8)?);
            let click = clean_click((f32_at(pos + 12)?, f32_at(pos + 16)?, f32_at(pos + 20)?)).unwrap_or((0.5, 0.5, 0.5));
            pos += 24; // position du joueur, puis point cliqué
            read_varu32(body, &mut pos)?; // bloc visé
            let (prediction, cooldown) = (*body.get(pos)?, *body.get(pos + 1)?);
            pos += 2;
            if (must_end && pos != body.len()) || pos > body.len() || prediction > 1 || cooldown > 1 {
                return None;
            }
            let near = (px - player.0).abs() < 3.0 && (py - player.1).abs() < 3.0 && (pz - player.2).abs() < 3.0;
            let reach = (x as f32 - px).abs() < 9.0 && (y as f32 - py).abs() < 9.0 && (z as f32 - pz).abs() < 9.0;
            if !near || !reach {
                return None;
            }
            Some(BlockUse { action, pos: (x, y, z), face, click })
        })();
        if attempt.is_some() {
            return attempt;
        }
    }
    None
}

/// Position où se pose un bloc quand on clique sur la face `face` du bloc `pos`.
pub fn place_target(pos: (i32, i32, i32), face: u8) -> Option<(i32, i32, i32)> {
    let (x, y, z) = pos;
    Some(match face {
        0 => (x, y - 1, z),
        1 => (x, y + 1, z),
        2 => (x, y, z - 1),
        3 => (x, y, z + 1),
        4 => (x - 1, y, z),
        5 => (x + 1, y, z),
        _ => return None,
    })
}

/// Bloc cliqué, connaissant l'endroit où le nouveau bloc se pose et la face cliquée.
pub fn unplace_target(target: (i32, i32, i32), face: u8) -> (i32, i32, i32) {
    let (x, y, z) = target;
    match face {
        0 => (x, y + 1, z),
        1 => (x, y - 1, z),
        2 => (x, y, z + 1),
        3 => (x, y, z - 1),
        4 => (x + 1, y, z),
        5 => (x - 1, y, z),
        _ => target,
    }
}

#[cfg(test)]
mod use_item_tests {
    use super::*;

    #[test]
    fn use_item_is_read_with_and_without_slot_flag() {
        for flag in [false, true] {
            let mut w = W::default();
            w.vari32(0);
            if flag {
                w.bool(false);
            }
            w.varu32(2);
            w.varu32(1); // une action d'inventaire, avec deux objets
            w.varu32(0);
            w.u8(0);
            w.varu32(0);
            block_item_net(&mut w, "minecraft:stone", 1, 3, 0);
            block_item_net(&mut w, "minecraft:stone", 1, 2, 0);
            w.vari32(0); // poser
            w.u8(1);
            w.vari32(5);
            w.vari32(-61);
            w.vari32(-4);
            w.u8(1);
            w.vari32(0);
            assert_eq!(read_use_item(&w.0), Some(BlockUse { action: 0, pos: (5, -61, -4), face: 1, click: (0.5, 0.5, 0.5) }), "octet : {}", flag);
        }
        // Paquet réel du jeu 1.26.65 (main vide, clic sur le bloc (-3, -60, 12), face nord)
        let real: Vec<u8> = (0..102)
            .step_by(2)
            .map(|i| u8::from_str_radix(&"0000020000020577180200000000000000000000e9443ec01c8569c2fe891b41b036643ec0a0003f00000000b4d1c4f10d0000"[i..i + 2], 16).unwrap())
            .collect();
        // le point cliqué est sur la face nord (z = 0), à mi-hauteur
        let expected = Some(BlockUse { action: 0, pos: (-3, -60, 12), face: 2, click: (f32::from_le_bytes([0xb0, 0x36, 0x64, 0x3e]), f32::from_le_bytes([0xc0, 0xa0, 0x00, 0x3f]), 0.0) });
        assert_eq!(read_use_item(&real), expected);
        assert_eq!(read_use_item_scan(&real, (-2.97, -58.38, 9.72)), expected);
        // un joueur ailleurs dans le monde : la lecture de secours refuse
        assert_eq!(read_use_item_scan(&real, (100.0, -58.0, 100.0)), None);
        // même fin de paquet précédée d'un début inconnu : retrouvée quand même
        let mut odd = vec![0x09, 0xff, 0x33, 0x7a, 0x01, 0x02];
        odd.extend_from_slice(&real[4..]);
        assert_eq!(read_use_item(&odd), None);
        assert_eq!(read_use_item_scan(&odd, (-2.97, -58.38, 9.72)), expected);
        assert_eq!(place_target((5, -61, -4), 1), Some((5, -60, -4)));
        assert_eq!(place_target((0, 0, 0), 9), None);
        assert_eq!(read_use_item(&[0, 0]), None);
        assert_eq!(read_use_item(&[]), None);
    }
}

// ------------------------------------------ Objets issus des données du jeu ----

/// Liste complète des objets, d'après les données du jeu :
/// (nom, identifiant, version, composants au format réseau).
pub fn item_registry_full(items: &[(&str, i16, i32, Option<&[u8]>)]) -> Vec<u8> {
    let mut w = W::default();
    w.varu32(items.len() as u32);
    for (name, id, version, components) in items {
        w.string(name);
        w.i16(*id);
        w.bool(components.is_some());
        w.vari32(*version);
        match components {
            Some(nbt) => w.0.extend_from_slice(nbt),
            None => w.0.extend_from_slice(&[0x0a, 0x00, 0x00]),
        }
    }
    w.0
}

/// Description d'une pile quelconque. `block` : identifiant réseau du bloc que l'objet
/// pose (0 si ce n'est pas un bloc).
pub fn item_stack(w: &mut W, id: i16, count: u8, net_id: i32, block: u32) {
    if count == 0 {
        w.0.extend_from_slice(&[0u8; 8]);
        return;
    }
    w.i16(id);
    w.0.extend_from_slice(&(count as u16).to_le_bytes());
    w.varu32(0); // variante de l'objet
    if net_id != 0 {
        w.bool(true);
        w.vari32(net_id);
    } else {
        w.bool(false);
    }
    w.varu32(block);
    w.varu32(0); // données supplémentaires : aucune
}

/// Contenu d'un conteneur : (identifiant d'objet, quantité, numéro de pile, bloc posé).
pub fn container_items(window: u32, slots: &[(i16, u8, i32, u32)], net_ids: bool, f: crate::versions::Features) -> Vec<u8> {
    let mut w = W::default();
    w.varu32(window);
    w.varu32(slots.len() as u32);
    for (id, count, net_id, block) in slots {
        item_stack(&mut w, *id, *count, if net_ids { *net_id } else { 0 }, *block);
    }
    w.u8(0);
    if f.editor_migration_version {
        w.bool(false);
    } else {
        w.u32(0);
    }
    w.0.extend_from_slice(&[0u8; 8]);
    w.0
}

#[cfg(test)]
mod data_item_tests {
    use super::*;

    #[test]
    fn generic_stack_matches_the_block_specific_one() {
        let (mut a, mut b) = (W::default(), W::default());
        block_item_net(&mut a, "minecraft:stone", 1, 64, 7);
        item_stack(&mut b, 1, 64, 7, block_hash("minecraft:stone"));
        assert_eq!(a.0, b.0);
        let mut empty = W::default();
        item_stack(&mut empty, 1, 0, 3, 5);
        assert_eq!(empty.0, vec![0u8; 8]);
        // registre : nom, identifiant, indicateur, version, composants
        let reg = item_registry_full(&[("minecraft:stick", 323, 2, None)]);
        assert_eq!(reg[0], 1);
        assert_eq!(&reg[reg.len() - 7..], &[0x43, 0x01, 0, 4, 0x0a, 0, 0]);
    }
}

#[cfg(test)]
mod creative_tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn creative_requests_captured_from_the_game_are_understood() {
        // Clic avec Maj sur le sable : 64 exemplaires déposés dans l'inventaire
        let sand = unhex("0125030c0ef2040112140101010e6d696e6563726166743a73616e64000100e3f28d420a00000000000000000000010101403c0032edffffff0c00000000000000ffffffff");
        let r = read_stack_requests(&sand).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, -19);
        assert_eq!(r[0].created.as_deref(), Some("minecraft:sand"));
        assert_eq!(r[0].actions[0], StackAction::CraftCreative { creative_id: 626 });
        assert_eq!(r[0].actions[1], StackAction::Results);
        assert_eq!(
            r[0].actions[2],
            StackAction::Place { count: 64, from: SlotRef { container: 60, slot: 50, net_id: -19 }, to: SlotRef { container: 12, slot: 0, net_id: 0 } }
        );
        // Clic simple sur la bedrock : prise au curseur
        let bedrock = unhex("011d030c0ed307011214010101116d696e6563726166743a626564726f636b000100fbf9b1ad0f0a00000000000000000000010000403c0032f1ffffff3b00000000000000ffffffff");
        let r = read_stack_requests(&bedrock).unwrap();
        assert_eq!(r[0].created.as_deref(), Some("minecraft:bedrock"));
        assert!(matches!(r[0].actions[2], StackAction::Take { count: 64, to: SlotRef { container: 59, .. }, .. }));
        // Objet dont l'entier après le nom vaut 2
        let cluster = unhex("0109030c0ec7060112140101011a6d696e6563726166743a616d6574687973745f636c757374657202010082f98cb60c0a00000000000000000000010101403c0032fbffffff0c00000000000000ffffffff");
        assert_eq!(read_stack_requests(&cluster).unwrap()[0].created.as_deref(), Some("minecraft:amethyst_cluster"));
    }
}

#[cfg(test)]
mod grouped_request_tests {
    use super::*;

    /// « Tout ramasser » dans un coffre : 27 demandes dans un seul paquet (relevé du jeu 1.26.65).
    #[test]
    fn many_requests_in_one_packet_are_read() {
        let one: [u8; 25] = [0xe5, 0x02, 1, 0, 0, 2, 7, 0, 0x19, 0x3a, 0xca, 0x9a, 0x3b, 0x3b, 0, 0, 0x3b, 0xca, 0x9a, 0x3b, 0, 0xff, 0xff, 0xff, 0xff];
        let mut body = vec![27u8];
        for _ in 0..27 {
            body.extend_from_slice(&one);
        }
        let requests = read_stack_requests(&body).expect("lisible");
        assert_eq!(requests.len(), 27);
        assert_eq!(requests[26].actions.len(), 1);
        // Mais pas un nombre absurde
        assert!(read_stack_requests(&[200, 1]).is_none());
    }
}

/// Objet jeté avec la touche « lâcher » (Q) : le jeu n'envoie pas de demande d'inventaire
/// mais une InventoryTransaction « normale » avec deux actions : l'objet qui apparaît dans le
/// monde (source 2, drapeau 0) et la case de l'inventaire qui se vide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldDrop {
    pub id: i16,
    pub count: u16,
    /// Case du sac (fenêtre 0) d'où vient l'objet, si le jeu l'a indiquée.
    pub slot: Option<usize>,
}

pub fn read_world_drop(body: &[u8]) -> Option<WorldDrop> {
    // L'objet vide n'a pas été relevé dans une capture : les deux écritures possibles sont
    // essayées, et seule une lecture qui tombe exactement sur la fin du paquet est gardée.
    // Relevé avec le jeu 1.26.63 : deux octets inconnus suivent le numéro de case de chaque
    // action ; les versions précédentes n'en avaient pas. Les deux dispositions sont essayées.
    for pad in [2usize, 0, 1] {
        for air_short in [false, true] {
            for slots_flag in [true, false] {
                if let Some(d) = world_drop_at(body, slots_flag, air_short, pad) {
                    return Some(d);
                }
            }
        }
    }
    None
}

fn read_net_item(d: &[u8], pos: &mut usize, air_short: bool) -> Option<(i16, u16)> {
    let id = i16::from_le_bytes([*d.get(*pos)?, *d.get(*pos + 1)?]);
    if id == 0 && air_short {
        *pos += 2;
        return Some((0, 0));
    }
    let count = u16::from_le_bytes([*d.get(*pos + 2)?, *d.get(*pos + 3)?]);
    skip_item(d, pos)?;
    Some((id, if id == 0 { 0 } else { count }))
}

fn world_drop_at(body: &[u8], slots_flag: bool, air_short: bool, pad: usize) -> Option<WorldDrop> {
    let mut pos = 0;
    let legacy = read_vari32(body, &mut pos)?;
    let has_slots = if slots_flag {
        let b = *body.get(pos)?;
        pos += 1;
        if b > 1 {
            return None;
        }
        b == 1
    } else {
        legacy != 0
    };
    if has_slots {
        let n = read_varu32(body, &mut pos)? as usize;
        if n > 64 {
            return None;
        }
        for _ in 0..n {
            pos += 1;
            let m = read_varu32(body, &mut pos)? as usize;
            pos = pos.checked_add(m)?;
        }
    }
    if read_varu32(body, &mut pos)? != 0 {
        return None; // pas une transaction « normale »
    }
    let actions = read_varu32(body, &mut pos)? as usize;
    if actions == 0 || actions > 16 {
        return None;
    }
    let mut thrown: Option<(i16, u16)> = None;
    let mut slot = None;
    for _ in 0..actions {
        let source = read_varu32(body, &mut pos)?;
        let mut window = None;
        let mut flags = None;
        match source {
            0 | 99999 => window = Some(read_vari32(body, &mut pos)?),
            2 => flags = Some(read_varu32(body, &mut pos)?),
            1 | 3 => {}
            _ => return None,
        }
        let s = read_varu32(body, &mut pos)?;
        pos = pos.checked_add(pad)?;
        let _old = read_net_item(body, &mut pos, air_short)?;
        let new = read_net_item(body, &mut pos, air_short)?;
        if source == 2 && flags == Some(0) && new.0 != 0 && new.1 > 0 {
            thrown = Some(new);
        }
        // Fenêtre du sac : 0 dans les anciennes versions, 1 relevé avec le jeu 1.26.63
        if source == 0 && matches!(window, Some(0) | Some(-1) | Some(1)) && (s as usize) < 36 {
            slot = Some(s as usize);
        }
    }
    if pos != body.len() {
        return None;
    }
    let (id, count) = thrown?;
    Some(WorldDrop { id, count, slot })
}

#[cfg(test)]
mod key_drop_tests {
    use super::*;

    fn item(id: i16, count: u16) -> Vec<u8> {
        let mut v = id.to_le_bytes().to_vec();
        v.extend_from_slice(&count.to_le_bytes());
        v.extend_from_slice(&[0, 0, 0, 0]); // variante, pas d'identifiant de pile, bloc, données
        v
    }

    fn packet(air: &[u8]) -> Vec<u8> {
        let mut p = vec![0, 0, 2]; // pas de demande ancienne, transaction normale, deux actions
        p.extend_from_slice(&[2, 0, 0]); // monde, lâcher, case 0
        p.extend_from_slice(air);
        p.extend(item(5, 1));
        p.extend_from_slice(&[0, 0, 3]); // fenêtre 0 (sac), case 3
        p.extend(item(5, 1));
        p.extend_from_slice(air);
        p
    }

    #[test]
    fn key_drop_sent_by_the_real_game_is_read() {
        // Q avec 60 blocs en main (case 0), relevé dans le journal du serveur avec le jeu 1.26.63
        let h = "a71201011d010000020001000000b8fb3c0000008ded8cdc050a00000000000000000000b8fb3b0000008ded8cdc050a0000000000000000000002000100000000000000000000b8fb010000008ded8cdc050a00000000000000000000";
        let body: Vec<u8> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect();
        let id = i16::from_le_bytes([0xb8, 0xfb]);
        assert_eq!(read_world_drop(&body), Some(WorldDrop { id, count: 1, slot: Some(0) }));
        // Ce n'est ni un coup porté ni un bloc posé
        assert!(crate::bedrock::read_attack(&body).is_none());
    }

    #[test]
    fn key_drop_is_read_whatever_the_empty_item_looks_like() {
        let want = WorldDrop { id: 5, count: 1, slot: Some(3) };
        assert_eq!(read_world_drop(&packet(&item(0, 0))), Some(want));
        assert_eq!(read_world_drop(&packet(&[0, 0])), Some(want));
        // Un coup porté (type 3) n'est pas un lâcher
        let mut attack = packet(&[0, 0]);
        attack[1] = 3;
        assert_eq!(read_world_drop(&attack), None);
    }
}
