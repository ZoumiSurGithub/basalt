-- Objets avec leurs propres images, fournies par un pack de ressources.
-- 1. Copiez docs/exemples/pack-exemple.mcpack dans resource_packs/ (à côté du serveur).
-- 2. Dans resource_packs/resource_packs.yml, ajoutez « - pack-exemple.mcpack » sous resource_stack.
-- 3. Copiez ce fichier dans plugins/.
-- Voir docs/PACKS.md.

server.items.create("exemple:rubis_taille", { name = "Rubis taillé", icon = "exemple_rubis", maxStack = 16, category = "nature", group = "Gemmes" })
server.items.create("exemple:lame_ecarlate", { name = "Lame écarlate", icon = "exemple_lame", attackDamage = 12, durability = 300 })

local Cadeau = {}
function Cadeau:onPlayerJoin(event)
  local joueur = event:getPlayer()
  joueur:give("exemple:rubis_taille", 5)
  joueur:give("exemple:lame_ecarlate", 1)
end
server.registerEvents(Cadeau)
