# Journal des versions

Les changements de chaque version de Basalt. La section « Non publié » reçoit au fil de l'eau

ce qui est fait sur la branche `dev` ; `./release.sh <version>` la transforme en version
datée au moment de publier (voir « Publier une version » dans le README).

Catégories : **Ajouté** (nouveautés), **Modifié** (changements de comportement),
**Corrigé** (bugs), **Retiré** (fonctions supprimées).

## [Non publié]

### Ajouté

- Créatures créées par les plugins : apparence propre avec `ownModel = true` — le pack de ressources définit la créature sous son nom (modèle, image), et elle est déclarée au jeu à la connexion.
- Plugins : événements sur les créatures, celles du jeu comme celles des plugins — `CreatureSpawnEvent` (annulable), `CreatureDamageEvent` (annulable, dégâts modifiables) et `CreatureDeathEvent`. `getEntity()` rend la créature (`getType`, `getHealth`, `getPosition`…) et permet d'agir sur elle (`setHealth`, `setNameTag`, `kill`, `remove`).
- Plugins : créer des créatures à partir de celles du jeu. `server.entities.create("monserveur:vache_doree", { looks = "minecraft:cow", name, health, speed, scale, width, height, drops, xp })` garde l'apparence et le comportement de l'espèce avec ses propres valeurs ; `server.entities.spawn(nom, x, y, z)` fait apparaître une créature, du jeu ou créée. Voir « Créer des créatures » dans `docs/PLUGINS.md`.
- Tutoriel `docs/VSCODE.md` pour écrire ses plugins dans Visual Studio Code, et fichier `.luarc.json` à la racine : il suffit d'installer l'extension Lua et d'ouvrir le dossier du serveur.
- Plugins : créer des blocs nouveaux. `server.blocks.create("monserveur:minerai", { name, texture, hardness, light, friction, category, group, drops, falls… })` ; le bloc est déclaré au jeu des joueurs, avec l'objet qui le pose et sa place dans le menu créatif. Images par face (`textureTop`, `textureBottom`), `transparent`, forme (`geometry`), `solid`, outil exigé (`tool`, `tier`), expérience (`xp`, `xpMax`). Blocs à états : orientation à la pose (`rotation = "axis"` ou `"facing"`, `textureFront`) et stades de croissance (`stages`). Voir « Créer des blocs » dans `docs/PLUGINS.md`.

### Retiré

- L'ancien terrain propre à Basalt. `level-type` vaut désormais `vanilla` (comme le jeu), `flat` ou `plugin:<nom>` ; `normal` reste accepté et désigne le terrain du jeu. Un monde créé avec l'ancien terrain est repris avec celui du jeu, et la console le signale.

## [0.0.3] — 2026-11-23

### Ajouté

