//! Vie et mort : dégâts, attaque d'un joueur, mort et réapparition.
//!
//! Dispositions d'après la documentation du protocole et les serveurs existants ; elles n'ont
//! pas été relevées auprès du serveur officiel 1.26. Si le jeu signale un de ces paquets
//! illisible, le serveur cesse de l'envoyer (voir la gestion du paquet 156).

#[allow(unused_imports)]
use super::*;

pub const ID_ACTOR_EVENT: u32 = 27;
pub const ID_SET_ACTOR_MOTION: u32 = 40;
pub const ID_RESPAWN: u32 = 45;
pub const ID_DEATH_INFO: u32 = 189;

/// Événements d'une entité (ActorEvent)
pub const EVENT_HURT: u8 = 2;
pub const EVENT_DEATH: u8 = 3;
pub const EVENT_RESPAWN: u8 = 18;
/// Objet au sol dont la pile change de taille (fusion) ; donnée : nouvelle quantité.
pub const EVENT_ITEM_STACK_SIZE: u8 = 69;

/// États de la réapparition (Respawn)
pub const RESPAWN_SEARCHING: u8 = 0;
pub const RESPAWN_READY: u8 = 1;
pub const RESPAWN_CLIENT_READY: u8 = 2;

/// Action « réapparaître » de PlayerAction.
pub const ACTION_RESPAWN: i32 = 7;

/// ActorEvent : l'entité rougit (dégât), tombe (mort), se relève…
pub fn actor_event(runtime_id: i64, event: u8, data: i32) -> Vec<u8> {
    let mut w = W::default();
    w.varu64(runtime_id as u64);
    w.u8(event);
    w.vari32(data);
    // Octet supplémentaire attendu par le jeu 1.26 : sans lui, il signale « read() overflow »
    // juste après les données (relevé dans le journal du serveur avec le vrai jeu). Sa
    // signification n'est pas connue ; 0 convient à un indicateur comme à un entier variable.
    w.u8(0);
    w.0
}

/// Respawn : position de réapparition (à hauteur des yeux), état, identifiant du joueur.
pub fn respawn(pos: (f32, f32, f32), state: u8, runtime_id: i64) -> Vec<u8> {
    let mut w = W::default();
    w.f32(pos.0);
    w.f32(pos.1);
    w.f32(pos.2);
    w.u8(state);
    w.varu64(runtime_id as u64);
    w.0
}

/// État annoncé par le jeu dans son paquet Respawn.
pub fn read_respawn_state(body: &[u8]) -> Option<u8> {
    body.get(12).copied()
}

/// SetActorMotion : élan donné à une entité (recul après un coup).
pub fn set_actor_motion(runtime_id: i64, motion: (f32, f32, f32)) -> Vec<u8> {
    let mut w = W::default();
    w.varu64(runtime_id as u64);
    w.f32(motion.0);
    w.f32(motion.1);
    w.f32(motion.2);
    w.varu64(0); // tick
    w.0
}

/// DeathInfo : cause affichée sur l'écran de mort.
pub fn death_info(message: &str) -> Vec<u8> {
    let mut w = W::default();
    w.string(message);
    w.varu32(0); // aucun paramètre : le texte est déjà rédigé
    w.0
}

/// PlayerAction : (identifiant du joueur, action).
pub fn read_player_action(body: &[u8]) -> Option<(u64, i32)> {
    let mut pos = 0;
    let id = read_varu64(body, &mut pos)?;
    let action = read_vari32(body, &mut pos)?;
    Some((id, action))
}

/// Coup porté : InventoryTransaction de type « objet utilisé sur une entité ».
/// Renvoie (identifiant de la cible, action). Le jeu 1.26 envoie 2 pour un coup (relevé dans
/// le journal du serveur avec le vrai jeu) ; les versions précédentes envoyaient 1.
pub fn read_attack(body: &[u8]) -> Option<(u64, u32)> {
    read_attack_at(body, false).or_else(|| read_attack_at(body, true))
}

/// Même lecture, avec ou sans le champ ajouté dans les versions récentes.
fn read_attack_at(body: &[u8], slots_flag: bool) -> Option<(u64, u32)> {
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
    if read_varu32(body, &mut pos)? != 3 {
        return None; // autre chose qu'un coup porté à une entité
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
    let target = read_varu64(body, &mut pos)?;
    let action = read_varu32(body, &mut pos)?;
    if action > 2 {
        return None;
    }
    Some((target, action))
}

/// L'action d'un InventoryTransaction « sur une entité » est-elle un coup ?
pub fn is_attack(action: u32) -> bool {
    action == 1 || action == 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combat_packets_have_the_documented_layout() {
        assert_eq!(actor_event(5, EVENT_HURT, 0), vec![5, 2, 0, 0]);
        let r = respawn((1.0, 2.0, 3.0), RESPAWN_READY, 300);
        assert_eq!(r.len(), 12 + 1 + 2);
        assert_eq!((r[12], &r[13..]), (RESPAWN_READY, &[0xac, 0x02][..]));
        assert_eq!(read_respawn_state(&r), Some(RESPAWN_READY));
        assert_eq!(set_actor_motion(1, (0.0, 0.5, 0.0)).len(), 1 + 12 + 1);
        assert_eq!(death_info("Mort"), vec![4, b'M', b'o', b'r', b't', 0]);
        // PlayerAction « réapparaître » : identifiant, action (zigzag), puis le reste
        assert_eq!(read_player_action(&[0x01, 0x0e, 0, 0, 0, 0]), Some((1, ACTION_RESPAWN)));
    }

    #[test]
    fn an_attack_transaction_names_its_target() {
        // Requête 0, aucune action d'inventaire, type 3, cible 7, action 1 (attaque), puis le reste
        let mut body = vec![0x00, 0x03, 0x00, 0x07, 0x01];
        body.extend_from_slice(&[0x00, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(read_attack(&body), Some((7, 1)));
        // Une utilisation d'objet sur un bloc (type 2) n'est pas un coup
        let other = vec![0x00, 0x02, 0x00, 0x07, 0x01];
        assert_eq!(read_attack(&other), None);
        // Coup tel que le jeu 1.26 l'envoie (relevé) : en-tête avec l'indicateur, cible 1, action 2
        let real = [0x00, 0x00, 0x03, 0x00, 0x01, 0x02, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let (target, action) = read_attack(&real).unwrap();
        assert_eq!(target, 1);
        assert!(is_attack(action));
    }
}
