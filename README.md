# Basalt

Socle de serveur **Minecraft Bedrock** écrit en Rust, dans l'esprit de PocketMine-MP :
un seul exécutable, un fichier `server.properties`, une console, et des plugins en Lua.

Le cœur du serveur n'utilise que la bibliothèque standard de Rust. Seul le transport
NetherNet (exigé par les jeux 1.26.60 et plus) s'appuie sur deux bibliothèques en Rust pur
(`str0m`, `p384`). Les plugins utilisent Lua 5.4, chargé au démarrage s'il est présent.

## État du projet

Basalt est un **serveur en construction**, mais déjà jouable : il a été essayé avec le vrai
jeu (Bedrock 1.26.63 à 1.26.66 sous Windows, deux comptes Xbox en même temps).

**Vérifié avec le vrai jeu :**

| Fonction | Détails |
|---|---|
| Connexion | NetherNet (jeux 1.26.60+), compte Xbox vérifié, liste des serveurs |
| Monde | casser et poser des blocs (escaliers, orientations, angles), sauvegarde et rechargement (sur terrain plat) |
| Joueurs | visibles entre eux avec leur skin, liste des joueurs (pause, Social, `/list`), rotation de la tête, position et rotation retrouvées à la reconnexion |
| Inventaire | sac, barre, armure, seconde main, coffres, fours, menu créatif, fabrication (vérifiée par le serveur) |
| Combat et survie | coups entre joueurs avec recul, dégâts de chute, mort, écran de réapparition, inventaire lâché à la mort (`keep-inventory` en option) |
| Modes de jeu | survie, créatif, aventure, spectateur |
| Objets au sol | lâcher (inventaire, Q, Ctrl+Q), physique, fusion des tas, ramassage, disparition au bout de 5 minutes |
| Commandes | chat et commandes en jeu, `/tp` et téléportation par les plugins |
| Anti-triche | contrôle des déplacements (le tricheur est replacé), fabrication vérifiée, inventaire tenu par le serveur |
| Plugins Lua | événements à la PocketMine, commandes, accès au monde, EasyEdit fourni (entièrement en plugin) |

**Écrit et testé automatiquement, pas encore essayé avec le vrai jeu :**

- génération du terrain à la manière du jeu (voir « Le monde » ci-dessous) ;
- connexion par RakNet (jeux plus anciens que 1.26.60), avec son chiffrement (ECDH + AES-256) ;
- installation et mise à jour sous Windows par les scripts fournis ;
- montée en charge (voir plus bas).

**Pas encore fait :** chargement de vrais mondes Minecraft (format LevelDB),
mondes multiples (Nether, End), blocs qui réagissent (eau, lave, sable, redstone),
monstres et animaux, faim et expérience, cycle jour/nuit animé, météo.

**Réglages de secours.** Si un joueur est replacé à tort, ou se déconnecte quand un autre
joueur apparaît, `movement-check=false` et `show-players=false` désactivent ces fonctions
le temps de corriger.

### Le monde

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
- **arbres** de chaque biome (chêne, grand chêne, bouleau, sapin, sapin géant, chêne noir,
  chêne pâle, acacia, jungle et jungle géante, cerisier, palétuvier), fleurs, herbes,
  fougères, buissons de baies, cactus, cannes à sucre, nénuphars, algues, pics de glace ;
- **minerais** à leurs hauteurs (émeraude dans les montagnes), poches de granite,
  diorite, andésite, tuf, gravier.

Pas encore générés : villages et autres structures, lacs, aquifères (l'eau souterraine du
jeu), filons géants de fer et de cuivre. Les valeurs ne sont pas celles du jeu : le monde
a la même allure, mais une graine ne redonne pas le même monde que dans Minecraft.

Dans `server.properties` :

- `level-type=normal` (comme le jeu) ou `level-type=flat` (plat) ;
- `level-seed=` : un nombre ou un texte ; vide, la graine est tirée au hasard.

Ces deux réglages ne servent qu'à la création d'un monde : le type de terrain et la graine
sont ensuite enregistrés dans `level.nbt`, et un monde existant garde les siens. Les mondes
créés avant la génération de terrain restent plats, et ceux créés avec le premier terrain
de Basalt gardent ce terrain. Pour un nouveau monde, changez
`world-dir` (ou supprimez le dossier du monde).

Le terrain lui-même n'est jamais enregistré : il se régénère à l'identique à partir de la
graine. Seuls les blocs modifiés par les joueurs le sont. Les chunks sont générés à
l'avance, sur plusieurs cœurs, autour des joueurs (environ 2 ms par chunk).

