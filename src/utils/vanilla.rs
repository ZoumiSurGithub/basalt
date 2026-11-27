//! `vanilla.properties` : les mécaniques du jeu que l'on peut couper une à une. Tout est
//! activé par défaut ; un serveur de mini-jeux, par exemple, coupe la faim, la météo ou la
//! pousse des cultures. Le fichier est créé au premier démarrage, à côté de server.properties.

use std::path::Path;

macro_rules! vanilla {
    ($( $field:ident, $key:literal, $doc:literal; )*) => {
        /// Les mécaniques du jeu, activées (vrai) ou coupées.
        #[derive(Clone, Debug, PartialEq)]
        pub struct Vanilla { $(pub $field: bool,)* }

        impl Default for Vanilla {
            fn default() -> Self {
                Vanilla { $($field: true,)* }
            }
        }

        impl Vanilla {
            /// Chaque réglage : (clé, explication).
            pub const KEYS: &'static [(&'static str, &'static str)] = &[$(($key, $doc),)*];

            /// Change un réglage. Faux si la clé n'existe pas.
            pub fn set(&mut self, key: &str, on: bool) -> bool {
                match key {
                    $($key => self.$field = on,)*
                    _ => return false,
                }
                true
            }

            pub fn get(&self, key: &str) -> Option<bool> {
                match key {
                    $($key => Some(self.$field),)*
                    _ => None,
                }
            }
        }
    };
}

vanilla! {
    hunger, "hunger", "La faim : la barre baisse en courant, en sautant, en se battant.";
    natural_regeneration, "natural-regeneration", "La vie remonte toute seule quand on a le ventre plein.";
    fall_damage, "fall-damage", "Les dégâts de chute.";
    fire_damage, "fire-damage", "Les dégâts du feu et de la lave.";
    drowning, "drowning", "La noyade des joueurs et des créatures.";
    pvp, "pvp", "Les joueurs peuvent se blesser entre eux.";
    daylight_burn, "daylight-burn", "Zombies et squelettes brûlent au soleil.";
    lightning, "lightning", "La foudre tombe pendant les orages.";
    snow_and_ice, "snow-and-ice", "Sous la pluie, la neige se dépose et l'eau gèle dans les biomes froids ; les chaudrons se remplissent.";
    crop_growth, "crop-growth", "Les cultures poussent.";
    tree_growth, "tree-growth", "Les pousses deviennent des arbres (la poudre d'os agit toujours).";
    grass_spread, "grass-spread", "L'herbe gagne la terre voisine.";
    leaf_decay, "leaf-decay", "Les feuilles d'un arbre abattu disparaissent.";
    liquid_flow, "liquid-flow", "L'eau et la lave coulent.";
    tnt, "tnt", "La TNT s'amorce et explose.";
    fishing, "fishing", "La canne à pêche lance son bouchon.";
    boats, "boats", "Les bateaux se posent.";
    double_chests, "double-chests", "Deux coffres voisins s'unissent en un grand coffre.";
    enchant_rules, "enchant-rules", "/enchant refuse un enchantement qui ne va pas sur l'objet, ou qui en exclut un autre.";
    mob_drops, "mob-drops", "Les créatures lâchent leur butin en mourant.";
    block_drops, "block-drops", "Les blocs cassés en survie lâchent leur objet.";
    experience, "experience", "Les orbes d'expérience (créatures, minerais, pêche, élevage).";
    durability, "durability", "Les outils, les armes et les armures s'usent.";
    trading, "trading", "Les échanges avec les villageois.";
    sleeping, "sleeping", "Dormir dans un lit fait passer la nuit.";
    bone_meal, "bone-meal", "La poudre d'os fait pousser cultures, pousses et herbe.";
    village_animals, "village-animals", "Les enclos et les étables des villages sont peuplés d'animaux.";
}

