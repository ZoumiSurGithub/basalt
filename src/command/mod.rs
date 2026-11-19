//! Commandes du jeu, comme dans Minecraft : mêmes noms, mêmes arguments, mêmes droits.
//!
//! Ce module décrit les commandes (`VanillaCommands`) et fournit de quoi lire leurs arguments ;
//! leur exécution, qui touche au monde et aux joueurs, est faite par le serveur.

/// Description d'une commande du jeu.
pub struct CommandDef {
    pub name: &'static str,
    /// Clé de traduction du jeu : chaque joueur lit la description dans sa langue
    pub description: &'static str,
    /// Formes de la commande, dans la syntaxe des plugin.yml
    pub usage: &'static str,
    /// Réservée aux opérateurs
    pub op: bool,
}

const fn def(name: &'static str, description: &'static str, usage: &'static str, op: bool) -> CommandDef {
    CommandDef { name, description, usage, op }
}

/// Les commandes du jeu connues du serveur.
pub struct VanillaCommands;

impl VanillaCommands {
    pub fn all() -> &'static [CommandDef] {
        static ALL: [CommandDef; 24] = [
            def("help", "commands.help.description", "/help", false),
            def("list", "commands.list.description", "/list", false),
            def("me", "commands.me.description", "/me <message:text>", false),
            def("tell", "commands.tell.description", "/tell <target:player> <message:text>", false),
            def("msg", "commands.tell.description", "/msg <target:player> <message:text>", false),
            def("w", "commands.tell.description", "/w <target:player> <message:text>", false),
            def("say", "commands.say.description", "/say <message:text>", true),
            def("gamemode", "commands.gamemode.description", "/gamemode <gameMode:survival|creative|s|c|0|1> [player:player]", true),
            def("give", "commands.give.description", "/give <player:player> <itemName:item> [amount:int]", true),
            def("clear", "commands.clear.description", "/clear [player:player] [itemName:item]", true),
            def("setblock", "commands.setblock.description", "/setblock <position:pos> <tileName:block>", true),
            def("fill", "commands.fill.description", "/fill <from:pos> <to:pos> <tileName:block>", true),
            def("time", "commands.time.description", "/time <mode:set> <time:day|night|noon|midnight|sunrise|sunset> | /time <mode:set|add> <amount:int> | /time <mode:query>", true),
            def("tp", "commands.tp.description", "/tp <destination:pos> | /tp <destination:player> | /tp <victim:player> <destination:pos>", true),
            def("teleport", "commands.tp.description", "/teleport <destination:pos> | /teleport <destination:player> | /teleport <victim:player> <destination:pos>", true),
            def("kill", "commands.kill.description", "/kill [target:player]", true),
            def("kick", "commands.kick.description", "/kick <name:player> [reason:text]", true),
            def("op", "commands.op.description", "/op <player:player>", true),
            def("deop", "commands.deop.description", "/deop <player:player>", true),
            def("allowlist", "commands.allowlist.description", "/allowlist <action:on|off|list|reload> | /allowlist <action:add|remove> <name>", true),
            def("whitelist", "commands.allowlist.description", "/whitelist <action:on|off|list|reload> | /whitelist <action:add|remove> <name>", true),
            def("stop", "commands.stop.description", "/stop", true),
            // Commandes propres au serveur, comme sur PocketMine
            def("status", "Affiche la charge et l'état du serveur", "/status", true),
            def("version", "Affiche la version du serveur", "/version", true),
        ];
        &ALL
    }

    pub fn get(name: &str) -> Option<&'static CommandDef> {
        Self::all().iter().find(|c| c.name == name)
    }
}

/// Découpe une ligne de commande en mots ; les guillemets regroupent plusieurs mots
/// (`/tell "Jean Dupont" bonjour`).
pub fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut started) = (false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
        if out.len() >= 64 {
            break;
        }
    }
    if started {
        out.push(current);
    }
    out
}

/// Une coordonnée telle que le jeu l'écrit : `12`, `-3.5`, `~` (ici) ou `~4` (ici + 4).
pub fn coordinate(word: &str, here: f32) -> Option<f32> {
    let value = match word.strip_prefix('~') {
        Some("") => here,
        Some(rest) => here + rest.parse::<f32>().ok()?,
        None => word.parse::<f32>().ok()?,
    };
    if value.is_finite() && value.abs() <= 3.0e7 {
        Some(value)
    } else {
        None
    }
}

/// Trois coordonnées à partir de `args[at]`, relatives à `here` si elles commencent par `~`.
pub fn position(args: &[String], at: usize, here: (f32, f32, f32)) -> Option<(f32, f32, f32)> {
    Some((coordinate(args.get(at)?, here.0)?, coordinate(args.get(at + 1)?, here.1)?, coordinate(args.get(at + 2)?, here.2)?))
}

/// Heure du monde demandée par `/time set` : un mot du jeu ou un nombre de ticks.
pub fn time_of(word: &str) -> Option<i64> {
    Some(match word {
        "day" => 1000,
        "noon" => 6000,
        "sunset" => 12000,
        "night" => 13000,
        "midnight" => 18000,
        "sunrise" => 23000,
        other => other.parse::<i64>().ok().filter(|t| (0..=i32::MAX as i64).contains(t))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_cut_like_the_game_does() {
        assert_eq!(tokens("tell \"Jean Dupont\"  bonjour à tous"), vec!["tell", "Jean Dupont", "bonjour", "à", "tous"]);
        assert_eq!(tokens("  "), Vec::<String>::new());
        assert_eq!(tokens("give Alex \"\" 3"), vec!["give", "Alex", "", "3"]);
    }

    #[test]
    fn coordinates_can_be_relative() {
        let args: Vec<String> = ["~", "~2", "-7.5"].iter().map(|s| s.to_string()).collect();
        assert_eq!(position(&args, 0, (1.0, 64.0, 3.0)), Some((1.0, 66.0, -7.5)));
        assert_eq!(position(&args, 1, (0.0, 0.0, 0.0)), None);
        assert_eq!(coordinate("abc", 0.0), None);
        assert_eq!(coordinate("1e30", 0.0), None);
    }

    #[test]
    fn times_and_definitions() {
        assert_eq!(time_of("night"), Some(13000));
        assert_eq!(time_of("250"), Some(250));
        assert_eq!(time_of("-4"), None);
        assert!(VanillaCommands::get("gamemode").map(|c| c.op).unwrap_or(false));
        assert!(!VanillaCommands::get("tell").map(|c| c.op).unwrap_or(true));
        // Chaque usage se lit sans perte : au moins une forme, jamais plus de 8
        for c in VanillaCommands::all() {
            let forms = crate::bedrock::command_overloads(c.usage);
            assert!(!forms.is_empty() && forms.len() <= 3, "{}", c.name);
        }
    }
}
