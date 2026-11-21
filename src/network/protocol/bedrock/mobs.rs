//! Créatures (animaux) : apparition d'une entité du jeu.
//!
//! Disposition de AddActor d'après la documentation du protocole et les serveurs existants ;
//! pas encore relevée auprès du serveur officiel 1.26. Les données d'entité s'écrivent comme
//! celles des objets au sol, relevées, elles (numéro, variante, type sur un octet, valeur).

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
        }
    }
}

/// Indicateurs d'une créature : respire (35), a des collisions (48), subit la gravité (49),
/// peut grimper (19), marche (« can walk », 64… inconnu : non mis).
pub const MOB_FLAGS: i64 = (1 << 19) | (1 << 35) | (1 << 48) | (1 << 49);

/// Données d'entité d'une créature : indicateurs, air, taille, boîte de collision, variante.
pub fn mob_metadata(width: f32, height: f32, variant: i32, color: u8, flags: i64) -> Vec<(u32, Meta)> {
    vec![
        (0, Meta::Long(flags)),
        (2, Meta::Int(variant)),
        (3, Meta::Byte(color)),
        (4, Meta::Text(String::new())),
        (7, Meta::Short(300)),
        (38, Meta::Float(1.0)),
        (42, Meta::Short(300)),
        (53, Meta::Float(width)),
        (54, Meta::Float(height)),
        (81, Meta::Byte(0)),
    ]
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
    // Attributs : la vie, pour la barre affichée et les animations
    w.varu32(1);
    w.string("minecraft:health");
    w.f32(0.0);
    w.f32(health);
    w.f32(health);
    write_metadata(&mut w, metadata);
    w.varu32(0); // propriétés entières
    w.varu32(0); // propriétés décimales
    w.varu32(0); // liens
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
    }
}
