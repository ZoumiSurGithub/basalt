//! Sécurité : authentification Xbox, cryptographie de vérification, clé du serveur.

pub mod auth;
pub mod crypto;
#[cfg(feature = "nethernet")]
pub mod identity;