### Si la connexion échoue avec le jeu

Mettez `debug-packets=true` dans `server.properties`, relancez le serveur et
reconnectez-vous. Tout ce que la console affiche est aussi enregistré dans
`logs/latest.log` (l'exécution précédente dans `logs/previous.log`) : c'est ce
fichier qu'il faut transmettre. Il indique chaque étape : ping reçu, ouverture de
la connexion, version annoncée par le jeu, paquets échangés, et à quelle étape la
connexion s'est terminée.

## Versions du jeu acceptées

Basalt est **multi-version** : chaque client annonce son numéro de protocole et le
serveur encode les paquets en conséquence. Des joueurs de versions différentes
peuvent être connectés en même temps.

| Protocole | Versions de Bedrock | Particularités gérées |
|---|---|---|
| 2168 | 1.26.40 à 1.26.44 | format de base |
| 2169 | 1.26.45 | — |
| 2193 | 1.26.50, 1.26.51 | `JigsawStructureData` exigé avant `StartGame` |
| 2225 | 1.26.60 à 1.26.65 | champ ajouté dans `LevelSettings`, drapeau ajouté dans `LevelChunk` |
| 2259 | préversion 1.26.70 | octet ajouté à la fin de `NetworkSettings` |

Les préversions intermédiaires (protocoles entre ces numéros) sont aussi acceptées.
Dans `server.properties` :

- `min-protocol` et `max-protocol` bornent les versions acceptées ;
- `game-version` et `protocol` indiquent la version annoncée dans la liste des serveurs.

Les versions antérieures à 1.26.40 sont refusées avec le message « client trop
ancien » : leurs formats de paquets n'ont pas encore été relevés. Pour en ajouter
une, il faut décrire ses différences dans `src/network/protocol/versions.rs` et les appliquer dans
`src/network/protocol/bedrock/`. À chaque nouvelle version du jeu, la même opération est nécessaire.

## Transport : RakNet et NetherNet

Bedrock a changé de transport réseau. Jusqu'à la 1.26.5x, le jeu joignait un serveur
en UDP par le protocole RakNet. Depuis la 1.26.60, les clients Windows et Android
utilisent **NetherNet** pour les serveurs tiers :

1. le jeu ouvre une connexion **TCP** sur le port du serveur et demande
   `GET /v1/join` : la réponse (JSON) donne le nom du serveur, affiché dans la liste ;
2. pour entrer, il envoie son offre par `POST /v1/join/{id}` ; la réponse ouvre une
   liaison **WebRTC** (chiffrement DTLS, transport SCTP) qui transporte le jeu.

Un serveur qui ne parle que RakNet affiche « Une erreur s'est produite » à ces joueurs.

Basalt gère les deux. La signalisation est dans `src/network/signaling.rs`, la liaison WebRTC
dans `src/network/nethernet.rs`, qui s'appuie sur la bibliothèque [`str0m`](https://github.com/algesten/str0m)
(Rust pur, sans compilateur C ni OpenSSL). C'est la seule partie du serveur qui a des
dépendances ; `cargo build --release --no-default-features` compile Basalt sans elle.

| Port | Rôle |
|---|---|
| `server-port` (19132) en UDP | RakNet |
| `server-port` (19132) en TCP | signalisation NetherNet (HTTP) |
| `nethernet-udp-port` (19140) en UDP | liaison WebRTC de NetherNet |

Les trois doivent être ouverts (et redirigés sur la box pour un serveur public, avec
`public-ip` renseigné dans `server.properties`).

**Clé du serveur.** Le jeu exige que le serveur signe ses réponses. Basalt crée sa clé au
premier démarrage dans `server-identity.key` : gardez ce fichier secret et conservez-le,
sinon les joueurs reverront la demande de confiance. Son empreinte est affichée au démarrage.

**Sécurité.** Avec `xbox-auth=true`, l'offre de connexion est vérifiée avant toute
négociation (jeton Xbox et signature qui le lie à la liaison chiffrée), et le login doit
ensuite porter la même clé : le login d'un autre joueur ne peut pas être rejoué. Avec
NetherNet, le chiffrement propre à Minecraft n'est pas utilisé : DTLS chiffre la liaison.

## Protection contre les paquets malveillants

Chaque donnée reçue du réseau est traitée comme hostile. Un client qui dépasse une
limite est déconnecté sur-le-champ ; la raison est écrite dans la console
(`déconnecté par sécurité : …`) et les autres joueurs ne sont pas affectés.

| Attaque | Protection |
|---|---|
| Paquets tronqués ou incohérents | toutes les lectures sont bornées ; le client est déconnecté |
| Paquet énorme envoyé par fragments | 3 Mo au plus pendant le login, 512 Ko ensuite |
| « Bombe » de décompression | la décompression s'arrête à la même limite |
| Rafale de paquets | 1 500 datagrammes et 300 lots par seconde et par joueur, 6 Mo/s de données |
| Saturation de la mémoire | files d'attente plafonnées par session (fragments, paquets en avance) |
| Connexions en masse depuis une adresse | `max-connections-per-ip` (5 par défaut) et débit d'ouvertures limité |
| Avalanche de pings ou d'ouvertures | 30 par seconde et par adresse, 3 000 au total ; le reste est ignoré |
| Connexion ouverte qui ne fait rien | 10 s pour s'identifier, 90 s pour entrer en jeu |
| Client factice qui n'acquitte rien | session fermée après 12 s sans accusé de réception |
| Renvois provoqués pour inonder un tiers | renvois sur demande limités, débit d'envoi plafonné |
| Bug du serveur déclenché par un paquet | l'erreur est interceptée : seul ce joueur est déconnecté |

Ces protections sont vérifiées par des tests (`cargo test`) : paquets aléatoires,
bombe de décompression, inondation, limite par adresse.

Autres protections :

- **Échanges chiffrés.** Les jeux récents passent par NetherNet, chiffré par sa liaison
  DTLS. En RakNet, le serveur chiffre la session comme le serveur officiel (échange de clés
  ECDH sur P-384, AES-256 en mode compteur, somme de contrôle sur chaque lot). Seul le
  possesseur de la clé du jeton Xbox peut suivre : un login intercepté puis rejoué ne mène
  à rien, et un lot altéré ou rejoué ferme la connexion.
- **Fabrication vérifiée.** Le résultat annoncé par le jeu doit venir d'une vraie recette
  (`data/recipes.json`) qui correspond exactement à la grille, et chaque case doit perdre
  ce que la recette consomme. Les recettes de l'établi et de la grille d'inventaire sont
  couvertes.

Ce qu'elles ne couvrent pas :

- **Les attaques par saturation du réseau (DDoS).** Si la connexion de la machine
  est noyée sous le trafic, aucun logiciel serveur ne peut l'empêcher : il faut une
  protection au niveau de l'hébergeur.
- **Les plugins**, qui sont du code de confiance (voir `docs/PLUGINS.md`).

## Montée en charge (1000 joueurs et plus)

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
sessions) et l'entrée en jeu, sur la même machine, sans authentification.
**Ce qu'ils ne mesurent pas** : un vrai gameplay. Aujourd'hui les joueurs ne se
voient pas et le monde est figé ; le coût par joueur augmentera avec chaque
fonction ajoutée, et une vraie connexion Internet perd des paquets, ce que la
machine locale ne reproduit pas.

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
  fils de travail, comme les logins.
