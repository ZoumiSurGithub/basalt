-- EasyEdit pour Basalt.
--
-- Les commandes sont celles du jeu : /wand, /set stone…
--
-- Le plugin gère la sélection de chaque joueur et les commandes ; le travail sur les blocs
-- est fait par le serveur (joueur:edit), qui tient aussi l'historique pour annuler.

local WAND = "minecraft:wooden_axe"
local PERMISSION = "easyedit.use"

-- Sélection de chaque joueur : { p1 = {x, y, z}, p2 = {x, y, z} }
local selections = {}

local function selection(player)
  local name = player:getName()
  selections[name] = selections[name] or {}
  return selections[name]
end

-- Bloc où se tiennent les pieds du joueur (sa position est celle de ses yeux)
local function feet(player)
  local x, y, z = player:getPosition()
  return math.floor(x), math.floor(y - 1.62 + 0.01), math.floor(z)
end

local function describe(p)
  return p[1] .. ", " .. p[2] .. ", " .. p[3]
end

local function volume(sel)
  return (math.abs(sel.p1[1] - sel.p2[1]) + 1) * (math.abs(sel.p1[2] - sel.p2[2]) + 1) * (math.abs(sel.p1[3] - sel.p2[3]) + 1)
end

local function set_corner(player, which, x, y, z)
  local sel = selection(player)
  local corner = sel[which]
  if corner and corner[1] == x and corner[2] == y and corner[3] == z then
    return -- clic maintenu : même bloc, rien de neuf à dire
  end
  sel[which] = { x, y, z }
  local text = "§d" .. (which == "p1" and "Premier" or "Second") .. " coin : " .. describe(sel[which])
  if sel.p1 and sel.p2 then
    text = text .. " §7(" .. volume(sel) .. " blocs)"
  end
  player:sendMessage(text)
end

-- Les six coordonnées de la sélection, ou nil (avec un message) s'il manque un coin
local function corners(player)
  local sel = selection(player)
  if not (sel.p1 and sel.p2) then
    player:sendMessage("§cChoisissez d'abord deux coins : /wand puis casser et cliquer, ou /pos1 et /pos2.")
    return nil
  end
  return sel.p1[1], sel.p1[2], sel.p1[3], sel.p2[1], sel.p2[2], sel.p2[3]
end

local DIRECTIONS = {
  north = { 0, 0, -1 }, n = { 0, 0, -1 }, nord = { 0, 0, -1 },
  south = { 0, 0, 1 }, s = { 0, 0, 1 }, sud = { 0, 0, 1 },
  east = { 1, 0, 0 }, e = { 1, 0, 0 }, est = { 1, 0, 0 },
  west = { -1, 0, 0 }, w = { -1, 0, 0 }, ouest = { -1, 0, 0 }, o = { -1, 0, 0 },
  up = { 0, 1, 0 }, u = { 0, 1, 0 }, haut = { 0, 1, 0 }, h = { 0, 1, 0 },
  down = { 0, -1, 0 }, d = { 0, -1, 0 }, bas = { 0, -1, 0 }, b = { 0, -1, 0 },
}

-- Direction écrite par le joueur, sinon celle où il regarde
local function direction(player, word)
  if word then
    local d = DIRECTIONS[word:lower()]
    if not d then
      player:sendMessage("§cDirection inconnue : " .. word .. " (north, south, east, west, up, down)")
      return nil
    end
    return d[1], d[2], d[3]
  end
  return player:getDirection()
end

-- Enregistre une commande réservée aux joueurs
local function command(name, usage, handler)
  server.command(name, function(args, player)
    if not player then
      server.log("La commande " .. name .. " s'utilise en jeu.")
      return
    end
    if handler(player, args) == false then
      player:sendMessage("§cUsage : " .. usage)
    end
  end)
end

command("wand", "/wand", function(player)
  player:give(WAND, 1)
  player:sendMessage("§dBaguette reçue : cassez un bloc pour le premier coin, cliquez sur un bloc pour le second.")
end)

command("pos1", "/pos1", function(player)
  set_corner(player, "p1", feet(player))
end)

command("pos2", "/pos2", function(player)
  set_corner(player, "p2", feet(player))
end)

command("size", "/size", function(player)
  local sel = selection(player)
  if not (sel.p1 and sel.p2) then
    player:sendMessage("§7Aucune sélection complète.")
    return
  end
  player:sendMessage("§dSélection : " .. describe(sel.p1) .. " → " .. describe(sel.p2) .. " §7(" .. volume(sel) .. " blocs)")
end)

command("set", "/set <bloc>", function(player, args)
  if not args[1] then return false end
  local x1, y1, z1, x2, y2, z2 = corners(player)
  if x1 then player:edit("set", x1, y1, z1, x2, y2, z2, args[1]) end
end)

