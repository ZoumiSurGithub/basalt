-- EasyEdit pour Basalt : édition du monde en volume, sur le modèle du plugin EasyEdit de
-- PocketMine. Les commandes sont celles du jeu : /wand, /set stone…
--
-- Tout est fait dans le plugin, avec l'API du monde du serveur (server.getWorld()). Le code
-- est rangé dans src/ :
--   config.lua      réglages (baguette, limites, historique)
--   positions.lua   positions rangées dans un entier, pieds du joueur
--   selection.lua   les deux coins de chaque joueur
--   pattern.lua     motifs « stone,dirt » et « 70%stone,30%dirt »
--   editor.lua      file d'opérations appliquées par tranches, historique
--   wand.lua        la baguette (écouteurs d'événements)
--   commands/       une famille de commandes par fichier

require("src.editor").start()

require("src.commands.selection")
require("src.commands.blocks")
require("src.commands.shapes")
require("src.commands.clipboard")
require("src.commands.move")
require("src.commands.history")
require("src.commands.help")

server.registerEvents(require("src.wand"))