/// Les réglages qui vivaient dans server.properties avant ce fichier : (clé, explication). Ils y
/// sont toujours lus ; écrits ici, c'est ce fichier qui l'emporte.
pub const MOVED: [(&str, &str); 6] = [
    ("spawn-animals", "Les animaux apparaissent dans le monde."),
    ("spawn-monsters", "Les monstres apparaissent la nuit et dans le noir."),
    ("mob-griefing", "Les créatures et les explosions abîment le monde."),
    ("fire-spreads", "Le feu brûle les blocs et gagne du terrain."),
    ("weather-cycle", "La pluie et l'orage viennent et repartent d'eux-mêmes."),
    ("keep-inventory", "Les joueurs gardent leur inventaire en mourant."),
];

/// Le fichier créé au premier démarrage. `moved` : la valeur actuelle des réglages hérités.
pub fn template(moved: &dyn Fn(&str) -> bool) -> String {
    let mut text = String::from(
        "# Les mécaniques du jeu, une par ligne : true (activée) ou false (coupée).\n# Tout ce qui est coupé ici l'est pour tout le serveur. Redémarrez après un changement.\n# Un plugin peut aller plus loin au cas par cas (annuler un dégât, changer un message de mort) :\n# voir docs/PLUGINS.md.\n\n",
    );
    for (key, doc) in MOVED {
        text.push_str(&format!("# {}\n{}={}\n\n", doc, key, moved(key)));
    }
    for (key, doc) in Vanilla::KEYS {
        text.push_str(&format!("# {}\n{}=true\n\n", doc, key));
    }
    text
}

/// Lit le texte du fichier : les réglages connus, et les réglages hérités qui y figurent.
pub fn parse(text: &str) -> (Vanilla, Vec<(String, bool)>, Vec<String>) {
    let (mut v, mut moved, mut unknown) = (Vanilla::default(), Vec::new(), Vec::new());
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, on) = (key.trim(), value.trim() != "false");
        if MOVED.iter().any(|m| m.0 == key) {
            moved.push((key.to_string(), on));
        } else if !v.set(key, on) {
            unknown.push(key.to_string());
        }
    }
    (v, moved, unknown)
}

/// Lit vanilla.properties ; s'il n'existe pas, le crée.
pub fn load(path: &Path, moved: &dyn Fn(&str) -> bool) -> (Vanilla, Vec<(String, bool)>) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => {
            let fresh = template(moved);
            match std::fs::write(path, &fresh) {
                Ok(()) => crate::info!("{} créé : les mécaniques du jeu s'y coupent une à une", path.display()),
                Err(e) => crate::warn!("Impossible de créer {} : {}", path.display(), e),
            }
            fresh
        }
    };
    let (v, moved, unknown) = parse(&text);
    for key in unknown {
        crate::warn!("vanilla.properties : réglage inconnu « {} »", crate::log::clean(&key, 48));
    }
    let off: Vec<&str> = Vanilla::KEYS.iter().map(|k| k.0).filter(|k| v.get(k) == Some(false)).collect();
    if !off.is_empty() {
        crate::info!("Mécaniques du jeu coupées (vanilla.properties) : {}", off.join(", "));
    }
    (v, moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vanilla_file_turns_things_off_one_by_one() {
        // Tout est activé par défaut, et le fichier créé se relit tel quel
        let fresh = template(&|key| key != "keep-inventory");
        let (v, moved, unknown) = parse(&fresh);
        assert_eq!(v, Vanilla::default());
        assert!(unknown.is_empty() && moved.len() == MOVED.len());
        assert!(moved.contains(&("keep-inventory".to_string(), false)) && moved.contains(&("weather-cycle".to_string(), true)));
        // Chaque réglage a sa ligne et son explication
        assert!(Vanilla::KEYS.iter().all(|(key, doc)| fresh.contains(&format!("{}=true", key)) && fresh.contains(doc)));
        // Une ligne à false coupe une mécanique, et rien d'autre
        let (v, moved, unknown) = parse("hunger=false\n# pvp=false\nfall-damage = false\ninconnu=true\nspawn-monsters=false\n");
        assert!(!v.hunger && !v.fall_damage && v.pvp && v.fishing);
        assert_eq!((moved, unknown), (vec![("spawn-monsters".to_string(), false)], vec!["inconnu".to_string()]));
        assert_eq!((v.get("hunger"), v.get("pvp"), v.get("rien")), (Some(false), Some(true), None));
    }
}
