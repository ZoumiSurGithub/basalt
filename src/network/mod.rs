//! Réseau : les deux transports (RakNet et NetherNet) et la signalisation HTTP.

pub mod framing;
#[cfg(feature = "nethernet")]
pub mod nethernet;
pub mod nethernet_thread;
#[cfg(not(feature = "nethernet"))]
#[path = "nethernet_stub.rs"]
pub mod nethernet;
pub mod raknet;
pub mod signaling;
pub mod protocol;
