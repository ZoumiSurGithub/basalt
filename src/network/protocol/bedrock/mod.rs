//! Couche « jeu » de Bedrock : paquets échangés avec le jeu, rangés par thème.
//!
//! Chaque sous-module est ré-exporté ici : le reste du serveur écrit `bedrock::play_status(…)`
//! sans se soucier du fichier où le paquet est défini.

mod ids;
mod codec;
mod login;
mod player;
mod spawn;
mod chunk;
mod debug;
mod state;
mod chat;
mod blocks;
mod items;
mod combat;
mod drops;
mod mobs;

pub use ids::*;
pub use codec::*;
pub use login::*;
pub use player::*;
pub use spawn::*;
pub use chunk::*;
pub use debug::*;
pub use state::*;
pub use chat::*;
pub use blocks::*;
pub use items::*;
pub use combat::*;
pub use drops::*;
pub use mobs::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for v in [0u32, 1, 127, 128, 300, 65535, u32::MAX] {
            let mut b = Vec::new();
            write_varu32(&mut b, v);
            let mut pos = 0;
            assert_eq!(read_varu32(&b, &mut pos), Some(v));
            assert_eq!(pos, b.len());
        }
    }

    #[test]
    fn input_is_validated() {
        let mut p = Vec::new();
        for v in [10.0f32, 450.0, 12.5, -58.38, -3.0, 0.0, 0.0, 90.0] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        let i = read_input(&p).unwrap();
        assert_eq!((i.pos, i.pitch, i.yaw, i.head_yaw), ((12.5, -58.38, -3.0), 10.0, 90.0, 90.0));
        assert_eq!(read_input(&p[..19]), None);
        p[8..12].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(read_input(&p), None);
        p[8..12].copy_from_slice(&1.0e9f32.to_le_bytes());
        assert_eq!(read_input(&p), None);
    }

    #[test]
    fn a_framed_packet_gives_the_same_batch() {
        let body = remove_actor(-3);
        for id in [ID_REMOVE_ACTOR, 300, 20_000] {
            let usual = encode_batch(&[(ID_MOVE_PLAYER, vec![1, 2]), (id, body.clone())], false);
            let framed = encode_batch_with(&[(ID_MOVE_PLAYER, vec![1, 2])], &frame_packet(id, &body), false);
            assert_eq!(usual, framed);
        }
    }

    #[test]
    fn batch_of_several_packets() {
        let b = encode_batch(&[(ID_MOVE_PLAYER, vec![1, 2]), (ID_REMOVE_ACTOR, remove_actor(-3))], true);
        let pk = decode_batch(&b[1..], true, 1024).unwrap();
        assert_eq!(pk, vec![(ID_MOVE_PLAYER, vec![1, 2]), (ID_REMOVE_ACTOR, vec![5])]);
    }

    #[test]
    fn zigzag_roundtrip() {
        for v in [0i32, -1, 1, -60, 4, i32::MIN, i32::MAX] {
            let mut w = W::default();
            w.vari32(v);
            let mut pos = 0;
            assert_eq!(read_vari32(&w.0, &mut pos), Some(v));
        }
    }

    #[test]
    fn block_hashes_differ() {
        assert_ne!(block_hash("minecraft:air"), block_hash("minecraft:stone"));
    }

    #[test]
    fn batch_roundtrip() {
        let p = encode_packet(ID_PLAY_STATUS, &play_status(3), true);
        let pk = decode_batch(&p[1..], true, 1024).unwrap();
        assert_eq!(pk, vec![(ID_PLAY_STATUS, vec![0, 0, 0, 3])]);
    }

    #[test]
    fn truncated_inputs_are_errors() {
        assert!(decode_batch(&[0x05, 0x01], false, 1024).is_err());
        assert!(decode_batch(&[0xff, 3, 2, 0, 0], true, 3).is_err());
        assert!(parse_login(&[0, 0, 0, 1, 0xff]).is_err());
        assert!(parse_login(&[0, 0, 0, 1, 4, 0xff, 0xff, 0xff, 0xff]).is_err());
    }
}
