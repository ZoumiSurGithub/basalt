# Écrire des plugins pour Basalt

Les plugins de Basalt sont des scripts **Lua 5.4**. Ce guide décrit tout ce qu'un
plugin peut faire aujourd'hui, avec des exemples vérifiés sur le serveur.

- [Premier plugin](#premier-plugin)
- [Où placer un plugin](#où-placer-un-plugin)
- [Cycle de vie](#cycle-de-vie)
- [Événements](#événements)
- [Écouteurs d'événements (comme PocketMine)](#écouteurs-dévénements-comme-pocketmine)
- [L'objet joueur](#lobjet-joueur), puis [le monde : lire et poser des blocs](#le-monde--lire-et-poser-des-blocs)
- [Commandes](#commandes)
- [Minuteries](#minuteries)
- [Référence de l'API](#référence-de-lapi)
- [Fichiers et configuration](#fichiers-et-configuration)
- [Erreurs et diagnostic](#erreurs-et-diagnostic)
- [Exemple complet](#exemple-complet)
- [Ce qui n'existe pas encore](#ce-qui-nexiste-pas-encore)

## Premier plugin

Créez le fichier `plugins/bonjour.lua` dans le dossier du serveur :

```lua
server.on("join", function(nom)
  server.log("Bienvenue à " .. nom .. " !")
end)
```

Démarrez le serveur, ou tapez `reload` dans la console s'il tourne déjà.
La console affiche `Plugin chargé : bonjour`.

## Où placer un plugin

| Forme | Emplacement | Nom du plugin |
|---|---|---|
| Un seul fichier | `plugins/bonjour.lua` | `bonjour` |
| Un dossier | `plugins/gardien/main.lua` | `gardien` |

- Le dossier des plugins se règle avec `plugins-dir` dans `server.properties`.
- Les plugins sont chargés par ordre alphabétique.
- Chaque plugin tourne dans **son propre interpréteur Lua** : deux plugins ne
  partagent ni variables ni fonctions.
- Sous Windows, les plugins demandent `lua54.dll` à côté de `basalt.exe` ;
  sous Linux, le paquet `liblua5.4-0`. Sans Lua, le serveur démarre et indique
  « Plugins désactivés ».

## Cycle de vie

1. **Chargement.** Le fichier est exécuté de haut en bas. C'est là que le plugin
   s'abonne aux événements et déclare ses commandes.
2. **`enable`.** Émis une fois que tous les plugins sont chargés.
3. **Fonctionnement.** Le serveur appelle vos fonctions quand un événement
   survient, qu'une commande est tapée ou qu'une minuterie arrive à échéance.
4. **`disable`.** Émis à l'arrêt du serveur et avant un `reload`.

La commande `reload` décharge puis recharge **tous** les plugins : les variables
en mémoire sont perdues. Enregistrez dans un fichier ce qui doit survivre.

Le serveur fonctionne à **20 ticks par seconde** et appelle les plugins depuis sa
boucle principale : une fonction qui dure longtemps bloque tout le serveur.

## La fiche du plugin : `plugin.yml`

Comme sur PocketMine et Altay, un plugin est un dossier contenant un fichier `plugin.yml`
qui le décrit. Le serveur lit cette fiche avant d'exécuter la moindre ligne du plugin.

```
plugins/
  Gardien/
    plugin.yml
    main.lua
    config.lua
```

```yaml
name: Gardien                 # obligatoire : lettres, chiffres, _ et -
version: 1.2.0                # obligatoire
main: main.lua                # fichier principal (main.lua par défaut)
api: 1                        # version de l'interface Basalt visée
description: Liste noire et expulsions
author: Enzo                  # ou  authors: [Enzo, Lea]
website: https://exemple.fr
depend: [Economie]            # plugins indispensables, chargés avant celui-ci
softdepend: [Monde]           # chargés avant s'ils sont présents

commands:
  expulser:
    description: Expulse un joueur
    usage: "/expulser <joueur>"
    permission: gardien.expulser

permissions:
  gardien.expulser:
    description: Autorise /expulser
    default: op               # op, notop, true (tout le monde) ou false (personne)
  gardien.voir:
    default: true
```

**Arguments des commandes.** La ligne `usage` décrit ce que le jeu affiche et propose quand
le joueur tape la commande : `<nom>` est obligatoire, `[nom]` facultatif. Un type peut suivre
le nom :

| Type | Ce que le jeu accepte |
|---|---|
| *(aucun)* ou `string` | un mot |
| `int`, `float` | un nombre |
| `player` | un joueur (nom ou sélecteur) |
| `text` | tout le reste de la ligne |
| `block` | un bloc du jeu, proposé pendant la frappe (`stone` ou `minecraft:stone`) |
| `item` | un objet du jeu, proposé de la même façon |
| `a\|b\|c` | un choix parmi ces mots : `<mode:on\|off>` |

Plusieurs formes d'une même commande se séparent par ` | ` :
`usage: "/set <bloc:block> | /set <motif:text>"`. Ce qui suit une parenthèse est une simple
remarque. Une commande enregistrée sans `usage` dans le `plugin.yml` reçoit un texte libre.
Le plugin reçoit toujours ses arguments sous forme de mots, comme avant.

| Clé | Rôle |
|---|---|
| `name`, `version` | identité du plugin, affichée au chargement et par la commande `plugins` |
| `main` | fichier exécuté au chargement ; il doit rester dans le dossier du plugin |
| `api` | si elle ne correspond pas à celle du serveur, le plugin n'est pas chargé |
| `depend` | si un de ces plugins manque ou échoue, celui-ci n'est pas chargé non plus |
| `softdepend` | ne change que l'ordre de chargement |
| `commands` | commandes annoncées ; le plugin les traite avec `server.command("expulser", fonction)` |
| `permissions` | permissions enregistrées, avec la règle appliquée par défaut |

Dans le plugin, la fiche est lisible : `server.plugin.name`, `server.plugin.version`,
`server.plugin.description`, `server.plugin.authors`, `server.plugin.website`.

Un plugin refusé (fiche incomplète, `api` inconnue, dépendance absente, dépendances en
boucle, deux plugins du même nom) est signalé dans la console avec la raison, et les autres
continuent de se charger. Les simples scripts sans fiche (`plugins/bonjour.lua`, ou un
dossier avec seulement `main.lua`) restent acceptés, sans version ni permissions.

### Permissions

Une permission a une règle par défaut (`default`) et peut être réglée joueur par joueur.

| Appel | Rôle |
|---|---|
| `joueur:hasPermission("gardien.expulser")` | le joueur a-t-il cette permission ? |
| `joueur:setPermission("gardien.expulser", true)` | l'accorde (`false` : la retire, `nil` : revient à la règle par défaut) |
| `joueur:isOp()` / `joueur:setOp(true)` | rang d'opérateur |

Les réglages par joueur sont sauvegardés avec ses données. Les opérateurs sont listés dans
`ops.txt`, à côté de `server.properties` (ils valent pour tous les mondes), et se gèrent aussi depuis la console : `op <joueur>`, `deop <joueur>`, `ops`.
Une permission que personne n'a déclarée est réservée aux opérateurs.

## Événements

On s'abonne avec `server.on(nom, fonction)`. Plusieurs fonctions peuvent écouter
le même événement.

| Événement | Arguments | Quand |
|---|---|---|
| `enable` | — | tous les plugins sont chargés |
| `disable` | — | arrêt du serveur ou `reload` |
| `connect` | `adresse` | un client ouvre une connexion réseau |
| `login` | `joueur` | le client a envoyé son pseudo ; il n'est pas encore en jeu |
| `load` | `joueur`, `nouveau` | les données du joueur viennent d'être chargées (`nouveau` vaut `true` à sa première visite, après son initialisation) |
| `join` | `joueur` | le joueur est apparu dans le monde |
| `chat` | `joueur`, `message` | un joueur a écrit dans le chat (message déjà nettoyé et limité à 256 caractères) |
| `save` | `joueur` | les données du joueur vont être enregistrées (toutes les minutes et à l'arrêt) |
| `block_break` | `joueur`, `x`, `y`, `z`, `bloc` | un joueur casse un bloc ; répondre `false` l'en empêche |
| `block_click` | `joueur`, `x`, `y`, `z`, `bloc` | un joueur clique sur un bloc (pour poser, ouvrir…) ; répondre `false` annule le geste |
| `death` | `joueur` | le joueur vient de mourir (chute, coup, `/kill`, vie mise à 0 par un plugin) |
| `respawn` | `joueur` | le joueur a cliqué sur « Réapparaître » ; il est de retour au point de réapparition, en pleine santé |
| `disconnect` | `adresse`, `nom` | la connexion est fermée ; `nom` vaut `nil` si le client n'avait pas atteint le login |

## Écouteurs d'événements (comme PocketMine)

En plus de `server.on`, Basalt reprend le système d'écouteurs de PocketMine : un
**écouteur** est une table de méthodes, chaque méthode reçoit un **objet événement** qu'elle
peut lire, modifier ou annuler. Le serveur applique le résultat final.

```lua
local Protection = {}

-- « on » + nom de l'événement sans « Event » : reçoit BlockBreakEvent
function Protection:onBlockBreak(event)
  local bloc = event:getBlock()
  if bloc:getY() < -60 and not event:getPlayer():isOp() then
    event:cancel()
    event:getPlayer():sendMessage("§cZone protégée")
  end
end

function Protection:onPlayerJoin(event)
  event:setJoinMessage("§a+ " .. event:getPlayer():getName())
end

-- Équivalents des annotations @priority et @handleCancelled
Protection.priorities = { onBlockBreak = EventPriority.HIGH }
Protection.handleCancelled = { onBlockBreak = false }

server.getPluginManager():registerEvents(Protection)   -- ou server.registerEvents(Protection)
```

Une méthode peut aussi porter le nom exact de l'événement (`function Protection:BlockBreakEvent(event)`).
Pour une seule fonction : `server.registerEvent("PlayerChatEvent", fonction, EventPriority.LOW, false)`.

**Ordre d'appel.** Les écouteurs sont appelés par priorité, `LOWEST`, `LOW`, `NORMAL` (par
défaut), `HIGH`, `HIGHEST`, puis `MONITOR`, tous plugins confondus : un écouteur `HIGH` d'un
plugin passe après un écouteur `LOW` d'un autre, et voit ce qu'il a changé. Un écouteur
`MONITOR` observe le résultat final sans rien modifier. Comme dans PocketMine, un événement
annulé n'est plus transmis aux écouteurs suivants, sauf à ceux qui demandent `handleCancelled`.

**Méthodes communes.** `event:getEventName()`, `event:isCancelled()`, `event:cancel()`,
`event:uncancel()`, `event:setCancelled(vrai ou faux)`, `event:isCancellable()`.

| Événement | Annulable | Lecture | Modification |
|---|---|---|---|
| `PlayerLoginEvent` | oui (le joueur est refusé) | `getPlayer()` | `setKickMessage(texte)` |
| `PlayerJoinEvent` | non | `getPlayer()`, `getJoinMessage()` | `setJoinMessage(texte)` (`""` : aucun message) |
| `PlayerQuitEvent` | non | `getPlayer()`, `getQuitMessage()`, `getQuitReason()` | `setQuitMessage(texte)` |
| `PlayerChatEvent` | oui | `getPlayer()`, `getMessage()` | `setMessage(texte)` |
| `PlayerCommandPreprocessEvent` | oui | `getPlayer()`, `getMessage()` (`"/commande arguments"`) | `setMessage(texte)` |
| `ServerCommandEvent` | oui | `getCommand()` (commande tapée dans la console) | `setCommand(texte)` |
| `BlockBreakEvent` | oui | `getPlayer()`, `getBlock()`, `getDrops()` | `setDrops({ { name = "minecraft:dirt", count = 2 } })` |
| `BlockPlaceEvent` | oui | `getPlayer()`, `getBlock()` (bloc qui va être posé) | — |
| `PlayerInteractEvent` | oui | `getPlayer()`, `getBlock()`, `getAction()` | — |
| `EntityDamageEvent` | oui | `getEntity()` (ou `getPlayer()`), `getCause()`, `getBaseDamage()` (ou `getDamage()`), `getDamager()` | `setBaseDamage(n)` |
| `EntityDamageByEntityEvent` | oui | comme `EntityDamageEvent`, seulement pour les coups entre joueurs | `setBaseDamage(n)` |
| `PlayerDeathEvent` | non | `getPlayer()`, `getDeathMessage()`, `getKeepInventory()` | `setDeathMessage(texte)`, `setKeepInventory(vrai)` |
| `PlayerRespawnEvent` | non | `getPlayer()`, `getRespawnPosition()` (x, y, z aux pieds) | `setRespawnPosition(x, y, z)` |
| `PlayerDropItemEvent` | oui (l'objet revient dans l'inventaire) | `getPlayer()`, `getItem()` | — |
| `EntityItemPickupEvent` | oui (l'objet reste au sol) | `getEntity()` (ou `getPlayer()`), `getItem()` | — |
| `PlayerGameModeChangeEvent` | oui | `getPlayer()`, `getNewGamemode()` (`GameMode.SURVIVAL` 0, `CREATIVE` 1, `ADVENTURE` 2, `SPECTATOR` 3) | — |

`getBlock()` renvoie un bloc : `bloc:getName()` (`"minecraft:stone"`), `bloc:getX()`,
`getY()`, `getZ()`, `getPosition()`. `getItem()` et `getDrops()` renvoient des tables
`{ name = "minecraft:stone", count = 3 }`. Les causes de dégâts sont dans
`EntityDamageEvent.CAUSE_FALL`, `CAUSE_VOID`, `CAUSE_ENTITY_ATTACK`, `CAUSE_SUICIDE`
(`/kill`) et `CAUSE_CUSTOM` (vie changée par un plugin).

Un événement que personne n'écoute ne coûte rien au serveur. Les plugins tournent sur leur
propre fil : si tous les écouteurs d'un événement n'ont pas répondu en 150 ms, le serveur
continue comme si personne n'écoutait (il ne reste jamais bloqué). Le déplacement (`PlayerMoveEvent`) n'existe pas encore, il serait trop fréquent
pour ce mécanisme.

## L'objet joueur

Les événements `load`, `join`, `chat`, `save`, `death` et `respawn` reçoivent un objet `joueur`. On peut aussi
l'obtenir avec `server.getPlayer(pseudo)` (ou `nil` s'il n'est pas en jeu) et
`server.getOnlinePlayers()` (la liste de tous les joueurs en jeu).

| Lecture | Modification | Donnée |
|---|---|---|
| `joueur:getName()` ou `joueur.name` | — | pseudo du compte |
| `joueur:getDisplayName()` | `joueur:setDisplayName(texte)` | nom montré dans le chat et les messages |
| `joueur:getNameTag()` | `joueur:setNameTag(texte)` | texte au-dessus de la tête |
| `joueur:getGamemode()` | `joueur:setGamemode(n)` | mode de jeu : `GameMode.SURVIVAL` (0), `CREATIVE` (1), `ADVENTURE` (2), `SPECTATOR` (3) |
| `joueur:getHealth()` | `joueur:setHealth(n)` | vie (de 0 à la vie maximale ; à 0, le joueur meurt) |
| `joueur:getMaxHealth()` | `joueur:setMaxHealth(n)` | vie maximale (20 par défaut) |
| `joueur:getFood()` | `joueur:setFood(n)` | faim, de 0 à 20 |
| `joueur:getXpLevel()` | `joueur:setXpLevel(n)` | niveau d'expérience |
| `joueur:getXpProgress()` | `joueur:setXpProgress(n)` | progression vers le niveau suivant, de 0 à 1 |
| `joueur:getPosition()` → `x, y, z` | `joueur:teleport(x, y, z)` | position |
| `joueur:getFirstPlayed()` | — | première visite (secondes, comme `server.time()`) |
| `joueur:isOnline()` | — | `true` tant que le joueur est en jeu |

Actions : `joueur:sendMessage(texte)`, `joueur:kick(raison)`, `joueur:save()`,
`joueur:give(objet, quantité)`.

Regard et main : `joueur:getItemInHand()` (nom de l'objet tenu, vide si la main est vide),
`joueur:getYaw()`, `joueur:getPitch()`, et `joueur:getDirection()` qui renvoie le décalage
`dx, dy, dz` de la direction regardée parmi les six.

### Commandes tapées en jeu

Une commande de plugin se lance depuis la console, ou en jeu par un joueur (`/expulser Léa`).
La fonction reçoit les arguments puis le joueur qui l'a tapée — `nil` depuis la console :

```lua
server.command("soin", function(args, joueur)
  if not joueur then server.log("À utiliser en jeu.") return end
  joueur:setHealth(joueur:getMaxHealth())
  joueur:sendMessage("§aVous voilà soigné.")
end)
```

Si la commande porte une `permission` dans `plugin.yml`, le serveur la vérifie avant d'appeler
le plugin ; un joueur qui ne l'a pas reçoit un refus. Sans permission déclarée, tout le monde
peut la lancer.

### Le monde : lire et poser des blocs

Comme `World` dans PocketMine, `server.getWorld()` donne accès aux blocs :

```lua
local world = server.getWorld()
local bloc = world:getBlock(x, y, z)              -- bloc:getName(), bloc:getState(), bloc:getX()…
world:setBlock(x, y, z, "minecraft:stone")        -- un nom, un état (entier) ou un bloc
local etats = world:getBlocks(x1, y1, z1, x2, y2, z2)
world:setBlocks({ x, y, z, etat, x, y, z, etat })  -- beaucoup de blocs en une fois
```

| Fonction | Rôle |
|---|---|
| `world:getBlock(x, y, z)` | le bloc à cette position : `getName()`, `getState()`, `getX()`, `getY()`, `getZ()` |
| `world:getBlockState(x, y, z)` | son état seulement (plus rapide) |
| `world:getBlocks(x1, y1, z1, x2, y2, z2)` | états de tout un volume (1 000 000 de blocs au plus), dans une table plate rangée par y, puis x, puis z |
| `world:setBlock(x, y, z, bloc)` | pose un bloc |
| `world:setBlocks(t)` | pose beaucoup de blocs : `t` = `{ x, y, z, état, x, y, z, état, … }` (1 000 000 au plus) |
| `server.blockState(nom)` | état par défaut d'un bloc (`"stone"` ou `"minecraft:stone"`), ou nil s'il n'existe pas |
| `server.blockName(état)` | nom du bloc d'un état |

Un **état** est un entier qui désigne un bloc avec toutes ses propriétés (orientation d'un
escalier, couleur de la laine…) : c'est ce qu'il faut garder pour recopier un bloc à
l'identique. Un nom désigne le bloc dans son état par défaut.

La lecture voit le monde tel qu'il était au début de l'appel. Les blocs posés sont appliqués
par le serveur à la fin de l'appel (envoyés aux joueurs bloc par bloc s'ils sont peu nombreux,
sinon par chunks entiers) et visibles dès l'appel suivant. Pour un très grand nombre de blocs,
posez-les par tranches avec `server.every(1, …)` : c'est ce que fait le plugin EasyEdit fourni
(`plugins/EasyEdit`), écrit entièrement avec ces fonctions, à lire comme exemple.

## Commandes

`server.command(nom, description, fonction)` ajoute une commande à la console
du serveur. Elle apparaît dans `help` avec sa description.

```lua
server.command("expulser", "expulse un joueur : expulser <pseudo> [raison]", function(args)
  if not args[1] then
    server.warn("usage : expulser <pseudo> [raison]")
    return
  end
  local raison = table.concat(args, " ", 2)
  if raison == "" then raison = "Expulsé par un administrateur." end
  server.kick(args[1], raison)
end)
```

- `args` est la liste des mots tapés après le nom : `expulser Bob trop bruyant`
  donne `{"Bob", "trop", "bruyant"}`.
- Le nom est mis en minuscules. Une barre oblique en tête (`/expulser`) est acceptée.
- Les commandes intégrées (`help`, `list`, `plugins`, `reload`, `motd`, `version`,
  `stop`) ne peuvent pas être remplacées.
- Si deux plugins déclarent le même nom, celui chargé en premier est utilisé.
- Les commandes des plugins se tapent en jeu (`/nom`) comme dans la console.

## Minuteries

| Fonction | Effet |
|---|---|
| `server.after(ticks, fonction)` | appelle `fonction` une fois, dans `ticks` ticks |
| `server.every(ticks, fonction)` | appelle `fonction` tous les `ticks` ticks |

`ticks` doit être un entier d'au moins 1. Repères : 20 = 1 seconde, 1200 = 1 minute.

```lua
server.every(1200, function()
  server.log(#server.players() .. " joueur(s) connecté(s)")
end)
```

Une minuterie ne peut pas être annulée. Pour la suspendre, testez une variable :

```lua
local actif = true
server.every(200, function()
  if not actif then return end
  -- travail périodique
end)
```

## Référence de l'API

Tout passe par la table globale `server`.

### Fonctions

| Fonction | Rôle |
|---|---|
| `server.on(événement, fonction)` | s'abonner à un événement |
| `server.command(nom, description, fonction)` | déclarer une commande console |
| `server.after(ticks, fonction)` | minuterie à déclenchement unique |
| `server.every(ticks, fonction)` | minuterie répétée |
| `server.log(texte)` | écrire une ligne d'information, préfixée du nom du plugin |
| `server.warn(texte)` | écrire un avertissement |
| `server.players()` | liste des pseudos connectés (login accepté), par exemple `{"Steve", "Alex"}` |
| `server.kick(pseudo, raison)` | expulser un joueur ; `raison` est facultative |
| `server.position(pseudo)` | position du joueur : renvoie `x, y, z`, ou rien s'il est inconnu |
| `server.motd()` | nom du serveur affiché dans la liste des serveurs |
| `server.set_motd(texte)` | changer ce nom (64 caractères au plus) |
| `server.stop()` | arrêter le serveur proprement |

Précisions :

- `server.kick` cherche le pseudo exact, majuscules comprises. Un pseudo inconnu
  est ignoré sans erreur.
- `server.kick`, `server.set_motd` et `server.stop` prennent effet **quand votre
  fonction se termine**, pas au milieu de son exécution.
- `server.position` renvoie la dernière position acceptée par le serveur, mise à
  jour à chaque tick. Avec `movement-check=true` (par défaut), les déplacements
  impossibles sont refusés et n'apparaissent donc pas ici.

  ```lua
  local x, y, z = server.position("Steve")
  if x then server.log(string.format("Steve est en %.1f, %.1f, %.1f", x, y, z)) end
  ```
- `server.players()` inclut les joueurs acceptés au login qui n'ont pas encore
  fini d'apparaître dans le monde.

### Constantes

| Constante | Exemple | Contenu |
|---|---|---|
| `server.version` | `"0.1.0"` | version de Basalt |
| `server.game_version` | `"1.26.65"` | version du jeu annoncée par le serveur |
| `server.protocol` | `2225` | protocole annoncé par le serveur |
| `server.max_players` | `20` | nombre maximal de joueurs |

## Sécurité : un plugin n'a que ce que le serveur lui donne

Par défaut (`plugin-sandbox=true` dans `server.properties`), un plugin ne voit que :

- l'objet `server` : événements, commandes, minuteries, joueurs, fichiers du plugin, heure ;
- le langage Lua de base : `string`, `table`, `math`, `utf8`, `coroutine`, et les fonctions
  `pairs`, `ipairs`, `next`, `select`, `type`, `tostring`, `tonumber`, `error`, `assert`,
  `pcall`, `xpcall`, `setmetatable`, `getmetatable`, `rawget`, `rawset`, `rawequal`, `rawlen` ;
- `print` (écrit dans la console du serveur) et `require` pour ses propres fichiers.

Tout le reste n'existe pas pour lui : `io`, `os`, `debug`, `package`, `load`, `loadfile`,
`dofile`, `collectgarbage`. Il ne peut donc ni toucher aux fichiers de la machine, ni lancer
un programme, ni charger du code natif ou compilé.

### Un plugin ne peut pas bloquer le serveur

Les plugins tournent sur leur propre fil d'exécution, séparé de la boucle du serveur. Le
serveur leur transmet les événements et n'attend leur réponse qu'un instant (25 millièmes de
seconde, 150 pour `login` dont il lit la réponse). Un plugin en règle répond bien avant, et
tout se passe comme s'il était appelé directement. Si un plugin traîne :

- le serveur continue sans lui : réseau, joueurs et monde ne ralentissent pas ;
- un appel Lua qui dépasse un quart de seconde est interrompu (3 secondes au chargement) ;
- au troisième dépassement, le plugin est désactivé jusqu'au prochain `reload` ;
- les événements arrivés entre-temps attendent dans une file de 1024 places ; au-delà, ils
  sont abandonnés, et la console nomme le plugin responsable ;
- la mémoire d'un plugin est plafonnée (`plugin-memory-mb`, 64 Mo par défaut).

Conséquence à connaître : si un plugin met plus de 150 millièmes de seconde à répondre à
`login`, le joueur est accepté sans son avis ; et ce qu'un plugin en retard demande
(`setHealth`, `kick`…) est appliqué quand il a fini, pas avant.

Pour un plugin de confiance qui a besoin de plus, nommez-le dans `plugin-trusted=nom1,nom2` :
il retrouve toute la bibliothèque standard de Lua, et la console le signale à chaque chargement.

Une seule opération très longue de la bibliothèque standard (une recherche de motif
pathologique sur un texte énorme) ne peut pas être interrompue en cours de route : elle occupe
le fil des plugins jusqu'à son retour, mais pas le serveur.

## Fichiers et configuration

Chaque plugin a son dossier, `plugins/<nom>/`, créé automatiquement (même pour un plugin en
un seul fichier). Il y lit et écrit avec l'objet `server` ; les noms sont relatifs à ce
dossier, et tout nom qui en sort (`..`, chemin absolu) est refusé.

| Fonction | Rôle |
|---|---|
| `server.read(nom)` | contenu du fichier, ou `nil` s'il n'existe pas |
| `server.write(nom, texte)` | remplace le fichier ; renvoie `true` si c'est fait |
| `server.append(nom, texte)` | ajoute à la fin du fichier |
| `server.time()` | heure en secondes (comme `os.time()`) |
| `server.date(format)` | date en texte, par défaut `2026-11-18 23:30:00` |
| `server.clock()` | temps processeur écoulé, pour mesurer une durée |
| `server.connect(événement, fonction)` | autre nom de `server.on` |

```lua
local vus = {}
for ligne in (server.read("vus.txt") or ""):gmatch("[^\n]+") do vus[ligne] = true end

server.on("join", function(joueur)
  local nom = joueur:getName()
  if not vus[nom] then
    vus[nom] = true
    server.append("vus.txt", nom .. "\n")
  end
end)
```

Découper un plugin en plusieurs fichiers avec `require` (nom relatif au dossier du plugin) :

```lua
-- plugins/gardien/config.lua
return { bannis = { "Griefer123" }, protocole_minimum = 2193 }
```

```lua
-- plugins/gardien/main.lua
local config = require("config")
```

Un point dans le nom désigne un sous-dossier : `require("src.commands.util")` charge
`src/commands/util.lua`. Chaque fichier n'est exécuté qu'une fois ; les suivants reçoivent la
même valeur, ce qui permet de partager un état entre fichiers. Le plugin EasyEdit fourni est
rangé ainsi (`main.lua` puis `src/`), comme les plugins PocketMine.

## Erreurs et diagnostic

- Une erreur dans un plugin n'arrête pas le serveur. Elle est affichée avec le
  fichier et la ligne : `[ERREUR] [gardien] plugins/gardien/main.lua:28: boum`
- Une erreur pendant le chargement empêche ce plugin d'être chargé ; les autres
  continuent.
- Un appel mal formé de l'API est signalé sans interrompre le plugin :
  `[ATTENTION] [gardien] appel invalide : server.on(événement, fonction)`
- La commande `plugins` liste les plugins chargés ; `reload` les recharge après
  une modification.

## Exemple complet

`plugins/gardien/main.lua` : liste noire, version minimale, journal des
nouveaux joueurs et commande d'expulsion.

```lua
local config = require("config")

local vus = {}
for ligne in (server.read("vus.txt") or ""):gmatch("[^\n]+") do vus[ligne] = true end

server.on("enable", function()
  server.log("liste noire : " .. #config.bannis .. " pseudo(s)")
end)

server.on("login", function(joueur)
  for _, nom in ipairs(config.bannis) do
    if joueur.name:lower() == nom:lower() then
      return "Vous êtes banni de ce serveur."
    end
  end
  if joueur.protocol < config.protocole_minimum then
    return "Mettez Minecraft à jour pour rejoindre ce serveur."
  end
end)

server.on("join", function(joueur)
  local nom = joueur:getName()
  if not vus[nom] then
    vus[nom] = true
    server.log("Premier passage de " .. nom)
    server.append("vus.txt", nom .. "\n")
  end
end)

server.command("expulser", "expulse un joueur : expulser <pseudo> [raison]", function(args)
  if not args[1] then
    server.warn("usage : expulser <pseudo> [raison]")
    return
  end
  local raison = table.concat(args, " ", 2)
  if raison == "" then raison = "Expulsé par un administrateur." end
  server.kick(args[1], raison)
end)
```

`plugins/gardien/config.lua` :

```lua
return {
  bannis = { "Griefer123" },
  protocole_minimum = 2193,
}
```

## Ce qui n'existe pas encore

L'API suit l'avancement du serveur. Ne sont pas encore accessibles aux plugins :

- un événement de déplacement (`PlayerMoveEvent`), trop fréquent pour le fil des plugins ;
- le contenu des inventaires (seuls `joueur:give` et `joueur:getItemInHand` existent) ;
- les entités autres que les joueurs ;
- l'annulation d'une minuterie ;
- la communication directe entre plugins (en dehors des événements qu'ils écoutent tous).

Ces fonctions seront ajoutées à mesure que le serveur les prendra en charge ; ce
document sera mis à jour en même temps.
