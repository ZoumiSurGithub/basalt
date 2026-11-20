-- /set, /replace, /walls, /count
local selection = require("src.selection")
local pattern = require("src.pattern")
local editor = require("src.editor")
local command = require("src.commands.util").command

local world = server.getWorld()
local AIR = server.blockState("minecraft:air")

command("set", "/set <bloc>", function(player, args)
  if not args[1] then return false end
  local r, choose = selection.region(player), pattern(player, args[1])
  if not (r and choose) then return end
  local keys, news = editor.fill(r, nil, choose)
  editor.enqueue(player, "Remplissage", keys, news)
end)

command("replace", "/replace <ancien> <nouveau>", function(player, args)
  if not (args[1] and args[2]) then return false end
  local r, choose = selection.region(player), pattern(player, args[2])
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
  local keys, news = editor.fill(r, function()
    i = i + 1
    return server.blockName(states[i]) == from_name
  end, choose)
  editor.enqueue(player, "Remplacement", keys, news)
end)

command("walls", "/walls <bloc>", function(player, args)
  if not args[1] then return false end
  local r, choose = selection.region(player), pattern(player, args[1])
  if not (r and choose) then return end
  local keys, news = editor.fill(r, function(x, _, z)
    return x == r.x1 or x == r.x2 or z == r.z1 or z == r.z2
  end, choose)
  editor.enqueue(player, "Murs", keys, news)
end)

command("count", "/count [bloc]", function(player, args)
  local r = selection.region(player)
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
