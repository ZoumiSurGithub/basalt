//! Créatures (animaux) : apparition d'une entité du jeu.
//!
//! Disposition de AddActor relevée auprès du serveur officiel 1.26.65 (paquet d'un marchand
//! ambulant, outil de capture) : attributs, 69 données d'entité, et quatre octets nuls à la
//! fin (propriétés entières, propriétés décimales, liens, puis un octet de plus).

#[allow(unused_imports)]
use super::*;

pub const ID_ADD_ACTOR: u32 = 13;

/// Une donnée d'entité : (numéro, valeur).
pub enum Meta {
    Byte(u8),
    Short(i16),
    Int(i32),
    Float(f32),
    Text(String),
    Long(i64),
    Vec3(f32, f32, f32),
    /// Document NBT vide (un « compound » sans nom ni contenu)
    EmptyNbt,
}

/// Écrit des données d'entité.
pub fn write_metadata(w: &mut W, entries: &[(u32, Meta)]) {
    w.varu32(entries.len() as u32);
    for (key, value) in entries {
        w.varu32(*key);
        let t: u8 = match value {
            Meta::Byte(_) => 0,
            Meta::Short(_) => 1,
            Meta::Int(_) => 2,
            Meta::Float(_) => 3,
            Meta::Text(_) => 4,
            Meta::Long(_) => 7,
            Meta::Vec3(..) => 8,
            Meta::EmptyNbt => 5,
        };
        // La variante répète le type (comme dans les paquets relevés)
        w.varu32(t as u32);
        w.u8(t);
        match value {
            Meta::Byte(v) => w.u8(*v),
            Meta::Short(v) => w.i16(*v),
            Meta::Int(v) => w.vari32(*v),
            Meta::Float(v) => w.f32(*v),
            Meta::Text(v) => w.string(v),
            Meta::Long(v) => w.vari64(*v),
            Meta::Vec3(x, y, z) => {
                w.f32(*x);
                w.f32(*y);
                w.f32(*z);
            }
            Meta::EmptyNbt => w.0.extend_from_slice(&[0x0a, 0x00, 0x00]),
        }
    }
}

/// Indicateurs d'une créature, ceux du serveur officiel : peut grimper (19), 22, 34,
/// respire (35), a des collisions (48), subit la gravité (49).
pub const MOB_FLAGS: i64 = (1 << 19) | (1 << 22) | (1 << 34) | (1 << 35) | (1 << 48) | (1 << 49);

/// Données d'entité d'une créature : celles relevées auprès du serveur officiel, avec la
/// boîte de collision, la variante, la couleur et les indicateurs de cette créature.
#[allow(clippy::excessive_precision)]
pub fn mob_metadata(width: f32, height: f32, variant: i32, color: u8, flags: i64) -> Vec<(u32, Meta)> {
    let mut list = vec![
        (0, Meta::Long(844476474458112)),
        (1, Meta::Int(1)),
        (2, Meta::Int(0)),
        (3, Meta::Byte(0)),
        (4, Meta::Text("".into())),
        (5, Meta::Long(-1)),
        (6, Meta::Long(0)),
        (7, Meta::Short(300)),
        (10, Meta::Byte(0)),
        (22, Meta::Byte(0)),
        (37, Meta::Long(-1)),
        (38, Meta::Float(1.0)),
        (39, Meta::Byte(0)),
        (42, Meta::Short(300)),
        (43, Meta::Int(0)),
        (44, Meta::Byte(0)),
        (45, Meta::Int(0)),
        (46, Meta::Int(0)),
        (53, Meta::Float(0.6000000238418579)),
        (54, Meta::Float(1.899999976158142)),
        (55, Meta::Int(-1)),
        (56, Meta::Vec3(0.0, 0.0, 0.0)),
        (57, Meta::Byte(0)),
        (58, Meta::Float(0.0)),
        (59, Meta::Byte(0)),
        (60, Meta::Float(0.0)),
        (68, Meta::Long(0)),
        (70, Meta::Byte(0)),
        (71, Meta::Text("".into())),
        (72, Meta::Text("".into())),
        (73, Meta::Byte(1)),
        (74, Meta::Byte(0)),
        (75, Meta::Int(0)),
        (76, Meta::Int(0)),
        (77, Meta::Int(0)),
        (78, Meta::Int(-1)),
        (81, Meta::Byte(0)),
        (82, Meta::Byte(0)),
        (91, Meta::Int(0)),
        (92, Meta::Long(-9_223_372_036_821_221_376)),
        (101, Meta::Int(0)),
        (102, Meta::Int(0)),
        (103, Meta::Int(0)),
        (104, Meta::Int(0)),
        (106, Meta::Int(3)),
        (107, Meta::Byte(1)),
        (108, Meta::Float(8.0)),
        (109, Meta::Float(16.0)),
        (110, Meta::Text("ambient".into())),
        (111, Meta::Float(1.0)),
        (114, Meta::Int(0)),
        (115, Meta::Int(0)),
        (116, Meta::Int(0)),
        (117, Meta::Int(0)),
        (118, Meta::EmptyNbt),
        (119, Meta::Byte(0)),
        (120, Meta::Float(0.0)),
        (124, Meta::Float(1.0)),
        (125, Meta::Int(20)),
        (126, Meta::Int(615)),
        (131, Meta::Long(0)),
        (132, Meta::Text("".into())),
        (134, Meta::Float(0.0)),
        (135, Meta::Float(0.0)),
        (136, Meta::Int(-1)),
        (137, Meta::Int(-1)),
        (138, Meta::Int(-1)),
        (139, Meta::Long(0)),
        (140, Meta::Float(64.0)),
    ];
    for (key, value) in list.iter_mut() {
        match *key {
            0 => *value = Meta::Long(flags),
            2 => *value = Meta::Int(variant),
            3 => *value = Meta::Byte(color),
            53 => *value = Meta::Float(width),
            54 => *value = Meta::Float(height),
            _ => {}
        }
    }
    list
}

