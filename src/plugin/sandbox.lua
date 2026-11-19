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
