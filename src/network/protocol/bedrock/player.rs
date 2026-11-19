//! Joueurs : lecture des mouvements, déplacement, apparition et disparition des autres joueurs.

#[allow(unused_imports)]
use super::*;

/// Ce que le serveur lit d'un paquet PlayerAuthInput.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Input {
    pub pitch: f32,
    pub yaw: f32,
    pub pos: (f32, f32, f32),
    pub head_yaw: f32,
}

/// Lit le début de PlayerAuthInput : rotation (2 flottants), position (3),
/// vecteur de déplacement (2), rotation de la tête (1). Refuse les valeurs aberrantes.
pub fn read_input(body: &[u8]) -> Option<Input> {
    let f = |at: usize| -> Option<f32> {
        body.get(at..at + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (pitch, yaw, x, y, z) = (f(0)?, f(4)?, f(8)?, f(12)?, f(16)?);
    let head_yaw = f(28).unwrap_or(yaw);
    let sane = x.is_finite() && y.is_finite() && z.is_finite() && x.abs() < 3.0e7 && z.abs() < 3.0e7 && y.abs() < 1.0e5;
    if !sane {
        return None;
    }
    let angle = |a: f32| if a.is_finite() { a % 360.0 } else { 0.0 };
    Some(Input { pitch: angle(pitch).max(-90.0).min(90.0), yaw: angle(yaw), pos: (x, y, z), head_yaw: angle(head_yaw) })
}

/// Déplace un joueur chez un client. `teleport` : repositionnement imposé par le serveur.
pub fn move_player(id: i64, pos: (f32, f32, f32), pitch: f32, yaw: f32, head_yaw: f32, teleport: bool) -> Vec<u8> {
    let mut w = W::default();
    w.varu64(id as u64);
    w.f32(pos.0);
    w.f32(pos.1);
    w.f32(pos.2);
    w.f32(pitch);
    w.f32(yaw);
    w.f32(head_yaw);
    w.u8(if teleport { 2 } else { 0 }); // mode : normal ou téléportation
    w.bool(true); // au sol
    w.varu64(0); // monture
    if teleport {
        w.i32(0); // cause : inconnue
        w.i32(0); // type d'entité source
    }
    w.varu64(0); // tick
    w.0
}

/// Fait apparaître un autre joueur chez un client.
pub fn add_player(uuid: &[u8; 16], name: &str, id: i64, pos: (f32, f32, f32), pitch: f32, yaw: f32, gamemode: i32) -> Vec<u8> {
    let mut w = W::default();
    w.0.extend_from_slice(uuid);
    w.string(name);
    w.varu64(id as u64); // identifiant d'exécution
    w.string(""); // identifiant de chat de la plateforme
    w.f32(pos.0);
    w.f32(pos.1);
    w.f32(pos.2);
    for _ in 0..3 {
        w.f32(0.0); // vitesse
    }
    w.f32(pitch);
    w.f32(yaw);
    w.f32(yaw); // rotation de la tête
    w.vari32(0); // objet tenu : aucun
    w.vari32(gamemode);
    // Données de l'entité : drapeaux (nom visible, collisions, gravité) et nom affiché
    w.varu32(2);
    w.varu32(0);
    w.varu32(7);
    w.vari64((1 << 14) | (1 << 15) | (1 << 48) | (1 << 49));
    w.varu32(4);
    w.varu32(4);
    w.string(name);
    w.varu32(0); // propriétés entières
    w.varu32(0); // propriétés flottantes
    w.0.extend_from_slice(&id.to_le_bytes()); // capacités : joueur concerné
    w.u8(1); // permissions : membre
    w.u8(0); // permissions de commande
    w.u8(0); // aucune couche de capacités
    w.varu32(0); // liens
    w.bool(false); // pas de bloc porteur
    w.string(""); // identifiant d'appareil
    w.i32(-1); // plateforme inconnue
    w.0
}

pub fn remove_actor(id: i64) -> Vec<u8> {
    let mut w = W::default();
    w.vari64(id);
    w.0
}
