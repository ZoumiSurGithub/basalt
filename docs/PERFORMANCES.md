# Performances

La boucle du jeu (20 ticks par seconde) ne fait que la logique du jeu. Le reste tourne sur
d'autres cœurs : la liaison NetherNet (chiffrement, lecture et envoi), la génération du
terrain, l'encodage et la compression des chunks, la vérification des connexions Xbox, la
préparation des liaisons, les plugins, l'écriture sur le disque et du journal.

Le serveur est conçu pour de grandes populations, et un outil de charge permet de
le vérifier : `cargo run --release --example stress -- 1000` simule des joueurs
qui se connectent, entrent en jeu, puis envoient chacun 20 paquets par seconde.

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
- **Tampons réseau agrandis** (8 Mo demandés au système).
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
