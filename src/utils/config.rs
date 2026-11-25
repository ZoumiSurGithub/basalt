//! Lecture de `server.properties` (créé avec les valeurs par défaut s'il est absent).

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Config {
    pub server_name: String,
    pub sub_motd: String,
    pub ip: String,
    pub port: u16,
    pub max_players: u32,
    pub game_version: String,
    pub protocol: i32,
    pub min_protocol: i32,
    pub max_protocol: i32,
    pub gamemode: String,
    pub world_name: String,
    pub view_distance: i32,
    /// Distance de simulation (en chunks) : au-delà, les créatures restent figées (`tick-distance`)
    pub tick_distance: i32,
    pub debug: bool,
    /// Mise au point : renvoie le terrain sous d'autres formes aux joueurs entrés (`debug-probes`)
    pub probes: bool,
    pub max_per_ip: usize,
    pub xbox_auth: bool,
    /// Joueurs simulés acceptés sans compte Xbox, depuis la machine du serveur seulement
    pub local_bots: bool,
    /// Liste blanche : seuls les joueurs de `white-list.txt` (et les opérateurs) peuvent entrer
    pub white_list: bool,
    /// Inventaire gardé à la mort (règle « keepInventory » du jeu). Non : il tombe au sol.
    pub keep_inventory: bool,
    /// Compresser ce que le serveur envoie (chunks, gros paquets)
    pub network_compression: bool,
    /// Terrain des nouveaux mondes : « vanilla » (relief, biomes, grottes), « flat » (plat) ou
    /// « plugin:<nom> »
    pub level_type: String,
    /// Graine des nouveaux mondes (vide : au hasard)
    pub level_seed: String,
    /// Dossier du serveur : celui de `server.properties`, où vivent `ops.txt` et `white-list.txt`
    pub root: std::path::PathBuf,
    /// Fichier de configuration, quand elle en vient (pour y enregistrer un changement)
    pub file: Option<std::path::PathBuf>,
    pub http_signaling: bool,
    pub nethernet_udp_port: u16,
    /// Fils entre lesquels les liaisons NetherNet sont réparties (1 à 16)
    pub nethernet_threads: usize,
    pub public_ip: String,
    pub identity_domain: String,
    pub identity_cpk_jwk: bool,
    pub movement_check: bool,
    pub show_players: bool,
    /// Animaux (vaches, cochons, moutons, poules) autour des joueurs
    pub spawn_animals: bool,
    /// Des monstres apparaissent la nuit autour des joueurs en survie
    pub spawn_monsters: bool,
    /// Port de l'API de gestion à distance (0 : éteinte)
    pub api_port: u16,
    /// Adresse sur laquelle l'API écoute (127.0.0.1 : la machine elle-même seulement)
    pub api_bind: String,
    /// Tests : ouvre l'API sur un port choisi par le système, écrit dans api-port.txt
    pub api_test: bool,
    /// Les explosions des monstres cassent les blocs
    pub mob_griefing: bool,
    /// Le feu brûle les blocs et gagne du terrain (`fire-spreads`)
    pub fire_spreads: bool,
    pub player_view: i32,
    pub max_visible: usize,
    pub auth_keys_file: String,
    pub plugins_dir: String,
    /// Dossier où le monde est enregistré
    pub world_dir: String,
    /// Bac à sable des plugins
    pub plugin_sandbox: bool,
    /// Annoncer les commandes des plugins au jeu (« / » avec autocomplétion)
    pub commands_in_game: bool,
    /// Plugins dispensés du bac à sable (noms séparés par des virgules)
    pub plugin_trusted: String,
    /// Mémoire maximale d'un plugin protégé, en Mo
    pub plugin_memory_mb: usize,
    pub timeout_secs: u64,
    /// Transport de test (TCP, cette machine seulement) : ouvert par les tests de bout en
    /// bout, jamais par server.properties
    pub test_transport: bool,
}

