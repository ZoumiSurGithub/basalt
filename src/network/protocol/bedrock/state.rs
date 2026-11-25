//! État initial du joueur : attributs, capacités, données, heure, réglages.

use super::*;

// ------------------------------------------------- État initial du joueur ----
// Paquets qu'un serveur officiel envoie avant l'apparition : sans eux, l'interface
// de jeu (barres de vie, inventaire) ne peut pas s'afficher. Formats d'après la
// documentation officielle ; le jeu signale ceux qu'il ne sait pas lire, et le
// serveur les retire alors de la liste (voir `Server::refused_extras`).

pub const ID_SET_TIME: u32 = 10;
pub const ID_UPDATE_ATTRIBUTES: u32 = 29;
pub const ID_SET_ACTOR_DATA: u32 = 39;
pub const ID_PLAYER_HOTBAR: u32 = 48;
pub const ID_INVENTORY_CONTENT: u32 = 49;
pub const ID_CREATIVE_CONTENT: u32 = 145;
pub const ID_UPDATE_ABILITIES: u32 = 187;

/// SetTime : heure du jour.
pub fn set_time(time: i32) -> Vec<u8> {
    let mut w = W::default();
    w.vari32(time);
    w.0
}

/// Inventaire créatif vide.
pub fn creative_content_empty() -> Vec<u8> {
    vec![0, 0]
}

/// Attributs d'un joueur neuf.
pub fn update_attributes(runtime_id: u64) -> Vec<u8> {
    update_attributes_with(runtime_id, 20.0, 20.0, 20.0, 0, 0.0)
}

/// Attributs du joueur à l'apparition : vie, faim, expérience, et aussi vitesses, attaque…
pub fn update_attributes_with(runtime_id: u64, health: f32, max_health: f32, food: f32, level: i32, progress: f32) -> Vec<u8> {
    attributes(runtime_id, health, max_health, food, level, progress, true)
}

/// Mise à jour en cours de partie : seulement la vie, la faim et l'expérience. Renvoyer la
/// vitesse de marche annulerait le sprint du joueur (le client remet sa vitesse à 0,1).
pub fn update_vitals(runtime_id: u64, health: f32, max_health: f32, food: f32, level: i32, progress: f32) -> Vec<u8> {
    attributes(runtime_id, health, max_health, food, level, progress, false)
}

const VITALS: [&str; 4] = ["minecraft:health", "minecraft:player.hunger", "minecraft:player.level", "minecraft:player.experience"];

/// Attributs du joueur (vie, faim, expérience…) ; `full` envoie aussi ceux qui ne changent pas.
fn attributes(runtime_id: u64, health: f32, max_health: f32, food: f32, level: i32, progress: f32, full: bool) -> Vec<u8> {
    const BIG: f32 = 3.402_823_5e38;
    // (nom, minimum, maximum, valeur, valeur par défaut)
    let list: [(&str, f32, f32, f32, f32); 10] = [
        ("minecraft:health", 0.0, max_health, health.min(max_health), max_health),
        ("minecraft:player.hunger", 0.0, 20.0, food, 20.0),
        ("minecraft:player.saturation", 0.0, 20.0, 5.0, 5.0),
        ("minecraft:player.exhaustion", 0.0, 20.0, 0.0, 0.0),
        ("minecraft:player.level", 0.0, 24791.0, level as f32, 0.0),
        ("minecraft:player.experience", 0.0, 1.0, progress, 0.0),
        ("minecraft:movement", 0.0, BIG, 0.1, 0.1),
        ("minecraft:underwater_movement", 0.0, BIG, 0.02, 0.02),
        ("minecraft:attack_damage", 0.0, BIG, 1.0, 1.0),
        ("minecraft:absorption", 0.0, 16.0, 0.0, 0.0),
    ];
    let list: Vec<_> = list.iter().filter(|a| full || VITALS.contains(&a.0)).collect();
    let mut w = W::default();
    w.varu64(runtime_id);
    w.varu32(list.len() as u32);
    for (name, min, max, value, default) in list.iter().copied() {
        w.f32(*min);
        w.f32(*max);
        w.f32(*value);
        w.f32(*min); // minimum par défaut
        w.f32(*max); // maximum par défaut
        w.f32(*default);
        w.string(name);
        w.varu32(0); // aucun modificateur
    }
    w.varu64(0); // tick
    w.0
}

