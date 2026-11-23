//! Bac à sable des plugins : un plugin protégé ne sort pas de son dossier, ne lance rien
//! sur la machine, et ne peut ni figer le serveur ni épuiser sa mémoire.

use std::path::PathBuf;
use std::time::Instant;

use basalt::lua::{Action, Info, LVal, PluginManager};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("basalt-sandbox-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Charge un plugin et renvoie les textes qu'il a publiés avec `server.set_motd`.
fn run(tag: &str, code: &str, sandbox: bool) -> Option<(Vec<String>, PluginManager, PathBuf)> {
    let d = dir(tag);
    std::fs::write(d.join("essai.lua"), code).unwrap();
    let mut m = PluginManager::new(&d);
    if !m.available() {
        return None; // pas de bibliothèque Lua sur cette machine
    }
    m.set_sandbox(sandbox, "", 16);
    m.load_all(&Info::default(), &[]);
    let said = m.take_actions().into_iter().filter_map(|a| if let Action::SetMotd(s) = a { Some(s) } else { None }).collect();
    Some((said, m, d))
}

#[test]
fn only_what_the_server_gives_is_reachable() {
    let secret = dir("secret").join("secret.txt");
    std::fs::write(&secret, "confidentiel").unwrap();
    let code = format!(
        r#"
local out = {{}}
local function say(k, v) out[#out + 1] = k .. "=" .. tostring(v) end
for _, name in ipairs({{"io", "os", "debug", "package", "load", "loadfile", "dofile", "collectgarbage", "loadstring"}}) do
  say(name, _G[name])
end
say("dump", string.dump)
say("time", type(server.time()))
say("anciennes", select(2, pcall(function() return server.every end)))
say("outside", server.read([[{secret}]]))
say("parent", server.read("../essai.lua"))
say("drive", server.write("C:/x.txt", "x"))
say("require", (pcall(require, "plugins.autre.config")))
say("written", server.write("note.txt", "bon"))
say("appended", server.append("note.txt", "jour"))
say("inside", server.read("note.txt"))
say("module", require("extra").valeur)
local globals = {{}}
for k in pairs(_G) do globals[#globals + 1] = k end
table.sort(globals)
say("globals", table.concat(globals, ","))
server.setMotd(table.concat(out, ";"))
"#,
        secret = secret.display()
    );
    let d = dir("sys");
    std::fs::create_dir_all(d.join("essai")).unwrap();
    std::fs::write(d.join("essai").join("extra.lua"), "return { valeur = 42 }").unwrap();
    std::fs::write(d.join("essai.lua"), &code).unwrap();
    let mut m = PluginManager::new(&d);
    if !m.available() {
        return;
    }
    m.set_sandbox(true, "", 16);
    m.load_all(&Info::default(), &[]);
    let text: String = m.take_actions().into_iter().filter_map(|a| if let Action::SetMotd(s) = a { Some(s) } else { None }).collect();
    for expected in [
        "io=nil", "os=nil", "debug=nil", "package=nil", "load=nil", "loadfile=nil", "dofile=nil", "collectgarbage=nil",
        "dump=nil", "time=number", "server.every n'existe plus : utilisez server.everySeconds(secondes, fonction)", "outside=nil", "parent=nil", "drive=false", "require=false",
        "written=true", "appended=true", "inside=bonjour", "module=42",
        "globals=EntityDamageEvent,EventPriority,GameMode,PlayerInteractEvent,TextFormat,_G,_VERSION,assert,coroutine,error,getmetatable,ipairs,math,next,pairs,pcall,print,rawequal,rawget,rawlen,rawset,require,select,server,setmetatable,string,table,tonumber,tostring,type,utf8,xpcall",
    ] {
        assert!(text.contains(expected), "attendu « {} » dans : {}", expected, text);
    }
    assert_eq!(std::fs::read_to_string(d.join("essai").join("note.txt")).unwrap(), "bonjour");
}

#[test]
fn endless_loops_are_interrupted_even_when_caught() {
    let code = r#"
server.on("boucle", function() while true do end end)
server.on("ruse", function()
  while true do pcall(function() while true do end end) end
end)
server.on("coroutine", function()
  while true do coroutine.resume(coroutine.create(function() while true do end end)) end
end)
server.on("sain", function() server.setMotd("vivant") end)
"#;
    let (_, mut m, _d) = match run("loop", code, true) {
        Some(r) => r,
        None => return,
    };
    let interrupted = |m: &mut PluginManager, event: &str| {
        let start = Instant::now();
        m.dispatch(event, &[]);
        let took = start.elapsed().as_millis();
        assert!(took >= 150 && took < 3000, "{} : interrompu après {} ms", event, took);
    };
    interrupted(&mut m, "boucle");
    interrupted(&mut m, "ruse");
    // Le plugin reste utilisable après deux dépassements
    m.dispatch("sain", &[LVal::Nil]);
    assert_eq!(m.take_actions(), vec![Action::SetMotd("vivant".into())]);
    // Au troisième, il est désactivé : plus aucun de ses gestionnaires n'est appelé
    interrupted(&mut m, "coroutine");
    m.dispatch("sain", &[LVal::Nil]);
    assert_eq!(m.take_actions(), vec![]);
}

#[test]
fn memory_is_capped() {
    let code = r#"
server.on("gros", function() local s = string.rep("x", 200 * 1024 * 1024) end)
server.on("petits", function()
  local t = {}
  for i = 1, 100000000 do t[i] = "morceau numero " .. i end
end)
server.on("sain", function() server.setMotd("vivant") end)
"#;
    let (_, mut m, _d) = match run("mem", code, true) {
        Some(r) => r,
        None => return,
    };
    let start = Instant::now();
    m.dispatch("gros", &[]);
    m.dispatch("petits", &[]);
    assert!(start.elapsed().as_millis() < 3000);
    m.dispatch("sain", &[]);
    assert_eq!(m.take_actions(), vec![Action::SetMotd("vivant".into())]);
}

#[test]
fn trusted_plugins_keep_full_access() {
    let (said, m, _d) = match run("trust", r#"server.setMotd(type(os.execute) .. "," .. type(package) .. "," .. type(server.read))"#, false) {
        Some(r) => r,
        None => return,
    };
    assert_eq!(said, vec!["function,table,function".to_string()]);
    assert!(!m.plugins[0].sandboxed());
}

#[test]
fn player_objects_reach_plugins_and_change_the_player() {
    use basalt::lua::PlayerSnap;
    let code = r#"
server.on("join", function(player)
  local x, y, z = player:getPosition()
  server.setMotd(table.concat({
    "Bienvenue " .. player, player.name, player:getName(), player:getDisplayName(),
    player:getHealth(), player:getMaxHealth(), player:getXpLevel(), x, y, z,
    tostring(player:isOnline()), tostring(player == server.getPlayer("Enzo")),
    #server.getOnlinePlayers(), tostring(server.getPlayer("Personne")),
  }, "|"))
  player:setMaxHealth(40)
  player:setHealth(player:getMaxHealth())
  player:setXpLevel(player:getXpLevel() + 5)
  player:setDisplayName("Enzo le Grand")
  player:setNameTag("[Admin] Enzo")
  player:sendMessage("salut")
  player:teleport(1, 2, 3)
  player:save()
  player:kick("au revoir")
  server.log("vie lue après modification : " .. player:getHealth())
end)
"#;
    let (_, mut m, _d) = match run("player", code, true) {
        Some(r) => r,
        None => return,
    };
    let mut info = Info::default();
    std::sync::Arc::make_mut(&mut info.players).push("Enzo".into());
    std::sync::Arc::make_mut(&mut info.pdata).push(PlayerSnap {
        name: "Enzo".into(),
        display_name: "Enzo".into(),
        name_tag: "Enzo".into(),
        health: 12.0,
        max_health: 20.0,
        food: 20.0,
        xp_level: 3,
        xp_progress: 0.5,
        pos: (1.5, -58.0, 7.0),
        first_played: 1000,
        ..Default::default()
    });
    m.set_info(&info, 1);
    m.dispatch("join", &[LVal::Player("Enzo".into())]);
    let actions = m.take_actions();
    assert_eq!(
        actions,
        vec![
            Action::SetMotd("Bienvenue Enzo|Enzo|Enzo|Enzo|12.0|20.0|3|1.5|-59.62|7.0|true|true|1|nil".into()),
            Action::PlayerSet("Enzo".into(), "max_health".into(), LVal::Num(40.0)),
            Action::PlayerSet("Enzo".into(), "health".into(), LVal::Num(40.0)),
            Action::PlayerSet("Enzo".into(), "xp_level".into(), LVal::Num(8.0)),
            Action::PlayerSet("Enzo".into(), "display".into(), LVal::Str("Enzo le Grand".into())),
            Action::PlayerSet("Enzo".into(), "tag".into(), LVal::Str("[Admin] Enzo".into())),
            Action::PlayerMessage("Enzo".into(), "salut".into()),
            Action::PlayerTeleport("Enzo".into(), 1.0, 2.0 + 1.62, 3.0),
            Action::PlayerSave("Enzo".into()),
            Action::Kick("Enzo".into(), "au revoir".into()),
        ]
    );
}

#[test]
fn plugins_with_a_manifest_load_in_dependency_order() {
    use basalt::lua::{has_permission, PlayerSnap};
    use basalt::plugin::manifest::PermDefault;
    let d = dir("manifest");
    let write = |folder: &str, file: &str, text: &str| {
        std::fs::create_dir_all(d.join(folder)).unwrap();
        std::fs::write(d.join(folder).join(file), text).unwrap();
    };
    // « Gardien » dépend de « Economie », rangé après lui dans l'ordre alphabétique des dossiers
    write(
        "a-gardien",
        "plugin.yml",
        "name: Gardien\nversion: 1.2.0\nmain: src/demarrage.lua\napi: 1\ndescription: Liste noire\nauthor: Enzo\ndepend: [Economie]\ncommands:\n  expulser:\n    description: Expulse un joueur\n    permission: gardien.expulser\n  oubliee:\n    description: Jamais traitée\npermissions:\n  gardien.voir:\n    default: true\n",
    );
    write("a-gardien/src", "demarrage.lua", r#"
local outil = require("src.outil")
server.command("expulser", function(args) server.setMotd("expulsion de " .. args[1]) end)
server.on("enable", function()
  server.setMotd(server.plugin.name .. " " .. server.plugin.version .. " par " .. server.plugin.authors[1] .. " : " .. outil.message)
end)
server.on("join", function(joueur)
  server.setMotd(tostring(joueur:hasPermission("gardien.voir")) .. "," .. tostring(joueur:hasPermission("gardien.expulser")) .. "," .. tostring(joueur:isOp()))
  joueur:setPermission("gardien.expulser", true)
  server.log("après réglage : " .. tostring(joueur:hasPermission("gardien.expulser")))
end)
"#);
    write("a-gardien/src", "outil.lua", r#"return { message = "prêt" }"#);
    write("z-economie", "plugin.yml", "name: Economie\nversion: 0.3\n");
    write("z-economie", "main.lua", r#"server.setMotd("economie chargée")"#);
    // Fiches refusées : dépendance absente, interface inconnue, fichier principal hors du dossier
    write("orphelin", "plugin.yml", "name: Orphelin\nversion: 1\ndepend: [Absent]\n");
    write("orphelin", "main.lua", r#"server.setMotd("ne doit pas tourner")"#);
    write("futur", "plugin.yml", "name: Futur\nversion: 1\napi: 99\n");
    write("futur", "main.lua", r#"server.setMotd("ne doit pas tourner")"#);
    write("evade", "plugin.yml", "name: Evade\nversion: 1\nmain: ../z-economie/main.lua\n");

    let mut m = PluginManager::new(&d);
    if !m.available() {
        return;
    }
    m.set_sandbox(true, "", 16);
    m.load_all(&Info::default(), &[]);
    let names: Vec<&str> = m.plugins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Economie", "Gardien"]);
    assert_eq!(
        m.take_actions(),
        vec![Action::SetMotd("economie chargée".into()), Action::SetMotd("Gardien 1.2.0 par Enzo : prêt".into())]
    );
    // Commandes et permissions déclarées
    assert_eq!(m.commands(), vec![("expulser".to_string(), "Expulse un joueur".to_string(), "Gardien".to_string())]);
    assert_eq!(m.command_permission("expulser").as_deref(), Some("gardien.expulser"));
    let declared = m.permissions();
    assert!(declared.contains(&("gardien.voir".to_string(), PermDefault::Everyone)));
    assert!(declared.contains(&("gardien.expulser".to_string(), PermDefault::Op)));
    assert!(m.run_command("expulser", &["Griefer"]));
    assert_eq!(m.take_actions(), vec![Action::SetMotd("expulsion de Griefer".into())]);

    // Permissions vues par le plugin : accordée à tous, réservée aux opérateurs, puis réglée pour le joueur
    let player = PlayerSnap { name: "Enzo".into(), display_name: "Enzo".into(), health: 20.0, max_health: 20.0, ..Default::default() };
    assert!(has_permission(&player, &declared, "gardien.voir"));
    assert!(!has_permission(&player, &declared, "gardien.expulser"));
    assert!(!has_permission(&player, &declared, "inconnue"));
    let mut info = Info::default();
    std::sync::Arc::make_mut(&mut info.players).push("Enzo".into());
    std::sync::Arc::make_mut(&mut info.pdata).push(player);
    info.perms = std::sync::Arc::new(declared);
    m.set_info(&info, 1);
    m.dispatch("join", &[LVal::Player("Enzo".into())]);
    assert_eq!(
        m.take_actions(),
        vec![Action::SetMotd("true,false,false".into()), Action::PlayerSet("Enzo".into(), "perm:gardien.expulser".into(), LVal::Bool(true))]
    );
}

#[test]
fn a_stuck_plugin_never_holds_the_server() {
    use basalt::plugin::host::PluginHost;
    use std::time::Duration;
    let d = dir("host");
    // « lent » reste coincé plusieurs secondes dans une seule opération de la bibliothèque
    // standard, que rien ne peut interrompre ; « boucle » tourne sans fin en Lua.
    std::fs::write(
        d.join("essai.lua"),
        r#"
server.on("lent", function() string.find(string.rep("a", 300), ".-.-.-b") server.setMotd("lent fini") end)
server.on("boucle", function() while true do end end)
server.on("sain", function(n) server.setMotd("sain " .. n) end)
server.on("login", function() return "refusé par le plugin" end)
"#,
    )
    .unwrap();
    let mut host = PluginHost::new(&d, true, "", 16);
    if !host.available() {
        return;
    }
    host.load_all(&Info::default(), &[]);
    assert_eq!(host.catalog().plugins.len(), 1);

    // Plugin en règle : tout se passe comme un appel direct, réponse comprise
    assert_eq!(host.dispatch("login", &[]), vec![LVal::Str("refusé par le plugin".into())]);
    host.dispatch("sain", &[LVal::Int(1)]);
    assert_eq!(host.take_actions(), vec![Action::SetMotd("sain 1".into())]);

    // Plugin coincé : le serveur reprend la main en quelques dizaines de millisecondes,
    // et les appels suivants ne l'attendent plus du tout
    let start = Instant::now();
    host.dispatch("lent", &[]);
    let first = start.elapsed();
    assert!(first < Duration::from_millis(200), "premier appel : {:?}", first);
    let start = Instant::now();
    for i in 0..200 {
        host.dispatch("sain", &[LVal::Int(100 + i)]);
        host.run_timers(i as u64);
    }
    let burst = start.elapsed();
    assert!(burst < Duration::from_millis(200), "200 événements pendant le blocage : {:?}", burst);
    assert_eq!(host.take_actions(), vec![]);

    // Quand le plugin se débloque, ce qu'il avait en file est exécuté et ses effets arrivent
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    while seen.len() < 201 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        seen.extend(host.take_actions());
    }
    assert_eq!(seen.first(), Some(&Action::SetMotd("lent fini".into())));
    assert_eq!(seen.len(), 201);

    // Boucle sans fin : le serveur n'attend pas, le plugin est interrompu, puis désactivé à la troisième
    for _ in 0..3 {
        let start = Instant::now();
        host.dispatch("boucle", &[]);
        assert!(start.elapsed() < Duration::from_millis(200));
        std::thread::sleep(Duration::from_millis(400));
    }
    host.dispatch("sain", &[LVal::Int(2)]);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(host.take_actions(), vec![], "un plugin désactivé ne doit plus être appelé");
}

/// Copie un plugin fourni avec le serveur dans un dossier de test.
fn copy_bundled(plugin: &str, to: &std::path::Path) {
    let from = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins").join(plugin);
    let target = to.join(plugin);
    copy_tree(&from, &target);
}

/// Copie un dossier et ses sous-dossiers (les plugins peuvent être répartis en plusieurs fichiers).
fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &to.join(entry.file_name()));
        } else {
            std::fs::copy(&path, to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn easyedit_edits_the_world_by_itself() {
    use basalt::lua::PlayerSnap;
    use basalt::world::World;
    let d = dir("easyedit");
    copy_bundled("EasyEdit", &d);
    let mut m = PluginManager::new(&d);
    if !m.available() {
        return;
    }
    let data = basalt::data::GameData::load(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/data"))).unwrap();
    let names = std::sync::Arc::new(data.block_names_for_plugins());
    let mut world = World::new();
    let mut info = Info::default();
    info.blocks = Some(names.clone());
    info.world = Some(world.view());
    m.set_sandbox(true, "", 64);
    m.load_all(&info, &[]);
    assert_eq!(m.plugins.len(), 1, "EasyEdit doit se charger dans le bac à sable");
    let cmds: Vec<String> = m.commands().into_iter().map(|c| c.0).collect();
    for expected in ["wand", "pos1", "pos2", "set", "replace", "walls", "sphere", "hsphere", "cyl", "hcyl", "copy", "cut", "paste", "stack", "move", "count", "undo", "redo", "size", "easyedit"] {
        assert!(cmds.contains(&expected.to_string()), "commande {} absente", expected);
        assert_eq!(m.command_permission(expected).as_deref(), Some("easyedit.use"));
    }
    // Un opérateur, les yeux en (10.5, -58.38, -3.5) : ses pieds sont dans le bloc (10, -60, -4)
    std::sync::Arc::make_mut(&mut info.players).push("Enzo".into());
    std::sync::Arc::make_mut(&mut info.pdata).push(PlayerSnap { name: "Enzo".into(), display_name: "Enzo".into(), pos: (10.5, -58.38, -3.5), op: true, held: "minecraft:wooden_axe".into(), yaw: 0.0, pitch: -80.0, ..Default::default() });
    info.perms = std::sync::Arc::new(m.permissions());
    let stone = data.default_state("minecraft:stone").unwrap();
    let grass = data.default_state("minecraft:grass_block").unwrap();
    let air = data.default_state("minecraft:air").unwrap();

    // Le serveur, réduit à l'essentiel : à chaque tick, une vue du monde pour le plugin, puis
    // les blocs qu'il pose sont appliqués. Renvoie les messages adressés au joueur.
    let tick = std::cell::Cell::new(1u64);
    let run = |m: &mut PluginManager, world: &mut World, info: &mut Info, command: Option<(&str, &[&str])>, ticks: u64| -> Vec<String> {
        let mut said = Vec::new();
        let apply = |actions: Vec<Action>, world: &mut World, said: &mut Vec<String>| {
            for a in actions {
                match a {
                    Action::SetBlocks(list) => {
                        for (x, y, z, s) in list {
                            world.set(x, y, z, s);
                        }
                    }
                    Action::PlayerMessage(_, text) => said.push(text),
                    _ => {}
                }
            }
        };
        if let Some((name, args)) = command {
            info.world = Some(world.view());
            m.set_info(info, tick.get());
            m.run_command_as(name, args, Some("Enzo"));
            m.clear_world();
            let actions = m.take_actions();
            apply(actions, world, &mut said);
        }
        for _ in 0..ticks {
            tick.set(tick.get() + 1);
            info.world = Some(world.view());
            m.set_info(info, tick.get());
            m.run_timers(tick.get());
            m.clear_world();
            let actions = m.take_actions();
            apply(actions, world, &mut said);
        }
        said
    };

    // Sans sélection : un message, rien ne change
    let said = run(&mut m, &mut world, &mut info, Some(("set", &["stone"])), 2);
    assert!(said.iter().any(|s| s.contains("deux coins")), "{:?}", said);
    assert_eq!(world.edit_count(), 0);

    // La baguette : casser = premier coin, cliquer = second ; les deux gestes sont annulés
    info.world = Some(world.view());
    m.set_info(&info, tick.get());
    let broke = m.fire("BlockBreakEvent", r#"{"player":"Enzo","block":{"name":"minecraft:grass_block","x":0,"y":-61,"z":0},"drops":[]}"#).unwrap();
    assert!(broke.contains("\"cancelled\":true"), "{}", broke);
    let clicked = m.fire("PlayerInteractEvent", r#"{"player":"Enzo","block":{"name":"minecraft:stone","x":4,"y":-58,"z":4},"action":1}"#).unwrap();
    assert!(clicked.contains("\"cancelled\":true"), "{}", clicked);
    m.take_actions();

    // /set stone : 5 × 4 × 5 = 100 blocs, posés par le plugin lui-même
    let said = run(&mut m, &mut world, &mut info, Some(("set", &["stone"])), 3);
    assert!(said.iter().any(|s| s.contains("100 blocs modifiés")), "{:?}", said);
    assert_eq!(world.get(0, -61, 0), stone);
    assert_eq!(world.get(4, -58, 4), stone);
    assert_eq!(world.get(5, -58, 4), air);
    // /count stone
    let said = run(&mut m, &mut world, &mut info, Some(("count", &["stone"])), 0);
    assert_eq!(said, vec!["§a100 blocs.".to_string()]);
    // /undo : le terrain d'origine revient ; /redo le refait
    run(&mut m, &mut world, &mut info, Some(("undo", &[])), 3);
    assert_eq!(world.get(0, -61, 0), grass);
    assert_eq!(world.get(4, -58, 4), air);
    assert_eq!(world.edit_count(), 0);
    run(&mut m, &mut world, &mut info, Some(("redo", &[])), 3);
    assert_eq!(world.get(4, -58, 4), stone);

    // /copy puis /paste : la copie est relative aux pieds du joueur (10, -60, -4)
    run(&mut m, &mut world, &mut info, Some(("copy", &[])), 0);
    std::sync::Arc::make_mut(&mut info.pdata)[0].pos = (30.5, -58.38, -3.5);
    run(&mut m, &mut world, &mut info, Some(("paste", &["a"])), 3);
    assert_eq!(world.get(20, -61, 0), stone, "collé 20 blocs plus loin");
    assert_eq!(world.get(24, -58, 4), stone);

    // Une grosse opération (sphère de rayon 20, plus de 20 000 blocs) se fait en plusieurs ticks
    std::sync::Arc::make_mut(&mut info.pdata)[0].pos = (100.5, -38.38, 100.5);
    let said = run(&mut m, &mut world, &mut info, Some(("sphere", &["glass", "20"])), 1);
    assert!(said.iter().any(|s| s.contains("en cours")), "{:?}", said);
    let said = run(&mut m, &mut world, &mut info, None, 4);
    assert!(said.iter().any(|s| s.contains("Sphère") && s.contains("blocs modifiés")), "{:?}", said);
    let glass = data.default_state("minecraft:glass").unwrap();
    assert_eq!(world.get(100, -40, 100), glass, "centre de la sphère");

    // La baguette donne une hache en bois ; depuis la console, rien ne se passe
    m.run_command_as("wand", &[], Some("Enzo"));
    assert!(m.take_actions().contains(&Action::PlayerGive("Enzo".into(), "minecraft:wooden_axe".into(), 1)));
    let _ = std::fs::remove_dir_all(&d);
}

/// Les outils simples : config.yml, données enregistrées, minuteries en secondes qu'on
/// arrête, message à tous, couleurs, joueur trouvé sans tenir compte des majuscules, et un
/// événement mal écrit signalé avec le bon nom.
#[test]
fn simple_tools_for_beginners() {
    use basalt::lua::PlayerSnap;
    let code = r#"
local config = server.getConfig({ message = "Bienvenue !", kit = true, diamants = 3, mondes = { "a", "b" } })
local points = server.loadData("points", {})
points.Steve = (points.Steve or 0) + 1
server.saveData("points", points)
local relu = server.loadData("points", {})
local compte = 0
local boucle = server.everySeconds(0.1, function(t) compte = compte + 1 if compte == 3 then t:stop() end end)
local plus_tard = server.afterSeconds(1, function() server.log("jamais") end)
plus_tard:stop()
server.on("joinn", function() end)
server.on("quit", function(joueur) server.setMotd("parti:" .. joueur:getName()) end)
server.command("etat", function()
  server.broadcast(TextFormat.GREEN .. config.message)
  server.setMotd(table.concat({ config.message, tostring(config.kit), config.diamants, #config.mondes,
    relu.Steve, compte, tostring(boucle:isRunning()), tostring(plus_tard:isRunning()),
    tostring(server.getPlayer("enzo")) }, "|"))
end)
"#;
    let (_, mut m, d) = match run("simple", code, true) {
        Some(r) => r,
        None => return,
    };
    let yml = std::fs::read_to_string(d.join("essai").join("config.yml")).unwrap();
    assert!(yml.contains("message: \"Bienvenue !\"") && yml.contains("kit: true") && yml.contains("diamants: 3") && yml.contains("mondes: [\"a\", \"b\"]"), "{}", yml);
    let mut info = Info::default();
    std::sync::Arc::make_mut(&mut info.players).push("Enzo".into());
    std::sync::Arc::make_mut(&mut info.pdata).push(PlayerSnap { name: "Enzo".into(), ..Default::default() });
    m.set_info(&info, 1);
    for tick in 1..=20 {
        m.run_timers(tick);
    }
    m.take_actions();
    m.run_command("etat", &[]);
    let actions = m.take_actions();
    assert!(actions.contains(&Action::PlayerMessage("Enzo".into(), "§aBienvenue !".into())), "{:?}", actions);
    assert!(actions.contains(&Action::SetMotd("Bienvenue !|true|3|2|1|3|false|false|Enzo".into())), "{:?}", actions);
    m.dispatch("disconnect", &[LVal::Str("1.2.3.4:5".into()), LVal::Str("Enzo".into())]);
    assert!(m.take_actions().contains(&Action::SetMotd("parti:Enzo".into())));
    // L'administrateur change un réglage : il est relu, et les autres restent
    std::fs::write(d.join("essai").join("config.yml"), "message: Salut\ndiamants: 5 # commentaire\n").unwrap();
    let mut m2 = PluginManager::new(&d);
    m2.set_sandbox(true, "", 16);
    m2.load_all(&Info::default(), &[]);
    m2.set_info(&info, 1);
    m2.take_actions();
    m2.run_command("etat", &[]);
    let actions = m2.take_actions();
    assert!(actions.iter().any(|a| matches!(a, Action::SetMotd(s) if s.starts_with("Salut|true|5|2|2|"))), "{:?}", actions);
    let yml = std::fs::read_to_string(d.join("essai").join("config.yml")).unwrap();
    assert!(yml.contains("kit: true"), "réglage manquant ajouté : {}", yml);
}

/// Les exemples du guide du débutant (docs/exemples) se chargent et font ce qu'ils disent.
#[test]
fn beginner_examples_work() {
    use basalt::lua::PlayerSnap;
    let d = dir("exemples");
    for f in ["bienvenue", "spawn", "annonces", "protection", "points"] {
        std::fs::copy(format!("{}/docs/exemples/{}.lua", env!("CARGO_MANIFEST_DIR"), f), d.join(format!("{}.lua", f))).unwrap();
    }
    let mut m = PluginManager::new(&d);
    if !m.available() {
        return;
    }
    m.set_sandbox(true, "", 16);
    m.load_all(&Info::default(), &[]);
    let mut info = Info::default();
    for (name, op) in [("Admin", true), ("Lea", false)] {
        std::sync::Arc::make_mut(&mut info.players).push(name.into());
        std::sync::Arc::make_mut(&mut info.pdata).push(PlayerSnap { name: name.into(), op, pos: (100.5, 65.62, 200.5), ..Default::default() });
    }
    m.set_info(&info, 1);
    m.take_actions();
    // Bienvenue : message, annonce, kit pour un nouveau
    m.dispatch("join", &[LVal::Player("Lea".into())]);
    m.dispatch("load", &[LVal::Player("Lea".into()), LVal::Bool(true)]);
    let a = m.take_actions();
    assert!(a.contains(&Action::PlayerMessage("Lea".into(), "§aBienvenue sur le serveur !".into())), "{:?}", a);
    assert!(a.contains(&Action::PlayerMessage("Admin".into(), "§eLea vient d'arriver".into())), "{:?}", a);
    assert!(a.iter().any(|x| matches!(x, Action::PlayerGive(n, i, 8) if n == "Lea" && i == "minecraft:bread")), "{:?}", a);
    // Spawn : refusé à un non-opérateur, enregistré puis utilisé
    m.run_command_as("setspawn", &[], Some("Lea"));
    m.run_command_as("setspawn", &[], Some("Admin"));
    m.run_command_as("spawn", &[], Some("Lea"));
    let a = m.take_actions();
    assert!(a.contains(&Action::PlayerMessage("Lea".into(), "§cRéservé aux opérateurs.".into())), "{:?}", a);
    assert!(a.contains(&Action::PlayerTeleport("Lea".into(), 100.5, 65.62, 200.5)), "{:?}", a);
    assert!(std::fs::read_to_string(d.join("spawn").join("spawn.data")).unwrap().contains("y = 64"));
    // Protection : près du centre, Léa ne casse pas, l'administrateur si
    let r = m.dispatch("block_break", &[LVal::Player("Lea".into()), LVal::Num(3.0), LVal::Num(64.0), LVal::Num(-5.0), LVal::Str("minecraft:stone".into())]);
    assert!(r.contains(&LVal::Bool(false)), "{:?}", r);
    let r = m.dispatch("block_break", &[LVal::Player("Admin".into()), LVal::Num(3.0), LVal::Num(64.0), LVal::Num(-5.0), LVal::Str("minecraft:stone".into())]);
    assert!(!r.contains(&LVal::Bool(false)), "{:?}", r);
    // Points : un bloc cassé par Léa dans la zone protégée (compté avant le refus), un autre plus loin
    m.dispatch("block_break", &[LVal::Player("Lea".into()), LVal::Num(300.0), LVal::Num(64.0), LVal::Num(300.0), LVal::Str("minecraft:dirt".into())]);
    m.take_actions();
    m.run_command_as("points", &[], Some("Lea"));
    let a = m.take_actions();
    assert!(a.iter().any(|x| matches!(x, Action::PlayerMessage(n, t) if n == "Lea" && t.contains("Vous avez 2 points"))), "{:?}", a);
    // Annonces : toutes les 10 minutes
    for tick in 1..=12_000 {
        m.run_timers(tick);
    }
    let a = m.take_actions();
    assert!(a.iter().any(|x| matches!(x, Action::PlayerMessage(n, t) if n == "Lea" && t.contains("Pensez à lire les règles"))), "{:?}", a);
}
