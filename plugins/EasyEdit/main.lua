-- EasyEdit pour Basalt : édition du monde en volume, sur le modèle du plugin EasyEdit de
-- PocketMine. Les commandes sont celles du jeu : /wand, /set stone…
--
-- Tout est fait ici, dans le plugin, avec l'API du monde du serveur (server.getWorld()) :
-- sélection, motifs, formes, presse-papiers, historique. Les grosses opérations sont
-- appliquées par tranches, un peu à chaque tick, pour ne jamais bloquer le serveur.

local WAND = "minecraft:wooden_axe"
local PERMISSION = "easyedit.use"
-- Blocs touchés par une opération au plus
local MAX_BLOCKS = 250000
-- Blocs traités par tick (20 ticks par seconde)
local PER_TICK = 20000
-- Opérations que l'on peut annuler, par joueur, et blocs gardés au total dans l'historique
local HISTORY = 20
local HISTORY_BLOCKS = 400000

local world = server.getWorld()
local AIR = server.blockState("minecraft:air")

-- ---------------------------------------------------------------------------------------
-- Positions : x, y, z rangés dans un seul entier (moins de mémoire pour les gros volumes)
-- ---------------------------------------------------------------------------------------
local XZ, YS = 33554432, 2048 -- 2^25 : décalage de x et z ; plage de y
local function pack(x, y, z)
  return ((x + XZ) * YS + (y + 1024)) * 67108864 + (z + XZ)
end
local function unpack3(k)
  local z = k % 67108864 - XZ
  local t = k // 67108864
  return t // YS - XZ, t % YS - 1024, z
end

-- ---------------------------------------------------------------------------------------
-- Sélection de chaque joueur : { p1 = {x, y, z}, p2 = {x, y, z} }
-- ---------------------------------------------------------------------------------------
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

-- La sélection rangée (coins min et max), ou nil (avec un message)
local function region(player)
  local sel = selection(player)
  if not (sel.p1 and sel.p2) then
    player:sendMessage("§cChoisissez d'abord deux coins : /wand puis casser et cliquer, ou /pos1 et /pos2.")
    return nil
  end
  local r = {
    x1 = math.min(sel.p1[1], sel.p2[1]), y1 = math.min(sel.p1[2], sel.p2[2]), z1 = math.min(sel.p1[3], sel.p2[3]),
    x2 = math.max(sel.p1[1], sel.p2[1]), y2 = math.max(sel.p1[2], sel.p2[2]), z2 = math.max(sel.p1[3], sel.p2[3]),
  }
  r.volume = (r.x2 - r.x1 + 1) * (r.y2 - r.y1 + 1) * (r.z2 - r.z1 + 1)
  if r.volume > MAX_BLOCKS then
    player:sendMessage("§cSélection trop grande : " .. r.volume .. " blocs (maximum " .. MAX_BLOCKS .. ").")
    return nil
  end
  return r
end

