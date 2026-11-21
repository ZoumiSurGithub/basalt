-- Message de bienvenue, et un petit kit pour les nouveaux joueurs.
-- À copier dans le dossier plugins du serveur, puis taper « reload » dans la console.

-- Les réglages : ils sont écrits dans plugins/bienvenue/config.yml, que l'on peut modifier
local config = server.getConfig({
  message = "Bienvenue sur le serveur !",
  kit = true,
})

-- Quand un joueur arrive dans le monde
server.on("join", function(joueur)
  joueur:sendMessage(TextFormat.GREEN .. config.message)
  server.broadcast(TextFormat.YELLOW .. joueur:getName() .. " vient d'arriver")
end)

-- Quand les données d'un joueur sont chargées : « nouveau » vaut true à sa première visite
server.on("load", function(joueur, nouveau)
  if nouveau and config.kit then
    joueur:give("minecraft:bread", 8)
    joueur:give("minecraft:stone_sword", 1)
    joueur:sendMessage(TextFormat.AQUA .. "Voici un petit kit pour bien commencer.")
  end
end)

-- Quand un joueur part
server.on("quit", function(joueur)
  server.broadcast(TextFormat.GRAY .. joueur:getName() .. " est parti")
end)
