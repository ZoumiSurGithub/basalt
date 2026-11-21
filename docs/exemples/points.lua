-- Un point par bloc cassé ; /points montre son score et le meilleur joueur.

local points = server.loadData("points", {})

server.on("block_break", function(joueur, x, y, z, bloc)
  local nom = joueur:getName()
  points[nom] = (points[nom] or 0) + 1
end)

-- Enregistre les scores toutes les minutes (et pas à chaque bloc : c'est plus léger)
server.everySeconds(60, function()
  server.saveData("points", points)
end)
server.on("disable", function()
  server.saveData("points", points)
end)

server.command("points", function(args, joueur)
  local meilleur, record = nil, 0
  for nom, score in pairs(points) do
    if score > record then meilleur, record = nom, score end
  end
  local texte = "Personne n'a encore de point."
  if meilleur then texte = "Meilleur joueur : " .. meilleur .. " avec " .. record .. " points." end
  if joueur then
    joueur:sendMessage(TextFormat.AQUA .. "Vous avez " .. (points[joueur:getName()] or 0) .. " points. " .. texte)
  else
    server.log(texte)
  end
end)