impl Default for Config {
    /// Valeurs par défaut, celles du fichier server.properties créé au premier démarrage.
    fn default() -> Self {
        Config {
            server_name: "Serveur Basalt".into(),
            sub_motd: "Basalt".into(),
            ip: "0.0.0.0".into(),
            port: 19132,
            max_players: 20,
            game_version: "1.26.65".into(),
            protocol: 2225,
            min_protocol: crate::versions::MIN_PROTOCOL,
            max_protocol: crate::versions::MAX_PROTOCOL,
            gamemode: "Survival".into(),
            world_name: "Monde".into(),
            view_distance: 6,
            tick_distance: 4,
            debug: false,
            probes: false,
            max_per_ip: 5,
            xbox_auth: true,
            local_bots: false,
            white_list: false,
            keep_inventory: false,
            network_compression: true,
            level_type: "vanilla".into(),
            level_seed: String::new(),
            root: std::path::PathBuf::from("."),
            file: None,
            http_signaling: true,
            nethernet_udp_port: 19140,
            nethernet_threads: 1,
            public_ip: String::new(),
            identity_domain: "basalt.local".into(),
            identity_cpk_jwk: true,
            movement_check: true,
            show_players: true,
            spawn_animals: true,
            spawn_monsters: true,
            api_port: 0,
            api_bind: "127.0.0.1".into(),
            api_test: false,
            mob_griefing: true,
            fire_spreads: true,
            player_view: 4,
            max_visible: usize::MAX,
            auth_keys_file: String::new(),
            plugins_dir: "plugins".into(),
            world_dir: "world".into(),
            plugin_sandbox: true,
            commands_in_game: true,
            plugin_trusted: String::new(),
            plugin_memory_mb: 64,
            timeout_secs: 10,
            test_transport: false,
        }
    }
}

