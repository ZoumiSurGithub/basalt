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
pub fn add_player(uuid: &[u8; 16], name: &str, id: i64, pos: (f32, f32, f32), pitch: f32, yaw: f32, gamemode: i32, flags: i64) -> Vec<u8> {
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
    // Objet tenu : aucun. Même écriture qu'une case vide d'inventaire (relevée avec le jeu
    // 1.26.65 : huit octets nuls), et non un simple zéro comme autrefois.
    super::item_stack(&mut w, 0, 0, 0, 0);
    w.vari32(gamemode);
    // Données de l'entité : drapeaux (nom visible, collisions, gravité) et nom affiché
    // Chaque donnée s'écrit : numéro, variante, type (un octet), valeur — comme dans le
    // paquet de données d'entité que le jeu 1.26.65 accepte déjà à l'entrée. Sans l'octet
    // de type, le jeu répondait « invalid enum value ».
    w.varu32(2);
    w.varu32(0); // n° 0 : indicateurs
    w.varu32(7);
    w.u8(7); // entier 64 bits
    w.vari64(flags);
    w.varu32(4); // n° 4 : nom affiché
    w.varu32(4);
    w.u8(4); // texte
    w.string(name);
    w.varu32(0); // propriétés entières
    w.varu32(0); // propriétés flottantes
    w.0.extend_from_slice(&id.to_le_bytes()); // capacités : joueur concerné
    w.u8(1); // permissions : membre
    w.u8(0); // permissions de commande
    // Une couche de capacités, la couche de base, copiée du paquet officiel : toutes les
    // capacités définies, valeurs d'un joueur ordinaire, vitesses de vol et de marche
    w.u8(1);
    w.0.extend_from_slice(&1u16.to_le_bytes());
    w.u32(0x000f_ffff);
    w.u32(0x0000_003f);
    w.f32(0.05);
    w.f32(1.0);
    w.f32(0.1);
    w.varu32(0); // liens
    // Un octet nul suit les liens dans le paquet du serveur officiel 1.26.65 (relevé par
    // l'outil de capture) : sans lui, le jeu répond « invalid string ».
    w.u8(0);
    w.string(""); // identifiant d'appareil
    // Plateforme : 8, la valeur envoyée par le serveur officiel pour un joueur Windows. Le jeu
    // vérifie cette valeur ; « inconnue » (-1) n'a jamais été vue acceptée.
    w.i32(8);
    w.0
}

pub const ID_MOVE_ACTOR_ABSOLUTE: u32 = 18;

/// Déplace une entité (ici un autre joueur) chez un client : position absolue et angles
/// arrondis sur un octet. `pos` est la position des yeux pour un joueur.
pub fn move_actor_absolute(id: i64, pos: (f32, f32, f32), pitch: f32, yaw: f32, head_yaw: f32, on_ground: bool) -> Vec<u8> {
    // Un tour complet tient sur 256 valeurs
    let angle = |a: f32| ((a.rem_euclid(360.0) / 360.0 * 256.0) as u32 & 0xff) as u8;
    let mut w = W::default();
    w.varu64(id as u64);
    w.u8(on_ground as u8); // drapeaux : au sol
    w.f32(pos.0);
    w.f32(pos.1);
    w.f32(pos.2);
    w.u8(angle(pitch));
    w.u8(angle(yaw));
    w.u8(angle(head_yaw));
    w.0
}

pub fn remove_actor(id: i64) -> Vec<u8> {
    let mut w = W::default();
    w.vari64(id);
    w.0
}

#[cfg(test)]
mod add_player_tests {
    use super::*;

    #[test]
    fn another_player_moves_with_a_compact_packet() {
        let p = move_actor_absolute(7, (1.0, 64.0, -2.0), -45.0, 90.0, 180.0, true);
        let mut expected = vec![7u8, 1];
        for v in [1.0f32, 64.0, -2.0] {
            expected.extend_from_slice(&v.to_le_bytes());
        }
        // -45° = 315° -> 224 ; 90° -> 64 ; 180° -> 128
        expected.extend_from_slice(&[224, 64, 128]);
        assert_eq!(p, expected);
    }

    /// Paquet du serveur officiel 1.26.65, relevé par l'outil de capture (618 octets) : son
    /// début, jusqu'au nombre de données d'entité, et sa fin, après ces données. Le pseudo et
    /// l'identifiant d'appareil y sont remplacés par des valeurs neutres de même longueur.
    const OFFICIAL_HEAD: &[u8] = b"\x00\x40\x00\xde\xc0\x17\x5a\xb4\x02\x00\x00\x00\x00\x00\x00\x80\x0b\x54\x65\x6d\x6f\x69\x6e\x54\x65\x73\x74\x31\x01\x00\x00\x00\x00\x3f\x00\x00\x8e\x42\x00\x00\x00\x3f\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x0a\x50";
    const OFFICIAL_TAIL: &[u8] = b"\x00\x00\x01\x00\x00\x00\xf0\xff\xff\xff\x01\x00\x02\x05\x00\x3f\x01\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\xff\xff\x0f\x00\x3f\x00\x00\x00\xcd\xcc\x4c\x3d\x00\x00\x80\x3f\xcd\xcc\xcc\x3d\x00\x00\x20\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x30\x08\x00\x00\x00";

    #[test]
    fn add_player_is_laid_out_like_the_official_one() {
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&OFFICIAL_HEAD[..16]);
        let ours = add_player(&uuid, "TemoinTest1", 1, (0.5, 71.0, 0.5), 0.0, 0.0, 5, super::super::REMOTE_PLAYER_FLAGS);
        // Tout le début est identique, à l'octet près ; seul le nombre de données diffère
        // (le serveur officiel en envoie 80, deux suffisent)
        assert_eq!(&ours[..75], &OFFICIAL_HEAD[..75]);
        assert_eq!((ours[75], OFFICIAL_HEAD[75]), (2, 80));
        // Première donnée : numéro 0, variante 7, type 7 — comme dans le paquet officiel
        assert_eq!(&ours[76..79], &[0, 7, 7]);

        // Fin officielle : propriétés (0, 0), capacités, liens, octet nul, appareil, plateforme
        let t = OFFICIAL_TAIL;
        assert_eq!(&t[..2], &[0, 0]);
        assert_eq!((t[10], t[11], t[12]), (1, 0, 2), "permissions, puis deux couches de 22 octets");
        let after_layers = 13 + 2 * 22;
        assert_eq!(&t[after_layers..after_layers + 3], &[0, 0, 32], "liens, octet nul, longueur de l'identifiant d'appareil");
        assert_eq!(&t[t.len() - 4..], &[8, 0, 0, 0]);
        // La nôtre suit le même ordre : une seule couche (la couche de base, identique à
        // l'officielle) et pas d'identifiant d'appareil
        let n = ours.len();
        let base_layer = &t[13 + 22..13 + 44];
        assert_eq!(&ours[n - 42..n - 40], &[0, 0], "propriétés");
        assert_eq!(&ours[n - 40..n - 32], &1i64.to_le_bytes());
        assert_eq!(&ours[n - 32..n - 29], &[1, 0, 1], "permissions, puis une couche");
        assert_eq!(&ours[n - 29..n - 7], base_layer);
        assert_eq!(&ours[n - 7..], &[0, 0, 0, 8, 0, 0, 0]);
    }
}
