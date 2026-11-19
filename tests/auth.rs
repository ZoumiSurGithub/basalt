//! Vérification des jetons Xbox : un jeton valide est accepté, toutes les
//! falsifications classiques sont refusées. Jetons produits par
//! tests/auth/generate_fixtures.py.

use std::collections::HashMap;
use std::path::Path;

use basalt::auth::{verify, AuthError, KeySet};

const NOW: u64 = 1_800_000_000; // 2027

fn fixtures() -> HashMap<&'static str, &'static str> {
    include_str!("auth/fixtures.txt")
        .lines()
        .filter_map(|l| {
            let i = l.find(' ')?;
            Some((&l[..i], &l[i + 1..]))
        })
        .collect()
}

fn keys() -> KeySet {
    KeySet::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/auth/keys.json")).unwrap()
}

#[test]
fn valid_token_is_accepted() {
    let f = fixtures();
    let id = verify(Some(f["token_valid"]), f["client_valid"], &keys(), NOW).unwrap();
    assert_eq!((id.name.as_str(), id.xuid.as_str()), ("Alex", "2535412345678901"));
    assert_eq!(id.cpk.len(), 160, "clé publique du joueur en base64");
}

#[test]
fn forged_tokens_are_rejected() {
    let f = fixtures();
    let k = keys();
    let invalid = |token: &str, client: &str| match verify(Some(f[token]), f[client], &k, NOW) {
        Err(AuthError::Invalid(_)) => {}
        other => panic!("{} / {} : refus attendu, obtenu {:?}", token, client, other),
    };
    for token in [
        "token_expired",
        "token_not_yet_valid",
        "token_wrong_audience",
        "token_wrong_issuer",
        "token_other_signer",
        "token_tampered",
        "token_alg_none",
        "token_alg_hs256",
        "token_no_xuid",
        "token_bad_cpk",
    ] {
        invalid(token, "client_valid");
    }
    // Jeton authentique rejoué avec des données signées par une autre clé
    invalid("token_valid", "client_other_key");
    invalid("token_valid", "client_alg_none");
    // Un jeton auto-signé (ES384) n'est pas un jeton Xbox
    invalid("self_signed_chain", "client_valid");
}

#[test]
fn missing_token_and_unknown_key() {
    let f = fixtures();
    let k = keys();
    assert_eq!(verify(None, f["client_valid"], &k, NOW), Err(AuthError::NotSignedIn));
    assert_eq!(verify(Some(""), f["client_valid"], &k, NOW), Err(AuthError::NotSignedIn));
    assert_eq!(verify(Some(f["token_unknown_kid"]), f["client_valid"], &k, NOW), Err(AuthError::UnknownKey));
    assert!(matches!(verify(Some("pas.un.jeton"), f["client_valid"], &k, NOW), Err(AuthError::Invalid(_))));
    assert!(matches!(verify(Some("a.b"), f["client_valid"], &k, NOW), Err(AuthError::Invalid(_))));
}

#[test]
fn expiry_uses_the_clock() {
    let f = fixtures();
    let k = keys();
    // Le jeton « expiré » était valide avant son échéance
    assert!(verify(Some(f["token_expired"]), f["client_valid"], &k, 1_700_000_050).is_ok());
    assert!(verify(Some(f["token_expired"]), f["client_valid"], &k, 1_700_000_100 + 61).is_err());
}