command("replace", "/replace <ancien> <nouveau>", function(player, args)
  if not (args[1] and args[2]) then return false end
  local x1, y1, z1, x2, y2, z2 = corners(player)
  if x1 then player:edit("replace", x1, y1, z1, x2, y2, z2, args[1], args[2]) end
end)

command("walls", "/walls <bloc>", function(player, args)
  if not args[1] then return false end
  local x1, y1, z1, x2, y2, z2 = corners(player)
  if x1 then player:edit("walls", x1, y1, z1, x2, y2, z2, args[1]) end
end)

local function sphere(hollow)
  return function(player, args)
    local radius = tonumber(args[2])
    if not (args[1] and radius) then return false end
    local x, y, z = feet(player)
    player:edit("sphere", x, y, z, radius, args[1], hollow)
  end
end
command("sphere", "/sphere <bloc> <rayon>", sphere(false))
command("hsphere", "/hsphere <bloc> <rayon>", sphere(true))

local function cylinder(hollow)
  return function(player, args)
    local radius = tonumber(args[2])
    if not (args[1] and radius) then return false end
    local x, y, z = feet(player)
    player:edit("cyl", x, y, z, radius, tonumber(args[3]) or 1, args[1], hollow)
  end
end
command("cyl", "/cyl <bloc> <rayon> [hauteur]", cylinder(false))
command("hcyl", "/hcyl <bloc> <rayon> [hauteur]", cylinder(true))

command("copy", "/copy", function(player)
  local x1, y1, z1, x2, y2, z2 = corners(player)
  if x1 then
    local x, y, z = feet(player)
    player:edit("copy", x1, y1, z1, x2, y2, z2, x, y, z)
  end
end)

command("cut", "/cut", function(player)
  local x1, y1, z1, x2, y2, z2 = corners(player)
  if x1 then
    local x, y, z = feet(player)
    player:edit("cut", x1, y1, z1, x2, y2, z2, x, y, z)
  end
end)

command("paste", "/paste [a]   (a : sans les vides)", function(player, args)
  local x, y, z = feet(player)
  player:edit("paste", x, y, z, args[1] == "a" or args[1] == "-a")
end)

-- //stack [fois] [direction] et //move [blocs] [direction] : les deux arguments sont libres
local function along(operation)
  return function(player, args)
    local amount, word = tonumber(args[1]), args[2]
    if args[1] and not amount then
      amount, word = tonumber(args[2]), args[1] -- « //stack up 3 » est accepté aussi
    end
    local x1, y1, z1, x2, y2, z2 = corners(player)
    if not x1 then return end
    local dx, dy, dz = direction(player, word)
    if dx then
      player:edit(operation, x1, y1, z1, x2, y2, z2, dx, dy, dz, amount or 1)
    end
  end
end
command("stack", "/stack [fois] [direction]", along("stack"))
command("move", "/move [blocs] [direction]", along("move"))

command("count", "/count [bloc]", function(player, args)
  local x1, y1, z1, x2, y2, z2 = corners(player)
  if x1 then
    if args[1] then
      player:edit("count", x1, y1, z1, x2, y2, z2, args[1])
    else
      player:edit("count", x1, y1, z1, x2, y2, z2)
    end
  end
end)

command("undo", "/undo", function(player) player:edit("undo") end)
command("redo", "/redo", function(player) player:edit("redo") end)

command("easyedit", "/easyedit", function(player)
  player:sendMessage("§d— EasyEdit " .. server.plugin.version .. " —")
  player:sendMessage("§7Sélection : §f/wand  /pos1  /pos2  /size")
  player:sendMessage("§7Blocs : §f/set  /replace  /walls  /count")
  player:sendMessage("§7Formes : §f/sphere  /hsphere  /cyl  /hcyl")
  player:sendMessage("§7Presse-papiers : §f/copy  /cut  /paste [a]")
  player:sendMessage("§7Déplacer : §f/stack  /move   §7Historique : §f/undo  /redo")
end)

-- La baguette : casser un bloc choisit le premier coin, cliquer dessus le second.
-- Répondre false empêche le bloc d'être cassé, ou un autre d'être posé.
local function wand_used(player)
  return player:getItemInHand() == WAND and player:hasPermission(PERMISSION)
end

server.on("block_break", function(player, x, y, z)
  if wand_used(player) then
    set_corner(player, "p1", x, y, z)
    return false
  end
end)

server.on("block_click", function(player, x, y, z)
  if wand_used(player) then
    set_corner(player, "p2", x, y, z)
    return false
  end
end)

server.on("disconnect", function(_, name)
  if name then selections[name] = nil end
end)
