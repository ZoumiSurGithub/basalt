# API de gestion à distance

Basalt peut répondre à une application de gestion (ou à vos propres scripts) par une petite
API HTTP qui renvoie du JSON. Elle **lit** l'état du serveur, et permet quelques **actions**
d'administration à qui présente le jeton qui en donne le droit.

## Le tableau de bord

L'API vient avec une page web : ouvrez `http://127.0.0.1:8765/` dans un navigateur (avec
le port choisi), collez un jeton, et vous avez, par la barre de navigation :

- **Aperçu** : les ticks par seconde, la durée de marche, les joueurs, la mémoire, les plugins ;
- **Joueurs** : la liste des connectés avec recherche, et pour chacun une fiche d'où le
  téléporter (à des coordonnées ou auprès d'un autre joueur), changer son mode de jeu, lui
  donner un objet ou un effet, lui écrire, le soigner, vider son inventaire, le tuer,
  l'expulser ;
- **Objets et blocs** : la liste complète avec recherche, et un bouton pour désactiver ou
  réactiver chacun ;
- **Analyse** : le compte de tous les objets de la carte et ce qui ressemble à une
  duplication (voir « Analyser la carte ») ;
- **Packs** : ceux envoyés aux joueurs, les fichiers du dossier, et un bouton pour chiffrer ;
- **Console** : lancer une commande, recharger les plugins.

Avec le jeton de lecture, la page montre tout mais ne propose aucune action. Le jeton reste
dans l'onglet du navigateur et s'efface quand on le ferme ; « Fermer la session » l'efface
tout de suite.

Thème sombre ou clair, au choix (le bouton en haut à droite ; le choix est gardé). Sur
téléphone, la navigation passe en bas de l'écran.

Pensée pour un serveur chargé : la liste des joueurs vient par pages de cinquante, la
recherche est faite par le serveur, et rien n'est rafraîchi quand l'onglet du navigateur
est caché.

## Les images des objets et des blocs

Basalt ne livre aucune image du jeu : elles appartiennent à Mojang. Le tableau de bord les
lit là où elles se trouvent chez vous.

- **Vos objets et blocs** (ceux des plugins) : leurs images viennent de vos packs de
  ressources, ceux de `resource_packs.yml`. Rien à faire. Les packs chiffrés sont lus avec
  leur clé.
- **Les objets et blocs du jeu** : déposez le pack de ressources du jeu dans un dossier
  `vanilla_resource_pack`, à côté de `server.properties`. Mojang publie ce pack pour les
  créateurs sur <https://github.com/Mojang/bedrock-samples/releases> : téléchargez
  l'archive de votre version, et copiez-y le contenu de son dossier `resource_pack`.

```
vanilla_resource_pack/
  blocks.json
  textures/
    item_texture.json
    terrain_texture.json
    items/
    blocks/
```

Rechargez ensuite les plugins (`reload`, ou le bouton de l'onglet Console). Ce dossier est
ignoré par git : ne le publiez pas avec votre serveur.

À savoir :

- Un bloc est montré par la texture de son côté, à plat (pas en volume).
- Quelques objets du jeu n'auront pas d'image : leur icône ne porte pas leur nom, et le
  lien entre les deux n'est pas dans le pack. Les textures au format `.tga` ne sont pas lues.
- Les images sont demandées une à une, seulement pour les lignes à l'écran, et gardées une
  heure par le navigateur.

## L'allumer

Dans `server.properties` :

```
api-port=8765
api-bind=127.0.0.1
```

Au démarrage suivant, la console affiche l'adresse écoutée, et un jeton d'accès est créé
dans `api-token.txt`, à côté de `server.properties`. `api-port=0` (la valeur par défaut)
laisse l'API éteinte.

## Les jetons

Deux jetons sont créés au premier démarrage, à côté de `server.properties` :

| Fichier | Droit |
|---|---|
| `api-token.txt` | **lecture** : toutes les adresses `GET` |
| `api-admin-token.txt` | lecture et **actions** : les adresses `POST` |

Chaque requête présente le sien :

```
Authorization: Bearer <contenu du fichier>
```

- Sans jeton, ou avec un mauvais, la réponse est `401`. Une action demandée avec le jeton
  de lecture reçoit `403`.
- Un jeton vaut un mot de passe : ne le publiez pas (ces fichiers sont ignorés par git), ne
  l'envoyez pas dans une capture d'écran. Donnez celui de lecture à ce qui ne fait
  qu'afficher, et gardez celui d'administration pour vous.
- Pour en changer, supprimez le fichier et redémarrez : un nouveau est créé.

## Accès depuis une autre machine

Basalt parle en HTTP simple : **il ne chiffre pas lui-même**. Par défaut l'API n'écoute
donc que sur la machine du serveur (`127.0.0.1`), et c'est un outil fait pour ça qui
chiffre le trajet jusqu'à vous. Laissez `api-bind=127.0.0.1` dans tous les cas ci-dessous :
rien n'est à ouvrir dans le pare-feu ni dans la box, et le profil réseau de Windows ne
change pas, car ces outils se connectent vers l'extérieur.

| Vous voulez… | Prenez |
|---|---|
| y accéder depuis votre téléphone ou un autre de vos appareils | Tailscale |
| une adresse https à ouvrir de n'importe où, sans rien installer côté téléphone | un tunnel Cloudflare |
| passer par un serveur Linux que vous administrez déjà | un tunnel SSH |
| une adresse à vous (`panel.exemple.fr`) | un serveur HTTPS devant l'API |

### Tailscale (privé : vos appareils seulement)

1. Installez Tailscale sur le PC du serveur et sur le téléphone, avec le même compte.
2. Activez HTTPS pour votre réseau dans la console d'administration de Tailscale.
3. Sur le PC du serveur, dans PowerShell :

   ```
   tailscale serve --bg 8765
   ```

   La commande affiche une adresse en `https://…ts.net`. Elle ne répond qu'à vos appareils
   connectés à Tailscale : personne d'autre ne voit même la page de connexion.

### Tunnel Cloudflare (adresse publique en https)

```
winget install Cloudflare.cloudflared
cloudflared tunnel --url http://localhost:8765
```

La commande affiche une adresse en `https://…trycloudflare.com`, sans compte. Elle change à
chaque lancement et s'arrête quand vous fermez la fenêtre. Cette adresse est **publique** :
quiconque la connaît arrive sur la page de connexion, et seul le jeton le sépare du
serveur. Ne la publiez pas, et fermez le tunnel quand vous n'en avez plus besoin.

### Tunnel SSH

`ssh -L 8765:127.0.0.1:8765 vous@votre-serveur`, puis ouvrez `http://127.0.0.1:8765`.

### Serveur HTTPS devant l'API

Caddy ou nginx, installé sur la machine du serveur, chiffre et transmet à
`127.0.0.1:8765`. Il doit poser l'en-tête `X-Forwarded-For` (Caddy le fait seul), sans
quoi Basalt ne sait pas distinguer les visiteurs.

### Ce que Basalt fait pour se protéger

- **Mauvais jetons en série** : huit jetons refusés en dix minutes depuis une même adresse
  l'écartent dix minutes (réponse `429`), même si elle finit par présenter le bon. C'est
  écrit dans le journal du serveur. Un bon jeton efface le compte.
- **Qui est au bout** : derrière un tunnel, toutes les connexions arrivent de la machine
  elle-même. Basalt lit alors l'adresse du visiteur dans `CF-Connecting-IP` ou
  `X-Forwarded-For`. Ces en-têtes ne sont crus que venant de la machine elle-même.
- **Jetons compromis** : `apitoken reset` dans la console en crée deux nouveaux sans
  redémarrer ; les anciens cessent aussitôt de fonctionner.
- **Page non chiffrée** : le tableau de bord affiche un avertissement s'il est ouvert en
  `http://` ailleurs que sur la machine du serveur.

### À éviter

`api-bind=0.0.0.0` ouvre l'API à tout le réseau sans chiffrement : le jeton circulerait en
clair. Basalt l'accepte mais l'écrit en avertissement au démarrage ; ne le faites que sur
un réseau privé.

## Lire (`GET`)

| Adresse | Ce qu'elle renvoie |
|---|---|
| `/v1` | le nom, la version et la liste des adresses |
| `/v1/status` | état du serveur : version, durée de marche, ticks par seconde, joueurs, mémoire, heure du monde, plugins |
| `/v1/players` | une page des joueurs en jeu, triés par nom : `{online, total, offset, players}`. Paramètres : `q` (cherche dans les noms), `limit` (50 par défaut, 200 au plus), `offset`. Chaque joueur : nom, mode, position des pieds, vie, faim, opérateur, effets |
| `/v1/items` | tous les objets : nom, numéro, pile, durabilité, dégâts, armure, venu d'un plugin (`custom`), désactivé (`disabled`) |
| `/v1/blocks` | tous les blocs : nom, nombre d'états, dureté, résistance aux explosions, lumière, `custom`, `disabled` |
| `/v1/disabled` | les objets et blocs désactivés |
| `/v1/map?cx=&cz=` | une tuile de la carte vue du dessus (voir « La carte ») : des octets, pas du JSON |
| `/v1/world/chunks?list=` | les chunks de la vue en relief (voir « La carte ») : des octets |
| `/v1/entities?x=&z=&r=` | joueurs et créatures autour d'un point (`r` : 96 blocs par défaut, 160 au plus), les plus proches d'abord, 400 au plus : `{total, entities}`, chacun avec `kind`, `name`, `player`, la position des pieds et sa boîte (`w`, `h`) |
| `/v1/world/block?x=&z=` | le bloc le plus haut d'une colonne : `{x, y, z, block}`. Avec `y`, le bloc de cette position. `409` si la zone n'est pas chargée |
| `/v1/scan` | l'analyse de la carte : `{"state": "idle"}`, `"running"`, ou `"done"` avec son rapport |
| `/v1/texture?kind=item&name=diamond` | l'image d'un objet (`kind=item`) ou d'un bloc (`kind=block`), en PNG ; `404` si aucun pack ne la fournit |
| `/v1/textures` | `{vanilla}` : le pack du jeu a-t-il été déposé dans `vanilla_resource_pack` |
| `/v1/packs` | les packs envoyés aux joueurs (nom, version, taille, chiffré ou non) et les fichiers du dossier `resource_packs/` |

```
curl -H "Authorization: Bearer $(cat api-token.txt)" http://127.0.0.1:8765/v1/status
```

```json
{"name":"Basalt","version":"0.0.5","uptime_seconds":412,"ticks_per_second":20,
 "worst_tick_delay_ms":3,"players_online":2,"players_max":20,"memory_mb":184,
 "world_time":6130,"world_edits":1520,"creatures":37,"plugins":["EasyEdit","Archipel"]}
```

## Agir (`POST`, jeton d'administration)

Le corps est un objet JSON. Chaque action est écrite au journal du serveur (« API : … »).

| Adresse | Corps | Effet |
|---|---|---|
| `/v1/players/action` | `{"name": "Alex", "action": …}` | agit sur un joueur en jeu (voir plus bas) |
| `/v1/scan` | — | lance une analyse de la carte (une seule à la fois) |
| `/v1/disable` | `{"name": "tnt"}` | désactive un objet ou un bloc, tout de suite |
| `/v1/enable` | `{"name": "tnt"}` | le réactive |
| `/v1/packs/encrypt` | `{"file": "monpack.mcpack"}` | chiffre un pack de `resource_packs/` (voir [PACKS.md](PACKS.md), « Chiffrer un pack ») ; le travail se fait en arrière-plan, le résultat s'écrit au journal et apparaît dans `/v1/packs` |
| `/v1/plugins/reload` | — | recharge tous les plugins (comme `reload` en console) |
| `/v1/command` | `{"command": "say Bonjour"}` | lance une commande de console (256 caractères au plus) |

```
curl -X POST -H "Authorization: Bearer $(cat api-admin-token.txt)" \
     -d '{"name":"tnt"}' http://127.0.0.1:8765/v1/disable
```

### Agir sur un joueur

`POST /v1/players/action`, avec le nom du joueur, l'action, et ses réglages :

| `action` | Réglages | Effet |
|---|---|---|
| `teleport` | `x`, `y`, `z` (pieds ; `y` de -64 à 320) | le téléporte à ces coordonnées |
| `teleport_to` | `target` : un autre joueur en jeu | le téléporte auprès de lui |
| `gamemode` | `mode` : `survival`, `creative`, `adventure`, `spectator` | change son mode de jeu |
| `give` | `item`, `count` (1 par défaut) | lui donne un objet |
| `effect` | `effect`, `seconds` (30), `level` (1) | lui donne un effet |
| `clear_effects` | — | retire ses effets |
| `heal` | — | lui rend toute sa vie et sa faim, et éteint le feu |
| `message` | `text` | lui écrit, à lui seul |
| `clear_inventory` | — | vide son inventaire |
| `kill` | — | le tue |
| `kick` | `reason` (facultative) | l'expulse |

```
curl -X POST -H "Authorization: Bearer $(cat api-admin-token.txt)" \
     -d '{"name":"Alex","action":"teleport","x":120,"y":64,"z":-33}' \
     http://127.0.0.1:8765/v1/players/action
```

Le joueur est désigné par son nom exact, tel quel (les espaces ne posent pas de problème,
contrairement à une commande). S'il n'est pas en jeu, la réponse est `404`.

### La carte

L'onglet Carte du tableau de bord montre le monde vu du dessus, les joueurs en direct, et
laisse cliquer un bloc pour le remplacer, le retirer ou y téléporter un joueur.

`GET /v1/map?cx=&cz=` renvoie la tuile de 4 × 4 chunks qui contient le chunk demandé :
64 × 64 colonnes, rangée par rangée du nord au sud, quatre octets chacune (rouge, vert,
bleu, puis `1` si la colonne est connue, `0` sinon). La couleur vient du nom du bloc le plus
haut ; les pentes sont ombrées. La tuile est dessinée hors du fil du serveur.

Seul le terrain **déjà en mémoire** est montré et modifiable, c'est-à-dire autour des
joueurs : la carte ne fait rien générer. Ailleurs, les colonnes sont inconnues et les
demandes sur un bloc répondent `409`.

**Les couleurs** sont celles des textures : la moyenne de la texture du dessus de chaque
bloc, lue dans vos packs de ressources puis dans le pack du jeu si vous l'avez déposé dans
`vanilla_resource_pack/` (voir « Les images des objets et des blocs »). L'herbe, les
feuilles et l'eau, grises dans les fichiers du jeu, sont teintes comme en plaine. Sans le
pack du jeu, Basalt n'a pas ces textures (il ne les distribue pas) : la carte prend alors
une couleur approchée d'après le nom du bloc. Les couleurs sont calculées une fois, au
premier affichage de la carte, sur un fil à part.

**La vue en relief** (bouton Relief) dessine les mêmes zones en cubes : on tourne autour
du point regardé, on vise un bloc précis, même sous un surplomb. Chaque joueur et chaque
créature y apparaît dans une boîte à sa vraie taille, avec son nom au-dessus (les joueurs
toujours, les créatures quand elles sont proches). Les grottes sous la surface ne sont pas
envoyées. Avec le pack du jeu dans `vanilla_resource_pack/`, chaque face porte sa vraie
texture (dessus, côtés et dessous distincts, plantes en plans croisés, feuilles et verre
à jour) et les joueurs sont dessinés en Steve, tournés comme en jeu ; sans lui, les blocs
ont une couleur unie et les joueurs un personnage aux couleurs simples. Les créatures
restent des boîtes à leur taille.

`GET /v1/world/chunks?list=cx.cz.version,…` (128 chunks au plus) : pour chaque chunk
demandé, `cx` et `cz` (i32), sa version (u32), un état (u8) et une longueur (u32), puis ses
octets. État `0` : pas en mémoire ; `1` : vous l'avez déjà dans cette version, rien n'est
renvoyé ; `2` : ses blocs suivent. Donnez `-1` comme version pour un chunk que vous n'avez
pas. Les blocs : y du bas (i16), hauteur (u16), taille de la palette (u8), la palette
(dix octets par entrée : rouge, vert, bleu, drapeaux — `1` petit bloc comme une plante, `2`
liquide, `4` texture à trous — puis trois cases de la planche en u16 : dessus, côtés,
dessous ; l'entrée 0 est l'air), puis pour chacune des 256 colonnes (`z * 16 + x`) des paires (longueur, entrée) du
bas vers le haut. Tout est préparé hors du fil du serveur.

`GET /v1/world/atlas` renvoie la planche des textures des blocs en PNG : des cases de 16
points, 64 par ligne, la case 0 blanche (`404` sans texture). `GET /v1/world/skin` renvoie
la peau de Steve du pack du jeu (`404` sans lui).

| Adresse | Corps | Effet |
|---|---|---|
| `POST /v1/world/block` | `{"x":…, "y":…, "z":…, "block":"stone"}` | change un bloc (`air` le retire) ; les joueurs le voient aussitôt |
| `POST /v1/world/fill` | le même, avec `x2`, `y2`, `z2` | remplit le pavé entre les deux coins, 32 768 blocs au plus |

Les deux répondent `{"ok":true, "block":…, "changed":n}` et sont écrits au journal.

### Analyser la carte

`POST /v1/scan` lance l'analyse ; `GET /v1/scan` dit où elle en est, puis rend son rapport.
Elle compte tous les objets rangés quelque part :

- dans les **conteneurs** (coffres, tonneaux, fours…) ;
- chez les **joueurs en jeu** (inventaire, armure, seconde main) ;
- chez les **joueurs absents**, d'après leurs fichiers enregistrés.

Le rapport donne, pour chaque sorte d'objet, le total, sa répartition entre conteneurs et
joueurs, et les cinq endroits qui en ont le plus. Il relève ensuite ce qui sort de
l'ordinaire :

| Alerte | Ce que c'est |
|---|---|
| Pile impossible | plus d'objets dans une case que le jeu n'en permet (200 blocs de diamant dans une case de 64) : cela ne s'obtient pas en jouant |
| Conteneurs jumeaux | plusieurs conteneurs bien remplis au contenu identique, case pour case : la trace habituelle d'un coffre copié |
| Objet désactivé | un objet que vous avez désactivé est encore en circulation |
| Objets rares | qui détient le plus de blocs de diamant, de netherite, d'élytres, de totems… |

Ce sont des **indices, pas des preuves**. Deux coffres rangés de la même façon existent
aussi chez les joueurs honnêtes, et une grosse réserve peut être le fruit d'une ferme.
L'analyse dit où regarder ; à vous de juger.

Le serveur ne fait qu'une copie de ce qu'il a en mémoire, puis le comptage et la lecture
des fichiers des joueurs absents se font à part : le jeu n'est pas ralenti. Ce qui n'est
pas compté : les objets tombés au sol, ceux des coffres de l'End, et le contenu des
boîtes de Shulker.

### Ce que « désactivé » veut dire

Un objet ou un bloc désactivé reste dans le monde et dans les inventaires, mais :

- on ne peut plus **le poser** ;
- on ne peut plus **s'en servir** d'un clic (le lancer, le manger, tirer avec).

Le joueur est prévenu (« Cet objet est désactivé sur ce serveur »). C'est fait pour couper
court à un bug ou à une duplication le temps de corriger. La liste est gardée dans
`disabled.txt` (un nom par ligne, modifiable à la main, relue au démarrage).

Ce que cela ne fait pas encore : empêcher de le fabriquer, de le casser, ou de frapper avec.

### `/v1/command`

Elle donne toute la console (`stop`, `op`, `kick`…) : c'est pour cela qu'elle demande le
jeton d'administration. La réponse dit seulement que la commande a été lancée ; ce qu'elle
affiche va au journal du serveur, pas dans la réponse.

## Codes de réponse

| Code | Sens |
|---|---|
| 200 | la réponse est dans le corps |
| 400 | requête illisible |
| 409 | zone non chargée : aucun joueur n'est à proximité |
| 401 | jeton manquant ou invalide (la réponse attend une demi-seconde) |
| 429 | trop de jetons refusés depuis cette adresse : elle est écartée dix minutes |
| 403 | action demandée avec le jeton de lecture |
| 404 | adresse inconnue, ou joueur, objet ou bloc introuvable |
| 405 | autre méthode que `GET` ou `POST` |
| 503 | trop de connexions à la fois (16 au plus), ou serveur occupé |

## Limites

- Deux niveaux de droits seulement (lecture, administration), un jeton chacun.
- Pas encore : changer une texture, analyser une carte, modifier les propriétés d'un bloc.
- Les réponses sont calculées au tick suivant la requête : compter jusqu'à 50 ms.