- Packs de ressources, réglés comme sur PocketMine dans `resource_packs/resource_packs.yml` : `resource_stack` liste les `.mcpack` et `.zip` à envoyer aux joueurs, `force_resources` les rend obligatoires ; pack chiffré avec sa clé dans un fichier `.key`. Voir `docs/PACKS.md`.
- Objets créés par les plugins : présents dans le menu créatif, dans l'onglet (`category`) et le groupe (`group`) choisis ; `creative = false` pour les en écarter.
- Plugins : événement `PlayerItemUseEvent` (clic avec un objet en main, annulable), pour donner un effet à un objet.
- Plugins : créer des objets nouveaux. `server.items.create("monserveur:rubis", { name, icon, maxStack, glint, food, saturation, alwaysEdible, leftover, durability, attackDamage, handEquipped })` ; l'objet est déclaré au jeu des joueurs à leur connexion, avec l'image d'un objet du jeu. Voir « Créer des objets » dans `docs/PLUGINS.md`.
- Plugins : modifier les objets et les blocs du jeu. `server.items.modify(nom, { food, saturation, leftover, durability, attackDamage })` et `server.blocks.modify(nom, { drops, falls, needsFloor, breakable })` ; `reset` pour revenir à la règle du jeu. Voir « Modifier objets et blocs » dans `docs/PLUGINS.md`.
- Plugins dans VS Code : `docs/vscode/basalt-api.lua` décrit l'API pour l'extension Lua (complétion, aide, fautes signalées) ; mode d'emploi dans `docs/PLUGINS.md`.
- Buissons à baies : un clic cueille les baies mûres (une ou deux, deux ou trois au dernier stade) et le buisson repart.
- Objets : chaque pile porte désormais sa variante et son usure, dans l'inventaire, au sol, dans la sauvegarde du joueur et sur le réseau.
- Lits de couleur : un lit pris au menu créatif garde sa couleur une fois posé, et la rend quand on le casse.
- Durabilité : pioches, haches, pelles, houes, épées et cisailles s'usent (un cran pour leur usage, deux pour l'autre) et cassent une fois usées ; la barre d'usure s'affiche. Outils, lits et autres objets uniques ne s'empilent plus.
- Minage : la vitesse des fissures suit l'outil tenu (le bon outil va plus vite, sans l'outil exigé c'est bien plus lent).
- Plugins : sept événements pour la survie et le monde — `PlayerItemConsumeEvent`, `PlayerExhaustEvent`, `PlayerExperienceChangeEvent`, `PlayerBedEnterEvent`, `PlayerBedLeaveEvent`, `LeavesDecayEvent`, `BlockFallEvent` — tous annulables sauf le lever du lit (voir `docs/PLUGINS.md`).
- Croissance (ticks aléatoires autour des joueurs, à la cadence du jeu) : blé, carottes, pommes de terre et betteraves poussent en huit stades, plus vite sur une terre arrosée ; les tiges de melon et de citrouille donnent leur fruit ; canne à sucre et cactus montent jusqu'à trois blocs ; l'herbe gagne la terre voisine ; la terre labourée s'arrose près de l'eau (4 blocs), sèche sinon et redevient de la terre si rien n'y pousse ; buissons à baies et verrues du Nether mûrissent. Événement `BlockGrowEvent` pour les plugins.
- Récoltes : une culture mûre rend sa récolte et de quoi replanter (blé et graines, 2 à 5 carottes ou pommes de terre, betterave et graines) ; pas mûre, seulement la graine.
- Pose : un bloc qui a besoin d'un support ne se pose que là où le jeu l'accepte — culture sur de la terre labourée, fleur ou pousse sur de la terre ou de l'herbe, cactus sur du sable, canne à sucre au bord de l'eau, torche, rail ou échelle contre un bloc plein. On ne peut plus empiler une plante sur une autre.
- Houe : un clic laboure l'herbe, la terre et les chemins (la terre stérile redevient de la terre).
- Minage : le bruit du bloc qu'on frappe, pour le joueur et ceux qui sont autour.
- Faim : courir, sauter, nager, casser des blocs et encaisser des coups fatiguent ; la
  fatigue entame la satiété puis la barre de faim. Barre à 18 ou plus, la vie remonte (plus
  vite le ventre plein) ; vide, le joueur dépérit jusqu'à un demi-cœur. Les aliments se
  mangent (clic maintenu) et rendent faim et satiété comme dans le jeu ; bol et fiole vides
  restent en main. Pas de faim en créatif.
- Expérience : créatures tuées, minerais cassés (charbon, diamant, émeraude, lapis, quartz,
  redstone), échanges avec les villageois et naissances d'animaux en donnent ; les niveaux
  suivent le barème du jeu. La mort la fait perdre (sauf `keep-inventory`). Elle est
  créditée directement : pas encore d'orbes à ramasser.
