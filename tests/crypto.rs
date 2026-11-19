//! Compare src/crypto.rs à OpenSSL : condensés et signatures de référence
//! produits par tests/auth/generate_vectors.py.

use basalt::crypto::{sha256, sha384, EcKey, RsaKey};

fn hex(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

#[test]
fn matches_openssl() {
    let text = include_str!("auth/crypto_vectors.txt");
    let (mut hashes, mut ec, mut rsa) = (0, 0, 0);
    for (n, line) in text.lines().enumerate() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "sha256" => {
                assert_eq!(sha256(&hex(f[1])).to_vec(), hex(f[2]), "ligne {}", n + 1);
                hashes += 1;
            }
            "sha384" => {
                assert_eq!(sha384(&hex(f[1])).to_vec(), hex(f[2]), "ligne {}", n + 1);
                hashes += 1;
            }
            "ec" => {
                let key = EcKey::from_spki(&hex(f[1])).unwrap();
                assert_eq!(key.verify_sha384(&hex(f[2]), &hex(f[3])), f[4] == "1", "ligne {}", n + 1);
                ec += 1;
            }
            "rsa" => {
                let key = RsaKey::new(&hex(f[1]), &hex(f[2])).unwrap();
                assert_eq!(key.verify_sha256(&hex(f[3]), &hex(f[4])), f[5] == "1", "ligne {}", n + 1);
                rsa += 1;
            }
            other => panic!("type de vecteur inconnu : {}", other),
        }
    }
    assert!(hashes >= 32 && ec >= 24 && rsa >= 8, "{} {} {}", hashes, ec, rsa);
}

#[test]
fn rejects_malformed_keys() {
    // Point hors de la courbe
    let mut spki = hex("3076301006072a8648ce3d020106052b8104002203620004");
    spki.extend_from_slice(&[1u8; 96]);
    assert!(EcKey::from_spki(&spki).is_err());
    assert!(EcKey::from_spki(&spki[..100]).is_err());
    // Clés RSA trop petites, paires, ou d'exposant invalide
    assert!(RsaKey::new(&[0xffu8; 128], &[1, 0, 1]).is_err());
    let mut even = vec![0xffu8; 256];
    even[255] = 0xfe;
    assert!(RsaKey::new(&even, &[1, 0, 1]).is_err());
    assert!(RsaKey::new(&[0xffu8; 256], &[1]).is_err());
    assert!(RsaKey::new(&[0xffu8; 256], &[2]).is_err());
}
