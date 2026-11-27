//! Entrée dans le monde : packs de ressources, StartGame, registres, zone de chunks.

use super::*;

/// Identifiant d'un pack tel que le jeu l'écrit : deux moitiés de 8 octets, petit-boutistes.
fn pack_uuid(w: &mut W, bytes: [u8; 16]) {
    let half = |b: &[u8]| u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
    w.u64(half(&bytes[..8]));
    w.u64(half(&bytes[8..]));
}

/// Liste des packs de ressources proposés au joueur. `forced` : le joueur doit les accepter
/// pour entrer.
///
/// Par pack, d'après la documentation officielle du protocole : identifiant, version,
/// taille, clé de déchiffrement (vide si le pack n'est pas chiffré), sous-pack, identité du
/// contenu, scripts, extension, lancer de rayons, adresse de téléchargement.
pub fn resource_packs_info(packs: &[crate::utils::packs::ResourcePack], forced: bool) -> Vec<u8> {
    let mut w = W::default();
    w.bool(forced && !packs.is_empty()); // packs obligatoires
    w.bool(false); // packs d'extension
    w.bool(false); // scripts
    w.bool(false); // désactivation forcée des « Vibrant Visuals »
    w.u64(0); // identifiant du modèle de monde (UUID)
    w.u64(0);
    w.string(""); // version du modèle
    // Nombre de packs : en entier à taille variable. Vérifié auprès du jeu 1.26.65 : écrit
    // sur deux octets, comme le décrivent les références plus anciennes du protocole, il
    // décale toute la suite et le jeu coupe (« invalid string », paquet 6).
    w.varu32(packs.len() as u32);
    for pack in packs {
        pack_uuid(&mut w, pack.uuid_bytes().unwrap_or([0; 16]));
        w.string(&pack.version);
        w.u64(pack.data.len() as u64);
        w.string(&pack.key);
        w.string(""); // sous-pack
        // Identité du contenu : celle du pack quand il est chiffré (le jeu y rattache la clé)
        w.string(if pack.key.is_empty() { "" } else { &pack.uuid });
        w.bool(false); // scripts
        w.bool(false); // pack d'extension
        w.bool(false); // lancer de rayons
        w.string(""); // adresse de téléchargement : le serveur envoie lui-même le fichier
    }
    w.0
}

/// Pile de packs de ressources : ceux que le jeu doit appliquer, dans l'ordre.
pub fn resource_pack_stack(packs: &[crate::utils::packs::ResourcePack], forced: bool) -> Vec<u8> {
    let mut w = W::default();
    w.bool(forced && !packs.is_empty()); // packs obligatoires
    w.varu32(packs.len() as u32); // liste des packs
    for pack in packs {
        w.string(&pack.uuid);
        w.string(&pack.version);
        w.string(""); // sous-pack
    }
    w.string("*"); // version de base du jeu
    w.u32(0); // expériences
    w.bool(false);
    w.u8(0); // packs de l'éditeur
    w.0
}

/// Réponses du jeu à la liste des packs.
pub const PACKS_REFUSED: u8 = 1;
pub const PACKS_SEND: u8 = 2;
pub const PACKS_HAVE_ALL: u8 = 3;
pub const PACKS_COMPLETED: u8 = 4;

/// Lit la réponse du jeu : son état, et les packs qu'il demande (`identifiant_version`).
///
/// Disposition relevée auprès du jeu 1.26.65 : le rang de l'état, son nom en toutes lettres
/// (`downloading`, `downloadingfinished`…), puis, pour une demande d'envoi, le nombre de
/// packs et leurs noms. Le nom fait foi ; le rang sert pour ceux qu'on n'a pas encore vus.
pub fn read_pack_response(body: &[u8]) -> Option<(u8, Vec<String>)> {
    let mut pos = 0;
    let rank = read_varu32(body, &mut pos)?;
    let text = |pos: &mut usize| -> Option<String> {
        let len = read_varu32(body, pos)? as usize;
        let bytes = body.get(*pos..pos.checked_add(len)?)?;
        *pos += len;
        Some(String::from_utf8_lossy(bytes).into_owned())
    };
    let name = text(&mut pos).unwrap_or_default().to_lowercase();
    let status = match name.as_str() {
        "downloading" => PACKS_SEND,
        "downloadingfinished" => PACKS_HAVE_ALL,
        n if n.contains("stack") => PACKS_COMPLETED,
        n if n.contains("refus") || n.contains("cancel") => PACKS_REFUSED,
        _ => match rank {
            0 => PACKS_REFUSED,
            1 => PACKS_SEND,
            2 => PACKS_HAVE_ALL,
            _ => PACKS_COMPLETED,
        },
    };
    let mut ids = Vec::new();
    if status == PACKS_SEND {
        for _ in 0..read_varu32(body, &mut pos).unwrap_or(0).min(64) {
            match text(&mut pos) {
                Some(id) => ids.push(id),
                None => break,
            }
        }
    }
    Some((status, ids))
}