/// Indicateurs (numéros du jeu) : bébé, amoureux, tondu.
pub const FLAG_BABY: i64 = 1 << 11;
pub const FLAG_IN_LOVE: i64 = 1 << 7;
pub const FLAG_SHEARED: i64 = 1 << 31;
/// Montures : sellée, apprivoisée, dirigée au clavier par son cavalier, saut chargé
pub const FLAG_SADDLED: i64 = 1 << 8;
pub const FLAG_TAMED: i64 = 1 << 28;
/// Tenue en laisse
pub const FLAG_LEASHED: i64 = 1 << 30;
/// Donnée d'entité : qui tient la laisse (identifiant unique, -1 personne)
pub const META_LEAD_HOLDER: u32 = 37;
/// Cabrée (monture qui refuse son cavalier)
pub const FLAG_REARING: i64 = 1 << 39;
pub const FLAG_WASD_CONTROLLED: i64 = 1 << 44;
pub const FLAG_CAN_POWER_JUMP: i64 = 1 << 45;
/// Cavalier (le joueur assis sur une monture)
pub const FLAG_RIDING: i64 = 1 << 2;
/// Données d'entité du cavalier : position du siège sur la monture
pub const META_SEAT_POSITION: u32 = 56;

/// Événements d'entité propres aux animaux (ActorEvent)
pub const EVENT_EAT_GRASS: u8 = 10;
pub const EVENT_LOVE_PARTICLES: u8 = 21;
/// Dressage d'une monture : raté (fumée, elle se cabre), réussi (cœurs)
pub const EVENT_TAME_FAIL: u8 = 6;
pub const EVENT_TAME_SUCCESS: u8 = 7;

/// SetActorLink : un cavalier monte (1) ou descend (0) d'une monture.
pub const ID_SET_ACTOR_LINK: u32 = 41;
pub const LINK_REMOVE: u8 = 0;
pub const LINK_RIDER: u8 = 1;

/// Un lien monture → cavalier, tel qu'il s'écrit dans SetActorLink et AddActor : monture,
/// cavalier, type, immédiat, à l'initiative du cavalier, vitesse de rotation de la monture.
fn write_link(w: &mut W, ridden: i64, rider: i64, kind: u8, immediate: bool) {
    w.vari64(ridden);
    w.vari64(rider);
    w.u8(kind);
    w.bool(immediate);
    w.bool(true);
    w.f32(0.0);
}

