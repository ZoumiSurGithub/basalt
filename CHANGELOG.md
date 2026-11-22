# Journal des versions

Les changements de chaque version de Basalt. La section « Non publié » reçoit au fil de l'eau

ce qui est fait sur la branche `dev` ; `./release.sh <version>` la transforme en version
datée au moment de publier (voir « Publier une version » dans le README).

Catégories : **Ajouté** (nouveautés), **Modifié** (changements de comportement),
**Corrigé** (bugs), **Retiré** (fonctions supprimées).

## [Non publié]

### Ajouté

- Comportements particuliers du jeu :
  - inventaire des montures apprivoisées (accroupi + clic droit, ou touche d'inventaire en
    selle) : selle, armure du cheval, tapis du lama, coffre de l'âne, de la mule et du lama
    (15 cases, 3 par point de force pour le lama), qui tombent à leur mort ; armure, tapis
    et coffre se posent aussi d'un clic droit ;
  - cochon sellé à monter et diriger avec une carotte sur un bâton (qui l'accélère) ;
  - moutons à teindre ;
  - champimeuh brune qui mange une fleur : son prochain bol est une soupe suspecte ;
  - chèvre qui fonce sur les joueurs (2 points de dégâts et un recul ; la hurleuse bien
    plus souvent), perd une corne contre le bois, la pierre, les minerais ou la glace
    compacte, et bondit de bloc en bloc ;
  - lamas en caravane derrière celui qu'on tient en laisse (10 au plus), qui crachent une
    fois sur qui les frappe (le crachat vole jusqu'à lui) ;
  - lapins qui grignotent les carottes plantées ;
  - monture sauvage qui gambade avec son cavalier pendant le dressage.
- Laisses : une laisse en main sur un animal l'attache au joueur, un clic droit sur une
  barrière l'y noue (nœud visible), clic sur l'animal ou le nœud pour détacher ; elle tire
  au-delà de 4 blocs, fort au-delà de 6 et casse à 10, comme dans le jeu. Les animaux
  noués à une barrière le restent après un redémarrage.
- Comportements du jeu pour chaque animal : vitesses du jeu (promenade, fuite, nourriture,
  petit qui suit ses parents), les petits suivent un adulte, les lapins fuient les joueurs
  qui ne leur tendent pas de carotte, les animaux évitent l'eau en se promenant et
  regardent parfois les joueurs proches. Le lama ne suit plus la nourriture ; chevaux, ânes
  et mules apprivoisés suivent les pommes et carottes dorées. Son de l'œuf pondu.
- Montures plus rapides : leur vitesse et la force de leur saut sont envoyées au jeu du
  cavalier (il gardait sinon la plus basse), et chevaux, ânes et lamas se promènent à un
  pas plus proche du jeu.
- Nouveaux animaux, avec les règles d'apparition du jeu : champimeuh (champs de
  champignons ; bol → soupe, seau → lait, cisailles → 5 champignons et elle redevient
  vache), chèvre (sommets ; 2 % de « hurleuses » ; lait), lama (collines venteuses,
  plateaux de savane ; 4 pelages), cheval (plaines, savanes ; 7 robes et 5 motifs), âne
  et mule (un cheval et un âne font une mule, qui n'a pas de petit). Vie, vitesse et saut
  des montures tirés au sort ; elles se soignent et leurs petits grandissent avec leur
  nourriture.
- Montures : clic droit main vide pour monter (s'accroupir pour descendre). Une monture
  sauvage se laisse apprivoiser ou jette son cavalier, de plus en plus patiente à chaque
  essai et quand on la nourrit (règles du jeu) ; apprivoisés, chevaux, ânes, lamas se
  reproduisent (carottes et pommes dorées, bottes de foin pour les lamas). Une selle
  posée sur un cheval, un âne ou une mule apprivoisés permet de les diriger ; elle tombe
  à leur mort.
  Les autres joueurs voient le cavalier assis sur sa monture (sa position leur est donnée
  d'après celle de la monture, pas d'après son jeu qui le place dessous).
  Les sauts d'une monture se voient chez les autres joueurs (en l'air, comme dans le jeu) ;
  une monture qui refuse d'être apprivoisée se cabre. Sons envoyés par le serveur comme le
  serveur officiel : selle posée, tonte, traite, repas, monture en colère, saut et
  atterrissage (pour les autres joueurs).
- Commande `/summon <animal> [position] [minecraft:entity_born | minecraft:on_tame]`.
- Guide d'installation pas à pas pour Windows et Linux : `docs/INSTALLER.md`.
- `/timings report` indique l'état de la machine pendant les mesures (processeur, temps
  volé par l'hébergeur, attente disque, mémoire, swap) et dit si les à-coups viennent d'elle.

### Modifié

- Publication sur GitHub : la version Windows attend que les tests passent (les deux
  versions publiées restent identiques) ; sur une branche, les tests de bout en bout (une
  minute, sensibles à la lenteur de la machine) ne sont plus lancés, seulement pour une
  version officielle (étiquette `v…`).
- Animaux rangés comme dans le jeu : enregistrés avec leur région (`world/entities/`),
  ils ne restent en mémoire que près des joueurs et retournent dans leur région quand
  plus personne n'est proche ; plus de limite au nombre d'animaux du monde. L'ancien
  fichier `mobs.json` est converti tout seul au premier démarrage (gardé en
  `mobs.json.ancien`).
- Distance de simulation réglable (`tick-distance`, 4 par défaut comme le serveur
  officiel) : au-delà, les animaux restent figés.
- Les fils de fond (génération du terrain, préparation des chunks, écritures disque)
  tournent en priorité basse sous Linux : la boucle du jeu passe avant eux quand la
  machine est chargée.

### Corrigé

- Connexion en 1.26.40 / 1.26.45 : le jeu ne pouvait lire ni la liste des packs de ressources
  (paquet 6) ni les chunks (paquet 58).
- Les animaux qui se promenaient renonçaient en plein saut devant une marche d'un bloc
  (ils sautaient sur place) ; le lapin, qui avance par bonds, ne la franchissait jamais.
- Boîte de collision des petits et des lapins trop petite (réduite deux fois : par
  Basalt puis par le jeu, qui applique lui-même la taille d'affichage).
- Un poulain (ânon, mule, petit lama) naît apprivoisé, comme dans le jeu ; un poulain
  tient sa vie, sa vitesse et son saut de la moyenne de ses parents et d'un tirage.
- Une monture apprivoisée mais blessée mangeait la carotte ou la pomme dorée sans devenir
  amoureuse : elle est maintenant soignée et prête à avoir un petit en même temps.
- Un joueur parti en selle revenait au point d'apparition du monde : il était enregistré
  à la position que donne son jeu, sous sa monture (dans le sol). Il est maintenant
  enregistré (et descend) debout à la place de sa monture ; et un joueur un peu enfoncé
  à son retour remonte jusqu'à 8 blocs au même endroit plutôt que d'être renvoyé au point
  d'apparition.
- Animaux et objets au sol invisibles après un redémarrage : ils étaient montrés au jeu
  avant le terrain où ils se trouvent, et le jeu les oubliait. Ils attendent maintenant
  que leur chunk soit arrivé. Le nombre d'animaux enregistrés s'affiche à l'arrêt.
- Les lapins avaient la taille d'un mouton : ils ont maintenant celle du jeu (bébés compris).
- À-coups du serveur : un animal au bord d'une zone pas encore générée faisait générer
  le terrain dans la boucle du jeu (jusqu'à plusieurs dixièmes de seconde avec une
  grande structure) ; les plans de structures récents ne sont plus tous oubliés d'un
  coup ; l'arrivée dans une région neuve installe les animaux peu à peu.
- Les animaux ne font plus leurs cris et leurs bruits en double : le jeu les joue déjà lui-même.

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
