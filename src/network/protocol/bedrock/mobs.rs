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

/// Événements d'entité propres aux animaux (ActorEvent)
pub const EVENT_EAT_GRASS: u8 = 10;
pub const EVENT_LOVE_PARTICLES: u8 = 21;

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
/// Sons d'une créature (LevelSoundEvent)
pub const SOUND_AMBIENT: u32 = 10;
pub const SOUND_DEATH: u32 = 14;
pub const SOUND_HURT: u32 = 17;
pub const SOUND_EAT: u32 = 31;

/// LevelSoundEvent : un son joué à une position, avec la créature qui l'émet (son cri).
/// Disposition d'après la documentation du protocole (pas encore relevée).
pub fn level_sound(sound: u32, pos: (f32, f32, f32), entity: &str, baby: bool, unique_id: i64) -> Vec<u8> {
    let mut w = W::default();
    w.varu32(sound);
    w.f32(pos.0);
    w.f32(pos.1);
    w.f32(pos.2);
    w.vari32(-1); // données en plus : aucune
    w.string(entity);
    w.bool(baby);
    w.bool(false); // volume relatif
    w.0.extend_from_slice(&unique_id.to_le_bytes());
    w.0
}

/// AddActor : une créature apparaît chez le joueur.
#[allow(clippy::too_many_arguments)]
pub fn add_actor(id: i64, kind: &str, pos: (f32, f32, f32), motion: (f32, f32, f32), yaw: f32, health: f32, metadata: &[(u32, Meta)]) -> Vec<u8> {
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
    let attributes: [(&str, f32, f32, f32); 2] = [("minecraft:health", 0.0, health, health), ("minecraft:movement", 0.0, 0.25, f32::MAX)];
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
    w.varu32(0); // liens
    // Un octet nul de plus, comme dans le paquet officiel (sans lui : « invalid numeric value »)
    w.u8(0);
    w.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_written_like_the_recorded_one() {
        // Même écriture que les données relevées des objets au sol : n° 38, taille 1,0
        let mut w = W::default();
        write_metadata(&mut w, &[(38, Meta::Float(1.0)), (0, Meta::Long(3 << 48))]);
        assert_eq!(&w.0[..8], &[2, 0x26, 3, 3, 0, 0, 0x80, 0x3f]);
        assert_eq!(&w.0[8..11], &[0, 7, 7]);
        let p = add_actor(5, "minecraft:cow", (1.0, 2.0, 3.0), (0.0, 0.0, 0.0), 90.0, 10.0, &mob_metadata(0.9, 1.4, 0, 0, MOB_FLAGS));
        assert_eq!(&p[..3], &[10, 5, 13]);
        assert_eq!(&p[p.len() - 4..], &[0, 0, 0, 0], "quatre octets nuls à la fin");
        // 69 données d'entité, comme le paquet officiel
        assert_eq!(mob_metadata(0.9, 1.4, 0, 0, MOB_FLAGS).len(), 69);
    }
}