- **Mesurer chaque ajout** avec l'outil de charge avant de le garder.

Même en suivant ces règles, 1000 joueurs dans une même zone d'un seul monde reste
difficile pour n'importe quel serveur Minecraft. Les grands réseaux répartissent
leurs joueurs sur plusieurs serveurs derrière un proxy ; Basalt devra permettre
les deux approches.

## Authentification Xbox

Avec `xbox-auth=true` (valeur par défaut), seuls les joueurs connectés à leur compte
Microsoft entrent, et leur pseudo est celui que Mojang certifie.

Ce que le serveur vérifie à chaque login :

1. le jeton du joueur est signé (RS256) par une clé du service d'autorisation de
   Minecraft ; tout autre algorithme est refusé ;
2. son émetteur, son audience (`api://auth-minecraft-services/multiplayer`) et ses
   dates de validité ;
3. les données de connexion du jeu sont signées (ES384) par la clé publique
   inscrite dans le jeton ;
4. le pseudo et l'identifiant Xbox sont pris **dans le jeton**, jamais dans ce que
   le client annonce par ailleurs.

Les clés de Mojang sont téléchargées en HTTPS au démarrage, avec la commande
`curl` du système (fournie avec Windows 10 et 11, et la plupart des Linux),
rafraîchies toutes les 30 minutes et gardées dans `auth-keys.json` pour les
redémarrages hors ligne. Seules des adresses `https://…minecraft-services.net`
sont acceptées. Tant qu'aucune clé n'est disponible, les joueurs sont refusés avec
un message explicite. `auth-keys-file` permet de fournir un fichier de clés à la main.

