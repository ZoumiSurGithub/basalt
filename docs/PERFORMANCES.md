# Performances

La boucle du jeu (20 ticks par seconde) ne fait que la logique du jeu. Le reste tourne sur
d'autres cœurs : la liaison NetherNet (chiffrement, lecture et envoi), la génération du
terrain, l'encodage et la compression des chunks, la vérification des connexions Xbox, la
préparation des liaisons, les plugins, l'écriture sur le disque et du journal.

## Mesures avec de vrais joueurs simulés (novembre 2026)

Mesures faites avec des bots qui jouent vraiment, par NetherNet comme le jeu : ils marchent,
courent, se battent, cassent et posent des blocs, meurent et réapparaissent, parlent dans le
chat. Pendant la plupart des mesures, un plugin de chaos leur donnait des objets au hasard (TNT, briquet,
seaux de lave et d'eau, arc, potions…), d'où des explosions, du feu et des milliers d'objets au
sol. Les bots étaient dispersés dans un rayon de 2 000 blocs, en survie, sur un monde vanilla.

Machine : un Ryzen 9 9900X (12 cœurs, 24 fils) sous Windows, qui faisait tourner **à la fois**
le serveur et tous les bots. Les chiffres viennent de `/timings report`, sur 4 à 6 minutes.

| Joueurs | Ticks par seconde | Tick moyen | Repos de la boucle | Déconnexions |
|---|---|---|---|---|
| 315 | 20,0 | 7 ms | 81 % | 1 (lot illisible, corrigé depuis) |
| 501 | 17,7 (mesuré avant les correctifs des objets au sol et du cache de chunks) | 30 ms | 39 % | 0 |
| 601 | 19,75 | 16 ms | 62 % | 0 |
| 629 | 20,0 | 18 ms | 55 % | 86 (« retry », corrigé depuis) |
| 801 | 18,4 à 18,6 | 21 ms | 48 à 50 % | 0 |

À 800, la boucle du jeu est encore au repos la moitié du temps, mais environ 12 % des ticks
dépassent 50 ms sans qu'aucune étape ne l'explique : à ce stade, c'est la machine, partagée
avec les 800 bots, qui fait attendre le serveur. La limite du serveur seul n'est pas encore
connue ; il faudrait faire tourner les bots sur une autre machine (l'outil ne sait pas encore
le faire : il se connecte sans compte Xbox, sur la même machine).

Repères pour la lecture d'un rapport à 800 joueurs : la logique du jeu prend environ 21 ms
par tick (créatures 5 ms, objets au sol 4 ms, joueurs vus par les autres 3 ms, envoi du
terrain 2,5 ms) ; le réseau occupe l'équivalent de 3 à 4 cœurs sur ses propres fils ; la
génération du terrain se fait sur les fils `chunks-…` et `terrain-…`, hors de la boucle.

### Ce qui a été corrigé pour y arriver

Au départ de ces tests, le serveur tombait à 7 à 10 ticks par seconde vers 330 à 500 joueurs,
et toutes les liaisons tombaient d'un coup vers 250. Dans l'ordre :

- **Un lot par joueur et par tour, plus un lot par paquet.** Chaque paquet partait seul, et
  donc chiffré seul : les fils NetherNet recevaient plus de 100 000 envois par seconde chacun,
  saturaient vers 250 joueurs, et toutes les liaisons tombaient. Les paquets d'un joueur sont
  maintenant regroupés, puis emballés et compressés sur son fil NetherNet.
- **Pas de génération de terrain pendant un déplacement.** Chaque mouvement de joueur pouvait
  faire générer un chunk sur place (170 µs par mouvement en moyenne, jusqu'à 135 ms).
- **Cache de chunks à la taille de ce que voient les joueurs** (de 8 192 à 65 536 chunks).
  Trop petit, il faisait régénérer sans cesse les chunks déjà vus (70 ms chacun).
- **Objets au sol** : le nouvel objet n'est montré qu'aux joueurs proches (avant, chaque
  objet lâché refaisait le tour de tous les objets pour chaque joueur : 15 ms par bloc cassé,
  plus d'une seconde quand un joueur mourait), la nature de chaque bloc est calculée une fois,
  chaque joueur ne regarde que les objets de son voisinage, et leurs déplacements sont envoyés
  d'après ce que chacun voit.
- **Un bloc qui change n'est annoncé qu'à ceux qui ont son chunk**, comme dans le jeu.
- **Le feu par dixièmes** : une forêt en feu ne fige plus un tick sur dix (jusqu'à 390 ms).
- **Sauvegarde sur un autre fil** : l'enregistrement automatique figeait le serveur 1 à 7 s
  et faisait reculer les joueurs (« rollback »).
- **Connexions** : un lot illisible isolé n'exclut plus le joueur, 30 s pour se connecter au
  lieu de 10, et une nouvelle connexion arrivant du même port qu'un joueur en jeu ne le coupe
  plus (le système réutilise les ports).
- **Fils NetherNet automatiques** (`nethernet-threads=auto`) : un pour 150 places, la moitié
  des cœurs au plus.

Pour refaire la mesure : `local-bots=true` et `max-players` assez grand dans
`server.properties`, `/timings on` dans la console, puis
`cargo run --release --example bots -- 600 127.0.0.1:19132 2000 --fils=12`, et
`/timings report` au bout de quelques minutes. Le journal écrit aussi un bilan de chaque fil
NetherNet toutes les 5 s au-delà de 50 sessions.

## Anciennes mesures (transport RakNet, retiré)

Le serveur est conçu pour de grandes populations, et un outil de charge permet de
le vérifier : `cargo run --release --example stress -- 1000` simule des joueurs
qui se connectent, entrent en jeu, puis envoient chacun 20 paquets par seconde.

Les chiffres ci-dessous datent de l'ancien transport RakNet, retiré depuis, et de joueurs
simulés qui ne faisaient que marcher : ils ne disent rien du coût d'un vrai gameplay.

Mesures sur une petite machine de 2 cœurs, qui faisait aussi tourner les joueurs simulés :

| Joueurs simulés | Processeur du serveur | Mémoire | Ticks par seconde | Joueurs perdus |
|---|---|---|---|---|
| 200 | 5 % d'un cœur | 4 Mo | 20,0 | 0 |
| 1 000 | 14 % d'un cœur | 7 Mo | 20,0 | 0 |
| 2 000 | 25 % d'un cœur | 10 Mo | 20,0 | 0 |
| 4 000 | 44 % d'un cœur | 18 Mo | 20,0 | 0 |

Avec le terrain qui suit les joueurs (1 000 joueurs qui marchent, environ 3 200
chunks envoyés par seconde) : 18 % d'un cœur, 10 Mo, 20,0 ticks par seconde, aucun joueur perdu.
Les quatre lignes du tableau datent d'avant cette fonction.

Cas le plus dur, 1 000 joueurs partant du même point et se voyant entre eux (48
joueurs affichés chacun) : 31 % d'un cœur, 12 Mo, 20,0 ticks par seconde, aucun joueur
perdu, mais **environ 110 Mbit/s envoyés**. Dans une foule, c'est la bande passante
qui limite, pas le processeur ; `max-visible-players` et `player-view-distance`
permettent de la réduire.

Par défaut (`max-visible-players=0`), il n'y a pas de plafond : chacun voit tous les joueurs
à portée de vue (`player-view-distance`). Le trafic croît alors avec le carré du nombre de
joueurs réunis ; pour un hub très fréquenté, un plafond (`max-visible-players=48` par
exemple) le réduit.

**Ce que ces chiffres mesurent** : le transport réseau (réception, accusés,
sessions), l'entrée en jeu, l'envoi du terrain et l'affichage des joueurs entre eux, sur
la même machine, sans authentification. **Ce qu'ils ne mesurent pas** : un vrai
gameplay (combat, objets au sol, eau qui coule, plugins). Le coût par joueur augmente avec
chaque fonction ajoutée, et une vraie connexion Internet perd des paquets, ce que la
machine locale ne reproduit pas. Sur le serveur Debian de test, avec deux vrais joueurs, la
commande `timings` relève 20 ticks par seconde.

**Compression réseau** (`network-compression=true`, par défaut) : les paquets de plus de
512 octets partent compressés (DEFLATE, compresseur intégré à Basalt). Avec 400 joueurs
simulés, chaque joueur reçoit environ 110 Ko pour entrer en jeu au lieu de 825 Ko, pour un
coût processeur négligeable : les chunks sont compressés sur les fils de travail, et les
gros paquets identiques pour tous (objets, biomes) une seule fois au démarrage. Sur un
réseau local très rapide, `network-compression=false` économise ce petit travail.

Choix d'architecture faits pour tenir la charge :

- **Boucle principale jamais bloquée** : lecture réseau non bloquante, bornée en
  nombre et en durée, pour que les 20 ticks par seconde restent réguliers.
- **Travail lourd sur les autres cœurs** : l'analyse des logins et la vérification
  des signatures Xbox (environ 2 ms par joueur) sont confiées à des fils de travail.
- **Tampons réseau agrandis** (16 Mo demandés au système pour la prise NetherNet).
- **Limites proportionnelles** : le débit d'ouvertures de session suit `max-players`.
- **Surveillance** : la commande `status` affiche les ticks par seconde, le pire
  retard et le trafic ; un avertissement apparaît si le serveur passe sous 18 ticks/s.

Pour un grand serveur :

- réglez `max-players` dans `server.properties` ;
- sous Linux, autorisez de grands tampons réseau :
  `sudo sysctl -w net.core.rmem_max=8388608 net.core.wmem_max=8388608` ;
- surveillez `status` pendant les pics d'arrivées.

Règles à respecter pour les fonctions à venir, afin de rester à 1000 joueurs :

- **Ne jamais tout envoyer à tout le monde.** Chaque joueur ne reçoit que ce qui se
  passe près de lui, avec un plafond de joueurs affichés et un rafraîchissement
  ralenti dans les foules ; sinon 1000 joueurs réunis représentent un million de
  mises à jour par tick. Les prochaines fonctions (chat local, blocs, entités)
  doivent suivre la même règle.
- **Terrain, compression et chiffrement hors de la boucle principale**, sur les
  fils de travail, comme les logins. L'eau et la lave sont plafonnées à 3 000 mises à
  jour par tick : une grande inondation coule plus lentement au lieu de ralentir le
  serveur. Les recherches `/locate` tournent sur un fil à part.
- **Mesurer chaque ajout** avec l'outil de charge avant de le garder.

Même en suivant ces règles, 1000 joueurs dans une même zone d'un seul monde reste
difficile pour n'importe quel serveur Minecraft. Les grands réseaux répartissent
leurs joueurs sur plusieurs serveurs derrière un proxy ; Basalt devra permettre
les deux approches.
