-- La baguette : casser un bloc choisit le premier coin, cliquer dessus le second.
-- Annuler l'événement empêche le bloc d'être cassé, ou un autre d'être posé.
local config = require("src.config")
local selection = require("src.selection")

local Wand = {}

-- Le joueur tient-il la hache de sélection, avec le droit de s'en servir ?
local function wand_used(player)
  return player:getItemInHand() == config.WAND and player:hasPermission(config.PERMISSION)
end

-- Clic gauche avec la hache : premier coin (le bloc n'est pas cassé).
function Wand:onBlockBreak(event)
  local player = event:getPlayer()
  if wand_used(player) then
    local b = event:getBlock()
    selection.set_corner(player, "p1", b:getX(), b:getY(), b:getZ())
    event:cancel()
  end
end

-- Clic droit avec la hache : second coin.
function Wand:onPlayerInteract(event)
  local player = event:getPlayer()
  if wand_used(player) then
    local b = event:getBlock()
    selection.set_corner(player, "p2", b:getX(), b:getY(), b:getZ())
    event:cancel()
  end
end

-- Le joueur part : sa sélection est oubliée.
function Wand:onPlayerQuit(event)
  selection.forget(event:getPlayer():getName())
end

return Wand