/// La vitesse de marche seule (0,1 pour un joueur sans effet, à l'arrêt du sprint).
pub fn update_movement(runtime_id: u64, value: f32) -> Vec<u8> {
    let mut w = W::default();
    w.varu64(runtime_id);
    w.varu32(1);
    w.f32(0.0);
    w.f32(3.402_823_5e38);
    w.f32(value);
    w.f32(0.0);
    w.f32(3.402_823_5e38);
    w.f32(0.1);
    w.string("minecraft:movement");
    w.varu32(0);
    w.varu64(0);
    w.0
}

/// Capacités selon le mode de jeu (voler, construire, attaquer…).
pub fn update_abilities(unique_id: i64, mode: crate::player::GameMode) -> Vec<u8> {
    use crate::player::GameMode;
    let mut w = W::default();
    w.0.extend_from_slice(&unique_id.to_le_bytes());
    w.u8(1); // permissions : membre
    w.u8(0); // commandes : niveau normal
    w.varu32(1); // une couche
    w.0.extend_from_slice(&1u16.to_le_bytes()); // couche de base
    w.u32((1 << 20) - 1); // toutes les capacités sont définies
    // Capacités : 0 construire, 1 miner, 2 portes et leviers, 3 coffres, 4 attaquer les joueurs,
    // 5 attaquer les créatures, 8 invulnérable, 9 en vol, 10 vol autorisé, 11 construction
    // instantanée, 13 et 19 vitesses de vol, 14 vitesse de marche, 17 traverser les blocs
    let speeds: u32 = (1 << 13) | (1 << 14) | (1 << 19);
    let values: u32 = match mode {
        GameMode::Survival => 0x3f | speeds,
        GameMode::Creative => 0x3f | speeds | (1 << 8) | (1 << 10) | (1 << 11),
        GameMode::Adventure => (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | speeds,
        GameMode::Spectator => speeds | (1 << 8) | (1 << 9) | (1 << 10) | (1 << 17),
    };
    w.u32(values);
    w.f32(0.05); // vitesse de vol
    w.f32(1.0); // vitesse de vol verticale
    w.f32(0.1); // vitesse de marche
    w.0
}

/// Indicateurs d'un autre joueur tel que le serveur officiel 1.26.65 les envoie (relevé) :
/// respire, a des collisions, subit la gravité — plus le nom toujours visible.
pub const REMOTE_PLAYER_FLAGS: i64 = 0x0006_0000_0010_0000 | (1 << 14) | (1 << 15);
/// Accroupi.
pub const FLAG_SNEAKING: i64 = 1 << 1;
/// En train de courir.
pub const FLAG_SPRINTING: i64 = 1 << 3;

/// Indicateurs du joueur chez lui : peut grimper (19), respire (35), a des collisions (48),
/// subit la gravité (49).
pub const OWN_PLAYER_FLAGS: i64 = (1i64 << 19) | (1 << 35) | (1 << 48) | (1 << 49);

/// Met à jour les indicateurs d'une entité chez un client (accroupi, en course…).
pub fn set_actor_flags(runtime_id: u64, flags: i64) -> Vec<u8> {
    let mut w = W::default();
    w.varu64(runtime_id);
    w.varu32(1); // une donnée
    w.varu32(0); // n° 0 : indicateurs
    w.varu32(7);
    w.u8(7); // entier 64 bits
    w.vari64(flags);
    w.varu32(0); // propriétés entières
    w.varu32(0); // propriétés décimales
    w.varu64(0); // tick
    w.0
}

/// Données d'entité du joueur.
pub fn set_actor_data_player(runtime_id: u64) -> Vec<u8> {
    let mut w = W::default();
    w.varu64(runtime_id);
    w.varu32(1); // une donnée
    w.varu32(0); // n° 0 : indicateurs
    w.varu32(7); // variante : entier 64 bits
    w.u8(7); // type : entier 64 bits
    // peut grimper (19), respire (35), a des collisions (48), subit la gravité (49).
    // Sans le drapeau de gravité, le joueur glisse et ne peut pas sauter.
    w.vari64(OWN_PLAYER_FLAGS);
    w.varu32(0); // propriétés entières
    w.varu32(0); // propriétés décimales
    w.varu64(0); // tick
    w.0
}

/// Contenu d'un inventaire entièrement vide.
pub fn inventory_content_empty(container: u32, slots: u32, f: crate::versions::Features) -> Vec<u8> {
    let mut w = W::default();
    w.varu32(container);
    w.varu32(slots);
    for _ in 0..slots {
        // Emplacement vide. En 1.26.60 un objet vide occupe 8 octets nuls (variante, ancien
        // format, nom, variante de l'objet, quantité sur 2 octets, bloc, données) : relevé
        // dans les paquets d'interaction envoyés par le jeu.
        w.0.extend_from_slice(&[0u8; 8]);
    }
    w.u8(0); // nom complet du conteneur
    if f.editor_migration_version {
        w.bool(false); // identifiant dynamique : absent (optionnel depuis 1.26.60)
    } else {
        w.u32(0);
    }
    w.0.extend_from_slice(&[0u8; 8]); // objet de stockage : aucun
    w.0
}

/// Case sélectionnée dans la barre d'objets.
pub fn player_hotbar(slot: u32) -> Vec<u8> {
    let mut w = W::default();
    w.varu32(slot);
    w.u8(0); // inventaire du joueur
    w.bool(true);
    w.0
}

pub const ID_SET_COMMANDS_ENABLED: u32 = 59;
pub const ID_SET_DIFFICULTY: u32 = 60;
pub const ID_SET_PLAYER_GAME_TYPE: u32 = 62;
pub const ID_PLAYER_LIST: u32 = 63;
pub const ID_GAME_RULES_CHANGED: u32 = 72;
pub const ID_AVAILABLE_ACTOR_IDENTIFIERS: u32 = 119;
pub const ID_UPDATE_ADVENTURE_SETTINGS: u32 = 188;

/// Paquets d'état du joueur dont le format reste à confirmer avec le vrai jeu.
pub const EXTRA_IDS: [u32; 19] = [
    80, // UpdateTrade (écran d'échange)
    56, // BlockActorData (cloche qui sonne)
    25, // LevelEvent (bruit des portes)
    52,
    302,
    ID_UPDATE_ATTRIBUTES,
    ID_UPDATE_ABILITIES,
    ID_SET_ACTOR_DATA,
    ID_SET_TIME,
    ID_CREATIVE_CONTENT,
    ID_INVENTORY_CONTENT,
    ID_PLAYER_HOTBAR,
    ID_SET_COMMANDS_ENABLED,
    ID_SET_DIFFICULTY,
    ID_SET_PLAYER_GAME_TYPE,
    ID_PLAYER_LIST,
    ID_GAME_RULES_CHANGED,
    ID_AVAILABLE_ACTOR_IDENTIFIERS,
    ID_UPDATE_ADVENTURE_SETTINGS,
];

/// Réglages d'aventure (combat entre joueurs, monde modifiable).
pub fn update_adventure_settings() -> Vec<u8> {
    // pas de dégâts joueur contre joueur désactivés, créatures et joueurs peuvent se battre,
    // monde modifiable, noms affichés, compétences automatiques
    vec![0, 0, 0, 1, 1]
}

/// Liste de biomes réduite au seul biome utilisé par le terrain plat : les plaines (n° 1).
/// Format d'après la documentation officielle (BiomeDefinitionData).
pub fn biome_definition_list_plains() -> Vec<u8> {
    let mut w = W::default();
    w.varu32(1); // un biome
    w.0.extend_from_slice(&0u16.to_le_bytes()); // son nom : chaîne n° 0
    w.0.extend_from_slice(&1u16.to_le_bytes()); // identifiant : 1
    w.f32(0.8); // température
    w.f32(0.4); // humidité
    w.f32(0.0); // neige sur le feuillage
    w.f32(0.1); // profondeur
    w.f32(0.2); // relief
    w.i32(0xff44_aff5u32 as i32); // couleur de l'eau sur les cartes
    w.bool(true); // pluie
    w.bool(false); // étiquettes : absentes
    w.bool(false); // données de génération : absentes
    w.varu32(1); // une chaîne
    w.string("plains");
    w.0
}

#[cfg(test)]
mod attribute_tests {
    /// Le sprint ne doit pas être annulé : une mise à jour de la vie ne contient pas la vitesse.
    #[test]
    fn vitals_update_leaves_movement_alone() {
        let p = super::update_vitals(1, 15.0, 20.0, 18.0, 0, 0.0);
        let text = String::from_utf8_lossy(&p);
        assert!(text.contains("minecraft:health") && text.contains("minecraft:player.hunger"));
        assert!(!text.contains("minecraft:movement"));
        assert_eq!(p[1], 4);
        let full = super::update_attributes_with(1, 15.0, 20.0, 18.0, 0, 0.0);
        assert!(String::from_utf8_lossy(&full).contains("minecraft:movement"));
    }
}
