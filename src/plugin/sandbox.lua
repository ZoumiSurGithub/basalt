-- Préambule des plugins Basalt, exécuté dans chaque plugin avant son propre code.
--
-- 1. Il complète l'objet `server` : fichiers du plugin, heure, alias.
-- 2. Pour un plugin protégé, il ne laisse ensuite que ce que le serveur donne : `server`,
--    le langage Lua de base (string, table, math, utf8, coroutine) et `require` pour les
--    propres fichiers du plugin. Tout le reste disparaît (io, os, debug, package, load…).
local dir = __basalt_dir
local protected = __basalt_protected
__basalt_dir = nil
__basalt_protected = nil

local io_open = io.open
local os_time, os_date, os_clock = os.time, os.date, os.clock
local real_loadfile = loadfile
local real_load = load

-- Nom de fichier donné par le plugin -> chemin dans son dossier, ou nil s'il en sort
local function inside(name)
  if type(name) ~= "string" or name == "" or #name > 200 then return nil end
  name = name:gsub("\\", "/")
  if name:sub(1, 1) == "/" or name:find("..", 1, true) or name:find(":", 1, true) then return nil end
  if not name:match("^[%w%._%-/ ]+$") then return nil end
  return dir .. name
end

-- Fichiers du plugin (dans plugins/<nom>/ uniquement)
function server.read(name)
  local path = inside(name)
  if not path then return nil end
  local f = io_open(path, "rb")
  if not f then return nil end
  local text = f:read("a")
  f:close()
  return text
end

-- Écrit ou complète un fichier du plugin ; faux si le nom sort de son dossier.
local function put(name, text, mode)
  local path = inside(name)
  if not path then return false end
  local f = io_open(path, mode)
  if not f then return false end
  f:write(tostring(text))
  f:close()
  return true
end

-- Fichiers du plugin : `write` remplace le contenu, `append` ajoute à la fin.
function server.write(name, text) return put(name, text, "wb") end
function server.append(name, text) return put(name, text, "ab") end

-- Heure
function server.time() return os_time() end
-- Temps processeur, pour chronométrer un bout de code.
function server.clock() return os_clock() end
-- Date en texte ; format de os.date, « 2026-11-21 12:00:00 » par défaut.
function server.date(format, time)
  if format ~= nil and type(format) ~= "string" then return nil end
  return os_date(format or "%Y-%m-%d %H:%M:%S", time)
end

-- Fonctions brutes du serveur, gardées ici et retirées de `server` : les plugins passent par
-- les objets joueur et les minuteries en secondes définis plus bas.
local raw_players, raw_kick, raw_after, raw_every = server._players, server._kick, server._after, server._every
server._players, server._kick, server._after, server._every = nil, nil, nil, nil

-- Anciennes fonctions retirées : un plugin qui les appelle reçoit une erreur qui dit quoi
-- utiliser à la place, au lieu de « attempt to call a nil value ».
local REMOVED = {
  every = "server.everySeconds(secondes, fonction)",
  after = "server.afterSeconds(secondes, fonction)",
  players = "server.getOnlinePlayers()",
  kick = "server.getPlayer(pseudo):kick(raison)",
  position = "server.getPlayer(pseudo):getPosition()",
  motd = "server.getMotd()",
  set_motd = "server.setMotd(texte)",
  connect = "server.on(événement, fonction)",
  getPluginManager = "server.registerEvents(écouteur)",
}
setmetatable(server, { __index = function(_, key)
  local instead = REMOVED[key]
  if instead then
    error("server." .. key .. " n'existe plus : utilisez " .. instead, 2)
  end
end })