- Orbes d'expérience : créatures tuées, minerais cassés et naissances lâchent une orbe, qui file vers le joueur proche et tinte quand il la ramasse ; fanfare à chaque niveau gagné.
- Blocs : miettes et bruit quand un bloc est cassé, bruit de la matière quand il est posé, fissures et éclats pendant le minage (durée d'après la dureté du bloc, sans tenir compte de l'outil).
- Pétales roses, fleurs sauvages et feuilles mortes : en reposer sur une touffe l'étoffe (jusqu'à quatre) au lieu de poser un second bloc au-dessus.
- Lits : on ne peut plus se coucher dans le lit où dort un villageois.
- Blocs qui tiennent sur un support : fleurs, herbes, pousses, champignons, cultures, canne à sucre, cactus, bambou, torches, lanternes, échelles, boutons, leviers, rails, tapis, plaques de pression, panneaux, portes, neige… se décrochent et tombent en objet quand le bloc qui les porte disparaît, de proche en proche. Une plante ou une porte en deux moitiés part en entier pour un seul objet.
- Gravité : sable, sable rouge, gravier, béton en poudre et enclumes tombent quand rien ne les porte (bloc cassé dessous, ou posés dans le vide), toute la colonne suit.
- Feuilles : privées de tronc (à plus de 6 blocs, de feuille en feuille), elles dépérissent en quelques secondes et laissent parfois une pousse, un bâton ou une pomme. Les feuilles posées par un joueur restent.
- Lits : un lit posé a ses deux moitiés (le pied devant le joueur, l'oreiller au-delà), et casser l'une emporte l'autre pour un seul lit rendu. Manger fait du bruit (mastication, puis rot).
- Lits : un clic fixe le point de réapparition (enregistré avec le joueur) ; la nuit, le
  joueur s'y couche, et la nuit passe quand tous les joueurs dorment depuis 5 secondes. Si
  le lit a disparu, on réapparaît au point du monde.
- Rapport de mesures : une ligne « Départs depuis le démarrage » compte les joueurs partis, par cause (partis d'eux-mêmes, liaison perdue, jeu figé, données non lues…).
- `nethernet-threads` (1 par défaut, 16 au plus) : répartit les liaisons NetherNet des joueurs sur plusieurs fils, sans changer de port. Un seul fil plafonnait vers un millier de joueurs.
- Côté joueur de NetherNet (`src/network/nethernet_client.rs`) : une liaison vers un serveur
  Basalt pour les joueurs simulés. Avec `local-bots=true`, les joueurs simulés de cette
  machine entrent aussi par NetherNet (sans compte, pseudo en « Bot… », sans limite de
  connexions par adresse).
- Villageois, la vie du village : la nuit ils se couchent dans un lit libre de leur maison
  et se lèvent au matin (un coup les réveille) ; leur métier vient du poste de travail
  libre le plus proche de chez eux (composteur, pupitre, tonneau…), un poste cassé le leur
  retire tant qu'ils n'ont rien échangé, un poste posé en donne un à un villageois sans
  métier ; ils passent à leur poste dans la journée ; deux adultes côte à côte font un
  petit s'il reste un lit libre dans le village, et le petit prend un métier en grandissant.
  Pour rentrer chez eux, aller au rendez-vous ou à leur poste, ils suivent un vrai chemin
  (détours, marches, portes) au lieu de marcher droit devant eux, et en cherchent un autre
  s'ils restent bloqués.
- Échanges avec les villageois : un clic sur un villageois qui a un métier ouvre l'écran
  d'échange. Deux offres par niveau, celles de son métier : il achète des matières
  premières contre une émeraude et vend ses produits. Le serveur vérifie le prix payé.
  Chaque échange lui rapporte de l'expérience ; il monte de niveau (novice, apprenti,
  compagnon, expert, maître), ce qui déverrouille ses offres suivantes dans l'écran et
  change l'insigne à sa ceinture. Une offre s'épuise après 12 ou 16 échanges et se
  recharge deux fois par jour, aux heures de travail. Niveau et usure sont enregistrés.
  Le villageois ne bouge pas pendant l'échange et se fait entendre (« hmm » le jour, à
  l'ouverture de l'échange, quand il est conclu). Les objets que le jeu vend enchantés
  le sont ici sans enchantement ; pas de cartes au trésor ni d'expérience pour le joueur.
- Golems de fer : un par village, au pied de la cloche. Il reste dans le village, s'en
  prend au joueur qui frappe un villageois ou un golem (jamais en créatif), l'envoie en l'air
  d'un coup de bras, ne recule pas sous les coups et laisse du fer et des coquelicots.
  `/summon iron_golem` fonctionne aussi.
- Golem de fer à bâtir : une citrouille sculptée (ou une citrouille-lanterne) posée au sommet
  d'un T de quatre blocs de fer ; celui-là ne s'en prend jamais aux joueurs. Un lingot de fer
  répare un golem blessé (25 points de vie), et ses fissures suivent sa vie.
