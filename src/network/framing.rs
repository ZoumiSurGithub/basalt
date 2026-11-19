//! Découpage des messages NetherNet sur le canal de données fiable.
//!
//! Chaque message WebRTC commence par un octet : le nombre de fragments qui
//! restent à venir. Zéro signifie « message complet » ou « dernier fragment » ;
//! le premier fragment d'un message découpé porte donc la valeur la plus haute.

/// Taille de la charge utile d'un fragment émis.
pub const SEGMENT: usize = 10_000;
/// Un message ne peut pas dépasser 256 fragments (compteur sur un octet).
pub const MAX_SEGMENTS: usize = 256;

/// Découpe un message en trames prêtes à être envoyées.
pub fn split(message: &[u8]) -> Result<Vec<Vec<u8>>, &'static str> {
    if message.is_empty() {
        return Err("message vide");
    }
    let count = (message.len() + SEGMENT - 1) / SEGMENT;
    if count > MAX_SEGMENTS {
        return Err("message trop volumineux pour NetherNet");
    }
    Ok(message
        .chunks(SEGMENT)
        .enumerate()
        .map(|(i, chunk)| {
            let mut frame = Vec::with_capacity(chunk.len() + 1);
            frame.push((count - 1 - i) as u8);
            frame.extend_from_slice(chunk);
            frame
        })
        .collect())
}

/// Reconstitue les messages reçus, avec une taille maximale imposée.
pub struct Reassembler {
    buf: Vec<u8>,
    /// Valeur attendue dans l'en-tête du prochain fragment, si un message est en cours.
    expected: Option<u8>,
    pub limit: usize,
}

impl Reassembler {
    pub fn new(limit: usize) -> Reassembler {
        Reassembler { buf: Vec::new(), expected: None, limit }
    }

    /// Octets déjà reçus d'un message encore incomplet.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// Ajoute une trame reçue. Renvoie le message s'il est complet.
    pub fn push(&mut self, frame: &[u8]) -> Result<Option<Vec<u8>>, &'static str> {
        let (&remaining, payload) = frame.split_first().ok_or("trame vide")?;
        if let Some(expected) = self.expected {
            if remaining != expected {
                return Err("fragments dans le désordre");
            }
        }
        if self.buf.len() + payload.len() > self.limit {
            return Err("message trop volumineux");
        }
        self.buf.extend_from_slice(payload);
        if remaining == 0 {
            self.expected = None;
            if self.buf.is_empty() {
                return Err("message vide");
            }
            return Ok(Some(std::mem::take(&mut self.buf)));
        }
        self.expected = Some(remaining - 1);
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_message_is_one_frame() {
        let frames = split(&[1, 2, 3]).unwrap();
        assert_eq!(frames, vec![vec![0, 1, 2, 3]]);
        let mut r = Reassembler::new(100);
        assert_eq!(r.push(&frames[0]).unwrap(), Some(vec![1, 2, 3]));
    }

    #[test]
    fn large_message_roundtrip() {
        let message: Vec<u8> = (0..25_001u32).map(|i| (i % 251) as u8).collect();
        let frames = split(&message).unwrap();
        assert_eq!(frames.iter().map(|f| f[0]).collect::<Vec<_>>(), vec![2, 1, 0]);
        let mut r = Reassembler::new(1 << 20);
        assert_eq!(r.push(&frames[0]).unwrap(), None);
        assert_eq!(r.push(&frames[1]).unwrap(), None);
        assert_eq!(r.push(&frames[2]).unwrap(), Some(message));
        // Le réassembleur est prêt pour le message suivant
        assert_eq!(r.push(&[0, 9]).unwrap(), Some(vec![9]));
    }

    #[test]
    fn rejects_abuse() {
        assert!(split(&[]).is_err());
        assert!(split(&vec![0u8; SEGMENT * MAX_SEGMENTS + 1]).is_err());
        let mut r = Reassembler::new(10);
        assert!(r.push(&[]).is_err());
        assert!(Reassembler::new(10).push(&[0]).is_err());
        assert!(Reassembler::new(10).push(&[0; 12]).is_err(), "limite de taille");
        let mut r = Reassembler::new(100);
        assert_eq!(r.push(&[3, 1]).unwrap(), None);
        assert!(r.push(&[1, 1]).is_err(), "fragment sauté");
    }
}
