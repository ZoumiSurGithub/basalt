-- /sphere, /hsphere, /cyl, /hcyl : formes autour du joueur
local config = require("src.config")
local positions = require("src.positions")
local pattern = require("src.pattern")
local editor = require("src.editor")
local command = require("src.commands.util").command

-- /sphere et /hsphere (creuse).
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
    local cx, cy, cz = positions.feet(player)
    local r = { x1 = cx - radius, y1 = cy - radius, z1 = cz - radius, x2 = cx + radius, y2 = cy + radius, z2 = cz + radius }
    local outer, inner = (radius + 0.5) ^ 2, (radius - 0.5) ^ 2
    local keys, news = editor.fill(r, function(x, y, z)
      local d = (x - cx) ^ 2 + (y - cy) ^ 2 + (z - cz) ^ 2
      return d <= outer and (not hollow or d >= inner)
    end, choose)
    editor.enqueue(player, hollow and "Sphère creuse" or "Sphère", keys, news)
  end
end

-- /cyl et /hcyl (creux).
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
    local cx, cy, cz = positions.feet(player)
    local r = { x1 = cx - radius, y1 = cy, z1 = cz - radius, x2 = cx + radius, y2 = cy + height - 1, z2 = cz + radius }
    if (2 * radius + 1) ^ 2 * height > config.MAX_BLOCKS * 2 then
      player:sendMessage("§cCylindre trop grand.")
      return
    end
    local outer, inner = (radius + 0.5) ^ 2, (radius - 0.5) ^ 2
    local keys, news = editor.fill(r, function(x, _, z)
      local d = (x - cx) ^ 2 + (z - cz) ^ 2
      return d <= outer and (not hollow or d >= inner)
    end, choose)
    editor.enqueue(player, hollow and "Cylindre creux" or "Cylindre", keys, news)
  end
end

command("sphere", "/sphere <bloc> <rayon>", sphere(false))
command("hsphere", "/hsphere <bloc> <rayon>", sphere(true))
command("cyl", "/cyl <bloc> <rayon> [hauteur]", cylinder(false))
command("hcyl", "/hcyl <bloc> <rayon> [hauteur]", cylinder(true))
