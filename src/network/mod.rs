//! Réseau : NetherNet (le transport des joueurs), sa signalisation HTTP, et le transport de test.

pub mod framing;
#[cfg(feature = "nethernet")]
pub mod nethernet;
#[cfg(feature = "nethernet")]
pub mod nethernet_client;
pub mod nethernet_thread;
#[cfg(not(feature = "nethernet"))]
#[path = "nethernet_stub.rs"]
pub mod nethernet;
pub mod transport;
pub mod signaling;
pub mod sockbuf;
pub mod protocol;
