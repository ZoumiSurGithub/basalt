-- La baguette : casser un bloc choisit le premier coin, cliquer dessus le second.
-- Annuler l'événement empêche le bloc d'être cassé, ou un autre d'être posé.
local config = require("src.config")
local selection = require("src.selection")

local Wand = {}

local function wand_used(player)
  return player:getItemInHand() == config.WAND and player:hasPermission(config.PERMISSION)
end

function Wand:onBlockBreak(event)
  local player = event:getPlayer()
  if wand_used(player) then
    local b = event:getBlock()
    selection.set_corner(player, "p1", b:getX(), b:getY(), b:getZ())
    event:cancel()
  end
end

function Wand:onPlayerInteract(event)
  local player = event:getPlayer()
  if wand_used(player) then
    local b = event:getBlock()
    selection.set_corner(player, "p2", b:getX(), b:getY(), b:getZ())
    event:cancel()
  end
end

function Wand:onPlayerQuit(event)
  selection.forget(event:getPlayer():getName())
end

return Wand
