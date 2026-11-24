-- Créatures nouvelles, à partir de celles du jeu.
-- À copier dans plugins/ ; voir « Créer des créatures » dans docs/PLUGINS.md.

-- Une grande vache qui rend de l'or
server.entities.create("exemple:vache_doree", {
  looks = "minecraft:cow",
  name = "Vache dorée",
  health = 40,
  scale = 1.5,
  drops = { ["minecraft:gold_ingot"] = { 1, 3 }, ["minecraft:leather"] = 1 },
  xp = 5,
  xpMax = 9,
})

-- Une petite poule très rapide
server.entities.create("exemple:poule_eclair", { looks = "minecraft:chicken", name = "Poule éclair", speed = 0.6, scale = 0.7, health = 2 })

-- Un loup géant, sans nom affiché, avec sa propre boîte de collision (2 blocs de large, 1,5 de haut)
server.entities.create("exemple:loup_alpha", { looks = "minecraft:wolf", health = 60, scale = 2, width = 2, height = 1.5 })

-- Un ogre hostile : une grande vache qui poursuit et frappe les joueurs en survie
server.entities.create("exemple:ogre", {
  looks = "minecraft:cow", name = "Ogre", scale = 1.8,
  health = 60, hostile = true, attackDamage = 6, sight = 20,
  drops = { ["minecraft:emerald"] = { 1, 2 } }, xp = 10,
})

-- Chaque joueur qui arrive trouve les trois premières à côté de lui ;
-- l'ogre, lui, se fait apparaître à la main : summon exemple:ogre
-- Un archer de glace : il tire des boules de neige, vite et de loin
server.entities.create("exemple:archer_glace", {
  looks = "minecraft:stray", name = "Archer de glace", health = 30,
  projectile = "minecraft:snowball", projectileDamage = 2, shootInterval = 0.8, shootRange = 24,
})

local Accueil = {}
function Accueil:onPlayerJoin(event)
  local x, y, z = event:getPlayer():getPosition()
  server.entities.spawn("exemple:vache_doree", x + 2, y, z)
  server.entities.spawn("exemple:poule_eclair", x - 2, y, z)
  server.entities.spawn("exemple:loup_alpha", x, y, z + 3)
end
server.registerEvents(Accueil)

-- Événements : la vache dorée affiche sa vie quand on la frappe, et sa mort est annoncée
local Suivi = {}

---@param event Event
function Suivi:onCreatureDamage(event)
  local creature = event:getEntity()
  if creature:getType() == "exemple:vache_doree" then
    local reste = math.max(0, math.floor(creature:getHealth() - event:getDamage()))
    creature:setNameTag("Vache dorée §c" .. reste .. " ♥")
  end
end

---@param event Event
function Suivi:onCreatureDeath(event)
  local joueur = event:getKiller()
  if event:getEntity():getType() == "exemple:vache_doree" and joueur then
    server.broadcast("§6" .. joueur:getName() .. " a abattu une vache dorée !")
  end
end

server.registerEvents(Suivi)
