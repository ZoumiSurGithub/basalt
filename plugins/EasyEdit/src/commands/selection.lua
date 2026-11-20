-- /wand, /pos1, /pos2, /size
local config = require("src.config")
local positions = require("src.positions")
local selection = require("src.selection")
local command = require("src.commands.util").command

command("wand", "/wand", function(player)
  player:give(config.WAND, 1)
  player:sendMessage("§dBaguette reçue : cassez un bloc pour le premier coin, cliquez sur un bloc pour le second.")
end)

command("pos1", "/pos1", function(player)
  selection.set_corner(player, "p1", positions.feet(player))
end)

command("pos2", "/pos2", function(player)
  selection.set_corner(player, "p2", positions.feet(player))
end)

command("size", "/size", function(player)
  local sel = selection.of(player)
  if not (sel.p1 and sel.p2) then
    player:sendMessage("§7Aucune sélection complète.")
    return
  end
  player:sendMessage("§dSélection : " .. positions.describe(sel.p1) .. " → " .. positions.describe(sel.p2) .. " §7(" .. selection.volume(sel) .. " blocs)")
end)
