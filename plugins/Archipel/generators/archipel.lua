-- Générateur « archipel » : des îles dans un océan, avec des palmiers et des phares.
--
-- Pour l'employer, dans server.properties (pour un nouveau monde) :
--   level-type=plugin:archipel
--
-- Un générateur renvoie une table de fonctions ; seule `terrain` est obligatoire.
-- Voir docs/PLUGINS.md, partie « Générateurs de terrain ».

-- Les blocs, résolus une fois pour toutes (plus rapide que de passer leur nom à chaque bloc)
local BEDROCK, STONE, DIRT = block("bedrock"), block("stone"), block("dirt")
local GRASS, SAND, WATER = block("grass_block"), block("sand"), block("water")
local LOG, LEAVES, HERB = block("jungle_log"), block("jungle_leaves"), block("short_grass")
local SEA = 62

-- Hauteur du sol : deux bruits superposés, un grand pour les îles, un petit pour le relief.
-- Une fonction pure de (x, z) : le même monde à chaque fois pour une graine donnée.
local function height(x, z)
  local n = noise(x / 160, z / 160) * 0.75 + noise(x / 40, z / 40) * 0.25
  return math.floor(SEA - 1 + n * 40)
end

local gen = {}

-- Utilisée par les structures (structure:height) pour se poser sur le sol, même hors du chunk
gen.height = height

-- Le terrain : chunk.x et chunk.z sont les coordonnées du chunk ; x et z vont de 0 à 15
function gen.terrain(chunk)
  chunk:fill(0, -64, 0, 15, -64, 15, BEDROCK)
  for x = 0, 15 do
    for z = 0, 15 do
      local h = height(chunk.x * 16 + x, chunk.z * 16 + z)
      chunk:fill(x, -63, z, x, h - 4, z, STONE)
      if h > SEA + 1 then
        chunk:fill(x, h - 3, z, x, h - 1, z, DIRT)
        chunk:set(x, h, z, GRASS)
        chunk:biome(x, z, "jungle")
      elseif h >= SEA then
        chunk:fill(x, h - 3, z, x, h, z, SAND)
        chunk:biome(x, z, "beach")
      else
        chunk:fill(x, h - 3, z, x, h, z, SAND)
        chunk:fill(x, h + 1, z, x, SEA, z, WATER)
        chunk:biome(x, z, h < SEA - 15 and "deep_ocean" or "warm_ocean")
      end
    end
  end
end

-- Les décorations : herbes et palmiers, avec le hasard du chunk (toujours le même)
local function palm(chunk, x, y, z)
  local top = y + chunk:random(5, 7)
  for dy = y + 1, top do chunk:set(x, dy, z, LOG) end
  chunk:set(x, top + 1, z, LEAVES)
  for _, d in ipairs({ { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }) do
    for i = 1, 2 do chunk:setAt(chunk.x * 16 + x + d[1] * i, top + 1 - (i - 1), chunk.z * 16 + z + d[2] * i, LEAVES) end
  end
end

function gen.decorate(chunk)
  for x = 0, 15 do
    for z = 0, 15 do
      local h = chunk:height(x, z)
      if chunk:get(x, h, z) == GRASS and chunk:random() < 0.2 then
        chunk:set(x, h + 1, z, HERB)
      end
    end
  end
  if chunk:random() < 0.5 then
    local x, z = chunk:random(3, 12), chunk:random(3, 12)
    local h = chunk:height(x, z)
    if chunk:get(x, h, z) == GRASS then palm(chunk, x, h, z) end
  end
end

-- Les structures : une par case d'une grille de 8 chunks (3 au moins entre deux).
-- `build` est appelée pour chaque chunk que la structure touche (`reach` chunks autour
-- de son départ) ; seuls les blocs du chunk en cours sont posés.
gen.structures = {
  {
    spacing = 8, separation = 3, salt = 7, reach = 1,
    -- Un phare, seulement sur la terre ferme
    check = function(s) return s:height(s.x, s.z) > SEA + 2 end,
    build = function(s)
      local y = s:height(s.x, s.z)
      s:fill(s.x - 2, y - 3, s.z - 2, s.x + 2, y, s.z + 2, "stone_bricks")
      for dy = 1, 14 do
        local color = (dy % 4 < 2) and "red_concrete" or "white_concrete"
        s:fill(s.x - 1, y + dy, s.z - 1, s.x + 1, y + dy, s.z + 1, color)
      end
      s:fill(s.x - 1, y + 15, s.z - 1, s.x + 1, y + 15, s.z + 1, "glass")
      s:set(s.x, y + 15, s.z, "sea_lantern")
      s:fill(s.x - 1, y + 16, s.z - 1, s.x + 1, y + 16, s.z + 1, "red_concrete")
    end,
  },
}

-- Point d'apparition : la première terre ferme en partant du centre
function gen.spawn()
  for r = 0, 4000, 16 do
    for _, p in ipairs({ { r, 0 }, { -r, 0 }, { 0, r }, { 0, -r } }) do
      local h = height(p[1], p[2])
      if h > SEA + 1 then return p[1], h + 1, p[2] end
    end
  end
  return 0, SEA + 1, 0
end

return gen
