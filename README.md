# Basalt

Socle de serveur **Minecraft Bedrock** écrit en Rust, dans l'esprit de PocketMine-MP :
un seul exécutable, un fichier `server.properties`, une console, et des plugins en Lua.

Aucune dépendance Rust externe : le projet se compile avec `cargo build --release`
et produit un binaire d'environ 600 Ko.

## État du projet

Basalt est un **serveur en construction**. Il sait désormais faire entrer un joueur
dans un monde plat, mais cette partie n'a pas encore été essayée avec le vrai jeu.

| Fonction | État |
|---|---|
| Serveur visible dans la liste des serveurs (ping RakNet) | fait |
| Connexion RakNet : poignée de main, fiabilité, ordre, fragmentation, réémission | fait |
| NetherNet, étape 1 : signalisation HTTP (le serveur apparaît dans la liste des jeux récents) | fait |
| NetherNet, étape 2 : liaison WebRTC par `str0m` (indispensable pour entrer avec un jeu 1.26.60 ou plus) | fait, **vérifié avec le jeu 1.26.65 sous Windows** (entrée dans le monde, déplacements) |
| Début de session Bedrock : réglages réseau, lecture du login (pseudo, XUID) | fait |
| Refus propre des versions du jeu différentes | fait |
| Console, `server.properties`, arrêt propre | fait |
| Plugins Lua : événements, commandes, minuteries, expulsion | fait |
| Entrée en jeu : packs de ressources, `StartGame`, monde plat, apparition | **écrit, à valider avec le jeu** |
| Terrain qui suit le joueur quand il se déplace | **écrit, à valider avec le jeu** |
| Contrôle des déplacements (vitesse, téléportation) | **écrit, à valider avec le jeu** |
| Joueurs visibles entre eux (les plus proches seulement) | **écrit, à valider avec le jeu** |
| Chat, casser et poser des blocs, inventaire | à faire |
| Authentification Xbox Live (vérification des jetons) | **écrit, à valider avec le jeu** |
| Chiffrement des échanges | à faire |
| Sauvegarde du monde | à faire |

Limites à connaître :

- **Non essayé avec un vrai client Minecraft.** Les tests automatiques utilisent un
  faux client : ils prouvent que le serveur envoie la bonne suite de paquets, pas que
  le jeu les accepte. Les paquets suivent la documentation officielle de Mojang et son
  journal des changements, mais certains détails d'encodage n'y figurent pas et ont été
  repris des versions précédentes du protocole. Les listes d'objets et de biomes sont
  envoyées vides.
- **Authentification non encore éprouvée en conditions réelles.** Voir la section
  « Authentification Xbox » ci-dessous.
- **Monde non modifiable.** Le terrain plat est infini et suit le joueur, mais ses
  actions (casser, poser, frapper) ne sont pas traitées.
- **Réglages de secours.** Si un vrai client est replacé à tort ou se déconnecte
  quand un autre joueur apparaît, `movement-check=false` et `show-players=false`
  désactivent ces deux fonctions le temps de corriger.
- **Pas de liste des joueurs ni de skins.** Les autres joueurs apparaissent sans
  leur skin personnalisé, et le menu pause ne les liste pas.

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

Ce qu'elles ne couvrent pas :

- **Les attaques par saturation du réseau (DDoS).** Si la connexion de la machine
  est noyée sous le trafic, aucun logiciel serveur ne peut l'empêcher : il faut une
  protection au niveau de l'hébergeur.
- **L'écoute du trafic** : les échanges ne sont pas encore chiffrés, ce qui laisse
  possible le rejeu d'un login intercepté (voir « Authentification Xbox »).
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

Limites actuelles :

- **Jamais essayé avec un vrai jeton.** Le téléchargement des clés et la lecture
  d'un jeton réel n'ont pas pu être essayés ; les tests utilisent des jetons
  fabriqués sur le même modèle. Les noms des champs du jeton (`xid`, `xname`,
  `cpk`) peuvent demander un ajustement.
- **Rejeu possible.** Sans chiffrement, quelqu'un qui intercepte le login d'un
  joueur peut le rejouer tant que le jeton n'a pas expiré. Le chiffrement de la
  session (échange de clés ECDH), qui prouve que le client possède sa clé privée,
  fermera cette faille ; il reste à écrire.
- **Ancien format non pris en charge.** Seul le jeton introduit en 2025 est
  accepté, pas l'ancienne chaîne de certificats.

