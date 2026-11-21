-- /setspawn (opérateurs) enregistre un point de rendez-vous, /spawn y emmène.

-- Le point est gardé dans un fichier : il survit aux redémarrages
local spawn = server.loadData("spawn", nil)

server.command("setspawn", function(args, joueur)
  if not joueur then
    server.log("À taper en jeu.")
    return
  end
  if not joueur:isOp() then
    joueur:sendMessage(TextFormat.RED .. "Réservé aux opérateurs.")
    return
  end
  local x, y, z = joueur:getPosition()
  spawn = { x = x, y = y, z = z }
  server.saveData("spawn", spawn)
  joueur:sendMessage(TextFormat.GREEN .. "Point de rendez-vous enregistré.")
end)

server.command("spawn", function(args, joueur)
  if not joueur then
    server.log("À taper en jeu.")
    return
  end
  if spawn == nil then
    joueur:sendMessage(TextFormat.RED .. "Aucun point de rendez-vous : un opérateur doit taper /setspawn.")
    return
  end
  joueur:teleport(spawn.x, spawn.y, spawn.z)
  joueur:sendMessage(TextFormat.GREEN .. "Téléporté au point de rendez-vous.")
end)
