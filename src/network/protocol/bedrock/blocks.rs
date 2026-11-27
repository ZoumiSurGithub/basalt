//! Interactions avec les blocs lues dans PlayerAuthInput.

use super::*;

// -------------------------------------------------- Actions sur les blocs ----

/// Interaction avec un bloc contenue dans PlayerAuthInput.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockUse {
    /// 0 : poser, 1 : utiliser, 2 : casser
    pub action: i32,
    pub pos: (i32, i32, i32),
    pub face: u8,
    /// Point cliqué sur la face, de 0 à 1 sur chaque axe (le centre quand il est inconnu)
    pub click: (f32, f32, f32),
}
/// Cherche dans PlayerAuthInput (format 1.26.60) une interaction avec un bloc. Ne lit que
/// le début de la transaction, avant la description de l'objet tenu en main.
pub fn read_block_use(body: &[u8]) -> Option<BlockUse> {
    let mut pos = 32; // rotation, position, déplacement, rotation de la tête
    let count = read_varu32(body, &mut pos)? as usize;
    if count > 96 {
        return None;
    }
    let mut interacts = false;
    for _ in 0..count {
        if read_vari32(body, &mut pos)? == 34 {
            interacts = true; // PerformItemInteraction
        }
    }
    if !interacts {
        return None;
    }
    read_varu32(body, &mut pos)?; // mode de saisie
    read_varu32(body, &mut pos)?; // mode de jeu
    read_vari32(body, &mut pos)?; // modèle d'interaction
    pos = pos.checked_add(8)?; // rotation d'interaction
    read_varu64(body, &mut pos)?; // tick
    pos = pos.checked_add(12)?; // vitesse
    if *body.get(pos)? != 1 {
        return None; // transaction annoncée mais absente
    }
    pos += 1;
    let legacy = read_vari32(body, &mut pos)?;
    let has_slots = *body.get(pos)?;
    pos += 1;
    if has_slots == 1 {
        let n = read_varu32(body, &mut pos)? as usize;
        if n > 64 {
            return None;
        }
        for _ in 0..n {
            pos += 1; // conteneur
            let m = read_varu32(body, &mut pos)? as usize;
            pos = pos.checked_add(m)?;
        }
    } else if has_slots != 0 {
        return None;
    }
    let _ = legacy;
    // Actions d'inventaire jointes au geste (par exemple l'usure de l'outil) : sautées
    let actions = read_varu32(body, &mut pos)? as usize;
    if actions > 16 {
        return None;
    }
    for _ in 0..actions {
        match read_varu32(body, &mut pos)? {
            0 | 99999 => pos += 1,
            2 => {
                read_varu32(body, &mut pos)?;
            }
            1 | 3 => {}
            _ => return None,
        }
        read_varu32(body, &mut pos)?;
        skip_item(body, &mut pos)?;
        skip_item(body, &mut pos)?;
    }
    let action = read_vari32(body, &mut pos)?;
    pos += 1; // origine du geste
    let x = read_vari32(body, &mut pos)?;
    let y = read_vari32(body, &mut pos)?;
    let z = read_vari32(body, &mut pos)?;
    let face = *body.get(pos)?;
    Some(BlockUse { action, pos: (x, y, z), face, click: (0.5, 0.5, 0.5) })
}

/// Drapeaux de saisie présents dans PlayerAuthInput (au plus 96).
pub fn input_flags(body: &[u8]) -> Vec<i32> {
    let mut pos = 32;
    let count = match read_varu32(body, &mut pos) {
        Some(c) if c <= 96 => c,
        _ => return Vec::new(),
    };
    (0..count).filter_map(|_| read_vari32(body, &mut pos)).collect()
}

