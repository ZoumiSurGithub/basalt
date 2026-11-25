//! Outils communs : journal, configuration, JSON, décompression.

#[macro_use]
pub mod log;
pub mod config;
pub mod inflate;
pub mod image;
pub mod deflate;
pub mod json;
pub mod yaml;
pub mod console;
pub mod timings;
pub mod background;
pub mod pack_crypt;
pub mod packs;

/// Passe le fil courant en priorité basse (Linux : « nice 10 »). Pour les fils de fond
/// (terrain, chunks, disque) : quand la machine est chargée, la boucle du jeu et le réseau
/// passent avant eux. Sans effet ailleurs.
pub fn lower_thread_priority() {
    #[cfg(target_os = "linux")]
    {
        extern "C" {
            fn setpriority(which: i32, who: u32, prio: i32) -> i32;
        }
        // Sous Linux, PRIO_PROCESS (0) avec who = 0 ne touche que le fil appelant
        unsafe {
            setpriority(0, 0, 10);
        }
    }
}