pub fn set_actor_link(ridden: i64, rider: i64, kind: u8, immediate: bool) -> Vec<u8> {
    let mut w = W::default();
    write_link(&mut w, ridden, rider, kind, immediate);
    w.0
}

/// MoveActorAbsolute envoyé par le jeu pour la monture qu'il dirige : (identifiant,
/// position, tangage, lacet). Refuse les valeurs aberrantes.
pub fn read_move_actor(body: &[u8]) -> Option<(u64, (f32, f32, f32), f32, f32)> {
    let mut pos = 0;
    let id = read_varu64(body, &mut pos)?;
    pos += 1; // drapeaux (au sol, téléportation)
    let f = |at: usize| body.get(at..at + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (x, y, z) = (f(pos)?, f(pos + 4)?, f(pos + 8)?);
    let angle = |at: usize| body.get(at).map(|b| *b as f32 * 360.0 / 256.0);
    let (pitch, yaw) = (angle(pos + 12)?, angle(pos + 13)?);
    let sane = x.is_finite() && y.is_finite() && z.is_finite() && x.abs() < 3.0e7 && z.abs() < 3.0e7 && y.abs() < 1.0e5;
    sane.then_some((id, (x, y, z), pitch, yaw))
}

/// SetActorData : données d'entité changées (bébé devenu adulte, mouton tondu…).
pub fn set_actor_data(id: i64, entries: &[(u32, Meta)]) -> Vec<u8> {
    let mut w = W::default();
    w.varu64(id as u64);
    write_metadata(&mut w, entries);
    w.varu32(0); // propriétés entières
    w.varu32(0); // propriétés décimales
    w.varu64(0); // tick
    w.0
}

pub const ID_LEVEL_SOUND_EVENT: u32 = 123;
/// Sons d'une créature (LevelSoundEvent) : depuis la 1.26, le son est désigné par son nom
pub const SOUND_AMBIENT: &str = "ambient";
pub const SOUND_DEATH: &str = "death";
pub const SOUND_HURT: &str = "hurt";
pub const SOUND_EAT: &str = "eat";
/// Sons que le serveur officiel envoie lui-même (le jeu ne les joue pas seul) : selle posée,
/// tonte, traite, monture qui refuse son cavalier, saut et atterrissage d'une monture
pub const SOUND_SADDLE: &str = "saddle";
pub const SOUND_SHEAR: &str = "shear";
pub const SOUND_MILK: &str = "milk";
pub const SOUND_MAD: &str = "mad";
pub const SOUND_JUMP: &str = "jump";
pub const SOUND_LAND: &str = "land";
/// Œuf pondu, laisse attachée ou rompue
pub const SOUND_PLOP: &str = "plop";
pub const SOUND_LEASH_PLACE: &str = "leashknot.place";
pub const SOUND_LEASH_BREAK: &str = "leashknot.break";

/// LevelSoundEvent : un son joué à une position, avec la créature qui l'émet (son cri).
/// Disposition relevée auprès du serveur officiel 1.26.65 : nom du son, position, données
/// en plus, entité, petit, volume relatif, identifiant unique, puis un octet nul.
pub fn level_sound(sound: &str, pos: (f32, f32, f32), entity: &str, baby: bool, unique_id: i64) -> Vec<u8> {
    let mut w = W::default();
    w.string(sound);
    w.f32(pos.0);
    w.f32(pos.1);
    w.f32(pos.2);
    w.vari32(-1); // données en plus : aucune
    w.string(entity);
    w.bool(baby);
    w.bool(false); // volume relatif
    w.0.extend_from_slice(&unique_id.to_le_bytes());
    w.u8(0);
    w.0
}

/// AddActor : une créature apparaît chez le joueur.
#[allow(clippy::too_many_arguments)]
pub fn add_actor(id: i64, kind: &str, pos: (f32, f32, f32), motion: (f32, f32, f32), yaw: f32, health: (f32, f32), mount: Option<(f32, f32)>, rider: Option<i64>, metadata: &[(u32, Meta)]) -> Vec<u8> {
    let mut w = W::default();
    w.vari64(id); // identifiant unique
    w.varu64(id as u64); // identifiant d'exécution
    w.string(kind);
    for v in [pos.0, pos.1, pos.2, motion.0, motion.1, motion.2] {
        w.f32(v);
    }
    w.f32(0.0); // tangage
    w.f32(yaw);
    w.f32(yaw); // tête
    w.f32(yaw); // corps
    // Attributs : (nom, minimum, valeur, maximum), comme le serveur officiel
    // Monture : sa vitesse et la force de son saut (le jeu du cavalier s'en sert pour la diriger)
    let mut attributes: Vec<(&str, f32, f32, f32)> = vec![("minecraft:health", 0.0, health.0, health.1), ("minecraft:movement", 0.0, mount.map(|m| m.0).unwrap_or(0.25), f32::MAX)];
    if let Some((_, jump)) = mount {
        attributes.push(("minecraft:horse.jump_strength", 0.0, jump, 2.0));
    }
    w.varu32(attributes.len() as u32);
    for (name, min, value, max) in attributes {
        w.string(name);
        w.f32(min);
        w.f32(value);
        w.f32(max);
    }
    write_metadata(&mut w, metadata);
    w.varu32(0); // propriétés entières
    w.varu32(0); // propriétés décimales
    // Liens : le cavalier déjà en selle
    match rider {
        Some(r) => {
            w.varu32(1);
            write_link(&mut w, id, r, LINK_RIDER, true);
        }
        None => w.varu32(0),
    }
    // Un octet nul de plus, comme dans le paquet officiel (sans lui : « invalid numeric value »)
    w.u8(0);
    w.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_packets() {
        // La position renvoyée par le jeu du cavalier se relit (angles sur 256 valeurs)
        let pk = move_actor_absolute(300, (1.5, 64.0, -2.25), 0.0, 90.0, 90.0, true);
        let (id, pos, _, yaw) = read_move_actor(&pk).unwrap();
        assert_eq!((id, pos), (300, (1.5, 64.0, -2.25)));
        assert!((yaw - 90.0).abs() < 1.5);
        assert_eq!(read_move_actor(&[1, 0, 0]), None);
        // Lien : monture, cavalier, type, immédiat, à l'initiative du cavalier, rotation
        assert_eq!(set_actor_link(3, 1, LINK_RIDER, false), vec![6, 2, 1, 0, 1, 0, 0, 0, 0]);
        // AddActor d'une monture : vitesse et saut parmi les attributs, cavalier parmi les liens
        let p = add_actor(3, "minecraft:horse", (0.0, 0.0, 0.0), (0.0, 0.0, 0.0), 0.0, (20.0, 25.0), Some((0.3, 0.7)), Some(1), &[]);
        let jump = b"minecraft:horse.jump_strength";
        assert!(p.windows(jump.len()).any(|w| w == jump));
        assert!(p.ends_with(&[1, 6, 2, 1, 1, 1, 0, 0, 0, 0, 0]), "{:?}", &p[p.len() - 12..]);
    }

    #[test]
    fn metadata_is_written_like_the_recorded_one() {
        // Même écriture que les données relevées des objets au sol : n° 38, taille 1,0
        let mut w = W::default();
        write_metadata(&mut w, &[(38, Meta::Float(1.0)), (0, Meta::Long(3 << 48))]);
        assert_eq!(&w.0[..8], &[2, 0x26, 3, 3, 0, 0, 0x80, 0x3f]);
        assert_eq!(&w.0[8..11], &[0, 7, 7]);
        let p = add_actor(5, "minecraft:cow", (1.0, 2.0, 3.0), (0.0, 0.0, 0.0), 90.0, (10.0, 10.0), None, None, &mob_metadata(0.9, 1.4, 0, 0, MOB_FLAGS));
        assert_eq!(&p[..3], &[10, 5, 13]);
        assert_eq!(&p[p.len() - 4..], &[0, 0, 0, 0], "quatre octets nuls à la fin");
        // 69 données d'entité, comme le paquet officiel
        assert_eq!(mob_metadata(0.9, 1.4, 0, 0, MOB_FLAGS).len(), 69);
        // Son relevé auprès du serveur officiel (« disappeared » d'un joueur), à l'identique
        let recorded = "0b64697361707065617265640000003fcdcc8f420000003f01106d696e6563726166743a706c61796572000001000000f0ffffff00";
        let ours: String = level_sound("disappeared", (0.5, 71.9, 0.5), "minecraft:player", false, -68_719_476_735).iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(ours, recorded);
    }
}