- Cloches : un clic les fait sonner et se balancer, pour tous les joueurs des environs.
- Loups : meutes de quatre dans les taïgas, les bosquets enneigés et (plus rarement) les
  forêts ; sauvages, ils chassent moutons et lapins et se retournent en meute contre le
  joueur qui en frappe un. Un os les apprivoise une fois sur trois : collier rouge (à
  teindre), 40 points de vie, assis ou debout d'un clic, ils suivent leur maître (et
  réapparaissent près de lui s'ils sont distancés), attaquent ce qu'il frappe et le loup
  qui le mord ; la viande les soigne, puis les rend amoureux (louveteau apprivoisé, au
  même maître). `/summon wolf`.
- Chats : errants dans les villages (près des lits), onze pelages ; sauvages, ils fuient le
  joueur qui ne leur tend pas de poisson et chassent les lapins. Morue ou saumon crus les
  apprivoisent une fois sur trois : collier (à teindre), assis ou debout d'un clic, ils
  suivent leur maître ; le poisson les soigne et les rend amoureux (chaton apprivoisé).
  `/summon cat`.
- Villageois : les villages sont habités, près des lits, au costume du
  biome et avec un métier (ou « nigaud ») ; ils se promènent sans quitter leur village,
  regardent les joueurs, fuient sous les coups. Ils ouvrent les portes en bois devant eux, qui se referment après
  leur passage (avec leur bruit), et vont régulièrement passer la porte la plus proche
  pour sortir de chez eux ou y rentrer. `/summon villager`.
- Jour et nuit : l'heure avance chez les joueurs (une journée dure 20 minutes, comme dans
  le jeu) ; `/time set` continue de la régler.
- Emploi du temps des villageois : la journée ils vaquent dans le village, l'après-midi
  (9000 à 11000) les adultes se retrouvent au centre, la nuit (à partir de 12000) chacun
  rentre chez lui par sa porte et n'en bouge plus jusqu'au matin.

### Modifié

