# Le monde

Comment Basalt génère et enregistre le monde : terrain, eau et lave, structures, réglages.

Un nouveau monde est généré comme dans le jeu depuis la 1.18 (`src/world/vanilla.rs`) :

- **climat** : continentalité, érosion, étrangeté (pics et vallées), température et
  humidité, comme les cinq bruits du jeu ; le relief en découle (océans profonds, côtes,
  plaines, plateaux, montagnes jusqu'à 250 blocs, rivières dans les vallées, marais) ;
- **biomes** choisis avec les tableaux du jeu : une cinquantaine, des océans gelés aux
  océans chauds, plaines, forêts (fleuries, de bouleaux, sombres), taïgas (géantes,
  enneigées), jungles, savanes, déserts, badlands (avec leurs bandes de terre cuite),
  marais et mangroves, prairies, cerisaies, bosquets, pentes et pics enneigés, pics
  rocheux, champs de champignons, pics de glace, plus les grottes luxuriantes, grottes de
  stalactites et abîmes en profondeur. Les biomes sont envoyés au jeu en 3D (couleurs de
  l'herbe et de l'eau, ciel) ;
- **grottes** : grandes cavités, galeries, boyaux étroits, entrées depuis la surface, lave
  tout en bas ;
- **surface** de chaque biome (sable, grès, terre cuite, neige, glace compacte, neige
  poudreuse, calcite, podzol, mycélium, boue, gravier, pierre sur les pentes raides) ;
- **arbres** de chaque biome, aux formes du jeu (chêne, grand chêne, bouleau, sapin, sapin
  géant, chêne noir, chêne pâle, acacia, jungle et jungle géante, cerisier, palétuvier),
  avec lianes et nids d'abeilles ;
- **sol** : fleurs, herbes, fougères, buissons et buissons à lucioles, baies, cactus et
  leurs fleurs, herbes sèches, cannes à sucre, nénuphars, pics de glace, feuilles mortes
  en forêt, pétales roses sous les cerisiers, fleurs sauvages dans les prairies ;
- **fonds marins** : herbes marines (hautes ou basses), laminaires, récifs de corail et
  cornichons de mer des océans chauds, icebergs des océans gelés, magma au fond des
  océans profonds. Les plantes sous l'eau contiennent bien de l'eau ;
- **minerais** à leurs hauteurs (émeraude dans les montagnes), poches de granite,
  diorite, andésite, tuf, gravier.

**Eau et lave** coulent comme dans le jeu (`src/world/liquid.rs`) : un liquide n'est
recalculé que lorsqu'un bloc change à côté de lui (bloc cassé, posé, seau vidé). L'eau
s'étale sur 7 blocs, la lave sur 3 ; le liquide se dirige vers le trou le plus proche,
tombe, et disparaît quand plus rien ne l'alimente. Deux sources d'eau en créent une
troisième entre elles ; l'eau qui touche la lave donne obsidienne, pierre ou pierre
taillée. Les seaux remplissent et vident ; un bloc posé dans l'eau (dalle, escalier,
clôture…) la garde, et la laisse en place quand on le casse. Les modifications faites par
les plugins (EasyEdit…) ne déclenchent pas l'écoulement, comme WorldEdit dans le jeu.

**Structures** (`src/world/structures.rs`), placées comme dans le jeu (une par case d'une
grille).

**Plans officiels.** Les **villages**, **avant-postes de pillards**, **cités antiques** et
**ruines de piste** sont assemblés avec les vrais plans du jeu, pièce par pièce, comme dans
le jeu (`src/world/templates.rs`) : mêmes maisons, mêmes rues, même centre de cité. La
cité antique est creusée dans une caverne aux parois irrégulières, couverte de plaques de
sculk avec capteurs, catalyseurs et hurleurs ; autour des maisons de village, le terrain
est raccordé en talus plutôt que coupé net.

Les mêmes plans donnent aussi :

- des **épaves** au fond des océans ou échouées sur les plages, avec leurs trois coffres
  (provisions, trésor, carte) ;
- des **ruines océaniques** de pierre (mers froides) ou de grès (mers chaudes), usées,
  souvent une grande entourée de petites, avec coffre et sable ou gravier suspect ;
- des **igloos** dans les plaines et taïgas enneigées, une fois sur deux avec la trappe,
  la longue échelle et le laboratoire caché avec son coffre ;
