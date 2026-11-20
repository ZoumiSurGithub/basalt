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

local function put(name, text, mode)
  local path = inside(name)
  if not path then return false end
  local f = io_open(path, mode)
  if not f then return false end
  f:write(tostring(text))
  f:close()
  return true
end

function server.write(name, text) return put(name, text, "wb") end
function server.append(name, text) return put(name, text, "ab") end

-- Heure
function server.time() return os_time() end
function server.clock() return os_clock() end
function server.date(format, time)
  if format ~= nil and type(format) ~= "string" then return nil end
  return os_date(format or "%Y-%m-%d %H:%M:%S", time)
end

-- server.connect est un autre nom de server.on
server.connect = server.on

-- Objet joueur. Il ne contient que le pseudo : chaque lecture interroge le serveur, donc
-- un objet gardé dans une variable reste juste, et `isOnline` dit si le joueur est là.
local pget, pset, pdo, kick = server._pget, server._pset, server._pdo, server.kick
local Player = {}
Player.__index = Player
Player.__name = "Player"
Player.__tostring = function(p) return p.name end
Player.__concat = function(a, b) return tostring(a) .. tostring(b) end
Player.__eq = function(a, b) return a.name == b.name end

function Player:getName() return self.name end
function Player:isOnline() return pget(self.name, "online") == true end
function Player:getDisplayName() return pget(self.name, "display") or self.name end
function Player:setDisplayName(text) pset(self.name, "display", tostring(text)) end
function Player:getNameTag() return pget(self.name, "tag") or self.name end
function Player:setNameTag(text) pset(self.name, "tag", tostring(text)) end
function Player:getHealth() return pget(self.name, "health") end
function Player:setHealth(value) pset(self.name, "health", value) end
function Player:getMaxHealth() return pget(self.name, "max_health") end
function Player:setMaxHealth(value) pset(self.name, "max_health", value) end
function Player:getFood() return pget(self.name, "food") end
function Player:setFood(value) pset(self.name, "food", value) end
function Player:getXpLevel() return pget(self.name, "xp_level") end
function Player:setXpLevel(value) pset(self.name, "xp_level", value) end
function Player:getXpProgress() return pget(self.name, "xp_progress") end
function Player:setXpProgress(value) pset(self.name, "xp_progress", value) end
function Player:getPosition() return pget(self.name, "pos") end
function Player:teleport(x, y, z) pdo(self.name, "teleport", x, y, z) end
function Player:getFirstPlayed() return pget(self.name, "first_played") end
function Player:sendMessage(text) pdo(self.name, "message", tostring(text)) end
function Player:kick(reason) kick(self.name, reason) end
function Player:save() pdo(self.name, "save") end
-- Donne des objets : joueur:give("minecraft:diamond", 3)
function Player:give(item, count) pdo(self.name, "give", tostring(item), count or 1) end
-- Regard et objet tenu
function Player:getYaw() return pget(self.name, "yaw") or 0 end
function Player:getPitch() return pget(self.name, "pitch") or 0 end
function Player:getItemInHand() return pget(self.name, "held") or "" end
-- Mode de jeu : 0 survie, 1 créatif, 2 aventure, 3 spectateur (numéros de PocketMine)
function Player:getGamemode() return pget(self.name, "gamemode") or 0 end
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
function Player:edit(operation, ...) pdo(self.name, "edit", tostring(operation), ...) end
-- Permissions : réglage propre au joueur, sinon règle par défaut déclarée dans plugin.yml
function Player:isOp() return pget(self.name, "op") == true end
function Player:setOp(value) pset(self.name, "op", value == true) end
function Player:hasPermission(node) return pget(self.name, "perm", tostring(node)) == true end
function Player:setPermission(node, value)
  if value ~= nil then value = (value == true) end
  pset(self.name, "perm:" .. tostring(node):lower(), value)
end

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
function server.getOnlinePlayers()
  local list = {}
  for i, name in ipairs(server.players()) do list[i] = wrap(name) end
  return list
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

local function capital(s) return s:sub(1, 1):upper() .. s:sub(2) end

-- Bloc vu par un écouteur
local Block = {}
Block.__index = Block
Block.__name = "Block"
function Block:getName() return self.name end
function Block:getPosition() return self.x, self.y, self.z end
function Block:getX() return self.x end
function Block:getY() return self.y end
function Block:getZ() return self.z end

local Event = {}
Event.__index = Event
Event.__name = "Event"
function Event:getEventName() return self._name end
function Event:isCancellable() return self._spec.cancellable == true end
function Event:isCancelled() return self._data.cancelled == true end
function Event:cancel()
  if not self._spec.cancellable then error(self._name .. " ne peut pas être annulé", 2) end
  self._data.cancelled = true
end
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
  if spec.fields[1][1] == "entity" then class.getPlayer = class.getEntity end
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

local PluginManager = { registerEvents = function(_, listener) return server.registerEvents(listener) end,
  registerEvent = function(_, ...) return server.registerEvent(...) end }
function server.getPluginManager() return PluginManager end

if not protected then return end

-- À partir d'ici : plugin protégé.

-- Chargement des propres fichiers du plugin : require("config") ou
-- require("plugins.<nom>.config"), texte Lua uniquement.
local loaded = {}
local standard = { string = string, table = table, math = math, utf8 = utf8, coroutine = coroutine }
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
local function limit(e)
  return type(e) == "string" and e:sub(1, 15) == "basalt: limite "
end
local function safe_pcall(f, ...)
  local r = pack(real_pcall(f, ...))
  if not r[1] and limit(r[2]) then real_error(r[2], 0) end
  return unpack(r, 1, r.n)
end
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
  EventPriority = true, EntityDamageEvent = true, PlayerInteractEvent = true, GameMode = true,
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