La vérification des signatures est écrite dans `src/security/crypto.rs` (SHA-256, SHA-384,
RSA, ECDSA P-384) et comparée à OpenSSL sur plusieurs centaines de signatures
valides et falsifiées (`tests/crypto.rs`). Ce code ne fait que vérifier des données
publiques ; il ne manipule aucun secret.

Vérifié avec de vrais comptes Xbox (jeu 1.26.63 à 1.26.66). Seul le jeton introduit en
2025 est accepté, pas l'ancienne chaîne de certificats.

Si l'authentification bloque vos essais, mettez `xbox-auth=false` : le serveur
l'indique alors clairement au démarrage, et les pseudos ne sont plus vérifiés.

## Installation

### Linux en une commande (depuis GitHub)

Rien à compiler : la dernière version publiée est téléchargée dans le dossier courant.

```sh
mkdir basalt && cd basalt
curl -sSfL https://raw.githubusercontent.com/ZoumiSurGithub/basalt/main/get.sh | sh
./start.sh
```

Pour mettre à jour : `./update.sh`, puis redémarrez le serveur. La configuration, le monde,
les listes et vos plugins ne sont jamais touchés ; l'archive est vérifiée par son empreinte
SHA-256 avant toute modification. Lua 5.4, nécessaire aux plugins, est livré dans l'archive :
rien à installer sur la machine.

**Publier une version** : créez une étiquette et envoyez-la.

```sh
git tag v0.1.0 && git push origin v0.1.0
```

GitHub compile alors Basalt pour Linux et Windows et publie les archives ; `get.sh` et
`update.sh` installent toujours la dernière.

### Debian / Ubuntu (compilation depuis les sources)

```sh
./install.sh                  # compile, résultat dans ./dist
cd dist && ./basalt
```

Pour un serveur permanent (utilisateur dédié + service systemd dans `/opt/basalt`) :

```sh
sudo ./install.sh --service
journalctl -u basalt -f
```

Le script installe au besoin `cargo`, `gcc`, `curl` et `liblua5.4-0` avec apt.
Ouvrez le port **19132/udp** dans votre pare-feu.

### Windows

Dans le dossier `windows`, double-cliquez sur **`Installer.cmd`**. L'installateur :

1. installe Rust s'il est absent (avec votre accord), puis compile le serveur ;
2. installe Basalt dans `%LOCALAPPDATA%\Basalt` ;
3. télécharge `lua54.dll` pour les plugins ;
4. ouvre le pare-feu pour `basalt.exe` (Windows demande votre accord) ;
5. crée un raccourci « Basalt » sur le Bureau et dans le menu Démarrer.

Pour vous connecter depuis le même PC : dans Minecraft, *Jouer > Serveurs >
Ajouter un serveur*, adresse `127.0.0.1`, port `19132`.

**Mettre à jour** : remplacez le dossier du code source par la nouvelle version, puis
double-cliquez sur `Mettre-a-jour.cmd`. Le serveur est recompilé et remplacé ;
`server.properties` et vos plugins sont conservés, et l'ancienne version reste
disponible sous le nom `basalt.exe.bak`.

**Mises à jour en ligne** : si le projet est publié sur GitHub (le fichier
`.github/workflows/build.yml` fabrique une version à chaque étiquette `v*`),
l'outil peut télécharger directement la dernière version, sans compiler :

```
Installer.cmd -Repo proprietaire/depot
Mettre-a-jour.cmd
```

**Désinstaller** : `Desinstaller.cmd` (ajoutez `-Purge` pour supprimer aussi vos
réglages et plugins).

Ces scripts n'ont pas encore été essayés sur une vraie machine Windows.

### Autres systèmes

`cargo build --release`, puis installez Lua 5.4 (`liblua5.4.so` ou `liblua.dylib`).

## Console

Les commandes du jeu se tapent aussi dans la console, sans `/` :

| Commande | Effet |
|---|---|
| `help` | liste des commandes, y compris celles des plugins |
| `list` | joueurs connectés |
| `say`, `me`, `tell` | messages |
| `gamemode`, `give`, `clear`, `kill`, `tp` | joueurs |
| `setblock`, `fill`, `time` | monde |
| `kick`, `op`, `deop`, `whitelist` | modération |
| `status` | charge du serveur : ticks par seconde, trafic, logins en attente |
| `timings on` / `timings off` | rapport détaillé du temps passé par le serveur (dossier `timings/`) |
| `plugins`, `reload` | plugins chargés, rechargement |
| `motd [texte]` | affiche ou change le nom du serveur |
| `version` | version du serveur et versions du jeu acceptées |
| `stop` | arrêt propre |

