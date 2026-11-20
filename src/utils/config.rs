//! Lecture de `server.properties` (créé avec les valeurs par défaut s'il est absent).

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Config {
    pub server_name: String,
    pub sub_motd: String,
    pub ip: String,
    pub port: u16,
    pub port6: u16,
    pub max_players: u32,
    pub game_version: String,
    pub protocol: i32,
    pub min_protocol: i32,
    pub max_protocol: i32,
    pub gamemode: String,
    pub world_name: String,
    pub view_distance: i32,
    pub debug: bool,
    /// Mise au point : renvoie le terrain sous d'autres formes aux joueurs entrés (`debug-probes`)
    pub probes: bool,
    pub max_per_ip: usize,
    pub xbox_auth: bool,
    /// Liste blanche : seuls les joueurs de `white-list.txt` (et les opérateurs) peuvent entrer
    pub white_list: bool,
    /// Inventaire gardé à la mort (règle « keepInventory » du jeu). Non : il tombe au sol.
    pub keep_inventory: bool,
    /// Dossier du serveur : celui de `server.properties`, où vivent `ops.txt` et `white-list.txt`
    pub root: std::path::PathBuf,
    /// Fichier de configuration, quand elle en vient (pour y enregistrer un changement)
    pub file: Option<std::path::PathBuf>,
    pub http_signaling: bool,
    pub nethernet_udp_port: u16,
    pub public_ip: String,
    pub identity_domain: String,
    pub identity_cpk_jwk: bool,
    pub movement_check: bool,
    pub show_players: bool,
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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server_name: "Serveur Basalt".into(),
            sub_motd: "Basalt".into(),
            ip: "0.0.0.0".into(),
            port: 19132,
            port6: 19133,
            max_players: 20,
            game_version: "1.26.65".into(),
            protocol: 2225,
            min_protocol: crate::versions::MIN_PROTOCOL,
            max_protocol: crate::versions::MAX_PROTOCOL,
            gamemode: "Survival".into(),
            world_name: "Monde plat".into(),
            view_distance: 6,
            debug: false,
            probes: false,
            max_per_ip: 5,
            xbox_auth: true,
            white_list: false,
            keep_inventory: false,
            root: std::path::PathBuf::from("."),
            file: None,
            http_signaling: true,
            nethernet_udp_port: 19140,
            public_ip: String::new(),
            identity_domain: "basalt.local".into(),
            identity_cpk_jwk: true,
            movement_check: true,
            show_players: true,
            player_view: 4,
            max_visible: 48,
            auth_keys_file: String::new(),
            plugins_dir: "plugins".into(),
            world_dir: "world".into(),
            plugin_sandbox: true,
            commands_in_game: true,
            plugin_trusted: String::new(),
            plugin_memory_mb: 64,
            timeout_secs: 10,
        }
    }
}

const TEMPLATE: &str = "\
# Configuration de Basalt
# Nom affiché dans la liste des serveurs
server-name=Serveur Basalt
sub-motd=Basalt
# Adresse et port d'écoute (UDP)
server-ip=0.0.0.0
server-port=19132
# true : répond aux jeux récents (1.26.60 et plus), qui cherchent le serveur en HTTP
# sur le port TCP de même numéro que server-port (transport NetherNet)
nethernet-signaling=true
# Port UDP de la liaison WebRTC (transport NetherNet). À ouvrir dans le pare-feu.
nethernet-udp-port=19140
# Adresse IP publique du serveur, à renseigner s'il est derrière une box ou un NAT
# (laisser vide sinon : l'adresse sur laquelle le joueur s'est connecté est utilisée)
public-ip=
# Port d'écoute IPv6 (0 pour désactiver)
server-portv6=19133
max-players=20
# Version annoncée dans la liste des serveurs (la plus courante chez vos joueurs)
game-version=1.26.65
protocol=2225
# Versions acceptées à la connexion, par numéro de protocole :
#   2168 = 1.26.40 à 1.26.44    2169 = 1.26.45    2193 = 1.26.50 et 1.26.51
#   2225 = 1.26.60 à 1.26.65    2259 = préversion 1.26.70
min-protocol=2168
max-protocol=2259
# Survival ou Creative
gamemode=Survival
# true : un joueur qui meurt garde son inventaire. false (comme dans le jeu) : son
# inventaire et son armure tombent au sol là où il est mort.
keep-inventory=false
world-name=Monde plat
# Rayon (en chunks) du terrain envoyé autour du point d'apparition
view-distance=6
# true : seuls les joueurs connectés à leur compte Xbox sont acceptés, et leur
# pseudo est vérifié. false : le pseudo annoncé est accepté sans contrôle.
xbox-auth=true
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
# Distance (en chunks) jusqu'à laquelle un joueur voit les autres
player-view-distance=4
# Nombre maximal d'autres joueurs affichés à chaque joueur
max-visible-players=48
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
        c.http_signaling = map.get("nethernet-signaling").map(|v| v != "false").unwrap_or(true);
        c.xbox_auth = map.get("xbox-auth").map(|v| v != "false").unwrap_or(true);
        c.white_list = map.get("white-list").map(|v| v == "true" || v == "on").unwrap_or(false);
        c.keep_inventory = map.get("keep-inventory").map(|v| v == "true" || v == "on").unwrap_or(false);
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
        num!("server-portv6", port6);
        num!("nethernet-udp-port", nethernet_udp_port);
        c.public_ip = text_of("public-ip", "");
        c.identity_domain = text_of("identity-domain", &c.identity_domain);
        c.identity_cpk_jwk = map.get("identity-cpk-format").map(|v| v != "spki").unwrap_or(true);
        num!("max-players", max_players);
        num!("protocol", protocol);
        num!("min-protocol", min_protocol);
        num!("max-protocol", max_protocol);
        if c.min_protocol > c.max_protocol {
            warn!("server.properties : min-protocol est supérieur à max-protocol");
            c.max_protocol = c.min_protocol;
        }
        num!("timeout", timeout_secs);
        num!("view-distance", view_distance);
        num!("max-connections-per-ip", max_per_ip);
        num!("player-view-distance", player_view);
        num!("max-visible-players", max_visible);
        c.player_view = c.player_view.max(1).min(16);
        c.max_visible = c.max_visible.min(500);
        c.max_per_ip = c.max_per_ip.max(1);
        c.view_distance = c.view_distance.max(2).min(16);
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
