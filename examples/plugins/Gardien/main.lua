-- Fichier principal, désigné par « main » dans plugin.yml.
local config = require("config")

server.on("enable", function()
  server.log(server.plugin.name .. " v" .. server.plugin.version .. " : " .. #config.bannis .. " pseudo(s) en liste noire")
end)

server.on("login", function(joueur)
  for _, banni in ipairs(config.bannis) do
    if joueur.name == banni then return "Vous êtes banni de ce serveur." end
  end
end)

server.on("join", function(joueur)
  if joueur:hasPermission("gardien.accueil") then
    joueur:sendMessage(config.accueil .. " " .. joueur:getDisplayName())
  end
end)

-- La description de la commande vient de plugin.yml
server.command("expulser", function(args)
  if #args == 0 then
    server.log("Usage : expulser <joueur> [raison]")
    return
  end
  local cible = server.getPlayer(args[1])
  if not cible then
    server.log(args[1] .. " n'est pas en jeu")
    return
  end
  cible:kick(args[2] or "Expulsé par un administrateur.")
end)
