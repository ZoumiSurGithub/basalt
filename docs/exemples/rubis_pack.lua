-- Objets avec leurs propres images, fournies par un pack de ressources.
-- 1. Copiez docs/exemples/pack-exemple.mcpack dans resource_packs/ (à côté du serveur).
-- 2. Dans resource_packs/resource_packs.yml, ajoutez « - pack-exemple.mcpack » sous resource_stack.
-- 3. Copiez ce fichier dans plugins/.
-- Voir docs/PACKS.md.

server.items.create("exemple:rubis_taille", { name = "Rubis taillé", icon = "exemple_rubis", maxStack = 16, category = "nature", group = "Gemmes" })
server.items.create("exemple:lame_ecarlate", { name = "Lame écarlate", icon = "exemple_lame", attackDamage = 12, durability = 300 })

-- Deux blocs : un minerai qui rend des rubis, un bloc lumineux
server.blocks.create("exemple:minerai_rubis", {
  name = "Minerai de rubis",
  texture = "exemple_minerai_rubis",
  hardness = 3,
  tool = "pickaxe",
  tier = "iron",          -- sans pioche en fer au moins, il ne rend rien
  xp = 3,
  xpMax = 7,
  category = "nature",
  drops = { ["exemple:rubis_taille"] = 1 },
})
server.blocks.create("exemple:bloc_rubis", { name = "Bloc de rubis", texture = "exemple_bloc_rubis", hardness = 5, light = 10 })

-- Blocs à états
-- Un tronc : couché selon la face où on le pose, comme une bûche
server.blocks.create("exemple:tronc_rouge", {
  name = "Tronc rouge", texture = "exemple_tronc_cote", textureTop = "exemple_tronc_dessus", textureBottom = "exemple_tronc_dessus",
  hardness = 2, tool = "axe", rotation = "axis", category = "nature",
})
-- Un fourneau : sa face avant regarde celui qui le pose
server.blocks.create("exemple:fourneau_rouge", {
  name = "Fourneau rouge", texture = "exemple_tronc_cote", textureFront = "exemple_four_avant",
  hardness = 3, tool = "pickaxe", rotation = "facing",
})
-- Une culture : trois stades, mûre elle rend des rubis ; avant, seulement de quoi la replanter
server.blocks.create("exemple:baie_rubis", {
  name = "Baie de rubis", stages = { "exemple_baie_1", "exemple_baie_2", "exemple_baie_3" },
  geometry = "minecraft:geometry.cross", transparent = true, solid = false, hardness = 0,
  needsFloor = true, category = "nature",
  drops = { ["exemple:rubis_taille"] = 2, ["exemple:baie_rubis"] = 1 },
})

-- Une créature avec sa propre apparence : le pack la définit sous le même nom
-- (entity/exemple_gardien.entity.json, son modèle et son image)
server.entities.create("exemple:gardien_rubis", {
  looks = "minecraft:iron_golem",   -- elle se comporte comme un golem de fer
  ownModel = true,                  -- mais son apparence vient du pack
  name = "Gardien de rubis",
  health = 80,
  width = 0.9, height = 2,
  drops = { ["exemple:rubis_taille"] = { 2, 4 } },
})

local Cadeau = {}
function Cadeau:onPlayerJoin(event)
  local joueur = event:getPlayer()
  joueur:give("exemple:rubis_taille", 5)
  joueur:give("exemple:lame_ecarlate", 1)
  joueur:give("exemple:minerai_rubis", 16)
  joueur:give("exemple:bloc_rubis", 16)
  joueur:give("exemple:tronc_rouge", 16)
  joueur:give("exemple:fourneau_rouge", 16)
  joueur:give("exemple:baie_rubis", 16)
  local x, y, z = joueur:getPosition()
  server.entities.spawn("exemple:gardien_rubis", x + 3, y, z)
end
server.registerEvents(Cadeau)