-- ---------------------------------------------------------------------------------------
-- Motifs : « stone », « stone,dirt », « 70%stone,30%cobblestone »
-- ---------------------------------------------------------------------------------------
local function pattern(player, text)
  local choices, total = {}, 0
  for part in tostring(text):gmatch("[^,]+") do
    local weight, name = part:match("^%s*(%d+)%%(.-)%s*$")
    if not weight then weight, name = 1, part:match("^%s*(.-)%s*$") end
    local state = server.blockState(name)
    if not state then
      player:sendMessage("§cBloc inconnu : " .. name)
      return nil
    end
    total = total + tonumber(weight)
    choices[#choices + 1] = { total, state }
    if #choices >= 16 then break end
  end
  if #choices == 0 then
    player:sendMessage("§cAucun bloc indiqué.")
    return nil
  end
  if #choices == 1 then
    local only = choices[1][2]
    return function() return only end
  end
  return function()
    local r = math.random() * total
    for _, c in ipairs(choices) do
      if r < c[1] then return c[2] end
    end
    return choices[#choices][2]
  end
end

-- ---------------------------------------------------------------------------------------
-- Opérations : une liste de blocs à poser, appliquée par tranches. Seuls les blocs qui
-- changent vraiment sont gardés dans l'historique.
-- ---------------------------------------------------------------------------------------
local queue = {}
local histories = {} -- pseudo -> { undo = {…}, redo = {…} }

local function history(name)
  histories[name] = histories[name] or { undo = {}, redo = {} }
  return histories[name]
end

-- Nouvelle opération : `keys` (positions rangées) et `news` (états), ou `new` (le même état
-- partout). `kind` : "do" (nouvelle), "undo" ou "redo".
local function enqueue(player, label, keys, news, kind)
  if #keys == 0 then
    player:sendMessage("§70 bloc modifié.")
    return
  end
  if #keys > MAX_BLOCKS then
    player:sendMessage("§cTrop de blocs : " .. #keys .. " (maximum " .. MAX_BLOCKS .. ").")
    return
  end
  queue[#queue + 1] = {
    player = player:getName(), label = label, kind = kind or "do",
    keys = keys, news = news, i = 1,
    done_keys = {}, done_olds = {}, done_news = {},
  }
  if #queue > 1 or #keys > PER_TICK then
    player:sendMessage("§7" .. label .. " : " .. #keys .. " blocs, en cours…")
  end
end

local function remember(op)
  if #op.done_keys == 0 then return end
  local h = history(op.player)
  local entry = { keys = op.done_keys, olds = op.done_olds, news = op.done_news, label = op.label }
  if op.kind == "undo" then
    -- Refaire, c'est rejouer l'opération d'origine : avant = après l'annulation, et inversement
    entry.olds, entry.news = entry.news, entry.olds
    h.redo[#h.redo + 1] = entry
  else
    if op.kind == "do" then h.redo = {} end
    h.undo[#h.undo + 1] = entry
    if #h.undo > HISTORY then table.remove(h.undo, 1) end
  end
  -- Historique borné en nombre de blocs, tous joueurs confondus : les plus anciens partent
  local total = 0
  for _, hh in pairs(histories) do
    for _, e in ipairs(hh.undo) do total = total + #e.keys end
    for _, e in ipairs(hh.redo) do total = total + #e.keys end
  end
  while total > HISTORY_BLOCKS and #h.undo > 1 do
    total = total - #h.undo[1].keys
    table.remove(h.undo, 1)
  end
end

-- Une tranche de la première opération en attente, à chaque tick
server.every(1, function()
  local op = queue[1]
  if not op then return end
  local writes, w = {}, 0
  local last = math.min(op.i + PER_TICK - 1, #op.keys)
  local news, keys = op.news, op.keys
  for i = op.i, last do
    local k = keys[i]
    local x, y, z = unpack3(k)
    local new = news[i]
    local old = world:getBlockState(x, y, z)
    if old and old ~= new then
      local n = #op.done_keys + 1
      op.done_keys[n], op.done_olds[n], op.done_news[n] = k, old, new
      writes[w + 1], writes[w + 2], writes[w + 3], writes[w + 4] = x, y, z, new
      w = w + 4
    end
  end
  if w > 0 then world:setBlocks(writes) end
  op.i = last + 1
  if op.i > #op.keys then
    table.remove(queue, 1)
    remember(op)
    local who = server.getPlayer(op.player)
    if who then
      local verb = op.kind == "undo" and "annulés" or "modifiés"
      who:sendMessage("§a" .. op.label .. " : " .. #op.done_keys .. " blocs " .. verb .. ".")
    end
  end
end)

-- Liste des positions d'un volume (avec un filtre facultatif) et de leurs nouveaux états
local function fill(r, keep, choose)
  local keys, news, n = {}, {}, 0
  for y = r.y1, r.y2 do
    for x = r.x1, r.x2 do
      for z = r.z1, r.z2 do
        if not keep or keep(x, y, z) then
          n = n + 1
          keys[n], news[n] = pack(x, y, z), choose(x, y, z)
        end
      end
    end
  end
  return keys, news
end

-- ---------------------------------------------------------------------------------------
-- Commandes
-- ---------------------------------------------------------------------------------------
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
  local r, choose = region(player), pattern(player, args[1])
  if not (r and choose) then return end
  local keys, news = fill(r, nil, choose)
  enqueue(player, "Remplissage", keys, news)
end)

command("replace", "/replace <ancien> <nouveau>", function(player, args)
  if not (args[1] and args[2]) then return false end
  local r, choose = region(player), pattern(player, args[2])
  if not (r and choose) then return end
  local from = server.blockState(args[1])
  if not from then
    player:sendMessage("§cBloc inconnu : " .. args[1])
    return
  end
  -- Tous les états de l'ancien bloc (orientation, couleur…) sont remplacés
  local from_name = server.blockName(from)
  local states = world:getBlocks(r.x1, r.y1, r.z1, r.x2, r.y2, r.z2)
  local i = 0
  local keys, news = fill(r, function()
    i = i + 1
    return server.blockName(states[i]) == from_name
  end, choose)
  enqueue(player, "Remplacement", keys, news)
end)

command("walls", "/walls <bloc>", function(player, args)
  if not args[1] then return false end
  local r, choose = region(player), pattern(player, args[1])
  if not (r and choose) then return end
  local keys, news = fill(r, function(x, _, z)
    return x == r.x1 or x == r.x2 or z == r.z1 or z == r.z2
  end, choose)
  enqueue(player, "Murs", keys, news)
end)

local function sphere(hollow)
  return function(player, args)
    local radius = tonumber(args[2])
    if not (args[1] and radius) then return false end
    radius = math.floor(radius)
    if radius < 1 or radius > 38 then
      player:sendMessage("§cRayon de 1 à 38.")
      return
    end
    local choose = pattern(player, args[1])
    if not choose then return end
    local cx, cy, cz = feet(player)
    local r = { x1 = cx - radius, y1 = cy - radius, z1 = cz - radius, x2 = cx + radius, y2 = cy + radius, z2 = cz + radius }
    local outer, inner = (radius + 0.5) ^ 2, (radius - 0.5) ^ 2
    local keys, news = fill(r, function(x, y, z)
      local d = (x - cx) ^ 2 + (y - cy) ^ 2 + (z - cz) ^ 2
      return d <= outer and (not hollow or d >= inner)
    end, choose)
    enqueue(player, hollow and "Sphère creuse" or "Sphère", keys, news)
  end
end
command("sphere", "/sphere <bloc> <rayon>", sphere(false))
command("hsphere", "/hsphere <bloc> <rayon>", sphere(true))

local function cylinder(hollow)
  return function(player, args)
    local radius, height = tonumber(args[2]), tonumber(args[3]) or 1
    if not (args[1] and radius) then return false end
    radius, height = math.floor(radius), math.floor(height)
    if radius < 1 or radius > 150 or height < 1 or height > 384 then
      player:sendMessage("§cRayon de 1 à 150, hauteur de 1 à 384.")
      return
    end
    local choose = pattern(player, args[1])
    if not choose then return end
    local cx, cy, cz = feet(player)
    local r = { x1 = cx - radius, y1 = cy, z1 = cz - radius, x2 = cx + radius, y2 = cy + height - 1, z2 = cz + radius }
    if (2 * radius + 1) ^ 2 * height > MAX_BLOCKS * 2 then
      player:sendMessage("§cCylindre trop grand.")
      return
    end
    local outer, inner = (radius + 0.5) ^ 2, (radius - 0.5) ^ 2
    local keys, news = fill(r, function(x, _, z)
      local d = (x - cx) ^ 2 + (z - cz) ^ 2
      return d <= outer and (not hollow or d >= inner)
    end, choose)
    enqueue(player, hollow and "Cylindre creux" or "Cylindre", keys, news)
  end
end
command("cyl", "/cyl <bloc> <rayon> [hauteur]", cylinder(false))
command("hcyl", "/hcyl <bloc> <rayon> [hauteur]", cylinder(true))

-- Presse-papiers de chaque joueur : positions relatives (rangées) et états
local clipboards = {}
local RXZ, RY = 33554432, 2048
local function pack_rel(dx, dy, dz)
  return ((dx + RXZ) * RY + (dy + 1024)) * 67108864 + (dz + RXZ)
end
local function unpack_rel(k)
  local dz = k % 67108864 - RXZ
  local t = k // 67108864
  return t // RY - RXZ, t % RY - 1024, dz
end

local function copy(player, r)
  local px, py, pz = feet(player)
  local states = world:getBlocks(r.x1, r.y1, r.z1, r.x2, r.y2, r.z2)
  local rel, sts, n, i = {}, {}, 0, 0
  for y = r.y1, r.y2 do
    for x = r.x1, r.x2 do
      for z = r.z1, r.z2 do
        i = i + 1
        n = n + 1
        rel[n], sts[n] = pack_rel(x - px, y - py, z - pz), states[i]
      end
    end
  end
  clipboards[player:getName()] = { rel = rel, states = sts }
  return n
end

command("copy", "/copy", function(player)
  local r = region(player)
  if not r then return end
  player:sendMessage("§a" .. copy(player, r) .. " blocs copiés.")
end)

command("cut", "/cut", function(player)
  local r = region(player)
  if not r then return end
  copy(player, r)
  local keys, news = fill(r, nil, function() return AIR end)
  enqueue(player, "Couper", keys, news)
end)

command("paste", "/paste [a]   (a : sans les vides)", function(player, args)
  local clip = clipboards[player:getName()]
  if not clip then
    player:sendMessage("§cRien n'a été copié : utilisez d'abord /copy ou /cut.")
    return
  end
  local skip_air = args[1] == "a" or args[1] == "-a"
  local px, py, pz = feet(player)
  local keys, news, n = {}, {}, 0
  for i, k in ipairs(clip.rel) do
    local s = clip.states[i]
    if not (skip_air and s == AIR) then
      local dx, dy, dz = unpack_rel(k)
      n = n + 1
      keys[n], news[n] = pack(px + dx, py + dy, pz + dz), s
    end
  end
  enqueue(player, "Collage", keys, news)
end)

-- /stack [fois] [direction] et /move [blocs] [direction] : les deux arguments sont libres
local function along(operation)
  return function(player, args)
    local amount, word = tonumber(args[1]), args[2]
    if args[1] and not amount then
      amount, word = tonumber(args[2]), args[1] -- « /stack up 3 » est accepté aussi
    end
    amount = math.floor(amount or 1)
    if amount < 1 or amount > 64 then
      player:sendMessage("§cNombre de 1 à 64.")
      return
    end
    local r = region(player)
    if not r then return end
    local dx, dy, dz = direction(player, word)
    if not dx then return end
    local states = world:getBlocks(r.x1, r.y1, r.z1, r.x2, r.y2, r.z2)
    local keys, news, n = {}, {}, 0
    if operation == "stack" then
      -- Copies successives, collées les unes aux autres
      local sx, sy, sz = (r.x2 - r.x1 + 1) * dx, (r.y2 - r.y1 + 1) * dy, (r.z2 - r.z1 + 1) * dz
      if r.volume * amount > MAX_BLOCKS then
        player:sendMessage("§cTrop de blocs : " .. r.volume * amount .. " (maximum " .. MAX_BLOCKS .. ").")
        return
      end
      for t = 1, amount do
        local i = 0
        for y = r.y1, r.y2 do
          for x = r.x1, r.x2 do
            for z = r.z1, r.z2 do
              i = i + 1
              n = n + 1
              keys[n], news[n] = pack(x + sx * t, y + sy * t, z + sz * t), states[i]
            end
          end
        end
      end
      enqueue(player, "Répétition", keys, news)
    else
      -- Déplacement : la destination reçoit les blocs, le reste de l'origine devient de l'air
      local ox, oy, oz = dx * amount, dy * amount, dz * amount
      local target = {}
      local i = 0
      for y = r.y1, r.y2 do
        for x = r.x1, r.x2 do
          for z = r.z1, r.z2 do
            i = i + 1
            local k = pack(x + ox, y + oy, z + oz)
            target[k] = true
            n = n + 1
            keys[n], news[n] = k, states[i]
          end
        end
      end
      for y = r.y1, r.y2 do
        for x = r.x1, r.x2 do
          for z = r.z1, r.z2 do
            local k = pack(x, y, z)
            if not target[k] then
              n = n + 1
              keys[n], news[n] = k, AIR
            end
          end
        end
      end
      -- La sélection suit les blocs
      local sel = selection(player)
      sel.p1 = { sel.p1[1] + ox, sel.p1[2] + oy, sel.p1[3] + oz }
      sel.p2 = { sel.p2[1] + ox, sel.p2[2] + oy, sel.p2[3] + oz }
      enqueue(player, "Déplacement", keys, news)
    end
  end
end
command("stack", "/stack [fois] [direction]", along("stack"))
command("move", "/move [blocs] [direction]", along("move"))

command("count", "/count [bloc]", function(player, args)
  local r = region(player)
  if not r then return end
  local states = world:getBlocks(r.x1, r.y1, r.z1, r.x2, r.y2, r.z2)
  local count = 0
  if args[1] then
    local wanted = server.blockState(args[1])
    if not wanted then
      player:sendMessage("§cBloc inconnu : " .. args[1])
      return
    end
    local name = server.blockName(wanted)
    for _, s in ipairs(states) do
      if server.blockName(s) == name then count = count + 1 end
    end
  else
    for _, s in ipairs(states) do
      if s ~= AIR then count = count + 1 end
    end
  end
  player:sendMessage("§a" .. count .. " blocs.")
end)

local function replay(player, from_stack, use_olds, kind, label)
  local h = history(player:getName())
  local entry = table.remove(from_stack(h))
  if not entry then
    player:sendMessage("§c" .. (kind == "undo" and "Rien à annuler." or "Rien à refaire."))
    return
  end
  local keys, news = {}, {}
  -- Dans l'ordre inverse pour annuler, comme à l'origine pour refaire
  local n = #entry.keys
  for i = 1, n do
    local j = use_olds and (n - i + 1) or i
    keys[i], news[i] = entry.keys[j], use_olds and entry.olds[j] or entry.news[j]
  end
  enqueue(player, label, keys, news, kind)
end

command("undo", "/undo", function(player)
  replay(player, function(h) return h.undo end, true, "undo", "Annulation")
end)

command("redo", "/redo", function(player)
  replay(player, function(h) return h.redo end, false, "redo", "Rétablissement")
end)

command("easyedit", "/easyedit", function(player)
  player:sendMessage("§d— EasyEdit " .. server.plugin.version .. " —")
  player:sendMessage("§7Sélection : §f/wand  /pos1  /pos2  /size")
  player:sendMessage("§7Blocs : §f/set  /replace  /walls  /count")
  player:sendMessage("§7Formes : §f/sphere  /hsphere  /cyl  /hcyl")
  player:sendMessage("§7Presse-papiers : §f/copy  /cut  /paste [a]")
  player:sendMessage("§7Déplacer : §f/stack  /move   §7Historique : §f/undo  /redo")
end)

-- ---------------------------------------------------------------------------------------
-- La baguette : casser un bloc choisit le premier coin, cliquer dessus le second.
-- Annuler l'événement empêche le bloc d'être cassé, ou un autre d'être posé.
-- ---------------------------------------------------------------------------------------
local function wand_used(player)
  return player:getItemInHand() == WAND and player:hasPermission(PERMISSION)
end

local Wand = {}
function Wand:onBlockBreak(event)
  local player = event:getPlayer()
  if wand_used(player) then
    local b = event:getBlock()
    set_corner(player, "p1", b:getX(), b:getY(), b:getZ())
    event:cancel()
  end
end
function Wand:onPlayerInteract(event)
  local player = event:getPlayer()
  if wand_used(player) then
    local b = event:getBlock()
    set_corner(player, "p2", b:getX(), b:getY(), b:getZ())
    event:cancel()
  end
end
function Wand:onPlayerQuit(event)
  selections[event:getPlayer():getName()] = nil
end
server.getPluginManager():registerEvents(Wand)
