-- /stack [fois] [direction] et /move [blocs] [direction] : les deux arguments sont libres
local config = require("src.config")
local positions = require("src.positions")
local selection = require("src.selection")
local editor = require("src.editor")
local util = require("src.commands.util")

local world = server.getWorld()
local AIR = server.blockState("minecraft:air")
local pack = positions.pack

-- Copies successives de la sélection, collées les unes aux autres
local function stack(player, r, states, amount, dx, dy, dz)
  if r.volume * amount > config.MAX_BLOCKS then
    player:sendMessage("§cTrop de blocs : " .. r.volume * amount .. " (maximum " .. config.MAX_BLOCKS .. ").")
    return
  end
  local sx, sy, sz = (r.x2 - r.x1 + 1) * dx, (r.y2 - r.y1 + 1) * dy, (r.z2 - r.z1 + 1) * dz
  local keys, news, n = {}, {}, 0
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
  editor.enqueue(player, "Répétition", keys, news)
end

-- Déplacement : la destination reçoit les blocs, le reste de l'origine devient de l'air,
-- et la sélection suit les blocs
local function move(player, r, states, amount, dx, dy, dz)
  local ox, oy, oz = dx * amount, dy * amount, dz * amount
  local keys, news, n = {}, {}, 0
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
  local sel = selection.of(player)
  sel.p1 = { sel.p1[1] + ox, sel.p1[2] + oy, sel.p1[3] + oz }
  sel.p2 = { sel.p2[1] + ox, sel.p2[2] + oy, sel.p2[3] + oz }
  editor.enqueue(player, "Déplacement", keys, news)
end

-- Commande qui agit dans la direction regardée (/move, /stack).
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
    local r = selection.region(player)
    if not r then return end
    local dx, dy, dz = util.direction(player, word)
    if not dx then return end
    local states = world:getBlocks(r.x1, r.y1, r.z1, r.x2, r.y2, r.z2)
    operation(player, r, states, amount, dx, dy, dz)
  end
end

util.command("stack", "/stack [fois] [direction]", along(stack))
util.command("move", "/move [blocs] [direction]", along(move))
