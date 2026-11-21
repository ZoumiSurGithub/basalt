-- Personne ne casse ni ne pose de bloc près du centre du monde, sauf les opérateurs.

local config = server.getConfig({
  rayon = 30,
  message = "Cette zone est protégée.",
})

-- Le bloc (x, z) est-il dans la zone protégée ?
local function protege(x, z)
  return math.abs(x) <= config.rayon and math.abs(z) <= config.rayon
end

-- Répondre false annule le geste du joueur
server.on("block_break", function(joueur, x, y, z, bloc)
  if protege(x, z) and not joueur:isOp() then
    joueur:sendMessage(TextFormat.RED .. config.message)
    return false
  end
end)

server.on("block_click", function(joueur, x, y, z, bloc)
  if protege(x, z) and not joueur:isOp() then
    return false
  end
end)