/// Gestes de minage joints à PlayerAuthInput (drapeau 35) : (action, bloc visé, face).
/// Actions : 0 début, 1 abandon, 2 fin, 18 le joueur continue de frapper.
///
/// Disposition relevée avec le jeu 1.26.65 : après la vitesse viennent trois octets de
/// présence — interaction, demande d'objets, gestes sur les blocs — puis, pour ces derniers,
/// leur nombre et pour chacun l'action, le bloc et la face (des zéros pour « fin »). Lu
/// seulement quand le paquet ne porte ni interaction ni demande d'objets, qui précèdent.
pub fn read_block_actions(body: &[u8]) -> Option<Vec<(i32, (i32, i32, i32), i32)>> {
    let mut pos = 32;
    let count = read_varu32(body, &mut pos)? as usize;
    if count > 96 {
        return None;
    }
    let mut mines = false;
    for _ in 0..count {
        mines |= read_vari32(body, &mut pos)? == 35;
    }
    if !mines {
        return None;
    }
    read_varu32(body, &mut pos)?; // mode de saisie
    read_varu32(body, &mut pos)?; // mode de jeu
    read_vari32(body, &mut pos)?; // modèle d'interaction
    pos = pos.checked_add(8)?; // rotation d'interaction
    read_varu64(body, &mut pos)?; // tick
    pos = pos.checked_add(12)?; // vitesse
    if body.get(pos..pos + 3)? != [0, 0, 1] {
        return None;
    }
    pos += 3;
    let n = read_varu32(body, &mut pos)?;
    if !(1..=16).contains(&n) {
        return None;
    }
    let mut out = Vec::new();
    for _ in 0..n {
        let action = read_vari32(body, &mut pos)?;
        let x = read_vari32(body, &mut pos)?;
        let y = read_vari32(body, &mut pos)?;
        let z = read_vari32(body, &mut pos)?;
        let face = read_vari32(body, &mut pos)?;
        if !(0..=40).contains(&action) {
            return None;
        }
        out.push((action, (x, y, z), face));
    }
    Some(out)
}

/// Geste fait avec l'objet tenu, sans viser de bloc (transaction d'inventaire à part).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemGesture {
    /// Clic dans le vide : début d'un repas, et fin du précédent si le bouton reste tenu
    ClickAir,
    /// Bouton relâché avant la fin (repas abandonné, arc détendu)
    Release,
}

/// Lit une transaction d'inventaire sans objets déplacés. Disposition relevée avec le jeu
/// 1.26.65 : identifiant (0), octet « cases jointes » (0), type (2 objet utilisé, 4 objet
/// relâché), nombre d'actions (0), puis pour un objet utilisé le geste (1 : dans le vide).
pub fn read_item_gesture(body: &[u8]) -> Option<ItemGesture> {
    let mut pos = 0;
    read_vari32(body, &mut pos)?;
    // Cases que le jeu a déjà changées de son côté (une armure enfilée d'un clic droit dans
    // le vide en annonce une : celle de l'armure) : sautées
    match *body.get(pos)? {
        0 => pos += 1,
        1 => {
            pos += 1;
            let n = read_varu32(body, &mut pos)? as usize;
            if n > 64 {
                return None;
            }
            for _ in 0..n {
                pos += 1; // conteneur
                let m = read_varu32(body, &mut pos)? as usize;
                pos = pos.checked_add(m)?;
            }
        }
        _ => return None,
    }
    let kind = read_varu32(body, &mut pos)?;
    let actions = read_varu32(body, &mut pos)? as usize;
    if actions == 0 {
        return match kind {
            2 => (read_vari32(body, &mut pos)? == 1).then_some(ItemGesture::ClickAir),
            4 => Some(ItemGesture::Release),
            _ => None,
        };
    }
    // Le geste arrive avec les objets qu'il déplace (la pièce d'armure qui quitte la main),
    // dont la disposition exacte n'est pas connue. Le geste lui-même se reconnaît : clic dans
    // le vide (1), origine, aucun bloc visé (0, 0, 0, face 255), puis la case tenue.
    if kind != 2 || actions > 16 {
        return None;
    }
    let rest = body.get(pos..)?;
    let found = rest.windows(8).any(|w| w[0] == 2 && w[1] <= 2 && w[2..5] == [0, 0, 0] && w[5] == 0xff && w[6] <= 16 && w[6] % 2 == 0 && w[7] == 0);
    found.then_some(ItemGesture::ClickAir)
}

/// Vrai si PlayerAuthInput annonce une interaction (sert au diagnostic).
pub fn input_has_interaction(body: &[u8]) -> bool {
    let mut pos = 32;
    let count = match read_varu32(body, &mut pos) {
        Some(c) if c <= 96 => c,
        _ => return false,
    };
    (0..count).any(|_| read_vari32(body, &mut pos) == Some(34))
}

