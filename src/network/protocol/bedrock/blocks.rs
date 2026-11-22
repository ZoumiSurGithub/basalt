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
