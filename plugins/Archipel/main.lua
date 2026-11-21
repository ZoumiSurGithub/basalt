-- Archipel : ce plugin fournit le générateur de terrain generators/archipel.lua.
--
-- Le générateur n'est pas lancé d'ici : le serveur le charge lui-même, sur ses fils de
-- génération, quand server.properties contient level-type=plugin:archipel.
-- Ce fichier principal sert à faire d'Archipel un plugin complet (listé par /plugins).

server.on("enable", function()
  server.log("générateur « archipel » disponible (level-type=plugin:archipel)")
end)