#[cfg(test)]
mod chat_tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len() / 2).map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// Clic droit dans le vide avec une pièce d'armure en main, relevé dans le journal d'un
    /// vrai jeu 1.26.65 : le paquet annonce la case d'armure changée et l'objet qui quitte la
    /// main. C'est bien un clic dans le vide (il n'était pas reconnu : l'armure restait en main
    /// pour le serveur).
    #[test]
    fn a_click_in_the_air_with_armour_in_hand_is_a_click_in_the_air() {
        let boots = hex("af31010106010102010001000006720201000000001cffff010a000003060044616d6167650000000000000000000000000000000000000000000200000000ff0c00720201000000001cffff010a000003060044616d616765000000000000000000000000001447e9c5723d8142f22a77c3000000000000000000000000000000");
        assert_eq!(read_item_gesture(&boots), Some(ItemGesture::ClickAir));
        let enchanted = hex("b7310101060103020100010000057402010000000038ffff010a000003060044616d6167650e000000090400656e63680a01000000020200696419000203006c766c02000000000000000000000000000000000000000200000000ff0a007402010000000038ffff010a000003060044616d6167650e000000090400656e63680a01000000020200696419000203006c766c0200000000000000000000003ef2e8c5723d814257057ac3000000000000000000000000000000");
        assert_eq!(read_item_gesture(&enchanted), Some(ItemGesture::ClickAir));
        // Un clic ordinaire dans le vide et un clic sur un bloc restent ce qu'ils étaient
        assert_eq!(read_item_gesture(&hex("0000020002")), Some(ItemGesture::ClickAir));
        assert_eq!(read_item_gesture(&hex("000002000001bb747cf503010a00")), None);
    }

    /// Paquets relevés dans le journal d'un vrai jeu 1.26.65 (sans la position du joueur).
    #[test]
    fn gestures_of_the_real_game_are_read() {
        // Début de minage : « début » puis « continue » sur le bloc 181 63 294, face du dessus
        let input = |tail: &str| {
            let mut body = vec![0u8; 32];
            body.extend(hex(tail));
            body
        };
        let start = input("034660640102008a12114294f62cc382070f298e332e90a0bdd8f305b60000010200ea027ecc040224ea027ecc0402000000000000000000004651ca3d3a7217bfb3d74cbf0000000000000000");
        assert_eq!(read_block_actions(&start), Some(vec![(0, (181, 63, 294), 1), (18, (181, 63, 294), 1)]));
        // La suite : « continue », tick après tick
        let go = input("034660640102008a12114294f62cc38307000000002e90a0bdb34692b50000010124ea027ecc0402000000000000000000004851ca3d397217bfb3d74cbf0000000000000000");
        assert_eq!(read_block_actions(&go), Some(vec![(18, (181, 63, 294), 1)]));
        // Un simple mouvement n'en porte pas
        let walk = input("0260640102008262d74108e394c1970847922c3d2e90a0bd344c85bc000000000000000000000000004529903ef725e9be3038583f0000000000000000");
        assert_eq!(read_block_actions(&walk), None);
        // Clic dans le vide avec un aliment, puis bouton relâché
        assert_eq!(read_item_gesture(&hex("000002000200000000ff0400080140000000000a00000000000000000000000000000000000000000000000000000000000000000000000000")), Some(ItemGesture::ClickAir));
        assert_eq!(read_item_gesture(&hex("000004000004080140000000000a0000000000000000000000000000000000000000000000")), Some(ItemGesture::Release));
        // Un bloc posé (cases jointes) n'est pas un geste dans le vide
        assert_eq!(read_item_gesture(&hex("877401011d010002010001000000840103000000000a00000000000000000000")), None);
    }

    #[test]
    fn chat_roundtrip_and_limits() {
        assert_eq!(read_chat(&text_raw("x")), None);
        assert_eq!(read_chat(&[0, 1, 1, 200]), None);
        assert_eq!(read_chat(&[]), None);
    }

    #[test]
    fn block_use_is_read_from_input() {
        let mut w = W::default();
        for _ in 0..8 {
            w.f32(0.0);
        }
        w.varu32(2);
        w.vari32(48);
        w.vari32(34);
        w.varu32(1);
        w.varu32(0);
        w.vari32(1);
        w.f32(0.0);
        w.f32(0.0);
        w.varu64(77);
        for _ in 0..3 {
            w.f32(0.0);
        }
        w.bool(true);
        w.vari32(0);
        w.bool(false);
        w.varu32(0);
        w.vari32(2);
        w.u8(1);
        w.vari32(-3);
        w.vari32(-61);
        w.vari32(5);
        w.u8(1);
        assert!(input_has_interaction(&w.0));
        assert_eq!(read_block_use(&w.0), Some(BlockUse { action: 2, pos: (-3, -61, 5), face: 1, click: (0.5, 0.5, 0.5) }));
        // sans le drapeau d'interaction, rien n'est lu
        let mut plain = vec![0u8; 32];
        plain.extend_from_slice(&[1, 96, 1, 0, 0]);
        assert_eq!(read_block_use(&plain), None);
        assert_eq!(read_block_use(&[0; 10]), None);
    }
}
