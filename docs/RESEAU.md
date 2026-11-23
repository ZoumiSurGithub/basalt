# Réseau et sécurité

Versions du jeu acceptées, transports, protections et authentification Xbox.

## Versions du jeu acceptées

Basalt est **multi-version** : chaque client annonce son numéro de protocole et le
serveur encode les paquets en conséquence. Des joueurs de versions différentes
peuvent être connectés en même temps.

| Protocole | Versions de Bedrock | Particularités gérées |
|---|---|---|
| 2225 | 1.26.60 à 1.26.66 | formats de base (relevés en 1.26.65) |
| 2259 | préversion 1.26.70 | octet ajouté à la fin de `NetworkSettings` |

Les préversions intermédiaires (protocoles entre ces numéros) sont aussi acceptées.
Dans `server.properties` :

- `min-protocol` et `max-protocol` bornent les versions acceptées ;
- `game-version` et `protocol` indiquent la version annoncée dans la liste des serveurs.

Les versions antérieures à 1.26.60 sont refusées : le jeu affiche qu'il doit être mis à
jour. La 1.26.60 a changé l'écriture des objets, des données d'entité et de plusieurs
paquets, et seuls les formats 1.26.60 et plus ont été relevés. Un `min-protocol` plus
bas dans `server.properties` est relevé à 2225 au démarrage. Pour en ajouter
une, il faut décrire ses différences dans `src/network/protocol/versions.rs` et les appliquer dans
`src/network/protocol/bedrock/`. À chaque nouvelle version du jeu, la même opération est nécessaire.

## Transport : NetherNet

Bedrock a changé de transport réseau. Jusqu'à la 1.26.5x, le jeu joignait un serveur en UDP
par le protocole RakNet. Depuis la 1.26.60, il utilise **NetherNet** pour les serveurs tiers :

1. le jeu ouvre une connexion **TCP** sur le port du serveur et demande
   `GET /v1/join` : la réponse (JSON) donne le nom du serveur, affiché dans la liste ;
2. pour entrer, il envoie son offre par `POST /v1/join/{id}` ; la réponse ouvre une
   liaison **WebRTC** (chiffrement DTLS, transport SCTP) qui transporte le jeu.

Basalt ne parle que NetherNet : les versions du jeu qui utilisaient RakNet sont antérieures à
la 1.26.60 et ne sont plus acceptées. La signalisation est dans `src/network/signaling.rs`, la
liaison WebRTC dans `src/network/nethernet.rs`, qui s'appuie sur la bibliothèque
[`str0m`](https://github.com/algesten/str0m) (Rust pur, sans compilateur C ni OpenSSL). C'est
la seule partie du serveur qui a des dépendances.

| Port | Rôle |
|---|---|
| `server-port` (19132) en TCP | signalisation NetherNet (HTTP) |
| `nethernet-udp-port` (19140) en UDP | liaison WebRTC de NetherNet |

Les deux doivent être ouverts (et redirigés sur la box pour un serveur public, avec
`public-ip` renseigné dans `server.properties`). Le port 19132 en UDP ne sert plus.

**Transport de test.** Les tests de bout en bout et quelques outils locaux utilisent une
liaison TCP toute simple (`src/network/transport.rs`), limitée à la machine du serveur. Elle
ne s'ouvre que par programme : aucun réglage de `server.properties` ne l'active, et un
serveur lancé normalement ne l'écoute pas. `cargo build --release --no-default-features`
compile Basalt sans NetherNet : il ne reste alors que ce transport, pour les tests.

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
| Rafale de paquets | 300 lots par seconde et par joueur, 6 Mo/s de données |
| Connexions en masse depuis une adresse | `max-connections-per-ip` (5 par défaut) |
| Connexion ouverte qui ne fait rien | 10 s pour s'identifier, 90 s pour entrer en jeu |
| Bug du serveur déclenché par un paquet | l'erreur est interceptée : seul ce joueur est déconnecté |

Ces protections sont vérifiées par des tests (`cargo test`) : messages aléatoires,
bombe de décompression, inondation, limite par adresse.

Autres protections :

- **Échanges chiffrés.** La liaison NetherNet est chiffrée par DTLS. Avec `xbox-auth=true`,
  l'offre de connexion porte le jeton Xbox du joueur et une signature qui le lie à cette
  liaison : un login intercepté puis rejoué ne mène à rien.
- **Fabrication vérifiée.** Le résultat annoncé par le jeu doit venir d'une vraie recette
  (`data/recipes.json`) qui correspond exactement à la grille, et chaque case doit perdre
  ce que la recette consomme. Les recettes de l'établi et de la grille d'inventaire sont
  couvertes.

Ce qu'elles ne couvrent pas :

- **Les attaques par saturation du réseau (DDoS).** Si la connexion de la machine
  est noyée sous le trafic, aucun logiciel serveur ne peut l'empêcher : il faut une
  protection au niveau de l'hébergeur.
- **Les plugins**, qui sont du code de confiance (voir `docs/PLUGINS.md`).

## Authentification Xbox

Avec `xbox-auth=true` (valeur par défaut), seuls les joueurs connectés à leur compte
Microsoft entrent, et leur pseudo est celui que Mojang certifie.

Exception pour les tests : avec `local-bots=true`, des joueurs simulés peuvent entrer sans
compte, mais seulement depuis la machine du serveur (adresse 127.x.x.x) et avec un pseudo
qui commence par « Bot ». Laissez `local-bots=false` en temps normal.

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

## Dépannage

Mettez `debug-packets=true` dans `server.properties`, relancez le serveur et
reconnectez-vous. Tout ce que la console affiche est aussi enregistré dans
`logs/latest.log` (l'exécution précédente dans `logs/previous.log`) : c'est ce
fichier qu'il faut transmettre. Il indique chaque étape : ping reçu, ouverture de
la connexion, version annoncée par le jeu, paquets échangés, et à quelle étape la
connexion s'est terminée.

**Réglages de secours.** Si un joueur est replacé à tort, ou se déconnecte quand un autre
joueur apparaît, `movement-check=false` et `show-players=false` désactivent ces fonctions
le temps de corriger.

## Beaucoup de joueurs : `nethernet-threads`

Par défaut, un seul fil lit, déchiffre et envoie tout le trafic NetherNet. Il suffit jusqu'à
quelques centaines de joueurs ; vers un millier, il sature. `nethernet-threads=4` dans
`server.properties` répartit les joueurs sur quatre fils (16 au plus). Comptez un fil pour
400 joueurs environ, sans dépasser le nombre de cœurs de la machine.

Le port UDP reste le même : rien à changer dans le pare-feu. Chaque joueur reste sur le même
fil du début à la fin de sa session.
