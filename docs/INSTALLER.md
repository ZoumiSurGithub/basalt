# Installer Basalt

Ce guide explique, pas à pas, comment installer un serveur Basalt sur **Windows** ou sur
**Linux**, le démarrer, s'y connecter depuis Minecraft et le mettre à jour.

Il faut le jeu **Minecraft Bedrock** (Windows, mobile, console) en version **1.26.60 ou
plus**. Le serveur, lui, fonctionne sur n'importe quel PC ou serveur récent : 1 Go de
mémoire libre et une connexion Internet suffisent pour commencer.

- [Windows](#windows)
- [Linux](#linux)
- [Se connecter au serveur](#se-connecter-au-serveur)
- [Ouvrir le serveur aux joueurs d'Internet](#ouvrir-le-serveur-aux-joueurs-dinternet)
- [Régler le serveur](#régler-le-serveur)
- [Devenir opérateur](#devenir-opérateur)
- [En cas de problème](#en-cas-de-problème)

---

## Windows

### 1. Récupérer Basalt

Sur la page GitHub du projet, cliquez sur **Code**, puis **Download ZIP**. Faites un clic
droit sur le fichier téléchargé, **Extraire tout…**, et choisissez un dossier (par exemple
le Bureau). Vous obtenez un dossier `basalt` qui contient un sous-dossier `windows`.

### 2. Installer

Deux façons, au choix.

**Sans rien compiler (conseillé)** : la dernière version publiée est téléchargée toute
prête. Ouvrez le dossier `basalt\windows`, cliquez dans la barre d'adresse de l'explorateur,
tapez `cmd` puis Entrée. Dans la fenêtre noire qui s'ouvre, tapez :

```
Installer.cmd -Repo ZoumiSurGithub/basalt
```

**En compilant le code** (plus long, utile pour essayer des changements pas encore
publiés) : double-cliquez simplement sur **`Installer.cmd`**. Rust est installé s'il manque
(avec votre accord), puis Basalt est compilé, ce qui prend plusieurs minutes la première
fois.

Dans les deux cas, l'installateur :

1. installe Basalt dans `%LOCALAPPDATA%\Basalt` (tapez ce nom dans la barre d'adresse de
   l'explorateur pour y aller) ;
2. télécharge `lua54.dll`, nécessaire aux plugins ;
3. télécharge une fois le serveur officiel de Mojang pour en garder les plans des
   structures (villages, manoirs, épaves…), puis jette le reste ; vous acceptez alors le
   [contrat de licence de Minecraft](https://www.minecraft.net/eula) ;
4. ouvre le pare-feu de Windows pour Basalt (Windows vous demande votre accord) ;
5. crée un raccourci **Basalt** sur le Bureau et dans le menu Démarrer.

### 3. Démarrer

Double-cliquez sur le raccourci **Basalt**. Une fenêtre s'ouvre : c'est la console du
serveur. Le serveur est prêt quand elle affiche qu'il écoute sur le port 19132. Pour
l'arrêter proprement, tapez `stop` dans cette fenêtre (le monde est alors enregistré).

Les réglages (`server.properties`), le monde et les plugins sont dans
`%LOCALAPPDATA%\Basalt`.

### 4. Mettre à jour

- Installé **sans compiler** : double-cliquez sur `windows\Mettre-a-jour.cmd`. La dernière
  version publiée est téléchargée.
- Installé **en compilant** : remplacez le dossier `basalt` par la nouvelle version du code,
  puis double-cliquez sur `windows\Mettre-a-jour.cmd`.

Arrêtez le serveur avant. Vos réglages, votre monde et vos plugins sont conservés ; la
version précédente reste disponible sous le nom `basalt.exe.bak`.

### Désinstaller

Double-cliquez sur `windows\Desinstaller.cmd`. Vos réglages, votre monde et vos plugins
sont gardés ; pour tout supprimer, lancez-le depuis `cmd` avec `Desinstaller.cmd -Purge`.

---

## Linux

Testé sur Debian 12 et Ubuntu 22.04 ou plus récent (processeur x86_64).

### 1. Installer en une commande

Rien à compiler : la dernière version publiée est téléchargée dans le dossier courant.

```sh
sudo apt install curl unzip
mkdir basalt && cd basalt
curl -sSfL https://raw.githubusercontent.com/ZoumiSurGithub/basalt/main/get.sh | sh
```

Le script vérifie l'archive (empreinte SHA-256), installe Basalt et Lua (pour les plugins),
puis télécharge une fois le serveur officiel de Mojang pour en garder les plans des
structures ; vous acceptez alors le
[contrat de licence de Minecraft](https://www.minecraft.net/eula). `unzip` sert à extraire
ces plans.

### 2. Démarrer

```sh
./start.sh
```

La console du serveur s'affiche. Tapez `stop` pour l'arrêter proprement (le monde est
alors enregistré). Les réglages (`server.properties`), le monde et les plugins sont dans
le dossier `basalt`.

Pour qu'il continue de tourner une fois votre session fermée, utilisez `screen` ou `tmux`,
ou mieux, un service (ci-dessous).

### 3. En faire un service (démarrage automatique)

Pour un serveur permanent, qui redémarre tout seul avec la machine, installez Basalt dans
`/opt/basalt` avec un utilisateur dédié :

```sh
sudo useradd --system --home /opt/basalt --shell /usr/sbin/nologin basalt
sudo mkdir -p /opt/basalt && sudo chown basalt:basalt /opt/basalt
cd /opt/basalt
sudo -u basalt sh -c 'curl -sSfL https://raw.githubusercontent.com/ZoumiSurGithub/basalt/main/get.sh | sh'
sudo cp /opt/basalt/basalt.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now basalt
```

Commandes utiles :

```sh
sudo systemctl status basalt      # état
journalctl -u basalt -f           # journal en direct
sudo systemctl restart basalt     # redémarrer (après une mise à jour)
sudo systemctl stop basalt        # arrêter
```

### 4. Mettre à jour

```sh
cd basalt                 # ou cd /opt/basalt
./update.sh               # dernière version publiée
./update.sh -v dev        # ou : dernière version de développement
```

Puis redémarrez le serveur (`stop` dans la console et `./start.sh`, ou
`sudo systemctl restart basalt`). Le script affiche les nouveautés de la version
installée. Vos réglages, votre monde, vos listes et vos plugins ne sont jamais touchés.

Avec le service, lancez la mise à jour en tant qu'utilisateur `basalt` :
`sudo -u basalt ./update.sh`.

### Compiler soi-même (facultatif)

Pour essayer un code qui n'est pas encore publié : `./install.sh` depuis le dossier du
code (installe Rust et les outils nécessaires avec apt, puis compile dans `./dist`).

---

## Se connecter au serveur

Dans Minecraft : **Jouer**, onglet **Serveurs**, **Ajouter un serveur** :

| Où est le serveur | Adresse à saisir | Port |
|---|---|---|
| Sur le même PC | `127.0.0.1` | 19132 |
| Sur un autre PC de la maison | l'adresse locale de ce PC (par exemple `192.168.1.20`) | 19132 |
| Ailleurs, par Internet | l'adresse publique du serveur ou son nom de domaine | 19132 |

Il faut être connecté à un compte Microsoft (Xbox) dans le jeu : Basalt vérifie l'identité
des joueurs. Pour trouver l'adresse locale d'un PC : `ipconfig` sous Windows,
`ip addr` sous Linux.

## Ouvrir le serveur aux joueurs d'Internet

Le serveur doit être joignable sur ces ports :

| Port | Protocole | À quoi il sert |
|---|---|---|
| 19132 | TCP | prise de contact des jeux (et liste des serveurs) |
| 19140 | UDP | liaison des jeux |

- **À la maison, derrière une box** : dans l'interface de votre box, redirigez ces deux
  ports vers le PC du serveur, puis indiquez votre adresse publique dans
  `server.properties` : `public-ip=…` (elle est affichée par des sites comme
  « quel est mon IP »).
- **Sur un serveur loué (VPS)** : ouvrez ces ports dans son pare-feu, par exemple
  `sudo ufw allow 19132/tcp` et `sudo ufw allow 19140/udp`.
- **Sous Windows**, l'installateur a déjà ouvert le pare-feu de Windows pour Basalt.

## Régler le serveur

Les réglages sont dans `server.properties`, à côté du programme (créé au premier
démarrage). Arrêtez le serveur avant de le modifier. Les plus utiles :

| Réglage | Rôle |
|---|---|
| `server-name` | nom affiché dans la liste des serveurs |
| `max-players` | nombre de joueurs en même temps |
| `gamemode` | mode des nouveaux joueurs : `Survival`, `Creative`, `Adventure`, `Spectator` |
| `level-name` | nom du monde principal : son dossier est `worlds/<level-name>`, avec les autres mondes (voir [MONDE.md](MONDE.md#où-et-comment-le-monde-est-enregistré)) |
| `level-type`, `level-seed` | terrain et graine d'un **nouveau** monde : `vanilla`, `flat`, `void` (vide, avec une plateforme au point d'apparition) ou `plugin:<nom>` |
| `view-distance` | distance de vue, en chunks (plus grand = plus de calcul) |
| `white-list` | `true` : seuls les joueurs de `white-list.txt` peuvent entrer |
| `keep-inventory` | `true` : on garde son inventaire à la mort |
| `spawn-animals` | `false` : pas d'animaux |
| `spawn-monsters` | `false` : pas de monstres qui apparaissent la nuit (monde paisible) |
| `api-port` | port de l'API de gestion à distance ; `0` : éteinte (voir [API.md](API.md)) |
| `api-bind` | adresse écoutée par l'API ; `127.0.0.1` : cette machine seulement |
| `mob-griefing` | `false` : les explosions (creeper…) blessent sans casser de blocs |
| `fire-spreads` | `false` : le feu ne brûle plus les blocs et ne gagne plus de terrain (il blesse toujours) |
| `weather-cycle` | `false` : la pluie et l'orage ne viennent plus d'eux-mêmes (`/weather` les commande toujours) |
| `tick-distance` | Distance de simulation en chunks (4 à 12) : au-delà, les animaux restent figés |
| `public-ip` | adresse publique, derrière une box |

### `vanilla.properties` : couper des mécaniques du jeu

Un second fichier, créé à côté de `server.properties` au premier démarrage, liste les
mécaniques du jeu une par une. Tout y vaut `true` ; passez une ligne à `false` pour la couper
sur tout le serveur, puis redémarrez. Un serveur de mini-jeux coupera par exemple la faim,
la météo et la pousse des cultures.

| Clé | Ce qu'elle coupe à `false` |
|---|---|
| `hunger`, `natural-regeneration` | la faim ; la vie qui remonte seule |
| `fall-damage`, `fire-damage`, `drowning`, `pvp` | les dégâts de chute, du feu et de la lave, la noyade, les coups entre joueurs |
| `daylight-burn`, `lightning`, `snow-and-ice` | les morts-vivants qui brûlent au soleil, la foudre, la neige et la glace sous la pluie |
| `crop-growth`, `tree-growth`, `grass-spread`, `leaf-decay`, `bone-meal` | la pousse des cultures et des arbres, l'herbe qui s'étend, les feuilles qui tombent, la poudre d'os |
| `liquid-flow`, `tnt` | l'eau et la lave qui coulent, la TNT |
| `fishing`, `boats`, `double-chests`, `trading`, `sleeping` | la pêche, les bateaux, les grands coffres, les échanges, le sommeil |
| `mob-drops`, `block-drops`, `experience`, `durability` | le butin des créatures et des blocs, l'expérience, l'usure des objets |
| `enchant-rules` | les refus de `/enchant` (objet inadapté, enchantements qui s'excluent) |
| `village-animals` | les animaux dans les enclos des villages |

Les réglages `spawn-animals`, `spawn-monsters`, `mob-griefing`, `fire-spreads`, `weather-cycle`
et `keep-inventory` y figurent aussi. Ils sont toujours lus dans `server.properties` ; s'ils
sont dans les deux fichiers, `vanilla.properties` l'emporte.

Pour une règle au cas par cas (pas de chute dans une zone, un message de mort maison), un
plugin fait mieux : voir « Dégâts et messages de mort » dans docs/PLUGINS.md.

Chaque réglage est expliqué dans le fichier lui-même.

## Devenir opérateur

Les opérateurs peuvent utiliser toutes les commandes en jeu (`/gamemode`, `/tp`,
`/locate`…) et écrire en couleur dans le chat. Dans la console du serveur, tapez :

```
op VotrePseudo
```

## En cas de problème

- **Le serveur n'apparaît pas ou la connexion échoue** : vérifiez les ports ci-dessus, et
  que le jeu est en version 1.26.60 ou plus. Le journal de la console dit souvent pourquoi
  un joueur est refusé.
- **« Structures : plans officiels… illisibles » ou structures simplifiées** : les plans de
  Mojang n'ont pas pu être téléchargés (pas d'Internet, `unzip` absent sous Linux).
  Relancez la mise à jour ; en attendant, Basalt utilise ses structures faites à la main.
- **Les plugins ne se chargent pas** : Lua 5.4 est absent. Sous Linux :
  `sudo apt install liblua5.4-0` ; sous Windows, relancez l'installateur.
- **Le serveur saccade** : en jeu, `/timings on`, jouez quelques minutes, puis
  `/timings report` ; le rapport (dossier `timings`) montre ce qui prend du temps.

Pour aller plus loin : écrire un plugin, voir [DEBUTER.md](DEBUTER.md) puis
[PLUGINS.md](PLUGINS.md).