/// Annonce d'un pack avant son envoi : taille des morceaux, leur nombre, taille et empreinte
/// du fichier.
pub fn resource_pack_data_info(pack: &crate::utils::packs::ResourcePack) -> Vec<u8> {
    let mut w = W::default();
    w.string(&pack.wire_id());
    w.u32(crate::utils::packs::CHUNK_SIZE as u32);
    w.u32(pack.chunk_count());
    w.u64(pack.data.len() as u64);
    w.varu32(pack.hash.len() as u32);
    w.0.extend_from_slice(&pack.hash);
    w.bool(false); // contenu payant
    w.u8(6); // sorte de pack : ressources
    w.0
}

/// Lit la demande d'un morceau : (pack, rang du morceau).
pub fn read_pack_chunk_request(body: &[u8]) -> Option<(String, u32)> {
    let mut pos = 0;
    let len = read_varu32(body, &mut pos)? as usize;
    let id = String::from_utf8_lossy(body.get(pos..pos + len)?).into_owned();
    pos += len;
    let b = body.get(pos..pos + 4)?;
    Some((id, u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
}

/// Un morceau de pack : rang, position de son premier octet dans le fichier, octets.
pub fn resource_pack_chunk_data(id: &str, index: u32, offset: u64, data: &[u8]) -> Vec<u8> {
    let mut w = W::default();
    w.string(id);
    w.u32(index);
    w.u64(offset);
    w.varu32(data.len() as u32);
    w.0.extend_from_slice(data);
    w.0
}

pub struct StartGame<'a> {
    pub entity_id: i64,
    pub gamemode: i32,
    pub position: (f32, f32, f32),
    /// Regard : (inclinaison, orientation)
    pub rotation: (f32, f32),
    pub world_name: &'a str,
    pub game_version: &'a str,
    /// Graine, et terrain plat ou non (seulement annoncés au jeu)
    pub seed: u64,
    pub flat: bool,
    /// Dimension du monde où le joueur entre (0 surface, 1 Nether, 2 End)
    pub dimension: i32,
    /// Point d'apparition du monde (bloc où se tiennent les pieds)
    pub spawn: (i32, i32, i32),
    pub features: crate::versions::Features,
    /// Blocs créés par les plugins : (nom, déclaration en NBT réseau)
    pub custom_blocks: &'a [(String, Vec<u8>)],
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
    w.f32(s.rotation.0); // inclinaison
    w.f32(s.rotation.1); // orientation

    // --- LevelSettings ---
    w.u64(s.seed); // graine
    w.i16(0); // biome d'apparition : par défaut
    w.string("plains");
    w.vari32(s.dimension); // dimension : surface, Nether ou End
    w.vari32(if s.flat { 2 } else { 1 }); // générateur : plat ou normal
    w.vari32(s.gamemode); // mode de jeu du monde
    w.bool(false); // hardcore
    w.vari32(1); // difficulté : facile
    w.vari32(s.spawn.0); // point d'apparition
    w.vari32(s.spawn.1);
    w.vari32(s.spawn.2);
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
    // Blocs créés par les plugins : nom, puis déclaration (composants, menu, numéro)
    w.varu32(s.custom_blocks.len() as u32);
    for (name, definition) in s.custom_blocks {
        w.string(name);
        w.0.extend_from_slice(definition);
    }
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

/// Listes vides, utilisées quand les vraies données ne sont pas disponibles.
pub fn item_registry() -> Vec<u8> {
    vec![0]
}

pub fn biome_definition_list() -> Vec<u8> {
    vec![0, 0]
}

/// Distance de vue accordée au joueur.
pub fn chunk_radius_updated(radius: i32) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(radius);
    w.0
}

/// Centre et rayon des chunks que le joueur doit afficher.
pub fn chunk_publisher_update(x: i32, y: i32, z: i32, radius_blocks: u32) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(x);
    w.vari32(y);
    w.vari32(z);
    w.varu32(radius_blocks);
    w.u32(0); // chunks enregistrés
    w.0
}

/// ChangeDimension : le joueur passe dans une autre dimension, à cette position. Format relevé
/// auprès du serveur officiel (protocole 2225) : dimension, position, « réapparition » (faux),
/// puis le numéro de l'écran de chargement (présent, sur quatre octets).
pub fn change_dimension(dimension: i32, pos: (f32, f32, f32), screen: u32) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(dimension);
    w.f32(pos.0);
    w.f32(pos.1);
    w.f32(pos.2);
    w.bool(false);
    w.bool(true);
    w.u32(screen);
    w.0
}

#[cfg(test)]
mod dimension_tests {
    /// Les deux paquets relevés : vers le Nether puis vers l'End, en (0.5, 80, 0.5).
    #[test]
    fn a_dimension_change_is_written_like_the_official_server_does() {
        let hex = |b: Vec<u8>| b.iter().map(|x| format!("{:02x}", x)).collect::<String>();
        assert_eq!(hex(super::change_dimension(1, (0.5, 80.0, 0.5), 0)), "020000003f0000a0420000003f000100000000");
        assert_eq!(hex(super::change_dimension(2, (0.5, 80.0, 0.5), 1)), "040000003f0000a0420000003f000101000000");
    }
}