Si l'authentification bloque vos essais, mettez `xbox-auth=false` : le serveur
l'indique alors clairement au démarrage, et les pseudos ne sont plus vérifiés.

## Installation

### Linux en une commande (depuis GitHub)

Rien à compiler : la dernière version publiée est téléchargée dans le dossier courant.

```sh
mkdir basalt && cd basalt
curl -sSfL https://raw.githubusercontent.com/@REPO@/main/get.sh | sh
./start.sh
```

Pour mettre à jour : `./update.sh`, puis redémarrez le serveur. La configuration, le monde,
les listes et vos plugins ne sont jamais touchés ; l'archive est vérifiée par son empreinte
SHA-256 avant toute modification. Les plugins demandent `sudo apt install liblua5.4-0`.

**Publier une version** (une fois le projet sur GitHub) : remplacez `@REPO@` par
`proprietaire/depot` dans `get.sh` et ce fichier, puis créez une étiquette :

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

| Commande | Effet |
|---|---|
| `help` | liste des commandes, y compris celles des plugins |
| `list` | joueurs connectés |
| `status` | charge du serveur : ticks par seconde, trafic, logins en attente |
| `plugins` | plugins chargés |
| `reload` | recharge tous les plugins |
| `motd [texte]` | affiche ou change le nom du serveur |
| `version` | version du serveur et versions du jeu acceptées |
| `stop` | arrêt propre |

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
| `src/entity/` | `Entity` → `Living` → `Human` : la hiérarchie des entités |
| `src/inventory/` | `Inventory` (base commune), `BaseInventory`, puis `PlayerInventory`, `ArmorInventory`, `ChestInventory`, `FurnaceInventory`, `CraftingGrid` ; `PlayerWindows` (tous les inventaires d'un joueur) ; `transaction.rs` (demandes du jeu) |
| `src/block/` | `Block` (base de tous les blocs), types de blocs (`types.rs`), registre `VanillaBlocks` couvrant tous les blocs du jeu |
| `src/tile/` | `Tile` (base des données attachées aux blocs), `ContainerTile`, `FurnaceTile`, `SignTile`, `TileMap` et sa sauvegarde |
| `src/item/` | `Item` (base de tous les objets), `Tool`, `Armor`, `Food`, `VanillaItems` ; `ItemStack` (une pile) |
| `src/level/` | `Level` : le monde, ses blocs modifiés et leur sauvegarde |
| `src/permission/` | `Permission`, règles par défaut, liste des opérateurs |
| `src/utils/` | journal (`log.rs`), `server.properties` (`config.rs`), JSON et base64 (`json.rs`), YAML (`yaml.rs`), décompression (`inflate.rs`) |
| `src/security/` | authentification Xbox (`auth.rs`), SHA-2 et signatures (`crypto.rs`), clé du serveur (`identity.rs`) |
| `src/network/` | RakNet (`raknet.rs`), signalisation HTTP (`signaling.rs`), liaison WebRTC (`nethernet.rs`), fragments (`framing.rs`) |
| `src/network/protocol/versions.rs` | versions acceptées et différences entre protocoles |
| `src/network/protocol/bedrock/` | paquets Bedrock, un fichier par thème : `codec`, `login`, `spawn`, `chunk`, `player`, `state`, `chat`, `blocks`, `items` |
| `src/data/` | chargement des données du jeu : objets, blocs et états, NBT, gzip |
| `data/` | fichiers de données du jeu 1.26.60 (projet BedrockData, licence CC0) |
| `src/plugin/` | fiche `plugin.yml` (`manifest.rs`), moteur Lua et API (`lua.rs`), préambule et bac à sable (`sandbox.lua`), fil d'exécution dédié (`host.rs`) |
| `examples/stress.rs` | outil de charge : simule des centaines ou milliers de joueurs |
| `tests/e2e.rs` | faux client : ping, connexion, login, entrée en jeu, plugin |

Tests : `cargo test`

## Suite proposée

1. Valider l'entrée en jeu avec un vrai client et corriger les paquets si besoin.
2. Chat et commandes en jeu (le format du paquet de chat a changé récemment et
   demande un essai avec le jeu).
3. Liste des joueurs et skins.
4. Casser et poser des blocs, inventaire, registre d'objets complet.
5. Chiffrement de la session (ECDH P-384, AES) pour compléter l'authentification.
6. Sauvegarde des mondes (format LevelDB de Bedrock).

Les étapes 5 et 6 demanderont probablement des bibliothèques externes
(cryptographie, LevelDB) plutôt que du code écrit à la main.

## Licence

MIT
