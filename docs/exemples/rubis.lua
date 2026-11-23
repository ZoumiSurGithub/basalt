-- Objets nouveaux : un rubis, un plat et une arme.
-- À copier dans plugins/ ; voir « Créer des objets » dans docs/PLUGINS.md.

server.items.create("exemple:rubis", { name = "Rubis", icon = "emerald", maxStack = 16, glint = true, category = "nature", group = "Gemmes" })
server.items.create("exemple:saphir", { name = "Saphir", icon = "diamond", maxStack = 16, category = "nature", group = "Gemmes" })

server.items.create("exemple:ragout_du_chef", {
  name = "Ragoût du chef",
  icon = "rabbit_stew",
  food = 8,
  saturation = 12,
  alwaysEdible = true,
  leftover = "minecraft:bowl",
})

server.items.create("exemple:lame_de_rubis", { name = "Lame de rubis", icon = "netherite_sword", attackDamage = 12, durability = 300 })

-- Chaque joueur reçoit les trois en arrivant
local Cadeau = {}
function Cadeau:onPlayerJoin(event)
  local joueur = event:getPlayer()
  joueur:give("exemple:rubis", 5)
  joueur:give("exemple:ragout_du_chef", 2)
  joueur:give("exemple:lame_de_rubis", 1)
end
server.registerEvents(Cadeau)

-- Un clic dans le vide avec le saphir : dix blocs plus haut
local Saphir = {}
function Saphir:onPlayerItemUse(event)
  if event:getItem().name == "exemple:saphir" then
    local joueur = event:getPlayer()
    local x, y, z = joueur:getPosition()
    joueur:teleport(x, y + 10, z)
  end
end
server.registerEvents(Saphir)