- Code rangé en gabarits, à la manière de PocketMine, sans changement en jeu : `item/consumable.rs` (ce qui se mange : faim, satiété, durée, reste), `item/durable.rs` (ce qui s'use), `block/behavior.rs` (ce qui tombe, tient sur un sol, pousse, dépérit). Le serveur interroge les gabarits au lieu de connaître objets et blocs par leur nom. Nouvelle page `docs/ARCHITECTURE.md`.
- `max-visible-players` vaut 0 par défaut, et 0 retire le plafond : chacun voit tous les joueurs à portée de vue (avant, le défaut était 48 et 0 n'affichait personne). Un `server.properties` existant garde sa valeur.
- Arrêt du serveur dans l'ordre de PocketMine : les joueurs sont enregistrés, puis les plugins désactivés et attendus (sans limite de temps ; 2 s avant, et sans attente s'ils étaient occupés), puis le monde écrit et l'écriture sur le disque attendue jusqu'au bout (60 s au plus avant). Ce qu'un plugin demande en se désactivant est encore appliqué et enregistré.
- Windows : Ctrl+C arrête le serveur proprement, comme la commande `stop` (avant, Windows tuait le processus sans rien enregistrer). Fermer la fenêtre déclenche aussi l'enregistrement, dans les quelques secondes que Windows accorde.
- Plugins : l'état du serveur transmis à chaque événement (liste des joueurs) n'est plus recopié pour chaque plugin, ce qui coûtait plus d'une milliseconde par événement avec plusieurs centaines de joueurs.
- Foules : la position de chaque joueur est mise en forme une seule fois par tick au lieu d'une fois par spectateur, et les rafraîchissements sont étalés sur plusieurs ticks (fin des à-coups à plusieurs centaines de joueurs regroupés).
- Tests de bout en bout : ils passent par un transport de test (`src/network/transport.rs`,
  TCP, cette machine seulement, ouvert par programme uniquement) au lieu de RakNet.
- Outil de charge (`examples/stress.rs`) : les joueurs simulés se connectent par NetherNet.
- Documentation : le README est refait (pourquoi choisir Basalt, fonctions, installation,
  crédits, téléchargements et Discord). Ses longues sections partent dans `docs/` : le monde,
  le réseau et la sécurité, les performances, la console, le développement, et une feuille
  de route à cases cochées.

### Retiré

- RakNet : l'ancien transport du jeu (avant la 1.26.60) est retiré du serveur, avec son
  chiffrement, son écoute IPv6 (`server-portv6`), sa réponse aux pings et l'auto-test du
  démarrage. Les joueurs n'entrent plus que par NetherNet ; le port 19132 en UDP ne sert
  plus (restent 19132 en TCP et 19140 en UDP). Seul l'outil de capture parle encore RakNet,
  au serveur officiel, qui n'accepte pas autrement un joueur sans compte.
- Ménage du code : environ 310 lignes jamais appelées supprimées (anciens essais d'écriture
  des chunks, chunk plat de test, fonctions de protocole remplacées depuis), ainsi que les
  variables et imports devenus inutiles. Aucun changement de comportement.

### Corrigé

- Après une coupure pour « jeu figé » (ou un départ juste après l'apparition), le serveur se rabattait sur une liste d'objets vide jusqu'au redémarrage : plus de menu créatif ni d'échanges avec les villageois, pour tout le monde. Ce repli automatique est retiré, et un joueur n'est tenu pour figé qu'après 60 secondes sans mouvement (15 avant).
- `/timings on` ne ralentit plus le serveur sous forte charge : chaque fil compte dans son coin et verse ses mesures dix fois par seconde, au lieu de prendre un verrou commun à chaque mesure (les fils du réseau se le disputaient, et les temps mesurés en étaient gonflés).
- Villages : devant les portes, les deux marches qui encadrent celle du milieu forment de nouveau des coins. La marche du milieu est un point d'attache du plan, que le calcul des coins ne voyait pas comme un escalier.
- Les objets dont le bloc porte un autre nom se posent : canne à sucre, graines, carotte, pomme de terre, redstone, ficelle, baies, fèves de cacao, répéteur, comparateur. Avant, le jeu dessinait un bloc que le serveur n'avait pas posé.
- La canne à sucre cassée à la main rend bien son objet (son nom d'objet n'était pas reconnu).
- Villageois : le réassort des échanges n'était pas retenu quand le villageois était déchargé (redémarrage, joueur parti puis revenu) ; il restait alors épuisé à la première visite suivante. Le dernier réassort est enregistré avec le villageois.
- L'heure du monde (jours écoulés compris) est enregistrée et reprise au démarrage : chaque redémarrage ramenait le monde au premier matin.
- L'avertissement « le jeu s'est figé » n'est plus écrit à chaque tick pour le même joueur en attendant sa coupure.
- Charge : l'arrivée d'un joueur n'attend plus la fin de toutes les écritures en cours sur le
  disque, seulement celle de sa propre sauvegarde. Pendant la sauvegarde périodique de 500
  joueurs, chaque arrivée figeait le serveur une demi-seconde.
- Charge : un événement de plugin que personne n'écoute (`save`, `connect`, `login`, `load`,
  `join`…) ne réveille plus le fil des plugins. Avec 1 250 joueurs simulés, la sauvegarde
  périodique figeait le serveur 0,8 s en moyenne (2 s au pire) à cause de l'événement `save`
  envoyé pour chaque joueur, et chaque arrivée coûtait 5 ms d'attente.
- Créatures dans l'eau : contre une berge de la hauteur de l'eau, elles se hissent dessus au
  lieu d'y buter sans fin (villageois, golems et animaux restaient coincés dans les mares).
- Portillons : ils s'ouvrent toujours en s'éloignant du joueur, quel que soit le côté d'où
  on clique.
- Lapins et chats sauvages ne fuient plus un joueur en créatif ou spectateur.
- Villageois et chats des villages : ils apparaissent dans la maison, au pied du lit, et
  plus sur le toit.
- Structures (plans officiels) : les barrières se relient au portillon voisin quand il
  vient d'un point d'attache du plan, et ne tendent plus un bras vers les fleurs, les
  citrouilles ou les pastèques.
- Villages : la terre mise à nu là où une maison entaille le terrain se couvre d'herbe
  (plus de plaques de terre autour des maisons) ; de même la terre que les plans posent à
  l'air libre, que l'herbe gagne vite dans le jeu.
- Villages : plus d'herbes, de fleurs ni de feuilles mortes poussées sur les chemins.
- Créatures : elles ne sautent plus sur les trappes, barrières, murets, portillons, vitres
  et barreaux (un villageois se retrouvait perché sur une trappe ; les animaux pouvaient
  franchir une barrière d'un saut).
- Villages : le relief épouse la base des maisons comme dans le jeu (la densité du terrain
  est modifiée autour de chaque pièce, sur 12 blocs : comblé dessous, dégagé dessus), au
  lieu d'être taillé en talus après coup ; les rues sont posées sur ce relief adapté. Le
  niveau du sol de chaque pièce est déduit de sa jointure, comme dans le jeu (un lampadaire
  ou une petite maison sont posés sur le sol, une grande maison y enfonce sa couche du bas).
- Villages au bord d'une plage ou d'un désert : les talus autour des maisons sont en sable
  comme le sol d'alentour, plus en blocs d'herbe.

## [0.0.2] — 2026-11-22

### Ajouté

- Logos, bannière GitHub et images Discord en PNG (`docs/images/`), bannière en tête du README.
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

- Compilation sur GitHub plus rapide : tests, version Linux et version Windows en même temps
  (la publication attend les trois), au lieu de l'un après l'autre.
- **Versions d'avant la 1.26.60 refusées** (1.26.40 à 1.26.51) : le jeu indique qu'il doit
  être mis à jour. Elles écrivent les objets et plusieurs paquets autrement, et n'étaient en
  pratique pas jouables (inventaire, déplacements). Un `min-protocol` plus bas est relevé à 2225.
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

- Monture dirigée : dégâts de chute à l'atterrissage, comme dans le jeu (au-delà de 3 blocs,
  moins 5 pour le cheval, l'âne et la mule ; rien dans l'eau).
- Inventaire d'une monture : la selle de l'âne et de la mule apparaît dans sa case, et les
  objets gardent leur numéro de pile d'une demande à l'autre (retirer la selle ou l'armure
  était refusé).
- Inventaire d'une monture : la selle et l'armure (ou le tapis) posées apparaissent dans
  leurs cases (envoyées dans le conteneur « équipement de monture », et plus en double dans
  la fenêtre). Le jeu plantait dès qu'on prenait un objet, l'écran ouvert : le contenu
  était envoyé sous un nom de conteneur qui n'est pas celui de l'écran.
- Montures : leurs données d'entité annoncent maintenant leur inventaire (écran de monture,
  nombre de cases). À zéro, le jeu plantait dès qu'on prenait un objet dans l'écran.
