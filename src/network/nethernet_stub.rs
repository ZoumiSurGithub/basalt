//! Remplaçant du transport NetherNet quand Basalt est compilé sans la
//! fonctionnalité `nethernet` (donc sans bibliothèque WebRTC).

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::raknet::RakEvent;

pub struct NetherNet {
    pub max_peers: usize,
    pub debug: bool,
}

impl NetherNet {
    pub const AVAILABLE: bool = false;

    pub fn bind(_ip: &str, _port: u16, _public_ip: &str, _domain: &str, _cpk_as_jwk: bool) -> Result<NetherNet, String> {
        Err("Basalt a été compilé sans le transport NetherNet".into())
    }
    pub fn key_fingerprint(&self) -> &str {
        ""
    }
    pub fn len(&self) -> usize {
        0
    }
    pub fn is_empty(&self) -> bool {
        true
    }
    pub fn accept(&mut self, _key: SocketAddr, _local_ip: IpAddr, _offer: &str, _cpk: Option<String>, _now: u64) -> Result<String, (u16, &'static str)> {
        Err((503, "NetherNet transport not available"))
    }
    pub fn poll(&mut self, _events: &mut Vec<RakEvent>) -> usize {
        0
    }
    pub fn flush(&mut self, _events: &mut Vec<RakEvent>) {}
    pub fn has(&self, _key: &SocketAddr) -> bool {
        false
    }
    pub fn bound_cpk(&self, _key: &SocketAddr) -> Option<String> {
        None
    }
    pub fn send(&mut self, _key: &SocketAddr, _message: &[u8]) {}
    pub fn close(&mut self, _key: &SocketAddr, _delay: Duration) {}
    pub fn abort(&mut self, _key: &SocketAddr, _reason: &'static str) {}
    pub fn set_game_limits(&mut self, _key: &SocketAddr) {}
    pub fn shutdown(&mut self) {}
}
