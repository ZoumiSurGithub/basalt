-- Outils communs aux commandes
local util = {}

-- Enregistre une commande réservée aux joueurs. Si `handler` renvoie false, l'usage est
-- rappelé au joueur.
function util.command(name, usage, handler)
  server.command(name, function(args, player)
    if not player then
      server.log("La commande " .. name .. " s'utilise en jeu.")
      return
    end
    if handler(player, args) == false then
      player:sendMessage("§cUsage : " .. usage)
    end
  end)
end

local DIRECTIONS = {
  north = { 0, 0, -1 }, n = { 0, 0, -1 }, nord = { 0, 0, -1 },
  south = { 0, 0, 1 }, s = { 0, 0, 1 }, sud = { 0, 0, 1 },
  east = { 1, 0, 0 }, e = { 1, 0, 0 }, est = { 1, 0, 0 },
  west = { -1, 0, 0 }, w = { -1, 0, 0 }, ouest = { -1, 0, 0 }, o = { -1, 0, 0 },
  up = { 0, 1, 0 }, u = { 0, 1, 0 }, haut = { 0, 1, 0 }, h = { 0, 1, 0 },
  down = { 0, -1, 0 }, d = { 0, -1, 0 }, bas = { 0, -1, 0 }, b = { 0, -1, 0 },
}

-- Direction écrite par le joueur, sinon celle où il regarde
function util.direction(player, word)
  if word then
    local d = DIRECTIONS[word:lower()]
    if not d then
      player:sendMessage("§cDirection inconnue : " .. word .. " (north, south, east, west, up, down)")
      return nil
    end
    return d[1], d[2], d[3]
  end
  return player:getDirection()
end

return util
