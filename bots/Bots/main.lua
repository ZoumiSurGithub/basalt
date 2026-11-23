-- Bots : /bots <nombre> [vitesse] [distance] fait apparaître des bots à votre position.
-- Ils partent en étoile, à angles égaux : deux bots partent dos à dos, quatre vers les
-- quatre points cardinaux, etc. Arrivés à `distance` blocs, ils font demi-tour, puis
-- repartent en sens inverse, et ainsi de suite. /bots stop les retire.
--
-- Les bots sont des PNJ du serveur (server.spawnNpc) : le serveur les fait marcher, monter
-- les marches et tomber ; le plugin ne leur donne que leur direction.

local MAX = 200            -- bots au plus
local VITESSE = 4.3        -- blocs par seconde (marche d'un joueur)
local DISTANCE = 100       -- blocs avant le demi-tour

local actifs = 0           -- nombre de bots en place
local vague = 0            -- change à chaque /bots : les anciennes minuteries s'arrêtent

-- Retire tous les bots et coupe les demi-tours en attente.
local function retirer()
  server.removeNpcs()
  actifs = 0
  vague = vague + 1
end

-- Message au joueur, ou dans la console si la commande vient de là.
local function dire(joueur, texte)
  if joueur then joueur:sendMessage(texte) else server.log(texte) end
end

-- Donne à chaque bot sa direction (sens = 1 à l'aller, -1 au retour)
local function marcher(n, vitesse, sens)
  for i = 1, n do
    local angle = 2 * math.pi * (i - 1) / n
    server.walkNpc("Bot" .. i, math.cos(angle) * vitesse * sens, math.sin(angle) * vitesse * sens)
  end
end

-- Demi-tour tous les `distance` blocs, tant que la vague est la même
local function aller_retour(n, vitesse, distance, sens, ma_vague)
  server.afterSeconds(distance / vitesse, function()
    if ma_vague ~= vague then return end
    marcher(n, vitesse, -sens)
    aller_retour(n, vitesse, distance, -sens, ma_vague)
  end)
end

server.command("bots", function(args, joueur)
  if args[1] == "stop" or args[1] == "0" then
    local n = actifs
    retirer()
    dire(joueur, "§e" .. n .. " bot(s) retiré(s).")
    return
  end
  local n = math.tointeger(tonumber(args[1] or ""))
  if not n or n < 1 then
    dire(joueur, "§cUsage : /bots <nombre> [vitesse] [distance] | /bots stop")
    return
  end
  if not joueur then
    server.log("À utiliser en jeu : les bots partent de votre position.")
    return
  end
  n = math.min(n, MAX)
  local vitesse = math.max(0.1, math.min(tonumber(args[2] or "") or VITESSE, 20))
  local distance = math.max(1, tonumber(args[3] or "") or DISTANCE)

  retirer()
  local x, y, z = joueur:getPosition()
  for i = 1, n do
    server.spawnNpc("Bot" .. i, x, y, z)
  end
  actifs = n
  marcher(n, vitesse, 1)
  aller_retour(n, vitesse, distance, 1, vague)
  dire(joueur, string.format("§a%d bot(s) en route à %.1f blocs/s, demi-tour tous les %d blocs. /bots stop pour les retirer.", n, vitesse, distance))
end)

server.on("disable", function() server.removeNpcs() end)
