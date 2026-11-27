# Journal des versions

Les changements de chaque version de Basalt. La section « Non publié » reçoit au fil de l'eau
ce qui est fait sur la branche `dev` ; `./release.sh <version>` la transforme en version
datée au moment de publier (voir « Publier une version » dans le README).

Catégories : **Ajouté** (nouveautés), **Modifié** (changements de comportement),
**Corrigé** (bugs), **Retiré** (fonctions supprimées).

## [Non publié]

## [0.0.7] — 2026-11-28

- Publication : `./release.sh 0.0.7` fait tout (vérifications, journal, numéro, commit, étiquette, envoi) après confirmation ; le programme publié prend toujours le numéro de l'étiquette (vérifié par `basalt --version` à la compilation), la branche dev s'annonce `<numéro>-dev.<commit>` et son étiquette `dev-latest` suit le dernier envoi ; l'archive Windows contient la documentation et a son empreinte sha256.
- Connexion : une demande d'entrée arrivant de la même adresse et du même port qu'un joueur déjà en jeu (le système réutilise les ports) ne coupe plus ce joueur ; elle reçoit une autre désignation. À 800 bots, 86 joueurs étaient coupés ainsi (« retry »).
- Croissance : les ticks aléatoires sautent les chunks pas encore générés au lieu de les produire sur place.
- Mesures : un chunk produit sur place est compté avec l'étape qui l'a demandé (« Chunk sur place ← … »).
- Objets au sol : les déplacements des objets sont mis en forme une fois et envoyés à chaque joueur d'après ce qu'il voit, au lieu de chercher les spectateurs de chaque objet parmi tous les joueurs (16 ms par tick à 800 joueurs).
- Objets au sol : ce qu'est chaque bloc pour un objet est calculé une fois (plus par son nom à chaque pas), et chaque joueur ne regarde que les objets de son voisinage — la physique et l'affichage des objets prenaient 15 à 30 ms par tick à 500 joueurs.
- Terrain : le cache de chunks grandit avec ce que les joueurs voient (de 8 192 à 65 536 chunks) ; les objets au sol dont le chunk n'est pas en mémoire attendent au lieu de le faire générer sur place. À 500 joueurs dispersés, des chunks étaient régénérés sans cesse (70 ms chacun, jusqu'à 1,5 s).
- Objets au sol : un objet lâché n'est montré qu'à ceux qui sont près, sans refaire le tour de tous les objets pour chaque joueur — casser un bloc coûtait 15 ms à 500 joueurs, et une mort (inventaire répandu) jusqu'à plus d'une seconde.
- Blocs : un bloc qui change n'est annoncé qu'aux joueurs qui ont son chunk chargé (comme dans le jeu), plus à tout le serveur ; les messages diffusés sont mis en forme une seule fois pour tous. Casser un bloc coûtait 13 ms à 500 joueurs.
- Réseau : les paquets d'un joueur sont regroupés en un seul lot par tour de boucle, comme le serveur officiel, au lieu d'un lot par paquet ; le lot est emballé et compressé sur le fil NetherNet du joueur, plus sur celui du jeu. Avec une foule (et Chaos), les fils NetherNet recevaient plus de 100 000 envois par seconde chacun et saturaient vers 250 joueurs : toutes les liaisons tombaient (« liaison perdue ») et plus personne ne pouvait entrer.
- Journal : bilan de chaque fil NetherNet toutes les 5 s au-delà de 50 sessions (sessions, envois en attente, ordres par seconde, tours saturés).
- NetherNet : tampons UDP agrandis (16 Mo côté serveur) — Windows n'en garde que quelques dizaines de Ko, et une rafale (explosions, chunks) faisait jeter des datagrammes, dont les vérifications de liaison : des joueurs coupés pour « liaison perdue ».
- Connexion : un lot de paquets illisible isolé est ignoré au lieu de déconnecter le joueur (déconnexion au-delà de 3 par seconde), et la raison exacte est écrite dans le journal ; un lot peut contenir 1024 paquets (128 avant) ; 30 s pour se connecter au lieu de 10 sur une machine chargée.
- Feu : les flammes sont traitées par dixièmes, à chaque tick, au lieu de toutes d'un coup tous les dix ticks — une forêt en feu ne fige plus le serveur (jusqu'à 390 ms relevés à 327 joueurs).
- Bateau : annoncé comme le fait le serveur officiel (relevé par capture) — il se heurte aux blocs, tombe et flotte ; le regard du passager est verrouillé comme en jeu.
- Aura de feu : une créature tuée d'un seul coup meurt en flammes, sa viande tombe cuite.
- Terrain : la zone servie est annoncée à chaque tick, sur la position exacte du joueur, avec les chunks qui viennent de partir — comme le serveur officiel (relevé par capture).
- Monde importé : ses créatures sont reprises à la première ouverture (espèce, position, bébé, apprivoisée, variante, couleur).
- Noyé : il coule et vit au fond, et remonte à la nage vers le joueur qu'il poursuit.
- Bateau : posé sur une herbe ou une fleur, il prend sa place au lieu de se poser dessus.
- Nether et End : deux générations de terrain, `nether` et `end`, pour un monde à part (`/world load <nom> nether|end`, ou `level-type`). Le jeu y affiche la bonne dimension (ciel, brouillard, hauteur), d'après les formats relevés auprès du serveur officiel.
- Nether : cinq biomes (terres désolées, forêts carmin et biscornue avec leurs grands champignons, vallée des âmes, deltas de basalte), mer de lave, minerais, forteresses de briques ; ghasts, blazes, wither squelettes, squelettes et endermen y apparaissent.
- End : île centrale, piliers d'obsidienne, fontaine de roche mère, îles lointaines avec leurs plantes de chorus ; des endermen y apparaissent.
- Portails : un cadre d'obsidienne allumé au briquet devient un portail du Nether (il s'éteint si le cadre est cassé) ; douze yeux de l'Ender ouvrent un portail de l'End. Le serveur ne téléporte pas : il émet `PlayerPortalEvent` pour les plugins (voir MultiWorld, `portails` dans config.lua).
- Forteresse du Nether complète : salle centrale, quatre ponts bordés de barrières, jardin de verrues du Nether, deux salles au coffre (butin des forteresses), terrasse des blazes, piliers jusqu'à la lave.
- Monde importé : son Nether et son End deviennent deux mondes à part (`<nom>_nether`, `<nom>_the_end`) à la première ouverture ; la base d'origine n'est pas modifiée.
- Créatures, objets au sol et bateaux : une copie au format du jeu est tenue dans la base du monde (clés `actorprefix` et `digp`), pour les mondes créés par Basalt. Basalt relit toujours ses propres fichiers.
- Bestiaire : trente-neuf créatures de plus (renard, ocelot, panda, ours polaire, tortue, grenouille, chameau, renifleur, tatou, arpenteur, golem de neige, chevaux squelette et zombie, lama et marchand ambulants ; chauve-souris, abeille, perroquet, allay ; morue, saumon, poisson tropical, poisson-globe, poulpe, poulpe luisant, dauphin, axolotl, têtard ; piglin, piglin barbare, piglin zombifié, hoglin, zoglin, cube de magma, endermite, ravageur, évocateur, squelette des marais, warden), avec la vie, la taille, la vitesse et le butin du jeu et un comportement générique : elles marchent, volent, nagent sans quitter l'eau, ou attaquent au contact. Piglins, piglins zombifiés, hoglins, cubes de magma et arpenteurs apparaissent au Nether.
- Bestiaire, comme en vanilla (d'après les fiches Mojang) : nourriture qui attire, reproduction et croissance des petits pour les nouveaux animaux (baies pour le renard, bambou pour le panda, cactus pour le dromadaire, fleurs pour l'abeille, champignon biscornu pour l'arpenteur, champignon carmin pour le hoglin…) ; vitesses de promenade, de fuite et d'attrait du jeu.
- Neutres : ours polaire, abeille et dauphin ne répondent qu'aux coups ; piglins, piglins zombifiés, hoglins, ours, abeilles et dauphins ameutent leurs semblables. Le piglin laisse en paix qui porte une pièce d'armure en or, et troque un lingot d'or (il l'admire six secondes, puis jette son lot).
- L'abeille pique (poison) puis meurt ; le renard et l'ocelot chassent les poules et les lapins, la grenouille gobe les petits slimes et cubes de magma ; le cube de magma saute et se divise comme le slime.
- Poissons, calamars et têtard s'étouffent hors de l'eau et y frétillent ; axolotl et dauphin se dessèchent sur la terre (cinq et deux minutes), le dauphin se noie au bout de quatre minutes sous l'eau. Un seau d'eau attrape un poisson, un axolotl ou un têtard, et le seau vidé le relâche.
- Montures : dromadaire, chevaux squelette et zombie se sellent et se dirigent d'emblée ; l'arpenteur se selle et se dirige au champignon biscornu sur un bâton, et marche sur la lave. Perroquet apprivoisé aux graines (le cookie l'empoisonne), il suit son maître et s'assoit.
- Grenouilles : elles pondent des têtards, qui deviennent grenouilles au bout de vingt minutes (froide, tempérée ou chaude selon le biome). Tatou : écailles au fil du temps et à la brosse ; renifleur : il déterre des graines.
- Apparition : renards (taïgas), ours polaires, tortues (plages), grenouilles (marais), pandas, ocelots et perroquets (jungles), tatous (savanes, badlands) ; dans l'eau, morues, calamars, saumons, poissons tropicaux, poissons-globes et dauphins selon l'océan ; calamars luisants et axolotls dans les grottes ; chauves-souris dans le noir. La nuit, zombies momifiés au désert, vagabonds dans le froid, squelettes des marais et slimes dans les marécages ; endermen et sorcières.
- Course : relâcher la touche de sprint ne fait plus cesser de courir (la course est suivie à ses débuts et à ses fins, comme la nage).
- Créatures : renards, loups et autres n'apparaissent plus sur les arbres (feuillage et troncs ne comptent pas comme un sol) ; tortues et grenouilles nagent sous l'eau et marchent au fond au lieu de flotter ; les poulpes et poissons restent sous la surface et tournent peu à peu ; les poissons apparaissent vraiment dans l'eau (la recherche visait surtout la terre, et l'eau profonde passait pour une grotte) ; des abeilles près des fleurs (forêt fleurie, prairie, cerisiers, plaines).
- /summon propose toutes les créatures dans l'aide à la saisie (warden, piglin, axolotl…) : une liste de choix d'une commande n'est plus coupée à 64 mots (256 maintenant), et un test veille à ce qu'aucune créature n'y manque.
- Tortues : elles pondent de un à quatre œufs sur le sable voisin, qui se fêlent la nuit puis éclosent en bébés tortues.
- Évocateur : il garde ses distances et fait jaillir des lignes de crocs (6 points).
- Warden, comme dans le jeu : invoqué, il sort de terre (7 s) ; il rugit en trouvant sa cible (4,2 s) et ne la lâche plus (64 blocs) ; hors de portée de ses bras, il charge son onde sonique (3 s, tir à 1,7 s : 10 points à travers l'armure et les murs, grand recul, 2 s de recharge) ; obscurité autour de lui toutes les six secondes ; une minute sans cible, il s'enfouit et disparaît. Animations de sortie, rugissement, onde, flair et enfouissement.
- Warden : il ne tire plus son onde sonique dès qu'on est hors de portée de ses bras, mais quand il ne peut pas atteindre sa cible (pas de chemin, trois secondes sans se rapprocher, ou cible perchée) ; sinon il marche sur elle, en contournant les murs par un chemin à sa taille (trois blocs de haut). Il ne recule plus sous les coups (comme le golem de fer ; le ravageur, le hoglin et le zoglin un peu moins). Il entend aussi les portes, trappes et leviers, les coffres ouverts, les objets jetés, les repas et les chutes ; le bruit met un tick par bloc à lui parvenir, la laine l'arrête, et marcher sur la laine ou un tapis est silencieux. À colère égale, un joueur passe avant une créature. Ses grognements suivent sa colère (calme, agité, furieux).
- Tailles : le bébé tortue n'a plus une tête énorme (0,16 comme dans le jeu, au lieu de 0,5) ; bébés panda, dromadaire, renifleur, tatou et dauphin à leur taille ; poisson-globe, poisson tropical et wither squelette adultes un peu plus grands, comme dans le jeu.
- Warden aveugle, comme dans le jeu : il entend les pas (pas ceux de qui avance accroupi), les blocs cassés et posés, les projectiles qui retombent, à 16 blocs, et flaire ce qui vit à 24 blocs ; sa colère monte contre chaque suspect, joueur ou créature (35 par bruit, 100 pour un coup, 1 de moins par seconde) et à 80 il rugit et poursuit le plus haï ; un bruit sans auteur, il va voir. Son cœur bat, de plus en plus vite avec la colère.
- Les chasseurs à pattes (renard, ocelot, loup) ne s'acharnent plus sur une proie dans l'eau.
- Golem de neige : il lance des boules de neige aux monstres (le blaze en souffre), laisse une traînée de neige et fond dans les biomes chauds et hors de la surface.
- Allay : on lui confie un objet, il rapporte à son joueur les objets pareils trouvés au sol (32 blocs) ; main vide, on le reprend.
- Axolotl : il chasse poissons, calamars et noyés sous l'eau. Poisson-globe : il empoisonne qui le frôle. Hoglin : il fuit le champignon biscornu, les portails et les ancres de réapparition. Warden : aveugle, il ne repère pas qui avance accroupi (sauf tout près).
- Poulpes : leur corps s'incline dans le sens de leur nage, tentacules derrière.
- Charge (501 joueurs simulés, Ryzen 9 9900X : 10,5 ticks/s, chaque paquet de mouvement coûtait 170 µs en moyenne et jusqu'à 135 ms) : le contrôle des pieds d'un joueur ne génère plus jamais de chunk sur la boucle du jeu (chunk pas encore là : la chute reste en suspens), et le cache de chunks passe de 2 048 à 8 192 (ils sortaient puis revenaient sans cesse avec des joueurs dispersés). Le rapport /timings détaille le traitement d'un mouvement et compte les chunks produits sur place.
- Plugins : `joueur:setItemInHand(objet, quantité)` remplace l'objet tenu en main (sans objet : main vide).
- NetherNet : `nethernet-threads=auto` par défaut (un fil pour 150 places, sans dépasser la moitié des cœurs). Mesuré à 248 joueurs simulés sur un seul fil : boucle du jeu à 20 ticks/s (3,7 ms par tick) mais fil NetherNet saturé (envoi, chiffrement, lecture), d'où le lag.
- Enregistrement du monde sans à-coups : l'encodage des chunks (des centaines quand on court dans du terrain neuf) et la copie des entités au format du jeu (jusqu'à 1 024 lectures de la base par nouvelle région) se font sur le fil des écritures, et non plus pendant le tick. Ils figeaient le serveur une à plusieurs secondes toutes les minutes (1,2 s mesurée, jusqu'à 7 s), d'où les retours en arrière en courant. Le rapport /timings détaille désormais l'enregistrement.
- Contrôle des déplacements : la distance permise se recharge à chaque mouvement du jeu (un par tick) et non plus à l'horloge. Après un à-coup du serveur, les mouvements en attente arrivaient d'un bloc et le joueur qui courait simplement était replacé (« rollback »).
- Warden : l'onde sonique se voit (anneaux « sonic_explosion » de sa poitrine à sa cible) ; il ne fonce plus tout droit dans un arbre : il ne va en ligne droite que si tout son corps passe (0,9 de large, 2,9 de haut), sinon il prend un chemin qui contourne.
- Course : quand la faim tombe à 6 ou moins, la course s'arrête aussi pour le serveur (elle restait imposée au joueur, qui courait sans plus toucher à la touche).
- Changement de dimension : le joueur n'est plus figé en arrivant (le serveur envoie « changement de dimension terminé », comme l'attend le jeu depuis la 1.19.50).
- Changement de dimension : l'écran de chargement du jeu se referme dès que les chunks autour du joueur sont partis.
- Outil de capture : tout ce qui est reçu en jeu est écrit dans `logs/capture-sequence.txt`.

### Ajouté

- **Mondes enregistrés au format du jeu** : `level.dat`, `levelname.txt` et une base LevelDB
  dans `db/` (lecteur et écrivain écrits de zéro, vérifiés contre le LevelDB de référence).
  Les chunks modifiés y sont écrits entiers ; un ancien `level.nbt` est converti puis gardé
  sous `level.nbt.ancien`.
- L'heure, la météo (avec sa durée restante) et les règles du monde sont dans `level.dat` ;
  les coffres, fours, panneaux et lits sont dans la base du monde, au format du jeu (objets
  avec usure et enchantements). `time.txt`, `weather.txt` et `tiles.nbt` ne servent plus.
- `/gamerule dodaylightcycle|doweathercycle`, par monde.
- **Rapport de plantage** : une erreur interne écrit `crashdumps/crash-<date>.txt` (erreur,
  endroit du code, pile d'appels, version, joueurs, mondes, plugins, dernières lignes du
  journal). Si elle touche la boucle principale, le serveur enregistre les joueurs et les
  mondes avant de s'arrêter. `debug-crash` (console) en produit un pour essai.
- Messages des commandes : une présentation unique (chevron vert : fait, rouge : refusé,
  jaune : mode d'emploi, gris : information) ; arrivées et départs en `[+]` / `[-]`.
- **Ouverture des mondes du jeu** : un dossier de monde ou un `.mcworld` déposé dans `worlds/`
  s'ouvre avec `/world load <nom>` : blocs, contenu des coffres et des fours, panneaux, heure,
  météo et règles du monde normal (créatures d'origine pas encore reprises).
- Tous les mondes sont rangés dans `worlds/<nom>/` ; le monde principal est
  `worlds/<level-name>` (nouveau réglage). Le dossier `world/` y est déplacé au démarrage.

- **Plusieurs mondes ouverts à la fois.** Chaque monde a ses joueurs, ses créatures, ses objets
  au sol, son heure et sa météo. Commande `/world list|load|unload|tp`, mondes rangés dans
  `worlds/<nom>/` et rouverts au démarrage ; un joueur revient dans le monde où il s'était
  arrêté. Pour les plugins : `server.worlds.list/load/unload/isLoaded/default`,
  `joueur:getWorld()`, `joueur:setWorld(nom)`. Plugin d'exemple `examples/plugins/MultiWorld`
  (`/mw`, `/monde`). Le tchat traverse les mondes.
- Plugins : `server.getWorld(nom)` lit et pose des blocs, place créatures et PNJ dans le monde
  nommé (`world:spawnEntity`, `world:spawnNpc`, `world:getPlayers`, `world:getName`).
- Terrain `void` (monde vide), avec une plateforme de pierre au point d'apparition.
- Les commandes du jeu atteignent un joueur d'un autre monde (`/give`, `/kill`, `/gamemode`…) ;
  `/tp <joueur>` emmène dans son monde. Un lit ne vaut que dans son monde.

- `vanilla.properties` : un fichier à part où chaque mécanique du jeu se coupe d'une ligne (faim, régénération, dégâts de chute, feu, noyade, combat entre joueurs, foudre, pousse des cultures et des arbres, écoulement des liquides, TNT, pêche, bateaux, butin, expérience, usure, échanges, sommeil…). Les réglages de jeu de `server.properties` y figurent aussi. Voir docs/INSTALLER.md.
- Plugins : `server.on("damage", …)` annule ou modifie un coup, et `server.on("death", …)` remplace ou supprime le message de mort.
- Villages : les enclos sont peuplés de deux à quatre vaches, moutons, cochons ou chevaux.
- Sons des blocs qui s'ouvrent : portes, portillons, trappes et boutons, avec la variante de leur matière (bois, fer, cuivre…) ; coffres, coffres de l'Ender, boîtes de Shulker et tonneaux, dont le couvercle s'ouvre et se referme chez tous ceux qui les voient.
- Coffres doubles : un coffre posé contre un autre de même sorte et de même orientation s'unit à lui (pas en étant accroupi) ; les deux s'ouvrent comme un seul inventaire de 54 cases, et redeviennent simples quand l'un est cassé. Deux coffres voisins arrivés autrement (`/setblock`, anciens mondes) s'unissent à leur première ouverture.
- Plugins : `server.inventory(titre, taille)` crée un écran de coffre de 27 ou 54 cases, que le plugin remplit (`setItem`), ouvre chez un joueur (`open`) et dont il reçoit les clics (`onClick`) et la fermeture (`onClose`). Verrouillé par défaut, pour faire des menus ; `{ free = true }` en fait un vrai coffre sans bloc. Voir docs/PLUGINS.md.
- Enchantements, suite. Sur les outils : fortune (les minerais rendent davantage) et toucher de soie (le bloc lui-même : minerais, pierre, herbe, verre, glace, feuilles…). Sur les armes : butin (les créatures lâchent plus), recul, châtiment contre les morts-vivants et fléau des arthropodes ; l'aura de feu enflamme aussi les joueurs. Sur l'arc : flamme et frappe. Sur l'armure : épines, protection contre les projectiles et contre les explosions. Raccommodage : l'expérience ramassée répare l'objet usé tenu en main ou porté. Semelles givrantes : l'eau gèle sous les pas, puis fond.
- Le gravier donne parfois du silex, et les feuilles cassées à la main une pousse, des bâtons ou une pomme ; la fortune améliore ces chances, ainsi que la récolte des cultures, du melon, de la pierre lumineuse et de la lanterne marine.

- Météo : la pluie et l'orage viennent et repartent d'eux-mêmes (`weather-cycle=false` pour l'empêcher), et se commandent avec `/weather <clear|rain|thunder> [secondes]` ou `/weather query`. La pluie éteint les feux et les joueurs à ciel ouvert, et les morts-vivants n'y brûlent plus au soleil. Pendant l'orage, la foudre tombe près des joueurs : elle blesse, enflamme et allume un feu ; `/summon lightning_bolt` la fait tomber où l'on veut. Dans les biomes enneigés la neige se dépose et l'eau gèle ; ailleurs la pluie arrose la terre labourée et remplit les chaudrons ; il ne pleut pas dans les déserts, les savanes et les badlands. La météo est gardée avec le monde.
- Arbalète : elle se charge en 1,25 seconde de clic maintenu, reste chargée, puis tire au clic suivant une flèche plus rapide et plus forte que celle de l'arc. Charge rapide et tir multiple agissent.
- Trident : brandi une demi-seconde puis lancé (8 de dégâts), il se ramasse là où il tombe. Ses enchantements agissent : loyauté (il revient), impulsion (dans l'eau ou sous la pluie, c'est le joueur qui est propulsé), canalisation (la foudre tombe sur la cible en plein orage), empalement (plus de dégâts dans l'eau ou sous la pluie). Un noyé sur seize naît armé d'un trident, le lance, et le lâche parfois en mourant (valeurs des fiches du jeu).
- Pêche : la canne lance un bouchon qui flotte ; après 5 à 30 secondes ça mord (éclaboussure, le bouchon plonge), et relever la ligne à ce moment rapporte une prise et un peu d'expérience. Poissons, rebuts et trésors suivent les tables de butin du jeu ; l'appât raccourcit l'attente et la chance de la mer favorise les trésors.
- Bateaux : l'objet se pose sur l'eau ou sur le sol, le bateau flotte, se monte d'un clic droit, se dirige, et se casse à coups de poing en rendant son objet. Deux places : le premier monté rame, le second est passager et prend les rames si le rameur descend. Un bateau vide suit le courant. Les dix bois, radeau de bambou compris. Les bateaux sont enregistrés avec le monde.
- Poudre d'os : une culture gagne de deux à cinq stades, une pousse a 45 % de chances de devenir un arbre, l'herbe se couvre d'herbes hautes et de fleurs.
- Les créatures se noient : quinze secondes d'air, puis deux points de vie par seconde. Les morts-vivants respirent sous l'eau ; le zombie resté quinze secondes sous l'eau devient un noyé, le zombie momifié un zombie (valeurs des fiches du jeu).

### Corrigé

- Créatures : leur corps a désormais une largeur dans les collisions (la moitié du corps
  passait dans le bloc d'à côté) et elles se posent sur le bord d'une marche.
- Dans l'eau, les animaux flottent le corps immergé et la tête dehors (ils semblaient marcher
  sur l'eau) ; zombies et squelettes coulent pour de bon et marchent au fond, même en
  poursuivant un joueur (ils remontaient le long des parois), et finissent par s'y noyer.
- Une pousse (poudre d'os ou temps) donne le même arbre que la génération du monde : chêne
  et grand chêne, bouleau, sapin, acacia, cerisier, arbre de la jungle ; quatre pousses en
  carré donnent le grand sapin, le grand arbre de la jungle, le chêne noir, le chêne pâle.
- Armure enfilée d'un clic droit, en visant un bloc comme dans le vide : le serveur l'enfile
  vraiment (elle n'était que visible ; dans le vide, le paquet du jeu n'était pas reconnu). Bruit de la pièce enfilée, selon sa matière, quel que soit le geste.
- La liste des joueurs est celle du serveur entier, tous mondes confondus.
- Monstres : ils contournent les obstacles pour atteindre leur proie (recherche de chemin,
  refaite chaque seconde) au lieu de buter en ligne droite ; ils ne frappent plus à travers
  un mur ; zombies et vindicateurs défoncent les portes en bois (douze secondes).
- Une explosion casse aussi les coffres, fours et lits ; leur contenu tombe au sol.
- Un animal mort en flammes (aura de feu) lâche sa viande cuite.
- Bateau : il ne traverse plus les blocs pleins.
- Nager continue quand on lâche la touche de sprint ; manger sous l'eau n'est plus interrompu.
- Grands sapins : leur cime a de nouveau des feuilles (le tronc dépassait, nu). Lianes des
  marais et de la jungle : elles ont une face, donc elles se voient.
- La liste des joueurs (« @ » dans le tchat, choix d'un joueur dans une commande) suit les
  arrivées et les départs, où que soient les joueurs.
- Créatures : elles traversent herbes marines et laminaires, et se tiennent à la bonne hauteur
  sur un lit ou une dalle basse (un chat couché sur un lit flottait au-dessus).
- Sprinter ou nager dans l'eau n'est plus annulé cinq fois par seconde par la mise à jour de
  l'air.

- Les clôtures en chêne noir des structures étaient remplacées par de la pierre.

- Accroupi avec un objet en main, cliquer sur un coffre, une porte ou un autre bloc utilisable pose l'objet au lieu d'utiliser le bloc, comme dans le jeu (on peut donc poser un coffre contre un autre).
- Armure enfilée d'un clic droit : l'échange n'est plus fait deux fois quand le jeu envoie aussi sa propre demande, ce qui le défaisait côté serveur.
- Effet de rapidité : le sprint ne s'interrompt plus (la vitesse envoyée au jeu tient compte du sprint, et repart à chaque changement d'allure).
- Les zombies et les squelettes coulent et marchent au fond de l'eau au lieu d'y flotter ; le noyé nage toujours.
- En bateau, le joueur est assis dans la coque et non plus dessous.

### Modifié

- `/enchant` applique les règles du jeu : un enchantement ne va que sur les objets qui l'acceptent, et pas avec un enchantement qu'il exclut. Les objets de plugins acceptent tout.

## [0.0.6] — 2026-11-25

### Ajouté

- Créatures volantes : le blaze (il tire des petites boules de feu), le ghast (ses boules de feu explosent) et le phantom (il tourne au-dessus du joueur, fond sur lui et mord ; il brûle au soleil). Par `/summon` ou `server.entities.spawn`. Une créature de plugin bâtie sur l'un d'eux vole aussi.
- API de gestion à distance : `api-port` dans `server.properties` ouvre une petite API HTTP (JSON). Un jeton de lecture donne l'état du serveur, les joueurs, les objets et les blocs ; un jeton d'administration permet en plus de désactiver ou réactiver un objet ou un bloc à chaud, de recharger les plugins et de lancer une commande de console. Éteinte par défaut, et limitée à la machine elle-même tant que `api-bind` ne dit pas autre chose. Voir `docs/API.md`.
- Tableau de bord web : l'API sert une page (à l'adresse de l'API, dans un navigateur) avec une barre de navigation, un thème sombre et un thème clair. Elle montre l'état du serveur, les joueurs, les objets et les blocs, les packs, et donne les actions d'administration. Elle fonctionne sur téléphone.
- Analyse de la carte (`POST /v1/scan`, et l'onglet Analyse du tableau de bord) : le compte de tous les objets rangés dans les conteneurs et chez les joueurs, présents ou absents, et ce qui ressemble à une duplication — piles impossibles, conteneurs au contenu identique, objets désactivés encore en circulation, détenteurs d'objets rares.
- Images des objets et des blocs dans le tableau de bord (`GET /v1/texture`). Celles de vos objets et blocs viennent de vos packs de ressources, chiffrés ou non ; celles du jeu, d'un pack que vous déposez vous-même dans `vanilla_resource_pack/` (Basalt n'en livre aucune).
- Gestion des joueurs à distance (`POST /v1/players/action` et la fiche d'un joueur dans le tableau de bord) : téléporter à des coordonnées ou auprès d'un autre joueur, changer le mode de jeu, donner un objet ou un effet, écrire, soigner, vider l'inventaire, tuer, expulser.
- Serveurs chargés : la liste des joueurs de l'API vient par pages avec recherche côté serveur ; les listes d'objets et de blocs sont composées une fois puis resservies ; les projectiles ne testent plus que les joueurs et créatures voisins ; le calcul de lumière des apparitions de monstres est plafonné par seconde et réparti entre les joueurs.
- Chiffrement des packs de ressources : `basalt encrypt-pack <pack>` (sans démarrer le serveur), `encryptpack <pack>` en console, ou `POST /v1/packs/encrypt` par l'API. Écrit le pack chiffré et sa clé à côté de l'original.
- Accès à l'API depuis l'extérieur : huit jetons refusés en dix minutes écartent une adresse dix minutes (`429`), l'adresse du visiteur est lue derrière un tunnel ou un serveur HTTPS local, `apitoken reset` renouvelle les jetons sans redémarrer, et le tableau de bord prévient quand la page n'est pas chiffrée. `docs/API.md` détaille Tailscale, le tunnel Cloudflare, SSH et le serveur HTTPS.
- Carte en direct dans le tableau de bord (`GET /v1/map`) : le monde vu du dessus autour des joueurs, leurs positions rafraîchies toutes les deux secondes, et un clic sur un bloc pour le remplacer, le retirer ou y téléporter un joueur (`GET` et `POST /v1/world/block`, `POST /v1/world/fill`). Seul le terrain déjà en mémoire est montré : la carte ne fait rien générer.
- Carte en relief dans le tableau de bord (`GET /v1/world/chunks`, `GET /v1/entities`) : le terrain chargé dessiné en cubes, les joueurs et les créatures dans une boîte à leur taille avec leur nom, et un clic pour viser un bloc précis. Un chunk n'est renvoyé que s'il a changé.
- Couleurs de la carte tirées des textures : la moyenne de la texture du dessus de chaque bloc, lue dans vos packs puis dans `vanilla_resource_pack/` (PNG et TGA). Sans le pack du jeu, la carte garde ses couleurs approchées.
- Vraies textures dans la carte en relief (`GET /v1/world/atlas`, `GET /v1/world/skin`) : avec le pack du jeu, chaque face de bloc porte sa texture et les joueurs sont dessinés en Steve, tournés comme en jeu.
- Feu : le briquet et la boule de feu allument un feu, qui brûle les blocs inflammables (selon les données du jeu), gagne ses voisins et s'éteint faute de combustible ; la lave enflamme ce qui brûle près d'elle, la petite boule de feu du blaze aussi. `fire-spreads=false` l'empêche de se propager.
- Brûlures : se tenir dans le feu ou la lave blesse et enflamme les joueurs comme les créatures ; l'eau éteint ; le blaze, le ghast et le wither squelette y sont insensibles.
- TNT : allumée au briquet, par le feu ou par une explosion, elle clignote, tombe et explose au bout de sa mèche (quatre secondes, moins dans une réaction en chaîne).
- Les effets en cours sont gardés quand un joueur se déconnecte, et reprennent à son retour pour le temps qu'il leur restait. Sa vitesse de marche est recalculée à ce moment-là (vitesse, lenteur).
- Mines abandonnées : au-dessus d'une grotte, la galerie passe sur un plancher de planches ; les rails et les étais ne flottent plus dans le vide.
- Noyade : la tête sous l'eau, un joueur perd son air en quinze secondes (les bulles s'affichent), puis un cœur par seconde ; la respiration aquatique l'en dispense.
- Nature : l'herbe gagne la terre voisine et meurt sous un bloc opaque ou sous l'eau ; une pousse d'arbre devient un arbre quand elle a la place (chêne, bouleau, épicéa et les autres essences sur le modèle du chêne).
- Créatures : un zombie sur vingt apparaît bébé (plus rapide, il ne grandit pas) ; le blaze blesse aussi au contact.
- Enchantements : un objet peut en porter jusqu'à cinq ; il brille, les affiche dans son infobulle, et les garde dans l'inventaire, au sol, dans les coffres et d'une session à l'autre. `/enchant <joueur> <enchantement> [niveau]` enchante l'objet tenu en main. Agissent déjà : protection, protection contre le feu, chute amortie, tranchant, aura de feu, solidité, efficacité, puissance, infinité, apnée.
- Les coffres gardent la variante et l'usure des objets (une potion rangée restait une bouteille d'eau après un redémarrage).
- Clic droit avec une pièce d'armure en main : elle est enfilée (celle qu'elle remplace revient en main). Le serveur la croyait restée dans la barre d'objets, et elle restait affichée dans sa case d'armure après avoir été retirée.
- La case de la barre d'objets tenue en main est gardée d'une session à l'autre. La vie et la faim sont redonnées au jeu deux secondes après l'arrivée : il affichait des barres pleines.
- Objets et blocs désactivés (`disabled.txt`) : on ne peut plus les poser ni s'en servir, sans toucher au monde.
- Potions : `potion` se boit, `splash_potion` et `lingering_potion` se lancent et éclaboussent les joueurs à moins de quatre blocs. Les 43 variantes du jeu sont reconnues. `/give <joueur> <objet> [quantité] [variante]` accepte un numéro, ou le nom d'une potion (`give Alex splash_potion 1 poison`).
- Sorcière : elle lance des potions de poison, de lenteur, de dégâts ou de faiblesse. Plugins : `potion` pour les créatures qui lancent des `splash_potion`.
- Aliments à effets : pomme dorée, chair putréfiée, poulet cru, œil d'araignée, pomme de terre empoisonnée, poisson-globe.
- Phantoms : la nuit, ils viennent d'eux-mêmes chercher les joueurs en survie restés trois jours sans dormir (ni mourir), à ciel ouvert. Dormir une nuit les écarte.
- Feu : la petite boule de feu du blaze enflamme le joueur cinq secondes (un demi-cœur par seconde). L'eau l'éteint, et l'effet de résistance au feu en protège.
- Boule de feu du ghast : un joueur qui la frappe la renvoie là où il regarde.
- Le blaze tire par rafales de trois.

### Modifié

- Grosses charges de TNT : quatre explosions au plus par tick, et le butin d'une explosion tombe en quelques piles au lieu de centaines d'objets. Le rapport de mesures détaille le feu, la TNT, les effets et l'API.
- La bedrock, les barrières et les blocs techniques ne cèdent à rien (explosion, feu, minage hors créatif), quoi qu'en disent les données ou un plugin ; seul un joueur en créatif les casse.
- Le vide sous le monde : les objets au sol et la TNT amorcée y tombent et disparaissent, au lieu de rester posés sur un fond invisible. Un objet brûlé par la lave ou le feu grésille.
- La rapidité et la lenteur agissent dès qu'on marche : le serveur donne au jeu la vitesse correspondante quand un effet commence ou finit, et le contrôle des déplacements en tient compte.
- Explosions calculées comme dans le jeu : des rayons partent du centre et s'usent dans chaque bloc selon sa résistance, si bien qu'un mur abrite ce qui est derrière lui. Une TNT laisse tomber tous les blocs qu'elle souffle, saute légèrement à l'allumage et siffle.
- Le phantom attaque comme dans le jeu : il tourne autour d'un point fixé 20 à 40 blocs au-dessus de l'endroit où était sa proie, sur un cercle de 5 à 15 blocs, ne pique que toutes les dix à vingt secondes et renonce s'il la manque.
- Le squelette, le vagabond et le pillard lèvent leur arme quand ils ont une cible : le serveur annonce cette cible au jeu.
- Créatures alignées sur les fiches du jeu : le ghast repère et tire jusqu'à 64 blocs, le blaze jusqu'à 48, le phantom et l'enderman repèrent à 64 ; la boule de feu du ghast file à 1,6 bloc par tick et celle du blaze à 1,3 ; le squelette tire toutes les 3 secondes, le pillard à 8 blocs ; l'enderman en colère court moitié plus vite ; la poule a sa vraie boîte (0,6 × 0,8).
- Le tir d'un blaze et celui d'un ghast font leur propre bruit, plus celui de l'arc. La sorcière vise comme dans le jeu (potion moins en cloche), et le lancer de potion du joueur a la même force quel que soit l'angle.
- La petite boule de feu (`small_fireball`) n'explose plus à l'impact, comme dans le jeu ; seule la grosse (`fireball`) explose.

## [0.0.5] — 2026-11-24

### Ajouté

- **Monstres** : quinze monstres du jeu. Zombie, momie, noyé, zombie-villageois, araignée, araignée venimeuse (elle empoisonne), poisson d'argent, wither squelette et vindicateur frappent ; squelette, vagabond et pillard tirent des flèches ; le creeper explose ; l'enderman, neutre, ne répond qu'aux coups et se téléporte ; le slime a trois tailles et se divise en mourant. Tous par `/summon` ou `server.entities.spawn`.
- **Apparition dans le noir** : zombies, squelettes, creepers et araignées apparaissent autour des joueurs en survie là où il fait noir, la nuit en surface et le jour dans les grottes. Une torche (ou tout bloc lumineux) les écarte. Ils disparaissent au loin, et zombies et squelettes brûlent au soleil. `spawn-monsters=false` pour un monde paisible.
- **Lumière** calculée selon les règles du jeu : elle contourne les murs, faiblit de 1 par bloc, et le ciel éclaire ce qui est à découvert.
- **Projectiles et explosions.** `mob-griefing=false` : les explosions blessent sans rien casser.
- **Arc** : un joueur le bande (clic tenu) et tire en relâchant ; pleine puissance en une seconde (6 points de dégâts). Une flèche est consommée et l'arc s'use, sauf en créatif.
- **Armure** : elle absorbe 4 % des dégâts par point (60 % pour une armure de fer complète, 80 % en diamant) contre les coups, les flèches et les explosions, et chaque pièce s'use. Elle se voit sur le joueur chez les autres. Les zombies ont leur armure naturelle.
- **Effets des joueurs** : commande `/effect <joueur> <effet> [secondes] [niveau]` et `/effect <joueur> clear`. Les trente effets du jeu s'affichent ; régénération, poison, flétrissement, soin et dégâts instantanés, faim, satiété, force, faiblesse et résistance agissent côté serveur.
- Les créatures se poussent au lieu de se traverser, et un joueur les pousse. Les araignées escaladent les murs quand elles poursuivent un joueur.
- Une demi-seconde d'invulnérabilité suit chaque coup reçu, comme dans le jeu : entouré de monstres, un joueur n'encaisse pas un coup par monstre et par seconde.
- Plugins, créatures : `hostile`, `attackDamage` et `sight` ; `projectile`, `projectileDamage`, `shootInterval` et `shootRange` pour en faire des tireurs ; `explosion` pour en faire des bombes.
- Plugins : `server.entities.shoot(...)` pour tirer un projectile, les événements `ProjectileHitEvent` et `ExplosionEvent`, et `joueur:addEffect`, `removeEffect`, `clearEffects`.
- Plugins, blocs créés : boîte réglable avec `width` et `height` (une dalle : `height = 0.5`).

### Modifié

- Nettoyage du code signalé par l'analyseur (constructions inutiles, bornes écrites à la main).
- Feuille de route mise à jour.

### Corrigé

- Pose de blocs : un bloc plein ne se pose plus là où se tient un joueur ou une créature (pour monter, il faut sauter d'abord).
- Bloc qui tombe (sable, gravier) : l'eau et la lave voisines comblent maintenant le trou qu'il laisse.
- Golem de fer des villages : il pouvait naître sur le puits ou sur un toit et y rester coincé. Il naît maintenant sur un endroit dégagé de la place, et un golem déjà coincé y est reposé.
- Eau : casser un bloc à côté d'une herbe marine ou d'une algue laisse maintenant l'eau s'y écouler (ces plantes n'étaient pas comptées comme de l'eau).
- Grande herbe marine : en casser une moitié emporte l'autre, qui laisse de l'eau.
- Réapparition : après une mort (ou un réveil), le joueur retrouve sa boîte de joueur debout ; il ne passe plus sous un bloc comme s'il rampait.
- `/summon` en jeu : toutes les créatures créées par les plugins y sont proposées et acceptées, pas seulement celles qui ont leur propre apparence.
- Terrain : pétales de cerisier, fleurs sauvages et feuilles mortes ne restent plus perchés sur une autre plante ; ils ne tiennent que sur le sol.

## [0.0.4] — 2026-11-24


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