En jeu, les mêmes commandes sont réservées aux opérateurs (`ops.txt`), sauf `help`,
`list`, `me` et `tell`.

## Écrire un plugin

Un plugin est un fichier `plugins/nom.lua`, ou un dossier `plugins/nom/main.lua`.

```lua
server.on("login", function(joueur)
  if joueur.name == "Griefer123" then
    return "Vous êtes banni."      -- un texte (ou false) refuse la connexion
  end
end)

server.command("bonjour", "dit bonjour", function(args)
  server.log("Bonjour " .. (args[1] or "tout le monde"))
end)
```

La documentation complète (événements, commandes, minuteries, fichiers, exemples)
est dans **[docs/PLUGINS.md](docs/PLUGINS.md)**.

## Organisation du code

| Dossier ou fichier | Contenu |
|---|---|
| `src/main.rs` | démarrage : configuration, signaux, console |
| `src/server.rs` | `Server` : boucle principale, joueurs connectés, console |
| `src/player/` | `Player` : un joueur, sa sauvegarde et son chargement (hérite de `Human`) |
| `src/entity/` | `Entity` → `Living` → `Human` : la hiérarchie des entités ; physique des objets au sol (`item_physics.rs`) |
| `src/inventory/` | `Inventory` (base commune), `BaseInventory`, puis `PlayerInventory`, `ArmorInventory`, `ChestInventory`, `FurnaceInventory`, `CraftingGrid` ; `PlayerWindows` (tous les inventaires d'un joueur) ; `transaction.rs` (demandes du jeu) |
| `src/block/` | `Block` (base de tous les blocs), types de blocs (`types.rs`), registre `VanillaBlocks` couvrant tous les blocs du jeu |
| `src/tile/` | `Tile` (base des données attachées aux blocs), `ContainerTile`, `FurnaceTile`, `SignTile`, `TileMap` et sa sauvegarde |
| `src/item/` | `Item` (base de tous les objets), `Tool`, `Armor`, `Food`, `VanillaItems` ; `ItemStack` (une pile) |
| `src/world/` | `World` : le monde, ses blocs modifiés et leur sauvegarde ; `vanilla.rs` : terrain à la manière du jeu ; `generator.rs` : terrain plat et premier terrain |
| `src/permission/` | `Permission`, règles par défaut, liste des opérateurs |
| `src/utils/` | journal (`log.rs`), `server.properties` (`config.rs`), JSON et base64 (`json.rs`), YAML (`yaml.rs`), décompression (`inflate.rs`) |
| `src/security/` | authentification Xbox (`auth.rs`), SHA-2 et signatures (`crypto.rs`), clé du serveur (`identity.rs`) |
| `src/network/` | RakNet (`raknet.rs`), signalisation HTTP (`signaling.rs`), liaison WebRTC (`nethernet.rs`), fragments (`framing.rs`) |
| `src/network/protocol/versions.rs` | versions acceptées et différences entre protocoles |
| `src/network/protocol/bedrock/` | paquets Bedrock, un fichier par thème : `codec`, `login`, `spawn`, `chunk`, `player`, `state`, `chat`, `blocks`, `items` |
| `src/data/` | chargement des données du jeu : objets, blocs et états, recettes (`recipes.rs`), NBT, gzip |
| `data/` | fichiers de données du jeu (projet BedrockData, licence CC0) ; `data/<protocole>/packets/` : paquets relevés auprès du serveur officiel par l'outil de capture |
| `src/plugin/` | fiche `plugin.yml` (`manifest.rs`), moteur Lua et API (`lua.rs`), préambule et bac à sable (`sandbox.lua`), fil d'exécution dédié (`host.rs`) |
| `examples/stress.rs` | outil de charge : simule des centaines ou milliers de joueurs |
| `tests/e2e.rs` | faux client : connexion, login, entrée en jeu, combat, objets, fabrication, plugins |
| `plugins/EasyEdit/` | plugin fourni : édition du monde en volume, exemple de plugin en plusieurs fichiers |

Tests : `cargo test`

## Suite proposée

1. **Le monde** : chargement des mondes Minecraft (format LevelDB), blocs qui réagissent
   (eau, lave, gravité), mondes multiples, villages et autres structures.
2. **Le jeu** : monstres et animaux, faim, expérience, temps de minage selon l'outil,
   jour et nuit.
3. **Montée en charge** : mesure avec 1000 joueurs simulés sur une vraie machine.

## Licence

MIT
