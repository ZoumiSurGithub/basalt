-- MultiWorld : plusieurs mondes ouverts à la fois sur le serveur.
--   /mw liste                         les mondes ouverts et leurs joueurs
--   /mw creer <nom> [type] [graine]   crée et ouvre un monde (vanilla, flat, void, nether ou end)
--   /mw ouvrir <nom>                  ouvre un monde du dossier worlds/
--   /mw fermer <nom>                  enregistre et ferme un monde
--   /mw tp <nom> [joueur]             envoie un joueur dans un monde
--   /mw ici                           dit dans quel monde vous êtes
--   /monde [nom]                      pour les joueurs : aller dans un monde public
local config = require("config")
local TYPES = { vanilla = true, flat = true, void = true, nether = true, ["end"] = true }

local function dire(joueur, texte)
  if joueur then joueur:sendMessage(texte) else server.log((texte:gsub("§.", ""))) end
end

local function est_public(nom)
  for _, monde in ipairs(config.publics or {}) do
    if monde:lower() == nom:lower() then return true end
  end
  return false
end

-- Les mondes de config.lua sont créés (ou ouverts) au démarrage
for nom, type_terrain in pairs(config.creer or {}) do
  if not server.worlds.isLoaded(nom) then
    server.worlds.load(nom, TYPES[type_terrain] and type_terrain or "flat")
  end
end

local actions = {}

function actions.liste(joueur)
  local comptes = {}
  for _, j in ipairs(server.getOnlinePlayers()) do
    local monde = j:getWorld()
    comptes[monde] = (comptes[monde] or 0) + 1
  end
  dire(joueur, "§6Mondes ouverts :")
  for i, monde in ipairs(server.worlds.list()) do
    local note = i == 1 and " §7(principal)" or (est_public(monde) and " §7(public)" or "")
    dire(joueur, "§f- " .. monde .. note .. " §7: " .. (comptes[monde] or 0) .. " joueur(s)")
  end
end

function actions.creer(joueur, nom, type_terrain, graine)
  if not nom then return dire(joueur, "§cUsage : /mw creer <nom> [vanilla|flat|void|nether|end] [graine]") end
  if server.worlds.isLoaded(nom) then return dire(joueur, "§cLe monde " .. nom .. " est déjà ouvert.") end
  type_terrain = type_terrain or "vanilla"
  if not TYPES[type_terrain] then return dire(joueur, "§cType de terrain inconnu : vanilla, flat, void, nether ou end.") end
  local ok = pcall(server.worlds.load, nom, type_terrain, graine)
  if not ok then return dire(joueur, "§cNom refusé : lettres, chiffres, - et _ seulement (32 au plus).") end
  dire(joueur, "§aMonde " .. nom .. " (" .. type_terrain .. ") ouvert. Allez-y avec /mw tp " .. nom)
end

function actions.ouvrir(joueur, nom)
  if not nom then return dire(joueur, "§cUsage : /mw ouvrir <nom>") end
  if server.worlds.isLoaded(nom) then return dire(joueur, "§cLe monde " .. nom .. " est déjà ouvert.") end
  local ok = pcall(server.worlds.load, nom)
  if not ok then return dire(joueur, "§cNom refusé : lettres, chiffres, - et _ seulement (32 au plus).") end
  dire(joueur, "§aMonde " .. nom .. " ouvert.")
end

function actions.fermer(joueur, nom)
  if not nom then return dire(joueur, "§cUsage : /mw fermer <nom>") end
  if not server.worlds.isLoaded(nom) then return dire(joueur, "§cLe monde " .. nom .. " n'est pas ouvert.") end
  if nom:lower() == server.worlds.default():lower() then return dire(joueur, "§cLe monde principal ne se ferme pas.") end
  server.worlds.unload(nom)
  dire(joueur, "§aMonde " .. nom .. " enregistré et fermé.")
end

function actions.tp(joueur, nom, pseudo)
  if not nom then return dire(joueur, "§cUsage : /mw tp <monde> [joueur]") end
  if not server.worlds.isLoaded(nom) then return dire(joueur, "§cLe monde " .. nom .. " n'est pas ouvert (voir /mw liste).") end
  local cible = joueur
  if pseudo then cible = server.getPlayer(pseudo) end
  if not cible then return dire(joueur, pseudo and "§cJoueur introuvable." or "§cDepuis la console : /mw tp <monde> <joueur>") end
  cible:setWorld(nom)
  cible:sendMessage("§aBienvenue dans le monde " .. nom .. ".")
  if pseudo and joueur ~= cible then dire(joueur, "§a" .. cible.name .. " est parti dans " .. nom .. ".") end
end

function actions.ici(joueur)
  if not joueur then return dire(nil, "La console n'est dans aucun monde.") end
  dire(joueur, "§fVous êtes dans le monde §6" .. joueur:getWorld())
end

server.command("mw", function(args, joueur)
  local action = actions[(args[1] or "liste"):lower()]
  if not action then
    return dire(joueur, "§cUsage : /mw <liste|creer|ouvrir|fermer|tp|ici> [monde] [type] [graine]")
  end
  action(joueur, args[2], args[3], args[4])
end)

server.command("monde", function(args, joueur)
  if not joueur then return dire(nil, "À utiliser en jeu.") end
  local nom = args[1]
  if not nom then
    return dire(joueur, "§fMondes ouverts aux joueurs : §6" .. table.concat(config.publics or {}, ", ") .. "§f, et §6" .. server.worlds.default())
  end
  local principal = nom:lower() == server.worlds.default():lower()
  if not principal and not est_public(nom) then return dire(joueur, "§cCe monde n'est pas ouvert aux joueurs.") end
  if not server.worlds.isLoaded(nom) then return dire(joueur, "§cCe monde est fermé pour le moment.") end
  joueur:setWorld(nom)
  joueur:sendMessage("§aBienvenue dans le monde " .. nom .. ".")
end)

-- Première connexion : le joueur arrive dans le monde choisi dans config.lua
local nouveaux = {}
server.on("load", function(joueur, premiere_fois)
  if premiere_fois then nouveaux[joueur.name] = true end
end)
server.on("join", function(joueur)
  local monde = config.premier_monde
  if nouveaux[joueur.name] and monde and server.worlds.isLoaded(monde) then
    joueur:setWorld(monde)
  end
  nouveaux[joueur.name] = nil
end)

-- Portails : un joueur qui en franchit un part dans le monde choisi dans config.lua, et
-- revient dans le monde principal s'il y est déjà
local Portails = {}
function Portails:onPlayerPortal(event)
  local monde = (config.portails or {})[event:getPortal()]
  if not monde or not server.worlds.isLoaded(monde) then return end
  local joueur = event:getPlayer()
  if event:getWorld():lower() == monde:lower() then
    joueur:setWorld(server.worlds.default())
  else
    joueur:setWorld(monde)
  end
end
server.registerEvents(Portails)