const TEMPLATE: &str = "\
# Configuration de Basalt
# Nom affiché dans la liste des serveurs
server-name=Serveur Basalt
sub-motd=Basalt
# Adresse et port d'écoute. Le jeu trouve le serveur en HTTP sur ce port, en TCP
# (signalisation du transport NetherNet). À ouvrir dans le pare-feu.
server-ip=0.0.0.0
server-port=19132
# false : ferme la signalisation (plus aucun joueur ne peut entrer)
nethernet-signaling=true
# Port UDP de la liaison WebRTC (transport NetherNet). À ouvrir dans le pare-feu.
nethernet-udp-port=19140
# Fils entre lesquels les liaisons des joueurs sont réparties (chiffrement, lecture,
# envoi), de 1 à 16. Un seul suffit jusqu'à quelques centaines de joueurs ; au-delà,
# comptez un fil pour 400 joueurs environ, sans dépasser le nombre de cœurs. Le port
# reste le même.
nethernet-threads=1
# Adresse IP publique du serveur, à renseigner s'il est derrière une box ou un NAT
# (laisser vide sinon : l'adresse sur laquelle le joueur s'est connecté est utilisée)
public-ip=
max-players=20
# Version annoncée dans la liste des serveurs (la plus courante chez vos joueurs)
game-version=1.26.65
protocol=2225
# Versions acceptées à la connexion, par numéro de protocole :
#   2225 = 1.26.60 à 1.26.66    2259 = préversion 1.26.70
# (les versions d'avant la 1.26.60 ne sont pas prises en charge)
min-protocol=2225
max-protocol=2259
# Mode de jeu des nouveaux joueurs : Survival, Creative, Adventure ou Spectator
gamemode=Survival
# true : un joueur qui meurt garde son inventaire. false (comme dans le jeu) : son
# inventaire et son armure tombent au sol là où il est mort.
keep-inventory=false
# Compression de ce que le serveur envoie (chunks environ 8 fois plus petits, connexion
# 7 fois plus légère). false : moins de calcul, beaucoup plus de bande passante (réseau local).
network-compression=true
# Terrain d'un nouveau monde : vanilla (comme le jeu : biomes, montagnes, grottes), flat (plat),
# ou plugin:<nom> pour le générateur d'un plugin (plugins/<plugin>/generators/<nom>.lua).
# Un monde déjà créé garde son terrain : pour en changer, utilisez un autre world-dir.
level-type=vanilla
# Graine d'un nouveau monde : un nombre ou un texte. Vide : au hasard.
level-seed=
world-name=Monde
# Rayon (en chunks) du terrain envoyé autour du point d'apparition
view-distance=6
# true : seuls les joueurs connectés à leur compte Xbox sont acceptés, et leur
# pseudo est vérifié. false : le pseudo annoncé est accepté sans contrôle.
xbox-auth=true
# true : des joueurs simulés (outil basalt-bots) peuvent entrer sans compte Xbox, mais
# seulement depuis cette machine (127.0.0.1) et avec un pseudo qui commence par « Bot ».
local-bots=false
# true : seuls les joueurs inscrits dans white-list.txt (et les opérateurs) peuvent entrer.
# Pratique pendant la mise au point d'un serveur. Console : whitelist on|off|add|remove|list
white-list=false
# Laisser vide : les clés de vérification sont téléchargées automatiquement (curl).
# Sinon, chemin d'un fichier de clés fourni à la main.
auth-keys-file=
# true : le serveur refuse les déplacements impossibles (vitesse, téléportation)
# et replace le joueur à sa dernière position valide
movement-check=true
# true : les joueurs se voient entre eux
show-players=true
# Animaux (vaches, cochons, moutons, poules, lapins) autour des joueurs
spawn-animals=true
# Monstres la nuit autour des joueurs en survie (false : monde paisible)
spawn-monsters=true
# API de gestion à distance (voir docs/API.md). 0 : éteinte.
# Les jetons d'accès sont créés au premier démarrage : api-token.txt (lecture)
# et api-admin-token.txt (actions).
api-port=0
# 127.0.0.1 : accessible depuis cette machine seulement. Les échanges ne sont pas
# chiffrés : pour un accès distant, passez par un tunnel SSH ou un serveur HTTPS.
api-bind=127.0.0.1
# Les explosions (creeper…) cassent les blocs (false : elles blessent sans rien détruire)
mob-griefing=true
# Distance de simulation (en chunks, de 4 à 12, comme le serveur officiel) : au-delà,
# les animaux restent figés jusqu'au retour d'un joueur
tick-distance=4
# Distance (en chunks) jusqu'à laquelle un joueur voit les autres
player-view-distance=4
# Nombre maximal d'autres joueurs affichés à chaque joueur.
# 0 : pas de plafond, chacun voit tous les joueurs à portée (player-view-distance).
# Dans une très grosse foule, le trafic croît avec le carré du nombre de joueurs réunis :
# un plafond (48 par exemple, 500 au plus) le réduit
max-visible-players=0
# Connexions simultanées autorisées depuis une même adresse IP
max-connections-per-ip=5
# true : journalise chaque paquet reçu et envoyé (pour le diagnostic)
debug-packets=false
plugins-dir=plugins
# true : chaque plugin est enfermé dans son dossier (plugins/<nom>/), sans accès au
# système, avec un temps d'exécution et une mémoire plafonnés
plugin-sandbox=true
# Annonce les commandes des plugins au jeu (/commande avec autocomplétion).
# Si le jeu se déconnecte à l'entrée, mettez false le temps du diagnostic.
commands-in-game=true
# Plugins de confiance, dispensés de cette protection (noms séparés par des virgules)
plugin-trusted=
# Mémoire maximale d'un plugin protégé, en Mo
plugin-memory-mb=64
# Secondes sans nouvelles avant de considérer un client comme déconnecté
timeout=10
";

