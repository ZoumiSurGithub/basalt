-- /easyedit : liste des commandes
local command = require("src.commands.util").command

command("easyedit", "/easyedit", function(player)
  player:sendMessage("§d— EasyEdit " .. server.plugin.version .. " —")
  player:sendMessage("§7Sélection : §f/wand  /pos1  /pos2  /size")
  player:sendMessage("§7Blocs : §f/set  /replace  /walls  /count")
  player:sendMessage("§7Formes : §f/sphere  /hsphere  /cyl  /hcyl")
  player:sendMessage("§7Presse-papiers : §f/copy  /cut  /paste [a]")
  player:sendMessage("§7Déplacer : §f/stack  /move   §7Historique : §f/undo  /redo")
end)
