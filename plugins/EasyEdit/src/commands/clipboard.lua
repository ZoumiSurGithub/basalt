-- /copy, /cut, /paste : presse-papiers de chaque joueur, gardé par rapport à sa position
local positions = require("src.positions")
local selection = require("src.selection")
local editor = require("src.editor")
local command = require("src.commands.util").command

local world = server.getWorld()
local AIR = server.blockState("minecraft:air")
local clipboards = {} -- pseudo -> { rel = positions relatives rangées, states = états }

-- Copie la sélection, positions relatives aux pieds du joueur (pour /paste).
local function copy(player, r)
  local px, py, pz = positions.feet(player)
  local states = world:getBlocks(r.x1, r.y1, r.z1, r.x2, r.y2, r.z2)
  local rel, sts, n = {}, {}, 0
  for y = r.y1, r.y2 do
    for x = r.x1, r.x2 do
      for z = r.z1, r.z2 do
        n = n + 1
        rel[n], sts[n] = positions.pack(x - px, y - py, z - pz), states[n]
      end
    end
  end
  clipboards[player:getName()] = { rel = rel, states = sts }
  return n
end

command("copy", "/copy", function(player)
  local r = selection.region(player)
  if not r then return end
  player:sendMessage("§a" .. copy(player, r) .. " blocs copiés.")
end)

command("cut", "/cut", function(player)
  local r = selection.region(player)
  if not r then return end
  copy(player, r)
  local keys, news = editor.fill(r, nil, function() return AIR end)
  editor.enqueue(player, "Couper", keys, news)
end)

command("paste", "/paste [a]   (a : sans les vides)", function(player, args)
  local clip = clipboards[player:getName()]
  if not clip then
    player:sendMessage("§cRien n'a été copié : utilisez d'abord /copy ou /cut.")
    return
  end
  local skip_air = args[1] == "a" or args[1] == "-a"
  local px, py, pz = positions.feet(player)
  local keys, news, n = {}, {}, 0
  for i, k in ipairs(clip.rel) do
    local s = clip.states[i]
    if not (skip_air and s == AIR) then
      local dx, dy, dz = positions.unpack(k)
      n = n + 1
      keys[n], news[n] = positions.pack(px + dx, py + dy, pz + dz), s
    end
  end
  editor.enqueue(player, "Collage", keys, news)
end)
