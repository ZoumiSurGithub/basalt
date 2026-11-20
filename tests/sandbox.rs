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
say("connect", server.connect == server.on)
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
server.set_motd(table.concat(out, ";"))
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
        "dump=nil", "time=number", "connect=true", "outside=nil", "parent=nil", "drive=false", "require=false",
        "written=true", "appended=true", "inside=bonjour", "module=42",
        "globals=EntityDamageEvent,EventPriority,PlayerInteractEvent,_G,_VERSION,assert,coroutine,error,getmetatable,ipairs,math,next,pairs,pcall,print,rawequal,rawget,rawlen,rawset,require,select,server,setmetatable,string,table,tonumber,tostring,type,utf8,xpcall",
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
server.on("sain", function() server.set_motd("vivant") end)
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
server.on("sain", function() server.set_motd("vivant") end)
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
    let (said, m, _d) = match run("trust", r#"server.set_motd(type(os.execute) .. "," .. type(package) .. "," .. type(server.read))"#, false) {
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
  server.set_motd(table.concat({
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
    info.players.push("Enzo".into());
    info.pdata.push(PlayerSnap {
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
            Action::SetMotd("Bienvenue Enzo|Enzo|Enzo|Enzo|12.0|20.0|3|1.5|-58.0|7.0|true|true|1|nil".into()),
            Action::PlayerSet("Enzo".into(), "max_health".into(), LVal::Num(40.0)),
            Action::PlayerSet("Enzo".into(), "health".into(), LVal::Num(40.0)),
            Action::PlayerSet("Enzo".into(), "xp_level".into(), LVal::Num(8.0)),
            Action::PlayerSet("Enzo".into(), "display".into(), LVal::Str("Enzo le Grand".into())),
            Action::PlayerSet("Enzo".into(), "tag".into(), LVal::Str("[Admin] Enzo".into())),
            Action::PlayerMessage("Enzo".into(), "salut".into()),
            Action::PlayerTeleport("Enzo".into(), 1.0, 2.0, 3.0),
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
server.command("expulser", function(args) server.set_motd("expulsion de " .. args[1]) end)
server.on("enable", function()
  server.set_motd(server.plugin.name .. " " .. server.plugin.version .. " par " .. server.plugin.authors[1] .. " : " .. outil.message)
end)
server.on("join", function(joueur)
  server.set_motd(tostring(joueur:hasPermission("gardien.voir")) .. "," .. tostring(joueur:hasPermission("gardien.expulser")) .. "," .. tostring(joueur:isOp()))
  joueur:setPermission("gardien.expulser", true)
  server.log("après réglage : " .. tostring(joueur:hasPermission("gardien.expulser")))
end)
"#);
    write("a-gardien/src", "outil.lua", r#"return { message = "prêt" }"#);
    write("z-economie", "plugin.yml", "name: Economie\nversion: 0.3\n");
    write("z-economie", "main.lua", r#"server.set_motd("economie chargée")"#);
    // Fiches refusées : dépendance absente, interface inconnue, fichier principal hors du dossier
    write("orphelin", "plugin.yml", "name: Orphelin\nversion: 1\ndepend: [Absent]\n");
    write("orphelin", "main.lua", r#"server.set_motd("ne doit pas tourner")"#);
    write("futur", "plugin.yml", "name: Futur\nversion: 1\napi: 99\n");
    write("futur", "main.lua", r#"server.set_motd("ne doit pas tourner")"#);
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
    info.players.push("Enzo".into());
    info.pdata.push(player);
    info.perms = declared;
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
server.on("lent", function() string.find(string.rep("a", 300), ".-.-.-b") server.set_motd("lent fini") end)
server.on("boucle", function() while true do end end)
server.on("sain", function(n) server.set_motd("sain " .. n) end)
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
    std::fs::create_dir_all(&target).unwrap();
    for entry in std::fs::read_dir(&from).unwrap().flatten() {
        std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
    }
}

#[test]
fn easyedit_turns_commands_into_world_edits() {
    use basalt::lua::PlayerSnap;
    let d = dir("easyedit");
    copy_bundled("EasyEdit", &d);
    let mut m = PluginManager::new(&d);
    if !m.available() {
        return;
    }
    m.set_sandbox(true, "", 16);
    m.load_all(&Info::default(), &[]);
    assert_eq!(m.plugins.len(), 1, "EasyEdit doit se charger dans le bac à sable");
    assert_eq!(m.plugins[0].manifest.version, "1.0.0");
    let names: Vec<String> = m.commands().into_iter().map(|c| c.0).collect();
    for expected in ["wand", "pos1", "pos2", "set", "replace", "walls", "sphere", "hsphere", "cyl", "hcyl", "copy", "cut", "paste", "stack", "move", "count", "undo", "redo", "size", "easyedit"] {
        assert!(names.contains(&expected.to_string()), "commande {} absente", expected);
        assert_eq!(m.command_permission(expected).as_deref(), Some("easyedit.use"));
    }
    // Un opérateur, les yeux en (10.5, -58.38, -3.5) : ses pieds sont dans le bloc (10, -60, -4)
    let mut info = Info::default();
    info.players.push("Enzo".into());
    info.pdata.push(PlayerSnap { name: "Enzo".into(), display_name: "Enzo".into(), pos: (10.5, -58.38, -3.5), op: true, held: "minecraft:wooden_axe".into(), yaw: 0.0, pitch: -80.0, ..Default::default() });
    info.perms = m.permissions();
    m.set_info(&info, 1);
    let edits = |m: &mut PluginManager| -> Vec<(String, Vec<LVal>)> {
        m.take_actions().into_iter().filter_map(|a| if let Action::Edit(_, op, args) = a { Some((op, args)) } else { None }).collect()
    };
    let n = |v: i32| LVal::Num(v as f64);
    let s = |v: &str| LVal::Str(v.to_string());

    // Sans sélection : un message, aucune édition
    assert!(m.run_command_as("set", &["stone"], Some("Enzo")));
    assert_eq!(edits(&mut m), vec![]);
    // La baguette : casser = premier coin, cliquer = second ; les deux sont annulés
    assert_eq!(m.dispatch("block_break", &[LVal::Player("Enzo".into()), LVal::Int(0), LVal::Int(-61), LVal::Int(0), s("minecraft:grass_block")]), vec![LVal::Bool(false)]);
    assert_eq!(m.dispatch("block_click", &[LVal::Player("Enzo".into()), LVal::Int(4), LVal::Int(-58), LVal::Int(4), s("minecraft:stone")]), vec![LVal::Bool(false)]);
    m.take_actions();
    m.run_command_as("set", &["stone,dirt"], Some("Enzo"));
    assert_eq!(edits(&mut m), vec![("set".to_string(), vec![n(0), n(-61), n(0), n(4), n(-58), n(4), s("stone,dirt")])]);
    m.run_command_as("replace", &["stone", "glass"], Some("Enzo"));
    assert_eq!(edits(&mut m)[0].0, "replace");
    // Par rapport au joueur : copie, sphère creuse, empilement vers le haut (il regarde en l'air)
    m.run_command_as("copy", &[], Some("Enzo"));
    assert_eq!(edits(&mut m), vec![("copy".to_string(), vec![n(0), n(-61), n(0), n(4), n(-58), n(4), n(10), n(-60), n(-4)])]);
    m.run_command_as("hsphere", &["glass", "6"], Some("Enzo"));
    assert_eq!(edits(&mut m), vec![("sphere".to_string(), vec![n(10), n(-60), n(-4), n(6), s("glass"), LVal::Bool(true)])]);
    m.run_command_as("stack", &["3"], Some("Enzo"));
    assert_eq!(edits(&mut m), vec![("stack".to_string(), vec![n(0), n(-61), n(0), n(4), n(-58), n(4), n(0), n(1), n(0), n(3)])]);
    m.run_command_as("move", &["north", "2"], Some("Enzo"));
    assert_eq!(edits(&mut m)[0].1[6..], [n(0), n(0), n(-1), n(2)]);
    m.run_command_as("paste", &["-a"], Some("Enzo"));
    assert_eq!(edits(&mut m), vec![("paste".to_string(), vec![n(10), n(-60), n(-4), LVal::Bool(true)])]);
    m.run_command_as("undo", &[], Some("Enzo"));
    assert_eq!(edits(&mut m), vec![("undo".to_string(), vec![])]);
    // La baguette donne une hache en bois ; depuis la console, rien ne se passe
    m.run_command_as("wand", &[], Some("Enzo"));
    assert!(m.take_actions().contains(&Action::PlayerGive("Enzo".into(), "minecraft:wooden_axe".into(), 1)));
    m.run_command("set", &["stone"]);
    assert_eq!(edits(&mut m), vec![]);
    // Sans la baguette en main, casser un bloc reste un geste normal
    info.pdata[0].held = "minecraft:stone".into();
    m.set_info(&info, 2);
    assert_eq!(m.dispatch("block_break", &[LVal::Player("Enzo".into()), LVal::Int(0), LVal::Int(0), LVal::Int(0), s("minecraft:stone")]), vec![LVal::Nil]);
}

#[test]
fn listeners_run_by_priority_across_plugins_like_pocketmine() {
    let d = dir("events");
    // Deux plugins : l'ordre de priorité passe avant l'ordre de chargement
    std::fs::write(
        d.join("a.lua"),
        r#"
local Ecouteur = {}
function Ecouteur:onBlockBreak(event)
  event:setDrops({ { name = "minecraft:dirt", count = 2 } })
end
Ecouteur.priorities = { onBlockBreak = EventPriority.LOW }
server.getPluginManager():registerEvents(Ecouteur)
server.registerEvent("BlockBreakEvent", function(event)
  if event:getPlayer():getName() == "Enzo" then event:cancel() end
end, EventPriority.HIGH)
-- Après l'annulation : n'est pas appelé (handleCancelled absent)
server.registerEvent("BlockBreakEvent", function(event) event:setDrops({}) end, EventPriority.HIGHEST)
server.registerEvent("BlockBreakEvent", function(event)
  local b = event:getBlock()
  server.set_motd(table.concat({ tostring(event:isCancelled()), event:getDrops()[1].name, b:getName(), b:getX(), event:getEventName() }, "|"))
end, EventPriority.MONITOR, true)
-- Dégâts : un écouteur de EntityDamageByEntityEvent n'entend que les coups entre joueurs
server.registerEvent("EntityDamageByEntityEvent", function(event)
  event:setBaseDamage(event:getBaseDamage() * 2)
end)
"#,
    )
    .unwrap();
    std::fs::write(
        d.join("b.lua"),
        r#"
server.registerEvent("BlockBreakEvent", function(event)
  event:setDrops({ { name = "minecraft:stone", count = 1 } })
end, EventPriority.LOWEST)
local ok, err = pcall(function() server.registerEvent("EvenementInconnu", function() end) end)
server.set_motd("inconnu refusé : " .. tostring(not ok))
"#,
    )
    .unwrap();
    let mut m = PluginManager::new(&d);
    if !m.available() {
        return;
    }
    m.set_sandbox(true, "", 16);
    m.load_all(&Info::default(), &[]);
    let said: Vec<String> = m.take_actions().into_iter().filter_map(|a| if let Action::SetMotd(s) = a { Some(s) } else { None }).collect();
    assert!(said.contains(&"inconnu refusé : true".to_string()), "{:?}", said);
    assert!(m.listening().iter().any(|k| k == "BlockBreakEvent#LOW"));

    let state = r#"{"player":"Enzo","block":{"name":"minecraft:stone","x":4,"y":-60,"z":7},"drops":[{"name":"minecraft:cobblestone","count":1}]}"#;
    let out = m.fire("BlockBreakEvent", state).expect("écouté");
    let v = basalt::json::parse(&out).unwrap();
    assert_eq!(v.get("cancelled").and_then(|c| c.as_bool()), Some(true));
    assert_eq!(v.get("drops").and_then(|d| d.as_arr()).and_then(|d| d[0].get("name")).and_then(|n| n.as_str()), Some("minecraft:dirt"));
    let said: Vec<String> = m.take_actions().into_iter().filter_map(|a| if let Action::SetMotd(s) = a { Some(s) } else { None }).collect();
    assert_eq!(said, vec!["true|minecraft:dirt|minecraft:stone|4|BlockBreakEvent".to_string()]);

    // Chute : pas d'auteur, la règle « par une entité » ne s'applique pas
    let fall = m.fire("EntityDamageEvent", r#"{"entity":"Enzo","cause":"FALL","baseDamage":3}"#).expect("écouté");
    assert_eq!(basalt::json::parse(&fall).unwrap().get("baseDamage").and_then(|n| n.as_f64()), Some(3.0));
    let hit = m.fire("EntityDamageEvent", r#"{"entity":"Enzo","cause":"ENTITY_ATTACK","baseDamage":3,"damager":"Alex"}"#).expect("écouté");
    assert_eq!(basalt::json::parse(&hit).unwrap().get("baseDamage").and_then(|n| n.as_f64()), Some(6.0));
    // Personne n'écoute : rien n'est appelé
    assert_eq!(m.fire("PlayerJoinEvent", r#"{"player":"Enzo"}"#), None);
    let _ = std::fs::remove_dir_all(&d);
}
