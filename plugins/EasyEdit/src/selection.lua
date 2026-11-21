-- Sélection de chaque joueur : { p1 = {x, y, z}, p2 = {x, y, z} }
local config = require("src.config")
local positions = require("src.positions")

local selection = {}
local selections = {}

-- Sélection du joueur (créée vide au besoin).
function selection.of(player)
  local name = player:getName()
  selections[name] = selections[name] or {}
  return selections[name]
end

-- Oublie la sélection d'un joueur parti.
function selection.forget(name)
  selections[name] = nil
end

-- Nombre de blocs de la sélection.
function selection.volume(sel)
  return (math.abs(sel.p1[1] - sel.p2[1]) + 1) * (math.abs(sel.p1[2] - sel.p2[2]) + 1) * (math.abs(sel.p1[3] - sel.p2[3]) + 1)
end

-- Choisit un coin ("p1" ou "p2") et le dit au joueur
function selection.set_corner(player, which, x, y, z)
  local sel = selection.of(player)
  local corner = sel[which]
  if corner and corner[1] == x and corner[2] == y and corner[3] == z then
    return -- clic maintenu : même bloc, rien de neuf à dire
  end
  sel[which] = { x, y, z }
  local text = "§d" .. (which == "p1" and "Premier" or "Second") .. " coin : " .. positions.describe(sel[which])
  if sel.p1 and sel.p2 then
    text = text .. " §7(" .. selection.volume(sel) .. " blocs)"
  end
  player:sendMessage(text)
end

-- La sélection rangée (coins min et max), ou nil (avec un message)
function selection.region(player)
  local sel = selection.of(player)
  if not (sel.p1 and sel.p2) then
    player:sendMessage("§cChoisissez d'abord deux coins : /wand puis casser et cliquer, ou /pos1 et /pos2.")
    return nil
  end
  local r = {
    x1 = math.min(sel.p1[1], sel.p2[1]), y1 = math.min(sel.p1[2], sel.p2[2]), z1 = math.min(sel.p1[3], sel.p2[3]),
    x2 = math.max(sel.p1[1], sel.p2[1]), y2 = math.max(sel.p1[2], sel.p2[2]), z2 = math.max(sel.p1[3], sel.p2[3]),
  }
  r.volume = (r.x2 - r.x1 + 1) * (r.y2 - r.y1 + 1) * (r.z2 - r.z1 + 1)
  if r.volume > config.MAX_BLOCKS then
    player:sendMessage("§cSélection trop grande : " .. r.volume .. " blocs (maximum " .. config.MAX_BLOCKS .. ").")
    return nil
  end
  return r
end

return selection