-- Modifier les objets et les blocs du jeu (voir docs/PLUGINS.md, « Modifier objets et blocs »).
-- Chaque valeur est vérifiée ici, pour que l'erreur désigne la ligne du plugin.
local modify = server._modify
server._modify = nil
local ITEM_KEYS = { food = "number", saturation = "number", leftover = "string", durability = "number", attackDamage = "number" }
local BLOCK_KEYS = { drops = "table", falls = "boolean", needsFloor = "boolean", breakable = "boolean" }
local function modifier(kind, keys, usage)
  local api = {}
  function api.modify(name, values)
    if type(name) ~= "string" or type(values) ~= "table" then error(usage, 2) end
    for key, value in pairs(values) do
      local expected = keys[key]
      if not expected then
        local known = {}
        for k in pairs(keys) do known[#known + 1] = k end
        table.sort(known)
        error("valeur inconnue « " .. tostring(key) .. " » ; valeurs possibles : " .. table.concat(known, ", "), 2)
      end
      if type(value) ~= expected then
        error(key .. " attend " .. expected .. ", reçu " .. type(value), 2)
      end
      if key == "drops" then
        -- { "minecraft:diamond", { "minecraft:stick", 2 } } ou { ["minecraft:diamond"] = 2 }
        local parts = {}
        for k, v in pairs(value) do
          local item, count
          if type(k) == "string" then item, count = k, v
          elseif type(v) == "table" then item, count = v[1] or v.item, v[2] or v.count or 1
          else item, count = v, 1 end
          if type(item) ~= "string" or type(count) ~= "number" or count < 1 or item:find("[=,]") then
            error("drops attend des objets et des quantités, par exemple { [\"minecraft:diamond\"] = 2 }", 2)
          end
          parts[#parts + 1] = item .. "=" .. math.floor(count)
        end
        table.sort(parts)
        value = table.concat(parts, ",")
      end
      modify(kind, name, key, tostring(value))
    end
  end
  function api.reset(name)
    if type(name) ~= "string" then error(usage, 2) end
    modify(kind, name, "reset", "")
  end
  return api
end
server.items = modifier("item", ITEM_KEYS, "server.items.modify(nom, { food = 8, ... })")
server.blocks = modifier("block", BLOCK_KEYS, "server.blocks.modify(nom, { falls = true, ... })")
-- Créer un contenu : vérifie le nom et les réglages, puis transmet la fiche au serveur
local function creator(kind, keys, usage)
  return function(name, values)
    if type(name) ~= "string" or (values ~= nil and type(values) ~= "table") then error(usage, 2) end
    if not name:match("^[a-z0-9_]+:[a-z0-9_]+$") then
      error("nom invalide « " .. name .. " » : minuscules, chiffres et _ autour de « : », par exemple monserveur:rubis", 2)
    end
    if name:match("^minecraft:") then
      error("l'espace de noms « minecraft » est réservé au jeu : pour changer un contenu du jeu, utilisez modify", 2)
    end
    local lines, after = {}, {}
    for key, value in pairs(values or {}) do
      local expected = keys[key]
      if not expected then
        local known = {}
        for k in pairs(keys) do known[#known + 1] = k end
        table.sort(known)
        error("réglage inconnu « " .. tostring(key) .. " » ; réglages possibles : " .. table.concat(known, ", "), 2)
      end
      if type(value) ~= expected then
        error(key .. " attend " .. expected .. ", reçu " .. type(value), 2)
      end
      if expected == "table" then
        after[key] = value
      elseif key == "_stages" then
        lines[#lines + 1] = "stages\t" .. value
      elseif key == "_drops" then
        lines[#lines + 1] = "drops\t" .. value
      else
        lines[#lines + 1] = key .. "\t" .. tostring(value):gsub("[\t\r\n]", " ")
      end
    end
    table.sort(lines)
    modify(kind, name, "", table.concat(lines, "\n"))
    return after
  end
end
-- Créer un bloc (voir docs/PLUGINS.md, « Créer des blocs »)
local NEW_BLOCK_KEYS = {
  name = "string", texture = "string", textureTop = "string", textureBottom = "string", textureFront = "string", rotation = "string", stages = "table",
  hardness = "number", light = "number", friction = "number",
  transparent = "boolean", solid = "boolean", geometry = "string", tool = "string", tier = "string", xp = "number", xpMax = "number",
  width = "number", height = "number",
  category = "string", group = "string", creative = "boolean", drops = "table", falls = "boolean", needsFloor = "boolean", breakable = "boolean",
  _stages = "string",
}
local create_block = creator("new-block", NEW_BLOCK_KEYS, "server.blocks.create(\"monserveur:minerai\", { name = \"Minerai\", texture = \"minerai\" })")
function server.blocks.create(name, values)
  -- Butin, chute, appui : les mêmes réglages que pour un bloc du jeu, appliqués ensuite
  local behaviour, rest = {}, {}
  for key, value in pairs(values or {}) do
    if BLOCK_KEYS[key] then behaviour[key] = value else rest[key] = value end
  end
  if type(values) ~= "table" and values ~= nil then rest = values end
  -- Stades de croissance : la liste de leurs images, de la pousse à la maturité
  local stages = rest.stages
  if type(stages) == "table" then
    for i, image in ipairs(stages) do
      if type(image) ~= "string" then error("stages attend une liste d'images, par exemple { \"pousse\", \"epi\" }", 2) end
    end
    rest.stages = nil
    rest._stages = table.concat(stages, ",")
  end
  create_block(name, rest)
  if next(behaviour) then server.blocks.modify(name, behaviour) end
end
-- Créer une créature (voir docs/PLUGINS.md, « Créer des créatures »)
local NEW_ENTITY_KEYS = {
  looks = "string", ownModel = "boolean", name = "string", nameVisible = "boolean", health = "number", speed = "number", scale = "number",
  width = "number", height = "number", hostile = "boolean", attackDamage = "number", sight = "number",
  projectile = "string", projectileDamage = "number", shootInterval = "number", shootRange = "number", explosion = "number",
  xp = "number", xpMax = "number", drops = "table", _drops = "string",
}
local create_entity = creator("new-entity", NEW_ENTITY_KEYS, "server.entities.create(\"monserveur:vache_doree\", { looks = \"minecraft:cow\", health = 30 })")
server.entities = {}
function server.entities.create(name, values)
  local rest = {}
  for key, value in pairs(type(values) == "table" and values or {}) do rest[key] = value end
  if type(values) ~= "table" and values ~= nil then rest = values end
  -- Butin : { ["objet"] = 2 }, { ["objet"] = { 1, 3 } } (de 1 à 3) ou { "objet" }
  if type(rest) == "table" and type(rest.drops) == "table" then
    local parts = {}
    for k, v in pairs(rest.drops) do
      local item, lo, hi
      if type(k) == "string" then item = k else item, v = v, 1 end
      if type(v) == "table" then lo, hi = v[1], v[2] or v[1] else lo, hi = v, v end
      if type(item) ~= "string" or type(lo) ~= "number" or type(hi) ~= "number" or lo < 0 or hi < lo or item:find("[=,%-]") then
        error("drops attend des objets et des quantités, par exemple { [\"minecraft:gold_ingot\"] = { 1, 3 } }", 2)
      end
      parts[#parts + 1] = item .. "=" .. math.floor(lo) .. "-" .. math.floor(hi)
    end
    table.sort(parts)
    rest.drops = nil
    rest._drops = table.concat(parts, ",")
  end
  create_entity(name, rest)
end
-- Faire apparaître une créature, du jeu ou créée par un plugin
function server.entities.spawn(name, x, y, z)
  if type(name) ~= "string" or type(x) ~= "number" or type(y) ~= "number" or type(z) ~= "number" then
    error("server.entities.spawn(nom, x, y, z)", 2)
  end
  modify("spawn-entity", name, "", x .. "\t" .. y .. "\t" .. z)
end
-- Tirer un projectile : une entité du jeu (« minecraft:arrow ») lancée d'un point dans une direction
function server.entities.shoot(name, x, y, z, dx, dy, dz, options)
  local usage = "server.entities.shoot(nom, x, y, z, dx, dy, dz, { speed = 1.6, damage = 3, gravity = 0.05, explosion = 0 })"
  for _, v in ipairs({ x, y, z, dx, dy, dz }) do
    if type(v) ~= "number" then error(usage, 2) end
  end
  if type(name) ~= "string" or (options ~= nil and type(options) ~= "table") then error(usage, 2) end
  options = options or {}
  local function num(key, default)
    local v = options[key]
    if v == nil then return default end
    if type(v) ~= "number" or v ~= v or v < 0 then error(key .. " attend un nombre positif", 3) end
    return v
  end
  for key in pairs(options) do
    if key ~= "speed" and key ~= "damage" and key ~= "gravity" and key ~= "explosion" then error("réglage inconnu « " .. tostring(key) .. " » ; " .. usage, 2) end
  end
  modify("shoot", name, "", table.concat({ x, y, z, dx, dy, dz, num("speed", 1.6), num("damage", 3), num("gravity", 0.05), num("explosion", 0) }, "\t"))
end
-- Créer un objet (voir docs/PLUGINS.md, « Créer des objets »)
local NEW_ITEM_KEYS = {
  name = "string", icon = "string", category = "string", group = "string", creative = "boolean", maxStack = "number", food = "number", saturation = "number", alwaysEdible = "boolean",
  leftover = "string", durability = "number", attackDamage = "number", handEquipped = "boolean", glint = "boolean",
}
function server.items.create(name, values)
  local usage = "server.items.create(\"monserveur:rubis\", { name = \"Rubis\", icon = \"emerald\" })"
  if type(name) ~= "string" or (values ~= nil and type(values) ~= "table") then error(usage, 2) end
  if not name:match("^[a-z0-9_]+:[a-z0-9_]+$") then
    error("nom invalide « " .. name .. " » : minuscules, chiffres et _ autour de « : », par exemple monserveur:rubis", 2)
  end
  if name:match("^minecraft:") then
    error("l'espace de noms « minecraft » est réservé au jeu : pour changer un objet du jeu, utilisez server.items.modify", 2)
  end
  local lines = {}
  for key, value in pairs(values or {}) do
    local expected = NEW_ITEM_KEYS[key]
    if not expected then
      local known = {}
      for k in pairs(NEW_ITEM_KEYS) do known[#known + 1] = k end
      table.sort(known)
      error("réglage inconnu « " .. tostring(key) .. " » ; réglages possibles : " .. table.concat(known, ", "), 2)
    end
    if type(value) ~= expected then
      error(key .. " attend " .. expected .. ", reçu " .. type(value), 2)
    end
    lines[#lines + 1] = key .. "\t" .. tostring(value):gsub("[\t\r\n]", " ")
  end
  table.sort(lines)
  modify("new-item", name, "", table.concat(lines, "\n"))
end

-- Objet joueur. Il ne contient que le pseudo : chaque lecture interroge le serveur, donc
-- un objet gardé dans une variable reste juste, et `isOnline` dit si le joueur est là.
local pget, pset, pdo, kick = server._pget, server._pset, server._pdo, raw_kick
local Player = {}
Player.__index = Player
Player.__name = "Player"
Player.__tostring = function(p) return p.name end
Player.__concat = function(a, b) return tostring(a) .. tostring(b) end
Player.__eq = function(a, b) return a.name == b.name end

-- Lectures et écritures simples : chaque appel demande la valeur au serveur, ou lui
-- transmet la nouvelle (appliquée à la fin de l'appel du plugin).
function Player:getName() return self.name end
function Player:isOnline() return pget(self.name, "online") == true end
function Player:getDisplayName() return pget(self.name, "display") or self.name end
function Player:setDisplayName(text) pset(self.name, "display", tostring(text)) end
function Player:getNameTag() return pget(self.name, "tag") or self.name end
function Player:setNameTag(text) pset(self.name, "tag", tostring(text)) end
function Player:getHealth() return pget(self.name, "health") end
-- Effets : speed, slowness, haste, strength, regeneration, poison, night_vision…
function Player:addEffect(effect, seconds, level)
  if type(effect) ~= "string" or (seconds ~= nil and type(seconds) ~= "number") or (level ~= nil and type(level) ~= "number") then
    error("joueur:addEffect(effet, secondes, niveau) : par exemple joueur:addEffect(\"speed\", 30, 1)", 2)
  end
  pset(self.name, "effect", effect .. "\t" .. math.max(0, math.floor((level or 1) - 1)) .. "\t" .. math.max(0, seconds or 30))
end
function Player:removeEffect(effect)
  if type(effect) ~= "string" then error("joueur:removeEffect(effet)", 2) end
  pset(self.name, "effect", effect .. "\t0\t0")
end
function Player:clearEffects() pset(self.name, "effect", "clear\t0\t0") end
function Player:setHealth(value) pset(self.name, "health", value) end
function Player:getMaxHealth() return pget(self.name, "max_health") end
function Player:setMaxHealth(value) pset(self.name, "max_health", value) end
function Player:getFood() return pget(self.name, "food") end
function Player:setFood(value) pset(self.name, "food", value) end
function Player:getXpLevel() return pget(self.name, "xp_level") end
function Player:setXpLevel(value) pset(self.name, "xp_level", value) end
function Player:getXpProgress() return pget(self.name, "xp_progress") end
function Player:setXpProgress(value) pset(self.name, "xp_progress", value) end
-- Position des pieds ; `teleport` attend elle aussi les pieds.
function Player:getPosition() return pget(self.name, "pos") end
function Player:teleport(x, y, z) pdo(self.name, "teleport", x, y, z) end
-- Date de première connexion, en secondes comme server.time().
function Player:getFirstPlayed() return pget(self.name, "first_played") end
-- Actions sur le joueur, exécutées par le serveur quand l'appel du plugin se termine.
function Player:sendMessage(text) pdo(self.name, "message", tostring(text)) end
function Player:kick(reason) kick(self.name, reason) end
function Player:save() pdo(self.name, "save") end
-- Donne des objets : joueur:give("minecraft:diamond", 3)
function Player:give(item, count) pdo(self.name, "give", tostring(item), count or 1) end
-- Regard et objet tenu
function Player:getYaw() return pget(self.name, "yaw") or 0 end
-- Angle vertical du regard : négatif vers le haut, positif vers le bas.
function Player:getPitch() return pget(self.name, "pitch") or 0 end
-- Nom de l'objet tenu en main (texte vide si la main est vide).
function Player:getItemInHand() return pget(self.name, "held") or "" end
-- Mode de jeu : 0 survie, 1 créatif, 2 aventure, 3 spectateur (numéros de PocketMine)
function Player:getGamemode() return pget(self.name, "gamemode") or 0 end
-- Le changement passe par PlayerGameModeChangeEvent, qu'un plugin peut annuler.
function Player:setGamemode(mode) pset(self.name, "gamemode", mode) end
-- Direction du regard, en décalage de blocs : (dx, dy, dz) parmi les six directions
function Player:getDirection()
  local pitch, yaw = self:getPitch(), self:getYaw() % 360
  if pitch <= -60 then return 0, 1, 0 end
  if pitch >= 60 then return 0, -1, 0 end
  if yaw >= 315 or yaw < 45 then return 0, 0, 1 end
  if yaw < 135 then return -1, 0, 0 end
  if yaw < 225 then return 0, 0, -1 end
  return 1, 0, 0
end
-- Édition du monde en volume (voir le plugin EasyEdit) : le serveur exécute l'opération et
-- répond lui-même au joueur
-- Permissions : réglage propre au joueur, sinon règle par défaut déclarée dans plugin.yml
function Player:isOp() return pget(self.name, "op") == true end
-- Permissions : réglage propre au joueur, sinon la règle par défaut de plugin.yml.
function Player:setOp(value) pset(self.name, "op", value == true) end
function Player:hasPermission(node) return pget(self.name, "perm", tostring(node)) == true end
-- `nil` efface le réglage propre au joueur (la règle par défaut s'applique de nouveau).
function Player:setPermission(node, value)
  if value ~= nil then value = (value == true) end
  pset(self.name, "perm:" .. tostring(node):lower(), value)
end

-- Objet joueur : il ne garde que le pseudo, tout le reste est lu au moment voulu.
local function wrap(name)
  return setmetatable({ name = name }, Player)
end
server._wrap(wrap)
server._wrap, server._pget, server._pset, server._pdo = nil, nil, nil, nil

-- Joueur en jeu portant ce pseudo, ou nil
function server.getPlayer(name)
  name = tostring(name)
  if pget(name, "online") then return wrap(name) end
  return nil
end
-- Tous les joueurs acceptés au login, y compris ceux qui n'ont pas fini d'apparaître.
function server.getOnlinePlayers()
  local list = {}
  for i, name in ipairs(raw_players()) do list[i] = wrap(name) end
  return list
end

-- ---------------------------------------------------------------------------------------
-- Outils simples, pour écrire un plugin sans connaître la programmation : messages à tous,
-- couleurs, minuteries en secondes (qu'on peut arrêter), données enregistrées, config.yml.
-- ---------------------------------------------------------------------------------------

-- Joueur en jeu, sans tenir compte des majuscules (« steve » trouve « Steve »)
function server.getPlayer(name)
  name = tostring(name)
  if pget(name, "online") then return wrap(name) end
  local lower = name:lower()
  for _, n in ipairs(raw_players()) do
    if n:lower() == lower then return wrap(n) end
  end
  return nil
end

-- Message à tous les joueurs en jeu
function server.broadcast(text)
  text = tostring(text)
  for _, p in ipairs(server.getOnlinePlayers()) do p:sendMessage(text) end
end

-- Couleurs et styles du chat, comme TextFormat dans PocketMine : TextFormat.GREEN .. "Salut"
TextFormat = {
  BLACK = "§0", DARK_BLUE = "§1", DARK_GREEN = "§2", DARK_AQUA = "§3", DARK_RED = "§4", DARK_PURPLE = "§5",
  GOLD = "§6", GRAY = "§7", DARK_GRAY = "§8", BLUE = "§9", GREEN = "§a", AQUA = "§b", RED = "§c",
  LIGHT_PURPLE = "§d", YELLOW = "§e", WHITE = "§f", MINECOIN_GOLD = "§g",
  -- Couleurs des matériaux, propres à Bedrock
  MATERIAL_QUARTZ = "§h", MATERIAL_IRON = "§i", MATERIAL_NETHERITE = "§j", MATERIAL_REDSTONE = "§m",
  MATERIAL_COPPER = "§n", MATERIAL_GOLD = "§p", MATERIAL_EMERALD = "§q", MATERIAL_DIAMOND = "§s",
  MATERIAL_LAPIS = "§t", MATERIAL_AMETHYST = "§u", MATERIAL_RESIN = "§v",
  OBFUSCATED = "§k", BOLD = "§l", ITALIC = "§o", RESET = "§r",
}

-- Minuteries en secondes, qu'on peut arrêter :
--   local annonce = server.everySeconds(600, function() server.broadcast("Pensez à voter !") end)
--   annonce:stop()
local Timer = {}
Timer.__index = Timer
-- Minuterie rendue par afterSeconds / everySeconds.
function Timer:stop() self.stopped = true end
function Timer:isRunning() return not self.stopped end
-- Secondes -> ticks (20 par seconde), au moins 1. Erreur si ce n'est pas un nombre positif.
local function ticks_of(seconds, what)
  seconds = tonumber(seconds)
  if not seconds or seconds ~= seconds or seconds <= 0 then
    error(what .. " : un nombre de secondes plus grand que 0 est attendu", 3)
  end
  return math.max(1, math.floor(seconds * 20 + 0.5))
end
-- Appelle `fn` une fois après tant de secondes, sauf si la minuterie a été arrêtée avant.
function server.afterSeconds(seconds, fn)
  if type(fn) ~= "function" then error("server.afterSeconds(secondes, fonction) : la fonction manque", 2) end
  local timer = setmetatable({}, Timer)
  raw_after(ticks_of(seconds, "server.afterSeconds"), function()
    if timer.stopped then return end
    timer.stopped = true
    fn()
  end)
  return timer
end
-- Appelle `fn` toutes les tant de secondes, en lui passant la minuterie (pour qu'elle
-- puisse s'arrêter d'elle-même).
function server.everySeconds(seconds, fn)
  if type(fn) ~= "function" then error("server.everySeconds(secondes, fonction) : la fonction manque", 2) end
  local timer = setmetatable({}, Timer)
  raw_every(ticks_of(seconds, "server.everySeconds"), function()
    if not timer.stopped then fn(timer) end
  end)
  return timer
end
-- Données enregistrées : n'importe quelle table (nombres, textes, vrai/faux, tables),
-- gardée dans plugins/<plugin>/<nom>.data et retrouvée après un redémarrage.
--   local points = server.loadData("points", {})
--   points["Steve"] = (points["Steve"] or 0) + 1
--   server.saveData("points", points)
local function serialize(v, indent, depth)
  depth = depth + 1
  if depth > 32 then error("server.saveData : données trop imbriquées", 4) end
  local t = type(v)
  if t == "string" then return string.format("%q", v) end
  if t == "number" then
    if v ~= v or v == math.huge or v == -math.huge then return "0" end
    if math.type(v) == "integer" then return tostring(v) end
    return string.format("%.17g", v)
  end
  if t == "boolean" then return tostring(v) end
  if t == "table" then
    local keys = {}
    for k in pairs(v) do
      if type(k) == "string" or type(k) == "number" then keys[#keys + 1] = k end
    end
    table.sort(keys, function(a, b)
      if type(a) == type(b) then return a < b end
      return type(a) == "number"
    end)
    local inner = indent .. "  "
    local parts = {}
    for _, k in ipairs(keys) do
      local x = v[k]
      local tx = type(x)
      if tx == "string" or tx == "number" or tx == "boolean" or tx == "table" then
        local key = type(k) == "string" and k:match("^[%a_][%w_]*$") and k or ("[" .. serialize(k, inner, depth) .. "]")
        parts[#parts + 1] = inner .. key .. " = " .. serialize(x, inner, depth)
      end
    end
    if #parts == 0 then return "{}" end
    return "{\n" .. table.concat(parts, ",\n") .. "\n" .. indent .. "}"
  end
  return "nil"
end

-- Nom de fichier des données : <nom>.data, nom limité aux lettres, chiffres, _ et -.
local function data_file(name, what)
  name = tostring(name or "")
  if not name:match("^[%w_%-]+$") then
    error(what .. " : nom invalide « " .. name .. " » (lettres, chiffres, _ et - seulement)", 3)
  end
  return name .. ".data"
end

-- Écrit la valeur en Lua (« return { ... } »), lisible et modifiable à la main.
function server.saveData(name, value)
  local file = data_file(name, "server.saveData")
  return server.write(file, "return " .. serialize(value, "", 0) .. "\n")
end

-- Relit la valeur enregistrée, ou `default` si rien n'existe ou si le fichier est abîmé.
function server.loadData(name, default)
  local file = data_file(name, "server.loadData")
  local text = server.read(file)
  if not text then return default end
  -- Lu sans aucun accès au serveur : un fichier de données ne contient que des données
  local chunk = real_load(text, "=" .. file, "t", {})
  if not chunk then
    server.warn(file .. " est illisible : valeurs par défaut utilisées")
    return default
  end
  local ok, value = pcall(chunk)
  if not ok or value == nil then return default end
  return value
end

-- config.yml du plugin : réglages que l'administrateur modifie sans toucher au code.
--   local config = server.getConfig({ message = "Bienvenue !", kit = true, diamants = 3 })
-- Le fichier est créé avec ces valeurs s'il n'existe pas ; les réglages ajoutés plus tard
-- y sont complétés. Forme reconnue : « clé: valeur » (texte, nombre, true/false, [liste]).
local function yaml_value(raw)
  raw = raw:gsub("%s+#.*$", ""):match("^%s*(.-)%s*$")
  if raw == "true" or raw == "yes" or raw == "on" then return true end
  if raw == "false" or raw == "no" or raw == "off" then return false end
  if raw:match("^%-?%d+%.?%d*$") then return tonumber(raw) end
  local quoted = raw:match('^"(.*)"$') or raw:match("^'(.*)'$")
  if quoted then return (quoted:gsub('\\"', '"')) end
  local list = raw:match("^%[(.*)%]$")
  if list then
    local out = {}
    for item in list:gmatch("[^,]+") do
      if item:match("%S") then out[#out + 1] = yaml_value(item) end
    end
    return out
  end
  return raw
end

-- Valeur -> texte YAML, pour écrire config.yml.
local function yaml_text(v)
  local t = type(v)
  if t == "boolean" or t == "number" then return tostring(v) end
  if t == "table" then
    local parts = {}
    for i, x in ipairs(v) do parts[i] = yaml_text(x) end
    return "[" .. table.concat(parts, ", ") .. "]"
  end
  return '"' .. tostring(v):gsub('"', '\\"') .. '"'
end

-- Lit config.yml et complète le fichier avec les réglages qui y manquent.
function server.getConfig(defaults)
  defaults = defaults or {}
  if type(defaults) ~= "table" then error("server.getConfig(valeurs par défaut) : une table est attendue", 2) end
  local values, order = {}, {}
  local text = server.read("config.yml")
  if text then
    for line in text:gmatch("[^\r\n]+") do
      local key, raw = line:match("^([%w_%-%.]+)%s*:%s*(.-)%s*$")
      if key and not line:match("^%s*#") then
        values[key] = yaml_value(raw)
        order[#order + 1] = key
      end
    end
  end
  -- Réglages manquants : ajoutés au fichier avec leur valeur par défaut
  local missing = {}
  for key, value in pairs(defaults) do
    if values[key] == nil then
      values[key] = value
      missing[#missing + 1] = key
    end
  end
  if #missing > 0 then
    table.sort(missing)
    local add = {}
    if not text then add[1] = "# Réglages du plugin : modifiez les valeurs, puis tapez « reload » dans la console." end
    for _, key in ipairs(missing) do add[#add + 1] = key .. ": " .. yaml_text(defaults[key]) end
    local body = table.concat(add, "\n") .. "\n"
    if text and text ~= "" and text:sub(-1) ~= "\n" then body = "\n" .. body end
    server.append("config.yml", body)
  end
  return values
end

-- Événements de server.on : un nom inconnu est signalé (avec le nom le plus proche), au lieu
-- de ne jamais rien recevoir sans savoir pourquoi.
local KNOWN = {
  "enable", "disable", "connect", "login", "load", "join", "quit", "chat", "save", "block_break",
  "block_click", "death", "respawn", "disconnect",
}
-- Distance de Levenshtein, pour proposer l'événement au nom le plus proche.
local function distance(a, b)
  local d = {}
  for i = 0, #a do d[i] = { [0] = i } end
  for j = 0, #b do d[0][j] = j end
  for i = 1, #a do
    for j = 1, #b do
      local cost = a:sub(i, i) == b:sub(j, j) and 0 or 1
      d[i][j] = math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + cost)
    end
  end
  return d[#a][#b]
end
local raw_on = server.on
-- server.on avec deux aides : un nom inconnu est signalé, et « quit » donne l'objet joueur
-- au lieu de l'adresse.
function server.on(name, fn)
  if type(name) == "string" and not name:find("#", 1, true) then
    local known = false
    for _, k in ipairs(KNOWN) do
      if k == name then known = true end
    end
    if not known then
      local best, score = nil, 99
      for _, k in ipairs(KNOWN) do
        local s = distance(name:lower(), k)
        if s < score then best, score = k, s end
      end
      local hint = (best and score <= 3) and (" (vouliez-vous dire « " .. best .. " » ?)") or ""
      server.warn("événement inconnu « " .. name .. " »" .. hint .. " : le serveur ne l'émet pas, cette fonction ne sera pas appelée")
    end
    -- « quit » : le joueur est parti ; la fonction reçoit l'objet joueur
    if name == "quit" then
      if type(fn) ~= "function" then return raw_on(name, fn) end
      return raw_on("disconnect", function(_, who)
        if who then return fn(wrap(who)) end
      end)
    end
  end
  return raw_on(name, fn)
end


-- ---------------------------------------------------------------------------------------
-- Événements à la manière de PocketMine : écouteurs, priorités, annulation.
--
--   local Ecouteur = {}
--   function Ecouteur:onBlockBreak(event)           -- BlockBreakEvent
--     if event:getBlock():getName() == "minecraft:diamond_ore" then event:cancel() end
--   end
--   Ecouteur.priorities = { onBlockBreak = EventPriority.HIGH }   -- @priority
--   Ecouteur.handleCancelled = { onBlockBreak = true }            -- @handleCancelled
--   server.getPluginManager():registerEvents(Ecouteur)
--
-- Le serveur fait passer l'événement d'écouteur en écouteur (priorité LOWEST à MONITOR) ;
-- chacun peut le modifier ou l'annuler, et le serveur applique le résultat final.
-- ---------------------------------------------------------------------------------------

GameMode = { SURVIVAL = 0, CREATIVE = 1, ADVENTURE = 2, SPECTATOR = 3 }
EventPriority = { LOWEST = "LOWEST", LOW = "LOW", NORMAL = "NORMAL", HIGH = "HIGH", HIGHEST = "HIGHEST", MONITOR = "MONITOR" }

-- Champs de chaque événement : nom, type ("player", "block", "position", "drops", "item",
-- "string", "number", "boolean"), modifiable ou non. `cancellable` : peut être annulé.
local EVENTS = {
  PlayerLoginEvent = { cancellable = true, fields = { { "player", "player" }, { "kickMessage", "string", true } } },
  PlayerJoinEvent = { fields = { { "player", "player" }, { "joinMessage", "string", true } } },
  PlayerQuitEvent = { fields = { { "player", "player" }, { "quitMessage", "string", true }, { "quitReason", "string" } } },
  PlayerChatEvent = { cancellable = true, fields = { { "player", "player" }, { "message", "string", true } } },
  PlayerCommandPreprocessEvent = { cancellable = true, fields = { { "player", "player" }, { "message", "string", true } } },
  ServerCommandEvent = { cancellable = true, fields = { { "command", "string", true } } },
  BlockBreakEvent = { cancellable = true, fields = { { "player", "player" }, { "block", "block" }, { "drops", "drops", true } } },
  BlockPlaceEvent = { cancellable = true, fields = { { "player", "player" }, { "block", "block" } } },
  PlayerInteractEvent = { cancellable = true, fields = { { "player", "player" }, { "block", "block" }, { "action", "number" } } },
  EntityDamageEvent = { cancellable = true, fields = { { "entity", "player" }, { "cause", "string" }, { "baseDamage", "number", true }, { "damager", "player" } } },
  PlayerDeathEvent = { fields = { { "player", "player" }, { "deathMessage", "string", true }, { "keepInventory", "boolean", true } } },
  PlayerRespawnEvent = { fields = { { "player", "player" }, { "respawnPosition", "position", true } } },
  PlayerDropItemEvent = { cancellable = true, fields = { { "player", "player" }, { "item", "item" } } },
  EntityItemPickupEvent = { cancellable = true, fields = { { "entity", "player" }, { "item", "item" } } },
  PlayerGameModeChangeEvent = { cancellable = true, fields = { { "player", "player" }, { "newGamemode", "number" } } },
  PlayerItemUseEvent = { cancellable = true, fields = { { "player", "player" }, { "item", "item" } } },
  PlayerItemConsumeEvent = { cancellable = true, fields = { { "player", "player" }, { "item", "item" } } },
  PlayerExhaustEvent = { cancellable = true, fields = { { "player", "player" }, { "amount", "number" } } },
  PlayerExperienceChangeEvent = { cancellable = true, fields = { { "player", "player" }, { "amount", "number", true } } },
  PlayerBedEnterEvent = { cancellable = true, fields = { { "player", "player" }, { "bed", "block" } } },
  PlayerBedLeaveEvent = { fields = { { "player", "player" }, { "bed", "block" } } },
  LeavesDecayEvent = { cancellable = true, fields = { { "block", "block" } } },
  BlockFallEvent = { cancellable = true, fields = { { "block", "block" } } },
  BlockGrowEvent = { cancellable = true, fields = { { "block", "block" } } },
  CreatureSpawnEvent = { cancellable = true, fields = { { "entity", "creature" } } },
  CreatureDamageEvent = { cancellable = true, fields = { { "entity", "creature" }, { "damage", "number", true }, { "cause", "string" }, { "damager", "player" } } },
  CreatureDeathEvent = { fields = { { "entity", "creature" }, { "killer", "player" } } },
  ProjectileHitEvent = { cancellable = true, fields = { { "projectile", "string" }, { "x", "number" }, { "y", "number" }, { "z", "number" }, { "target", "string" }, { "player", "player" }, { "entity", "creature" } } },
  ExplosionEvent = { cancellable = true, fields = { { "x", "number" }, { "y", "number" }, { "z", "number" }, { "power", "number", true } } },
}
-- Événements « enfants » : un écouteur de EntityDamageByEntityEvent ne reçoit que les dégâts
-- causés par un autre joueur (et un écouteur de EntityDamageEvent reçoit tout).
local CHILDREN = { EntityDamageByEntityEvent = { parent = "EntityDamageEvent", when = function(d) return d.damager ~= nil end } }
EntityDamageEvent = { CAUSE_FALL = "FALL", CAUSE_VOID = "VOID", CAUSE_ENTITY_ATTACK = "ENTITY_ATTACK", CAUSE_SUICIDE = "SUICIDE", CAUSE_CUSTOM = "CUSTOM" }
PlayerInteractEvent = { LEFT_CLICK_BLOCK = 0, RIGHT_CLICK_BLOCK = 1 }

-- Petit encodeur JSON pour rendre l'état modifié au serveur
local function encode(v, depth)
  depth = (depth or 0) + 1
  if depth > 16 then return "null" end
  local t = type(v)
  if t == "nil" then return "null" end
  if t == "boolean" then return v and "true" or "false" end
  if t == "number" then
    if v ~= v or v == math.huge or v == -math.huge then return "null" end
    return string.format("%.17g", v)
  end
  if t == "string" then
    return '"' .. v:gsub('[%c"\\]', function(c)
      if c == '"' then return '\\"' elseif c == "\\" then return "\\\\" end
      return string.format("\\u%04x", c:byte())
    end) .. '"'
  end
  if t == "table" then
    local parts = {}
    if v[1] ~= nil or next(v) == nil then
      for i = 1, #v do parts[i] = encode(v[i], depth) end
      return "[" .. table.concat(parts, ",") .. "]"
    end
    for k, x in pairs(v) do
      if type(k) == "string" and type(x) ~= "function" then parts[#parts + 1] = encode(k, depth) .. ":" .. encode(x, depth) end
    end
    return "{" .. table.concat(parts, ",") .. "}"
  end
  return "null"
end

-- « message » -> « Message » (noms des méthodes get/set des événements).
local function capital(s) return s:sub(1, 1):upper() .. s:sub(2) end

-- Bloc vu par un écouteur
local Block = {}
Block.__index = Block
Block.__name = "Block"
-- Accès en lecture au bloc d'un événement.
function Block:getName() return self.name end
function Block:getPosition() return self.x, self.y, self.z end
function Block:getX() return self.x end
function Block:getY() return self.y end
function Block:getZ() return self.z end

-- ---------------------------------------------------------------------------------------
-- Le monde, comme World dans PocketMine : lire et poser des blocs.
--
--   local world = server.getWorld()
--   local bloc = world:getBlock(x, y, z)        -- bloc:getName(), bloc:getState()
--   world:setBlock(x, y, z, "minecraft:stone")  -- nom, état (entier) ou bloc
--
-- Un bloc est désigné par son nom (son état par défaut) ou par son « état » : un entier qui
-- distingue aussi l'orientation, la couleur… (server.blockState(nom), server.blockName(état)).
-- La lecture voit le monde au début de l'appel ; les blocs posés sont appliqués par le serveur
-- à la fin de l'appel, et visibles à l'appel suivant.
-- ---------------------------------------------------------------------------------------
local raw_block, raw_blocks, raw_setblocks = server._block, server._blocks, server._setblocks
-- État exact du bloc (orientation, couleur…), à garder pour le recopier à l'identique.
function Block:getState() return self.state end

-- Accepte un nom, un état ou un bloc, et renvoie l'état ; nil si le bloc n'existe pas.
local function state_of(block)
  local t = type(block)
  if t == "number" then return math.tointeger(block) end
  if t == "string" then return server.blockState(block) end
  if t == "table" and block.state then return block.state end
  if t == "table" and block.name then return server.blockState(block.name) end
  return nil
end

local World = {}
World.__index = World
World.__name = "World"
-- État seul, sans créer d'objet : le plus rapide pour lire beaucoup de blocs.
function World:getBlockState(x, y, z) return raw_block(x, y, z) end
-- Le bloc avec son nom et sa position, ou nil hors du monde.
function World:getBlock(x, y, z)
  x, y, z = math.floor(x), math.floor(y), math.floor(z)
  local state = raw_block(x, y, z)
  if not state then return nil end
  return setmetatable({ name = server.blockName(state), state = state, x = x, y = y, z = z }, Block)
end
-- États de tout un volume, dans une table plate rangée par y, puis x, puis z
function World:getBlocks(x1, y1, z1, x2, y2, z2) return raw_blocks(x1, y1, z1, x2, y2, z2) end
-- Pose un bloc (appliqué à la fin de l'appel du plugin).
function World:setBlock(x, y, z, block)
  local state = state_of(block)
  if not state then error("bloc inconnu : " .. tostring(block), 2) end
  raw_setblocks({ math.floor(x), math.floor(y), math.floor(z), state })
end
-- Pose en une fois : table plate { x, y, z, état, x, y, z, état, … } (1 000 000 blocs au plus)
function World:setBlocks(list) raw_setblocks(list) end
local the_world = setmetatable({}, World)
-- Il n'y a qu'un monde pour l'instant : toujours le même objet.
function server.getWorld() return the_world end
server._block, server._blocks, server._setblocks = nil, nil, nil

-- Créature (animal, villageois, créature créée par un plugin) telle qu'un événement la donne.
-- C'est un instantané : ses valeurs sont celles du moment de l'événement.
local Creature = {}
Creature.__index = Creature
Creature.__name = "Creature"
function Creature:getId() return self.id end
function Creature:getType() return self.type end
function Creature:getLooks() return self.looks end
function Creature:isCustom() return self.custom == true end
function Creature:isBaby() return self.baby == true end
function Creature:getPosition() return self.x, self.y, self.z end
function Creature:getHealth() return self.health end
function Creature:getMaxHealth() return self.maxHealth end
function Creature:getNameTag() return self.name end
function Creature:remove() modify("mob-act", string.format("%d", self.id), "remove", "") end
function Creature:kill() modify("mob-act", string.format("%d", self.id), "kill", "") end
function Creature:setHealth(value)
  if type(value) ~= "number" or value <= 0 then error("creature:setHealth(nombre positif)", 2) end
  modify("mob-act", string.format("%d", self.id), "health", tostring(value))
end
function Creature:setNameTag(text)
  if type(text) ~= "string" then error("creature:setNameTag(texte)", 2) end
  modify("mob-act", string.format("%d", self.id), "name", (text:gsub("[\t\r\n]", " ")))
end

local Event = {}
Event.__index = Event
Event.__name = "Event"
-- Méthodes communes à tous les événements.
function Event:getEventName() return self._name end
function Event:isCancellable() return self._spec.cancellable == true end
function Event:isCancelled() return self._data.cancelled == true end
-- Erreur si l'événement ne s'annule pas (PlayerJoinEvent par exemple).
function Event:cancel()
  if not self._spec.cancellable then error(self._name .. " ne peut pas être annulé", 2) end
  self._data.cancelled = true
end
-- `setCancelled(false)` revient à `uncancel()`, toute autre valeur annule.
function Event:uncancel() self._data.cancelled = false end
function Event:setCancelled(value)
  if value == false then self:uncancel() else self:cancel() end
end

-- Méthodes get/set de chaque événement, d'après la liste des champs
local CLASSES = {}
for name, spec in pairs(EVENTS) do
  local class = setmetatable({}, { __index = Event })
  class.__index = class
  class.__name = name
  for _, f in ipairs(spec.fields) do
    local field, kind, writable = f[1], f[2], f[3]
    class["get" .. capital(field)] = function(self)
      local v = self._data[field]
      if v == nil then return nil end
      if kind == "player" then return wrap(v) end
      if kind == "creature" then
        local copy = {}
        for k, value in pairs(v) do copy[k] = value end
        return setmetatable(copy, Creature)
      end
      if kind == "block" then return setmetatable({ name = v.name, x = v.x, y = v.y, z = v.z }, Block) end
      if kind == "position" then return v.x, v.y, v.z end
      if kind == "drops" or kind == "item" then
        -- copie : la modifier ne change rien tant qu'elle n'est pas rendue par set…
        local copy = {}
        if kind == "item" then return { name = v.name, count = v.count } end
        for i, d in ipairs(v) do copy[i] = { name = d.name, count = d.count } end
        return copy
      end
      return v
    end
    if writable then
      class["set" .. capital(field)] = function(self, a, b, c)
        if self._priority == "MONITOR" then error("un écouteur MONITOR ne doit rien modifier", 2) end
        if kind == "string" then self._data[field] = tostring(a)
        elseif kind == "number" then
          a = tonumber(a)
          if not a or a ~= a then error("nombre attendu", 2) end
          self._data[field] = a
        elseif kind == "boolean" then self._data[field] = (a == true)
        elseif kind == "position" then
          a, b, c = tonumber(a), tonumber(b), tonumber(c)
          if not (a and b and c) then error("position attendue : x, y, z", 2) end
          self._data[field] = { x = a, y = b, z = c }
        elseif kind == "drops" then
          local list = {}
          for i, d in ipairs(a or {}) do
            if i > 64 then break end
            list[i] = { name = tostring(d.name or d[1]), count = math.max(1, math.min(64, math.floor(tonumber(d.count or d[2]) or 1))) }
          end
          self._data[field] = list
        end
      end
    end
  end
  -- Noms à la PocketMine pour les plus courants
  if spec.fields[1][1] == "entity" and spec.fields[1][2] == "player" then class.getPlayer = class.getEntity end
  if name == "EntityDamageEvent" then
    class.getDamage = class.getBaseDamage
    class.setDamage = class.setBaseDamage
    class.getFinalDamage = class.getBaseDamage
  end
  CLASSES[name] = class
end

-- Enregistre une fonction pour un événement. `priority` : EventPriority (NORMAL par défaut) ;
-- `handleCancelled` : reçoit aussi les événements déjà annulés (non par défaut).
function server.registerEvent(name, fn, priority, handleCancelled)
  name = tostring(name)
  local target, when = name, nil
  if CHILDREN[name] then target, when = CHILDREN[name].parent, CHILDREN[name].when end
  local spec = EVENTS[target]
  if not spec then error("événement inconnu : " .. name, 2) end
  if type(fn) ~= "function" then error("fonction attendue pour " .. name, 2) end
  priority = priority or "NORMAL"
  if not EventPriority[priority] then error("priorité inconnue : " .. tostring(priority), 2) end
  server.on(target .. "#" .. priority, function(data)
    if type(data) ~= "table" then return nil end
    if data.cancelled and not handleCancelled then return nil end
    if when and not when(data) then return nil end
    local event = setmetatable({ _name = name, _spec = spec, _data = data, _priority = priority }, CLASSES[target])
    fn(event)
    if priority == "MONITOR" then return nil end
    return encode(data)
  end)
end

-- Enregistre toutes les méthodes d'un écouteur : `onPlayerJoin` (ou `PlayerJoinEvent`) reçoit
-- PlayerJoinEvent, etc. `listener.priorities` et `listener.handleCancelled` règlent chaque
-- méthode, comme les annotations @priority et @handleCancelled de PocketMine.
function server.registerEvents(listener)
  if type(listener) ~= "table" then error("écouteur attendu (une table de méthodes)", 2) end
  local count = 0
  for key, fn in pairs(listener) do
    if type(key) == "string" and type(fn) == "function" then
      local name = key
      if key:match("^on%u") then name = key:sub(3) .. "Event" end
      if EVENTS[name] or CHILDREN[name] then
        local prio = listener.priorities and listener.priorities[key]
        local cancelled = listener.handleCancelled and listener.handleCancelled[key]
        server.registerEvent(name, function(event) return fn(listener, event) end, prio, cancelled)
        count = count + 1
      end
    end
  end
  return count
end


if not protected then return end

-- À partir d'ici : plugin protégé.

-- Chargement des propres fichiers du plugin : require("config") ou
-- require("plugins.<nom>.config"), texte Lua uniquement.
local loaded = {}
local standard = { string = string, table = table, math = math, utf8 = utf8, coroutine = coroutine }
-- require limité aux fichiers du plugin (et aux bibliothèques standard autorisées).
local function own_require(name)
  name = tostring(name)
  if standard[name] then return standard[name] end
  if loaded[name] ~= nil then return loaded[name] end
  if not name:match("^[%w_%-%.]+$") or name:find("..", 1, true) then
    error("basalt: require refusé : " .. name, 2)
  end
  local path = name:gsub("%.", "/") .. ".lua"
  if path:sub(1, #dir) ~= dir then path = dir .. path end
  local f, err = real_loadfile(path, "t", _G)
  if not f then error("basalt: require impossible hors du dossier du plugin (" .. tostring(err) .. ")", 2) end
  local result = f(name)
  if result == nil then result = true end
  loaded[name] = result
  return result
end

-- Les dépassements de limites ne peuvent pas être interceptés par le plugin
local real_pcall, real_xpcall, real_error, real_resume = pcall, xpcall, error, coroutine.resume
local pack, unpack = table.pack, table.unpack
-- Erreur levée par une limite du serveur (temps, mémoire) ?
local function limit(e)
  return type(e) == "string" and e:sub(1, 15) == "basalt: limite "
end
-- pcall qui laisse passer les erreurs de limite : un plugin ne peut pas les étouffer.
local function safe_pcall(f, ...)
  local r = pack(real_pcall(f, ...))
  if not r[1] and limit(r[2]) then real_error(r[2], 0) end
  return unpack(r, 1, r.n)
end
-- Même chose pour xpcall : le gestionnaire d'erreur ne voit pas les erreurs de limite.
local function safe_xpcall(f, handler, ...)
  local r = pack(real_xpcall(f, function(e)
    if limit(e) then return e end
    return handler(e)
  end, ...))
  if not r[1] and limit(r[2]) then real_error(r[2], 0) end
  return unpack(r, 1, r.n)
end
coroutine.resume = function(co, ...)
  local r = pack(real_resume(co, ...))
  if not r[1] and limit(r[2]) then real_error(r[2], 0) end
  return unpack(r, 1, r.n)
end
string.dump = nil

-- Liste blanche : tout ce qui n'est pas ici est retiré
local keep = {
  server = true, string = true, table = true, math = true, utf8 = true, coroutine = true,
  pairs = true, ipairs = true, next = true, select = true, type = true,
  tostring = true, tonumber = true, error = true, assert = true,
  setmetatable = true, getmetatable = true,
  rawget = true, rawset = true, rawequal = true, rawlen = true,
  _G = true, _VERSION = true,
  EventPriority = true, EntityDamageEvent = true, PlayerInteractEvent = true, GameMode = true, TextFormat = true,
}
local names = {}
for k in pairs(_G) do names[#names + 1] = k end
for _, k in ipairs(names) do
  if not keep[k] then _G[k] = nil end
end
pcall = safe_pcall
xpcall = safe_xpcall
require = own_require
print = function(...)
  local parts = {}
  for i = 1, select("#", ...) do parts[i] = tostring((select(i, ...))) end
  server.log(table.concat(parts, "\t"))
end
