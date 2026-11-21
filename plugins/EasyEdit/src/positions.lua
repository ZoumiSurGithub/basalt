-- Positions : x, y, z rangés dans un seul entier (moins de mémoire pour les gros volumes).
-- Sert aussi aux positions relatives du presse-papiers (même plage de valeurs).
local positions = {}

local XZ, YS, ZS = 33554432, 2048, 67108864 -- 2^25 : décalage de x et z ; plage de y ; 2^26

-- Position -> un seul nombre (clé de table), et retour.
function positions.pack(x, y, z)
  return ((x + XZ) * YS + (y + 1024)) * ZS + (z + XZ)
end

function positions.unpack(k)
  local z = k % ZS - XZ
  local t = k // ZS
  return t // YS - XZ, t % YS - 1024, z
end

-- Bloc où se tiennent les pieds du joueur
function positions.feet(player)
  local x, y, z = player:getPosition()
  return math.floor(x), math.floor(y + 0.01), math.floor(z)
end

-- « x, y, z » pour les messages.
function positions.describe(p)
  return p[1] .. ", " .. p[2] .. ", " .. p[3]
end

return positions