impl Config {
    /// Lit server.properties ; s'il n'existe pas, le crée avec les valeurs par défaut.
    pub fn load(path: &Path) -> Config {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => {
                if let Err(e) = std::fs::write(path, TEMPLATE) {
                    warn!("Impossible de créer {} : {}", path.display(), e);
                } else {
                    info!("{} créé avec les valeurs par défaut", path.display());
                }
                TEMPLATE.to_string()
            }
        };
        let mut c = Config::parse(&text);
        // Fichier d'une version précédente : la nouvelle option y est ajoutée, avec son explication
        if !text.is_empty() && !text.lines().any(|l| l.trim_start().starts_with("level-type")) {
            let mut more = text.clone();
            if !more.ends_with('\n') {
                more.push('\n');
            }
            more.push_str("# Terrain d'un nouveau monde : vanilla (comme le jeu : biomes, montagnes, grottes), flat (plat),\n# ou plugin:<nom> pour le générateur d'un plugin (plugins/<plugin>/generators/<nom>.lua).\n# Un monde déjà créé garde son terrain : pour en changer, utilisez un autre world-dir.\nlevel-type=vanilla\n# Graine d'un nouveau monde : un nombre ou un texte. Vide : au hasard.\nlevel-seed=\n");
            let _ = std::fs::write(path, &more);
        }
        let text = std::fs::read_to_string(path).unwrap_or(text);
        if !text.is_empty() && !text.lines().any(|l| l.trim_start().starts_with("keep-inventory")) {
            let mut more = text.clone();
            if !more.ends_with('\n') {
                more.push('\n');
            }
            more.push_str("# true : un joueur qui meurt garde son inventaire. false (comme dans le jeu) : son\n# inventaire et son armure tombent au sol là où il est mort.\nkeep-inventory=false\n");
            let _ = std::fs::write(path, more);
        }
        c.root = path.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from("."));
        c.file = Some(path.to_path_buf());
        c
    }

    /// Enregistre un réglage dans le fichier de configuration : la ligne `clé=` est
    /// remplacée, ou ajoutée si elle n'existe pas. Le reste du fichier est conservé.
    pub fn store(&self, key: &str, value: &str) -> std::io::Result<()> {
        let path = match &self.file {
            Some(p) => p,
            None => return Ok(()), // configuration construite en mémoire (tests)
        };
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut found = false;
        let mut lines: Vec<String> = text
            .lines()
            .map(|line| {
                let is_key = line.split_once('=').map(|(k, _)| k.trim() == key).unwrap_or(false);
                if is_key && !line.trim_start().starts_with('#') {
                    found = true;
                    format!("{}={}", key, value)
                } else {
                    line.to_string()
                }
            })
            .collect();
        if !found {
            lines.push(format!("{}={}", key, value));
        }
        std::fs::write(path, lines.join("\n") + "\n")
    }

    /// Lit le texte de server.properties (« clé=valeur » par ligne).
    pub fn parse(text: &str) -> Config {
        let mut map = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(i) = line.find('=') {
                map.insert(line[..i].trim().to_string(), line[i + 1..].trim().to_string());
            }
        }
        let mut c = Config::default();
        let text_of = |k: &str, d: &str| map.get(k).cloned().unwrap_or_else(|| d.to_string());
        c.server_name = text_of("server-name", &c.server_name);
        c.sub_motd = text_of("sub-motd", &c.sub_motd);
        c.ip = text_of("server-ip", &c.ip);
        c.game_version = text_of("game-version", &c.game_version);
        c.gamemode = text_of("gamemode", &c.gamemode);
        c.world_name = text_of("world-name", &c.world_name);
        c.movement_check = map.get("movement-check").map(|v| v != "false").unwrap_or(true);
        c.show_players = map.get("show-players").map(|v| v != "false").unwrap_or(true);
        c.spawn_animals = map.get("spawn-animals").map(|v| v != "false").unwrap_or(true);
        c.api_port = map.get("api-port").and_then(|v| v.parse().ok()).unwrap_or(0);
        if let Some(v) = map.get("api-bind").filter(|v| !v.is_empty()) {
            c.api_bind = v.clone();
        }
        c.spawn_monsters = map.get("spawn-monsters").map(|v| v != "false").unwrap_or(true);
        c.mob_griefing = map.get("mob-griefing").map(|v| v != "false").unwrap_or(true);
        c.fire_spreads = map.get("fire-spreads").map(|v| v != "false").unwrap_or(true);
        c.http_signaling = map.get("nethernet-signaling").map(|v| v != "false").unwrap_or(true);
        c.xbox_auth = map.get("xbox-auth").map(|v| v != "false").unwrap_or(true);
        c.local_bots = map.get("local-bots").map(|v| v == "true" || v == "on").unwrap_or(false);
        c.white_list = map.get("white-list").map(|v| v == "true" || v == "on").unwrap_or(false);
        c.keep_inventory = map.get("keep-inventory").map(|v| v == "true" || v == "on").unwrap_or(false);
        c.network_compression = map.get("network-compression").map(|v| v != "false" && v != "off").unwrap_or(true);
        c.level_type = text_of("level-type", "vanilla");
        c.level_seed = map.get("level-seed").cloned().unwrap_or_default();
        c.auth_keys_file = text_of("auth-keys-file", "");
        c.debug = map.get("debug-packets").map(|v| v == "true").unwrap_or(false);
        c.probes = map.get("debug-probes").map(|v| v == "true").unwrap_or(false);
        c.plugins_dir = text_of("plugins-dir", &c.plugins_dir);
        c.world_dir = text_of("world-dir", &c.world_dir);
        c.commands_in_game = map.get("commands-in-game").map(|v| v != "false").unwrap_or(true);
        c.plugin_sandbox = map.get("plugin-sandbox").map(|v| v != "false").unwrap_or(true);
        c.plugin_trusted = text_of("plugin-trusted", "");
        c.plugin_memory_mb = map.get("plugin-memory-mb").and_then(|v| v.parse().ok()).unwrap_or(64);
        macro_rules! num {
            ($key:literal, $field:ident) => {
                if let Some(v) = map.get($key) {
                    match v.parse() {
                        Ok(n) => c.$field = n,
                        Err(_) => warn!("server.properties : valeur invalide pour {}", $key),
                    }
                }
            };
        }
        num!("server-port", port);
        num!("nethernet-udp-port", nethernet_udp_port);
        num!("nethernet-threads", nethernet_threads);
        c.nethernet_threads = c.nethernet_threads.clamp(1, 16);
        c.public_ip = text_of("public-ip", "");
        c.identity_domain = text_of("identity-domain", &c.identity_domain);
        c.identity_cpk_jwk = map.get("identity-cpk-format").map(|v| v != "spki").unwrap_or(true);
        num!("max-players", max_players);
        num!("protocol", protocol);
        num!("min-protocol", min_protocol);
        num!("max-protocol", max_protocol);
        if c.min_protocol < crate::versions::MIN_PROTOCOL {
            warn!(
                "server.properties : min-protocol={} n'est plus pris en charge, {} ({}) est utilisé",
                c.min_protocol,
                crate::versions::MIN_PROTOCOL,
                crate::versions::name(crate::versions::MIN_PROTOCOL)
            );
            c.min_protocol = crate::versions::MIN_PROTOCOL;
        }
        if c.min_protocol > c.max_protocol {
            warn!("server.properties : min-protocol est supérieur à max-protocol");
            c.max_protocol = c.min_protocol;
        }
        num!("timeout", timeout_secs);
        num!("view-distance", view_distance);
        num!("tick-distance", tick_distance);
        num!("max-connections-per-ip", max_per_ip);
        num!("player-view-distance", player_view);
        num!("max-visible-players", max_visible);
        c.player_view = c.player_view.clamp(1, 16);
        // 0 : pas de plafond, chacun voit tous les joueurs à portée de vue
        c.max_visible = if c.max_visible == 0 { usize::MAX } else { c.max_visible.min(500) };
        c.max_per_ip = c.max_per_ip.max(1);
        c.view_distance = c.view_distance.clamp(2, 16);
        c.tick_distance = c.tick_distance.clamp(4, 12);
        if c.timeout_secs == 0 {
            c.timeout_secs = 10;
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_inventory_is_off_unless_asked() {
        assert!(!Config::parse("").keep_inventory);
        assert!(!Config::parse("keep-inventory=false").keep_inventory);
        assert!(Config::parse("keep-inventory=true").keep_inventory);
        assert!(TEMPLATE.contains("keep-inventory=false"));
    }
}