/// La même annonce, avec la liste des chunks qui viennent d'être envoyés : le serveur
/// officiel la joint après chaque vague de chunks (nombre sur quatre octets, puis x et z de
/// chacun), et répète l'annonce à chaque tick avec la position exacte du joueur.
pub fn chunk_publisher_update_with(x: i32, y: i32, z: i32, radius_blocks: u32, sent: &[(i32, i32)]) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(x);
    w.vari32(y);
    w.vari32(z);
    w.varu32(radius_blocks);
    w.u32(sent.len() as u32);
    for (cx, cz) in sent {
        w.vari32(*cx);
        w.vari32(*cz);
    }
    w.0
}

#[cfg(test)]
mod publisher_tests {
    /// Octets relevés auprès du serveur officiel : joueur en (400, -58, 400), 128 blocs, sans
    /// chunk nouveau ; puis en (6, -58, 6) avec des chunks (les trois premiers du relevé).
    #[test]
    fn the_served_zone_is_announced_like_the_official_server_does() {
        let hex = |b: Vec<u8>| b.iter().map(|x| format!("{:02x}", x)).collect::<String>();
        assert_eq!(hex(super::chunk_publisher_update_with(400, -58, 400, 128, &[])), "a00673a006800100000000");
        assert_eq!(hex(super::chunk_publisher_update_with(6, -58, 6, 128, &[(-3, 2), (3, -2), (0, 0)])), "0c730c800103000000050406030000");
    }
}

#[cfg(test)]
mod pack_info_tests {
    use crate::utils::packs;

    #[test]
    fn pack_count_is_a_single_varint() {
        // 4 booléens, UUID (16 octets), chaîne vide (1 octet), nombre de packs (1 octet).
        let p = super::resource_packs_info(&[], true);
        assert_eq!(p.len(), 22);
        assert_eq!(*p.last().unwrap(), 0);
        assert_eq!(p[0], 0, "sans pack, rien n'est obligatoire");
    }

    #[test]
    fn packs_are_offered_announced_and_sent() {
        let manifest = br#"{"header":{"name":"Essai","uuid":"0fba4063-dba1-4281-9b89-ff9390653530","version":[1,0,3]}}"#;
        let key = "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345";
        let pack = packs::from_bytes(packs::zip_store(&[("manifest.json", manifest)]), key, "essai.mcpack").unwrap();
        let info = super::resource_packs_info(std::slice::from_ref(&pack), true);
        assert_eq!((info[0], info[21]), (1, 1));
        // Identifiant : deux moitiés retournées ; puis version, taille, clé
        assert_eq!(&info[22..30], &[0x81, 0x42, 0xa1, 0xdb, 0x63, 0x40, 0xba, 0x0f]);
        assert_eq!(&info[38..44], &[5, b'1', b'.', b'0', b'.', b'3']);
        assert_eq!(&info[44..52], &(pack.data.len() as u64).to_le_bytes());
        assert_eq!(info[52] as usize, key.len());
        assert_eq!(&info[53..85], key.as_bytes());
        // Le jeu demande le pack, le serveur l'annonce puis l'envoie par morceaux
        // (les deux réponses relevées auprès du jeu 1.26.65, octet pour octet)
        let id = pack.wire_id();
        let mut ask = vec![0x01, 0x0b];
        ask.extend_from_slice(b"downloading");
        ask.push(1);
        ask.push(id.len() as u8);
        ask.extend_from_slice(id.as_bytes());
        assert_eq!(super::read_pack_response(&ask), Some((super::PACKS_SEND, vec![id.clone()])));
        let mut done = vec![0x02, 0x13];
        done.extend_from_slice(b"downloadingfinished");
        assert_eq!(super::read_pack_response(&done), Some((super::PACKS_HAVE_ALL, vec![])));
        // États pas encore relevés : d'après leur rang
        assert_eq!(super::read_pack_response(&[0x00, 0x00]).map(|r| r.0), Some(super::PACKS_REFUSED));
        assert_eq!(super::read_pack_response(&[0x03, 0x00]).map(|r| r.0), Some(super::PACKS_COMPLETED));
        let announce = super::resource_pack_data_info(&pack);
        assert_eq!(announce[0] as usize, id.len());
        assert!(announce.windows(32).any(|w| w == pack.hash));
        let mut request = vec![id.len() as u8];
        request.extend_from_slice(id.as_bytes());
        request.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(super::read_pack_chunk_request(&request), Some((id.clone(), 0)));
        let (offset, bytes) = pack.chunk(0).unwrap();
        let chunk = super::resource_pack_chunk_data(&id, 0, offset, bytes);
        assert!(chunk.ends_with(bytes) && chunk.len() > bytes.len() + id.len());
        // La pile reprend les packs dans l'ordre
        let stack = super::resource_pack_stack(std::slice::from_ref(&pack), false);
        assert_eq!((stack[0], stack[1], stack[2] as usize), (0, 1, pack.uuid.len()));
    }
}
