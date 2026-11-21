-- Des annonces automatiques, l'une après l'autre, toutes les quelques minutes.

local config = server.getConfig({
  minutes = 10,
  annonces = { "Pensez à lire les règles !", "Rejoignez-nous sur Discord !" },
})

local numero = 1

server.everySeconds(config.minutes * 60, function()
  if #config.annonces == 0 then return end
  server.broadcast(TextFormat.GOLD .. "[Annonce] " .. TextFormat.WHITE .. config.annonces[numero])
  numero = numero + 1
  if numero > #config.annonces then numero = 1 end
end)
