//! Entrée dans le monde : packs de ressources, StartGame, registres, zone de chunks.

#[allow(unused_imports)]
use super::*;

/// Liste des packs de ressources : aucun.
pub fn resource_packs_info(features: crate::versions::Features) -> Vec<u8> {
    let mut w = W::default();
    w.bool(false); // pack obligatoire
    w.bool(false); // packs d'extension
    w.bool(false); // scripts
    w.bool(false); // désactivation forcée des « Vibrant Visuals »
    w.u64(0); // identifiant du modèle de monde (UUID)
    w.u64(0);
    w.string(""); // version du modèle
    if features.varint_pack_count {
        w.varu32(0); // nombre de packs : 0 (le jeu refuse tout octet en trop)
    } else {
        w.0.extend_from_slice(&[0, 0]); // nombre de packs : 0, sur deux octets
    }
    w.0
}

pub fn resource_pack_stack() -> Vec<u8> {
    let mut w = W::default();
    w.bool(false); // packs obligatoires
    w.varu32(0); // liste des packs
    w.string("*"); // version de base du jeu
    w.u32(0); // expériences
    w.bool(false);
    w.u8(0); // packs de l'éditeur
    w.0
}

pub struct StartGame<'a> {
    pub entity_id: i64,
    pub gamemode: i32,
    pub position: (f32, f32, f32),
    pub world_name: &'a str,
    pub game_version: &'a str,
    pub features: crate::versions::Features,
}

/// Paquet StartGame, d'après la documentation officielle du protocole
/// (mojang.github.io/bedrock-protocol-docs). C'est le paquet le plus fragile :
/// son format change presque à chaque version du jeu.
pub fn start_game(s: &StartGame) -> Vec<u8> {
    let mut w = W::default();
    w.vari64(s.entity_id); // identifiant unique
    w.varu64(s.entity_id as u64); // identifiant d'exécution
    w.vari32(s.gamemode);
    w.f32(s.position.0);
    w.f32(s.position.1);
    w.f32(s.position.2);
    w.f32(0.0); // inclinaison
    w.f32(0.0); // orientation

    // --- LevelSettings ---
    w.u64(0); // graine
    w.i16(0); // biome d'apparition : par défaut
    w.string("plains");
    w.vari32(0); // dimension : surface
    w.vari32(2); // générateur : plat
    w.vari32(s.gamemode); // mode de jeu du monde
    w.bool(false); // hardcore
    w.vari32(1); // difficulté : facile
    w.vari32(0); // point d'apparition
    w.vari32(GROUND_Y);
    w.vari32(0);
    w.bool(true); // succès désactivés
    w.vari32(0); // type de monde éditeur
    w.bool(false); // créé dans l'éditeur
    w.bool(false); // exporté de l'éditeur
    if s.features.editor_migration_version {
        w.u8(0); // version de migration éditeur
    }
    w.vari32(-1); // heure figée : non
    w.varu32(0); // offre Education
    w.bool(false); // fonctions Education
    w.string(""); // produit Education
    w.f32(0.0); // pluie
    w.f32(0.0); // orage
    w.bool(false); // contenu verrouillé à la plateforme
    w.bool(true); // multijoueur
    w.bool(true); // diffusion LAN
    w.vari32(4); // diffusion Xbox Live : public
    w.vari32(4); // diffusion plateforme : public
    w.bool(true); // commandes activées
    w.bool(false); // packs de textures obligatoires
    w.varu32(0); // règles du jeu
    w.u32(0); // expériences
    w.bool(false);
    w.bool(false); // coffre bonus
    w.bool(false); // carte de départ
    w.u8(1); // permissions : membre
    w.i32(4); // rayon de mise à jour des chunks
    for _ in 0..10 {
        // packs verrouillés, modèle verrouillé, gamertags MSA, modèle de monde,
        // option verrouillée, villageois v1, persona, skins, émotes
        w.bool(false);
    }
    w.string("*"); // version de base du jeu
    w.i32(0); // largeur limitée
    w.i32(0); // profondeur limitée
    w.bool(false); // type de Nether
    w.string(""); // ressource partagée Education
    w.string("");
    w.bool(false); // gameplay expérimental forcé : absent
    w.u8(0); // restriction du chat
    w.bool(false); // interactions désactivées
    w.vari32(0); // politique de connexion éditeur
    w.bool(false); // blocs anonymes en monde éditeur
    // --- fin de LevelSettings ---

    w.string(""); // identifiant du niveau
    w.string(s.world_name);
    w.string(""); // identité du modèle
    w.bool(false); // version d'essai
    w.vari32(0); // historique de retour arrière
    w.bool(false); // casse de blocs autoritaire
    w.u64(0); // temps courant
    w.vari32(0); // graine d'enchantement
    w.varu32(0); // propriétés de blocs personnalisés
    w.string(""); // identifiant de corrélation
    w.bool(true); // gestion des piles d'objets côté serveur
    w.string(s.game_version);
    w.0.extend_from_slice(&[0x0a, 0x00, 0x00]); // propriétés du joueur : NBT vide
    w.u64(0); // somme de contrôle du registre de blocs
    w.u64(0); // identifiant du modèle de monde
    w.u64(0);
    w.bool(false); // génération côté client
    w.bool(true); // les identifiants de blocs sont des hachages
    w.bool(false); // sons autoritaires
    w.bool(false); // informations de serveur : absentes
    for _ in 0..4 {
        w.string(""); // télémétrie
    }
    w.0
}

/// Règles de structures « jigsaw » : un NBT vide suffit, mais le paquet est obligatoire.
pub fn jigsaw_structure_data() -> Vec<u8> {
    // NBT « réseau » : un objet racine dont les quatre listes de règles sont vides.
    // Un objet vide ne suffit pas : le jeu affiche « Données de structure manquantes ».
    let mut out = vec![0x0a, 0x00];
    for key in ["processors", "template_pools", "jigsaws", "structure_sets"].iter() {
        out.push(0x09); // liste
        out.push(key.len() as u8);
        out.extend_from_slice(key.as_bytes());
        out.push(0x0a); // d'objets
        out.push(0x00); // longueur 0
    }
    out.push(0x00);
    out
}

/// VoxelShapes sans aucune forme : attendu par les jeux récents avant StartGame.
pub fn voxel_shapes() -> Vec<u8> {
    vec![0, 0, 0, 0]
}

pub fn item_registry() -> Vec<u8> {
    vec![0]
}

pub fn biome_definition_list() -> Vec<u8> {
    vec![0, 0]
}

pub fn chunk_radius_updated(radius: i32) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(radius);
    w.0
}

pub fn chunk_publisher_update(x: i32, y: i32, z: i32, radius_blocks: u32) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(x);
    w.vari32(y);
    w.vari32(z);
    w.varu32(radius_blocks);
    w.u32(0); // chunks enregistrés
    w.0
}
