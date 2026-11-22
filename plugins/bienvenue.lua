-- Plugin d'exemple pour Basalt.
-- Placez vos plugins dans ce dossier : un fichier .lua, ou un dossier contenant main.lua.
-- Documentation complète : docs/PLUGINS.md

server.on("enable", function()
  server.log("Bienvenue chargé (Basalt " .. server.version .. ", Bedrock " .. server.game_version .. ")")
end)

-- Appelé quand un joueur se connecte.
-- Renvoyer un texte (ou false) refuse la connexion avec ce message.
local bannis = { ["Griefer123"] = true }

server.on("login", function(joueur)
  server.log(joueur.name .. " arrive depuis " .. joueur.address)
  if bannis[joueur.name] then
    return "Vous êtes banni de ce serveur."
  end
end)

-- Appelé quand le joueur est apparu dans le monde. `joueur` est un objet :
-- vie, expérience, position, noms… (voir « L'objet joueur » dans docs/PLUGINS.md)
server.on("join", function(joueur)
  server.log(joueur .. " est en jeu (" .. #server.getOnlinePlayers() .. " connecté(s))")
  joueur:sendMessage("Bienvenue " .. joueur:getDisplayName() .. " ! Vie : " .. joueur:getHealth() .. "/" .. joueur:getMaxHealth())
end)

server.on("disconnect", function(adresse, nom)
  if nom then server.log(nom .. " est parti") end
end)

-- Commande utilisable dans la console : « annonce <texte> »
server.command("annonce", "change le nom affiché du serveur", function(args)
  if #args == 0 then
    server.log("Nom actuel : " .. server.getMotd())
  else
    server.setMotd(table.concat(args, " "))
    server.log("Nouveau nom demandé : " .. table.concat(args, " "))
  end
end)

-- Minuterie : toutes les 5 minutes
server.everySeconds(300, function()
  server.log(#server.getOnlinePlayers() .. " joueur(s) connecté(s)")
end)