- des **portails en ruine** partout sur la terre ferme, sur une plaque de netherrack et de
  magma, avec obsidienne pleureuse et coffre ;
- des **fossiles** enfouis sous les déserts et les marais ;
- des **manoirs des bois** dans les forêts sombres : plan d'étages tiré au hasard comme
  dans le jeu (couloirs à tapis rouge, pièces de 1 × 1, 1 × 2 et 2 × 2, pièces secrètes,
  escalier vers le troisième étage), murs, fenêtres, toits, coffres et socle de pierre.

Ces six-là n'existent qu'avec les plans officiels.

Ces plans appartiennent à Mojang : ils ne sont pas publiés avec Basalt. L'installation
(`get.sh`, `./update.sh`, ou `Installer.cmd` / `Mettre-a-jour.cmd` sous Windows) les
récupère toute seule, une fois : elle télécharge le serveur officiel depuis minecraft.net
(ce qui revient à accepter le [CLUF de Minecraft](https://www.minecraft.net/eula)), en
garde le dossier `behavior_packs/vanilla/structures` (rangé dans `structures` à côté de
Basalt) et jette le reste. `BASALT_NO_STRUCTURES=1` pour s'en passer. On peut aussi les
copier soi-même : dossier `structures`, ou archive `structures.zip`, à côté de
`server.properties`. Le journal du démarrage indique combien de plans ont été chargés.
Les lits gardent leur couleur ; les bannières des plans ne sont pas posées, et les
villageois, chats et golems sont installés par le serveur lui-même.

Sans ces fichiers, chaque structure est reconstruite à la main, au plus près du jeu :

- **villages** au style du jeu actuel (depuis la 1.14), dans les plaines, le désert, la
  savane, la taïga et les plaines enneigées, aux matériaux du biome : place centrale avec
  sa cloche (massifs de fleurs, puits de grès, estrade de terre cuite, feu de camp ou dalle
  de glace selon le biome), rues qui suivent le relief, maisons avec lits, maisons de
  métier avec leur bloc (haut fourneau, fumoir, table de cartographie, alambic, tonneau,
  table d'archerie, chaudron, pupitre, tailleur de pierre, métier à tisser, table de
  forgeron, meule), bibliothèques, champs avec composteur, enclos, lampadaires, décor de
  rue ;
- **pyramides du désert**, avec leur chambre secrète à quatre coffres piégée de TNT, et la
  pièce enfouie de sable suspect ajoutée en 1.20 ;
- **temples de la jungle**, sur deux étages, avec coffres, leviers et lianes ;
- **mines abandonnées** sous terre : galeries étayées, rails, toiles d'araignée, coffres ;
- **avant-postes de pillards** (plaines, désert, savane, taïga, neige, prairies) : tour de
  bouleau et de chêne noir, terrasse et coffre au sommet ; autour, cage, mannequins, tente
  ou bois empilé. Un emplacement sur cinq, jamais près d'un village, comme le jeu ;
- **cités antiques** au fond du deep dark (y = -51) : grande caverne, cadre de deepslate
  renforcé, allée à colonnes et lanternes d'âmes, maisons avec bougies et coffres, sculk
  avec capteurs et hurleurs ;
- **ruines de piste** (taïgas, forêt de bouleaux ancienne, jungle) : salles enfouies de
  briques de boue et de terre cuite, routes colorées, gravier suspect ; seul le haut d'une
  tour dépasse du sol ;
- **chambres des épreuves** entre y = -40 et -20 : hall de tuf et de cuivre, couloirs
  éclairés d'ampoules, salles de générateurs d'épreuves, de coffres-forts (vaults), réserve
  à tonneaux et salle à balcon.

Les blocs spéciaux sont posés tels quels : les générateurs d'épreuves et les
coffres-forts ne lancent pas encore de combat ni ne donnent de récompense, le gravier et
le sable suspects ne cachent pas encore d'objet, et les hurleurs n'appellent pas de Warden (pas encore
de monstres).

Les coffres se remplissent au premier clic (ou lâchent leur butin si on les casse sans les
ouvrir), toujours avec le même contenu pour une graine donnée.

`/locate structure <village|temple|desert_pyramid|jungle_temple|mineshaft|pillager_outpost|
ancient_city|trail_ruins|trial_chambers|shipwreck|ocean_ruin|igloo|ruined_portal|mansion>` et
`/locate biome <nom>` donnent les coordonnées de la plus proche (réservées aux opérateurs).
La recherche suit les mêmes règles que la génération, sur un fil à part.

Les valeurs ne sont pas celles du jeu : le monde a la même allure, mais une graine ne
redonne pas le même monde que dans Minecraft. Ce qui n'est pas encore généré est listé dans
la [feuille de route](ROADMAP.md).

Dans `server.properties` :

- `level-type=vanilla` (comme le jeu ; `normal` est accepté et veut dire la même chose),
  `level-type=flat` (plat), ou `level-type=plugin:<nom>`
  pour un générateur écrit en Lua par un plugin (relief, décorations, structures : voir
  [docs/PLUGINS.md](PLUGINS.md#générateurs-de-terrain) et l'exemple `plugins/Archipel`) ;
- `level-seed=` : un nombre ou un texte ; vide, la graine est tirée au hasard.

Ces deux réglages ne servent qu'à la création d'un monde : le type de terrain et la graine
sont ensuite enregistrés dans `level.nbt`, et un monde existant garde les siens. Les mondes
créés avant la génération de terrain restent plats. Pour un nouveau monde, changez
`world-dir` (ou supprimez le dossier du monde).

Le premier terrain propre à Basalt a été retiré : un monde créé avec lui est repris avec le
terrain du jeu. Ses constructions sont gardées, mais le relief autour d'elles n'est plus le
même ; la console le signale au démarrage.

Le terrain lui-même n'est jamais enregistré : il se régénère à l'identique à partir de la
graine. Seuls les blocs modifiés par les joueurs le sont (avec l'eau gardée dans les blocs
posés dans l'eau). Conséquence : une nouvelle version de Basalt qui améliore le terrain
l'applique aussi aux zones déjà explorées, sans toucher aux constructions. Les chunks sont
générés à l'avance, sur plusieurs cœurs, autour des joueurs (environ 1,2 ms par chunk,
1,6 ms près d'un village).

## Monstres dans le noir

Des zombies, des squelettes, des creepers et des araignées apparaissent **là où il fait noir**, autour des joueurs en
survie ou en aventure, à bonne distance (de 24 à 48 blocs), jusqu'à huit par joueur. Ils
poursuivent et frappent les joueurs qu'ils voient.

- **La nuit**, toute la surface est assez sombre.
- **Le jour**, seuls les endroits à l'abri du ciel le sont : grottes, mines, grands
  bâtiments sans fenêtre. Depuis la surface, les grottes du dessous se peuplent en une
  minute ou deux ; plus vite quand on est soi-même sous terre.
- **Une torche les empêche d'apparaître** partout où sa lumière arrive, soit à moins de
  14 blocs de chemin (la lumière contourne les murs, elle ne les traverse pas). Tout bloc
  lumineux compte : lanterne, pierre lumineuse, lave, bloc lumineux d'un plugin.
- Ils disparaissent d'eux-mêmes quand plus aucun joueur n'est près (toujours au-delà de
  128 blocs, peu à peu au-delà de 48).
- Au lever du jour, les zombies et les squelettes à ciel ouvert prennent feu ; les creepers
  et les araignées restent.
- Les squelettes tirent des flèches ; le creeper explose au contact et creuse le sol
  (`mob-griefing=false` : il blesse sans rien casser).
- Les araignées escaladent les murs pour atteindre le joueur qu'elles poursuivent.
- Les créatures ne se traversent plus : elles se poussent, et un joueur les pousse aussi.
- Aucun n'apparaît autour d'un joueur en créatif ou en spectateur.
- `spawn-monsters=false` dans `server.properties` donne un monde paisible. Les monstres
  qu'on fait apparaître soi-même (`summon zombie`) restent possibles.

### Comment la lumière est calculée

Le jeu calcule lui-même la lumière qu'il affiche ; Basalt ne la calcule que pour ses
règles, à l'endroit et au moment où il en a besoin (rien n'est enregistré dans le monde).
Les règles sont celles du jeu : une source éclaire à son niveau (14 pour une torche, 15
pour une lanterne) et perd 1 par bloc, 2 dans l'eau ; le ciel éclaire à 15 ce qui n'a rien
au-dessus de soi, puis sa lumière se répand de la même façon ; la nuit lui retire 11
niveaux. Un monstre demande aucune lumière de bloc et 7 au plus venant du ciel.

Limites actuelles : les dalles et les escaliers arrêtent la lumière comme des blocs
pleins, l'orage n'assombrit pas le ciel, et les monstres ne s'en prennent pas aux
villageois.