- Montures : à la première ouverture de leur inventaire, les cases de selle et d'armure
  n'avaient pas d'icône et refusaient les objets. Leur description est maintenant envoyée
  avant l'ouverture de l'écran, puis répétée après.
- Écran de monture : le jeu ferme en passant une fenêtre « -1 » ; Basalt prenait ça pour la
  fermeture de l'écran et refusait ensuite tout geste dans le coffre de l'âne. Cette
  fermeture est maintenant ignorée. La selle de l'âne est envoyée avec les deux cases
  d'équipement, comme pour le cheval.
- Inventaire des montures refait comme le jeu officiel : l'écran s'ouvre par le seul paquet
  des cases d'équipement (plus d'ouverture de conteneur en double, qui donnait des cases
  sans icône puis un écran figé) ; l'âne et la mule ont la selle seule et un coffre dont
  les cases sont comptées à partir de 1 ; le lama a son tapis et 3 cases de coffre par
  point de force ; les données d'entité annoncent 2 cases pour le cheval, 16 pour l'âne,
  la mule et le lama, et la force du lama.
- Âne, mule ou lama portant un coffre : la selle (ou le tapis) est envoyée en tête de la
  liste du coffre ; envoyée à part, elle était effacée à l'écran.
- Monture quittée en plein saut : le serveur la fait retomber et renvoie son état à tous,
  à la descente puis à l'atterrissage (elle gardait la pose du saut).
- Lama : il ne crache plus sur un joueur en créatif ou spectateur.
- Lama : son tapis se pose et se retire par l'inventaire (le jeu désigne sa case par le
  numéro 0, Basalt attendait le numéro 1).
- Crachat du lama : il part de la bouche et vole jusqu'à sa cible au lieu de rester immobile
  au-dessus du lama (données de projectile, position redessinée à chaque tick).
- Un jeu qui refusait les animaux les supprimait pour tout le serveur (et de la sauvegarde) :
  le refus est maintenant seulement signalé.
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
