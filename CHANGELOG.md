# Journal des versions

Les changements de chaque version de Basalt. La section « Non publié » reçoit au fil de l'eau
ce qui est fait sur la branche `dev` ; `./release.sh <version>` la transforme en version
datée au moment de publier (voir « Publier une version » dans le README).

Catégories : **Ajouté** (nouveautés), **Modifié** (changements de comportement),
**Corrigé** (bugs), **Retiré** (fonctions supprimées).

## [Non publié]

### Corrigé

- Un animal frappé ne fait plus deux bruits différents à la fois.

## [0.0.1] — 2026-11-21

Première version publiée.

### Ajouté

- Serveur Minecraft Bedrock écrit en Rust, pour le jeu 1.26.60 et plus (NetherNet), avec
  RakNet pour les jeux plus anciens ; comptes Xbox vérifiés.
- Monde à la manière du jeu : relief, biomes, grottes, arbres, décorations, fonds marins ;
  casser et poser des blocs (escaliers, barrières, murets et vitres reliés à leurs
  voisins), eau et lave qui coulent, seaux ; sauvegarde du monde.
- Structures avec les vrais plans du jeu, récupérés automatiquement à l'installation
  depuis le serveur officiel de Mojang : villages des cinq biomes, avant-postes de
  pillards, cités antiques (caverne et sculk), ruines de piste, épaves, ruines
  océaniques, igloos, portails en ruine, fossiles et manoirs des bois ; pyramides,
  temples de la jungle, mines et chambres des épreuves faits à la main. Coffres remplis
  à la première ouverture, `/locate structure` et `/locate biome`.
- Animaux : vaches, cochons, moutons, poules et lapins, installés à la découverte des
  terres selon le biome et enregistrés avec le monde ; promenade, fuite, combat, butin,
  élevage (nourriture, petits qui grandissent), tonte, traite, œufs, sons.
- Joueurs : skins, liste des joueurs, inventaire complet, coffres et fours, menu
  créatif, fabrication, combat, dégâts de chute, mort et réapparition, objets au sol,
  modes de jeu, chat en couleur pour les opérateurs, anti-triche des déplacements.
- Plugins Lua à la manière de PocketMine, avec des outils simples pour débuter
  (guide `docs/DEBUTER.md`) ; EasyEdit fourni.
- Installation et mise à jour en une commande sous Linux (`get.sh`, `update.sh`) et par
  double-clic sous Windows.
