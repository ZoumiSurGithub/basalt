//! Identifiants des paquets et codes d'état.

pub const ID_LOGIN: u32 = 0x01;
pub const ID_PLAY_STATUS: u32 = 0x02;
pub const ID_DISCONNECT: u32 = 0x05;
pub const ID_RESOURCE_PACKS_INFO: u32 = 6;
pub const ID_RESOURCE_PACK_STACK: u32 = 7;
pub const ID_RESOURCE_PACK_RESPONSE: u32 = 8;
pub const ID_START_GAME: u32 = 11;
pub const ID_ADD_PLAYER: u32 = 12;
pub const ID_REMOVE_ACTOR: u32 = 14;
pub const ID_MOVE_PLAYER: u32 = 19;
pub const ID_LEVEL_CHUNK: u32 = 58;
pub const ID_REQUEST_CHUNK_RADIUS: u32 = 69;
pub const ID_CHUNK_RADIUS_UPDATED: u32 = 70;
pub const ID_SET_LOCAL_PLAYER_INITIALIZED: u32 = 113;
pub const ID_CHUNK_PUBLISHER_UPDATE: u32 = 121;
pub const ID_BIOME_DEFINITION_LIST: u32 = 122;
pub const ID_PLAYER_AUTH_INPUT: u32 = 144;
pub const ID_ITEM_REGISTRY: u32 = 162;
pub const ID_JIGSAW_STRUCTURE_DATA: u32 = 313;
pub const ID_VOXEL_SHAPES: u32 = 337;
pub const ID_NETWORK_SETTINGS: u32 = 0x8f;
pub const ID_REQUEST_NETWORK_SETTINGS: u32 = 0xc1;

pub const STATUS_CLIENT_OUTDATED: i32 = 1;
pub const STATUS_SERVER_OUTDATED: i32 = 2;
pub const STATUS_LOGIN_SUCCESS: i32 = 0;
pub const STATUS_PLAYER_SPAWN: i32 = 3;
