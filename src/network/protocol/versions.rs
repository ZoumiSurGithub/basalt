//! Versions du jeu prises en charge.
//!
//! Chaque client annonce son numéro de protocole à la connexion ; le serveur
//! adapte ensuite l'encodage des paquets à ce numéro. Les différences sont
//! tirées du journal officiel du protocole (mojang.github.io/bedrock-protocol-docs).

/// Versions stables connues : (protocole, nom affiché).
pub const KNOWN: &[(i32, &str)] = &[
    (2168, "1.26.40"),
    (2169, "1.26.45"),
    (2193, "1.26.50"),
    (2225, "1.26.65"),
    (2259, "1.26.70 (préversion)"),
];

/// Plus ancien et plus récent protocole dont l'encodage est connu.
pub const MIN_PROTOCOL: i32 = 2168;
pub const MAX_PROTOCOL: i32 = 2259;

/// Nom de la version correspondant à un protocole (la plus proche en dessous).
pub fn name(protocol: i32) -> &'static str {
    KNOWN.iter().rev().find(|(p, _)| *p <= protocol).map(|(_, n)| *n).unwrap_or("inconnue")
}

/// Ce qui change d'un protocole à l'autre pour les paquets que le serveur envoie.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Features {
    /// 1.26.50 : le client exige JigsawStructureData avant StartGame.
    pub jigsaw_before_start: bool,
    /// 1.26.60 : LevelChunk se termine par « mise à jour de biomes ».
    pub chunk_biome_update_flag: bool,
    /// 1.26.60 : LevelSettings contient la version de migration de l'éditeur.
    pub editor_migration_version: bool,
    /// Préversion 1.26.70 (protocole 2259) : NetworkSettings se termine par le profil SCTP.
    pub sctp_recovery_profile: bool,
    /// 1.26.60 : dans ResourcePacksInfo, le nombre de packs est un entier à taille variable.
    pub varint_pack_count: bool,
    /// 1.26.50 : le client attend VoxelShapes avant StartGame.
    pub voxel_shapes: bool,
}

/// Ce que gère une version du jeu, d'après son numéro de protocole.
pub fn features(protocol: i32) -> Features {
    Features {
        jigsaw_before_start: protocol >= 2177,
        chunk_biome_update_flag: protocol >= 2208,
        editor_migration_version: protocol >= 2216,
        sctp_recovery_profile: protocol >= 2251,
        varint_pack_count: protocol >= 2208,
        voxel_shapes: protocol >= 2193,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_features() {
        assert_eq!(name(2168), "1.26.40");
        assert_eq!(name(2200), "1.26.50");
        assert_eq!(name(2225), "1.26.65");
        assert_eq!(name(100), "inconnue");
        assert!(!features(2169).jigsaw_before_start);
        assert!(features(2193).jigsaw_before_start && !features(2193).editor_migration_version);
        let f = features(2225);
        assert!(f.jigsaw_before_start && f.chunk_biome_update_flag && f.editor_migration_version);
        assert!(!f.sctp_recovery_profile && features(2259).sctp_recovery_profile);
    }
}
