//! Basalt — serveur Minecraft Bedrock.
//!
//! Le code est rangé comme celui de PocketMine et d'Altay :
//!
//!   server.rs    `Server` : la boucle principale, qui relie tout le reste
//!   player/      `Player` : un joueur (hérite de Human → Living → Entity)
//!   entity/      `Entity`, `Living`, `Human` : la hiérarchie des entités
//!   inventory/   `Inventory` (base), `BaseInventory`, puis `PlayerInventory`, `ArmorInventory`,
//!                `ChestInventory`, `FurnaceInventory`… et les transactions
//!   block/       `Block` (base), puis les blocs vanilla et `VanillaBlocks`
//!   item/        `Item` (base), puis `Tool`, `Armor`, `Food`… et `VanillaItems` ; `ItemStack` (une pile)
//!   level/       `Level` : le monde, ses blocs et sa sauvegarde
//!   tile/        `Tile` (base), puis `ContainerTile`, `FurnaceTile`, `SignTile` : les données
//!                attachées aux blocs (contenu d'un coffre, état d'un four…)
//!   network/     transports RakNet et NetherNet, signalisation, et `protocol/` (paquets Bedrock)
//!   nbt/         format NBT : lecture, écriture, fichiers `.nbt` (sauvegardes)
//!   permission/  `Permission`, règles par défaut, opérateurs
//!   plugin/      plugins Lua : fiche `plugin.yml`, chargement, bac à sable, fil dédié
//!   data/        données du jeu : objets, blocs et leurs états (dossier `data/`)
//!   security/    authentification Xbox, cryptographie, clé du serveur
//!   utils/       journal, configuration, JSON, YAML, décompression
//!   tools/       outils en ligne de commande (capture des données du jeu)
//!
//! Les modules les plus utilisés sont aussi accessibles par un nom court (`basalt::bedrock`,
//! `basalt::raknet`…), ce qui garde les chemins simples dans tout le code.

// `utils` en premier : il définit les macros de journal utilisées partout.
#[macro_use]
pub mod utils;
pub mod block;
pub mod command;
pub mod data;
pub mod entity;
pub mod inventory;
pub mod item;
pub mod level;
pub mod nbt;
pub mod network;
pub mod permission;
pub mod player;
pub mod plugin;
pub mod security;
pub mod server;
pub mod tile;
pub mod tools;

pub use level as world;
pub use network::protocol::{self, bedrock, versions};
pub use network::{framing, nethernet, raknet, signaling};
pub use plugin::lua;
pub use security::{auth, crypto};
#[cfg(feature = "nethernet")]
pub use security::identity;
pub use utils::{config, console, inflate, json, log, timings, yaml};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Compilation : branche ou étiquette, commit et date, inscrits par la publication GitHub.
pub const BUILD: Option<&str> = option_env!("BASALT_BUILD");
