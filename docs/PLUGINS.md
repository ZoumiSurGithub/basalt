# Écrire des plugins pour Basalt

Les plugins de Basalt sont des scripts **Lua 5.4**. Ce guide décrit tout ce qu'un
plugin peut faire aujourd'hui, avec des exemples vérifiés sur le serveur.

> **Vous débutez ?** Commencez par [Créer un plugin sans savoir programmer](DEBUTER.md) :
> les bases du Lua, les outils simples (messages, couleurs, minuteries en secondes, données
> enregistrées, `config.yml`) et des plugins prêts à l'emploi dans [`exemples`](exemples).

- [Premier plugin](#premier-plugin)
- [Où placer un plugin](#où-placer-un-plugin)
- [Cycle de vie](#cycle-de-vie)
- [La fiche du plugin : `plugin.yml`](#la-fiche-du-plugin--pluginyml), avec les [permissions](#permissions)
- [Événements](#événements)
- [Écouteurs d'événements (comme PocketMine)](#écouteurs-dévénements-comme-pocketmine)
- [L'objet joueur](#lobjet-joueur), puis [le monde : lire et poser des blocs](#le-monde--lire-et-poser-des-blocs)
- [Commandes](#commandes)
- [Minuteries](#minuteries)
- [Personnages non joueurs (PNJ)](#personnages-non-joueurs-pnj)
- [Référence de l'API](#référence-de-lapi)
- [Sécurité : un plugin n'a que ce que le serveur lui donne](#sécurité--un-plugin-na-que-ce-que-le-serveur-lui-donne)
- [Fichiers et configuration](#fichiers-et-configuration)
- [Erreurs et diagnostic](#erreurs-et-diagnostic)
- [Exemple complet](#exemple-complet)
- [Générateurs de terrain](#générateurs-de-terrain)
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
| Un dossier avec `plugin.yml` | `plugins/gardien/plugin.yml` (voir plus bas) | celui de la fiche |
| Un générateur de terrain | `plugins/<dossier>/generators/<nom>.lua`, dans un plugin avec `plugin.yml` | voir [Générateurs de terrain](#générateurs-de-terrain) |

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

Le serveur fonctionne à **20 ticks par seconde**. Les plugins tournent sur leur propre
fil : une fonction qui dure longtemps ne bloque pas le serveur, mais retarde le plugin
(voir [Un plugin ne peut pas bloquer le serveur](#un-plugin-ne-peut-pas-bloquer-le-serveur)).

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
| `biomes` | un biome du terrain vanilla, proposé pendant la frappe |
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
| `quit` | `joueur` | le joueur est parti (comme `disconnect`, avec l'objet joueur) |
| `disconnect` | `adresse`, `nom` | la connexion est fermée ; `nom` vaut `nil` si le client n'avait pas atteint le login |

Un nom d'événement inconnu est signalé dans la console, avec le nom le plus proche
(`événement inconnu « joinn » (vouliez-vous dire « join » ?)`).

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

server.registerEvents(Protection)
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
| `PlayerItemUseEvent` | oui (l'objet ne fait rien) | `getPlayer()`, `getItem()` | — |
| `PlayerItemConsumeEvent` | oui (l'aliment n'est pas mangé) | `getPlayer()`, `getItem()` | — |
| `PlayerExhaustEvent` | oui (le point de faim n'est pas perdu) | `getPlayer()`, `getAmount()` (fatigue qui va coûter un point) | — |
| `PlayerExperienceChangeEvent` | oui | `getPlayer()`, `getAmount()` (points gagnés) | `setAmount(n)` |
| `PlayerBedEnterEvent` | oui (le joueur ne se couche pas) | `getPlayer()`, `getBed()` | — |
| `PlayerBedLeaveEvent` | non | `getPlayer()`, `getBed()` | — |
| `LeavesDecayEvent` | oui (la feuille reste) | `getBlock()` | — |
| `BlockFallEvent` | oui (le bloc ne tombe pas) | `getBlock()` (sable, gravier… qui va descendre) | — |
| `BlockGrowEvent` | oui (rien ne pousse) | `getBlock()` (le bloc tel qu'il sera : culture grandie, herbe étendue, canne plus haute) | — |

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

Les événements `load`, `join`, `quit`, `chat`, `save`, `death` et `respawn` reçoivent un objet `joueur`. On peut aussi
l'obtenir avec `server.getPlayer(pseudo)` (ou `nil` s'il n'est pas en jeu ; les majuscules
ne comptent pas) et `server.getOnlinePlayers()` (la liste de tous les joueurs en jeu).

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
| `joueur:getPosition()` → `x, y, z` | `joueur:teleport(x, y, z)` | position **des pieds** (`teleport(joueur:getPosition())` ne déplace pas) |
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
posez-les par tranches avec `server.everySeconds(0.05, …)` (à chaque tick) : c'est ce que fait le plugin EasyEdit fourni
(`plugins/EasyEdit`), écrit entièrement avec ces fonctions, à lire comme exemple.

Comme WorldEdit dans le jeu, les blocs posés par un plugin ne déclenchent pas l'écoulement
de l'eau et de la lave : une source posée par un plugin reste immobile jusqu'à ce qu'un
joueur modifie un bloc à côté. Un coffre posé par un plugin est vide ; un coffre de
structure (village, temple, mine) reçoit son butin à sa première ouverture, même si le
plugin l'a lu entre-temps.

## Commandes

`server.command(nom, description, fonction)` ajoute une commande à la console
du serveur. Elle apparaît dans `help` avec sa description.

```lua
server.command("expulser", "expulse un joueur : expulser <pseudo> [raison]", function(args)
  if not args[1] then
    server.warn("usage : expulser <pseudo> [raison]")
    return
  end
  local cible = server.getPlayer(args[1])
  if not cible then
    server.warn(args[1] .. " n'est pas en jeu")
    return
  end
  local raison = table.concat(args, " ", 2)
  if raison == "" then raison = "Expulsé par un administrateur." end
  cible:kick(raison)
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
| `server.afterSeconds(secondes, fonction)` | appelle `fonction` une fois, dans tant de secondes (décimales permises) |
| `server.everySeconds(secondes, fonction)` | appelle `fonction` toutes les tant de secondes |

Le serveur tourne à 20 ticks par seconde : une durée est arrondie au tick (0,05 s) le plus
proche, et vaut au moins un tick.

Chacune renvoie une minuterie : `minuterie:stop()` l'arrête, `minuterie:isRunning()` dit si
elle tourne encore. Une minuterie répétée reçoit aussi elle-même en argument.

```lua
local compte = server.everySeconds(60, function()
  server.log(#server.getOnlinePlayers() .. " joueur(s) connecté(s)")
end)
-- plus tard : compte:stop()

server.everySeconds(1, function(minuterie)
  -- s'arrête d'elle-même quand le serveur est vide
  if #server.getOnlinePlayers() == 0 then minuterie:stop() end
end)
```

## Personnages non joueurs (PNJ)

Un plugin peut faire apparaître des personnages que les joueurs voient comme des joueurs
(nom au-dessus de la tête, dans la liste des joueurs). Le serveur les fait marcher : ils
montent les marches d'un bloc, tombent dans les trous et s'arrêtent devant un mur ; le
plugin ne leur donne que leur direction.

| Fonction | Effet |
|---|---|
| `server.spawnNpc(nom, x, y, z)` | fait apparaître un PNJ, pieds en `x, y, z` (ou le replace s'il existe) |
| `server.walkNpc(nom, vx, vz)` | le fait marcher en continu : vitesse en blocs par seconde sur x et z (un joueur marche à 4,3) ; `0, 0` l'arrête |
| `server.teleportNpc(nom, x, y, z)` | le déplace d'un coup |
| `server.removeNpc(nom)` | le retire |
| `server.removeNpcs()` | retire tous les PNJ du plugin |

```lua
server.command("garde", function(args, joueur)
  local x, y, z = joueur:getPosition()
  server.spawnNpc("Garde", x, y, z)
  server.walkNpc("Garde", 0, 2)            -- vers le sud, 2 blocs par seconde
end)
```

- Un PNJ appartient au plugin qui l'a créé : les autres ne peuvent ni le déplacer ni le
  retirer. Le nom (32 caractères au plus) l'identifie, sans tenir compte des majuscules.
- 512 PNJ au plus sur le serveur. Ils sont retirés par `reload` et ne sont pas enregistrés :
  recréez-les à l'événement `enable` si besoin.
- Les PNJ n'ont ni vie ni inventaire et ne réagissent pas aux coups.

## Modifier objets et blocs

Un plugin peut changer ce que valent les objets et les blocs du jeu : ce que nourrit un
aliment, combien de temps dure un outil, ce que rend un bloc cassé, s'il tombe. On le fait
une fois, au chargement du plugin ; le serveur applique ensuite la nouvelle règle partout,
sans que le plugin soit appelé à chaque fois.

```lua
-- Un pain qui rassasie entièrement, une épée en bois qui frappe fort mais casse vite
server.items.modify("minecraft:bread", { food = 20, saturation = 12 })
server.items.modify("minecraft:wooden_sword", { attackDamage = 9, durability = 10 })

-- Le gravier ne tombe plus et rend toujours du silex ; la pierre du fond est incassable
server.blocks.modify("minecraft:gravel", { falls = false, drops = { ["minecraft:flint"] = 1 } })
server.blocks.modify("minecraft:deepslate", { breakable = false })
```

### Objets : `server.items.modify(nom, valeurs)`

| Valeur | Type | Effet |
|---|---|---|
| `food` | nombre (0 à 20) | points de faim rendus |
| `saturation` | nombre (0 à 20) | satiété rendue |
| `leftover` | texte | objet qui reste en main après le repas (`"minecraft:bowl"`) ; `""` : rien |
| `durability` | nombre | utilisations avant de casser ; `0` : incassable |
| `attackDamage` | nombre | dégâts d'un coup porté avec l'objet |

### Blocs : `server.blocks.modify(nom, valeurs)`

| Valeur | Type | Effet |
|---|---|---|
| `drops` | table | butin à la place de celui du jeu ; `{}` : rien |
| `falls` | vrai ou faux | tombe quand rien ne le porte, comme le sable |
| `needsFloor` | vrai ou faux | se détache si le bloc du dessous disparaît, comme une fleur |
| `breakable` | vrai ou faux | peut être cassé en survie |

Le butin s'écrit de deux façons, que l'on peut mêler :

```lua
drops = { ["minecraft:diamond"] = 2 }                      -- objet = quantité
drops = { "minecraft:stick", { "minecraft:flint", 3 } }    -- liste ; sans quantité : 1
```

### Revenir à la règle du jeu

```lua
server.items.reset("minecraft:bread")
server.blocks.reset("minecraft:gravel")
```

La commande `reload` de la console remet aussi tout à la règle du jeu avant de recharger
les plugins : chacun refait alors ses modifications.

### À savoir

- On ne modifie que **ce qui existe** dans le jeu. Un nom inconnu est signalé dans la
  console (`objet inconnu « … »`) ; une valeur inconnue ou du mauvais type arrête le
  plugin sur la ligne fautive.
- `food`, `saturation` et `leftover` ne valent que pour ce qui se mange déjà : le jeu du
  joueur ne porte pas une pierre à la bouche.
- Le butin fixé par `drops` suit la règle du jeu pour l'outil : un minerai n'en rend
  toujours pas à la main.
- La barre d'usure est dessinée par le jeu du joueur d'après la durée de vie d'origine.
  Avec `durability`, l'objet casse bien au moment voulu, mais la barre peut ne pas être
  vide (ou l'être avant) à cet instant.
- Dureté, vitesse de minage, taille de pile, lumière et glissance sont calculées par le
  jeu du joueur : elles ne sont pas modifiables, pour que l'écran dise toujours la même
  chose que le serveur.
- Si deux plugins modifient la même valeur, le dernier chargé l'emporte.
- Pour une règle qui dépend des circonstances (du joueur, du lieu), utilisez plutôt un
  événement : `BlockBreakEvent` et son butin, `PlayerItemConsumeEvent`.

## Créer des objets

Un plugin peut ajouter au jeu des objets qui n'existent pas dans Minecraft. À la différence
d'un objet modifié, un objet créé est **déclaré au jeu du joueur** à sa connexion : taille
de pile, durée de vie, valeur nutritive sont appliquées par le jeu lui-même, et l'écran dit
exactement la même chose que le serveur.

```lua
-- Une pierre précieuse, qui s'empile par 16 et brille
server.items.create("monserveur:rubis", {
  name = "Rubis",
  icon = "emerald",
  maxStack = 16,
  glint = true,
})

-- Un plat : 8 de faim, se mange même le ventre plein, laisse un bol
server.items.create("monserveur:ragout_du_chef", {
  name = "Ragoût du chef",
  icon = "rabbit_stew",
  food = 8,
  saturation = 12,
  alwaysEdible = true,
  leftover = "minecraft:bowl",
})

-- Une arme : 12 de dégâts, casse après 300 coups
server.items.create("monserveur:lame_de_rubis", {
  name = "Lame de rubis",
  icon = "netherite_sword",
  attackDamage = 12,
  durability = 300,
})
```

Le nom s'écrit `espace:objet`, en minuscules, chiffres et `_`. L'espace de noms est le
vôtre (le nom de votre serveur ou de votre plugin) ; `minecraft` est réservé au jeu.

| Réglage | Type | Effet | Par défaut |
|---|---|---|---|
| `name` | texte | nom affiché au joueur | le nom de l'objet |
| `icon` | texte | image : celle d'un objet du jeu (`emerald`, `netherite_sword`, `bread`…) | `stick` |
| `maxStack` | nombre (1 à 64) | taille de pile | 64 |
| `glint` | vrai ou faux | brille comme un objet enchanté | faux |
| `category` | texte | onglet du menu créatif : `"construction"`, `"nature"`, `"equipment"`, `"items"` | `equipment` pour ce qui s'use ou frappe, `items` sinon |
| `group` | texte | groupe du menu créatif : les objets du même groupe se replient sous une seule case, qui porte ce nom | aucun |
| `creative` | vrai ou faux | présent dans le menu créatif | vrai |
| `food` | nombre (0 à 20) | points de faim rendus : l'objet se mange | ne se mange pas |
| `saturation` | nombre (0 à 20) | satiété rendue | `food` × 0,6 |
| `alwaysEdible` | vrai ou faux | se mange le ventre plein | faux |
| `leftover` | texte | objet qui reste en main après le repas | rien |
| `durability` | nombre | utilisations avant de casser ; l'objet ne s'empile plus et se tient comme un outil | ne s'use pas |
| `attackDamage` | nombre | dégâts d'un coup | 1 |
| `handEquipped` | vrai ou faux | tenu en main comme un outil | faux |

### Menu créatif et groupes

Un objet créé apparaît dans le menu créatif, à la fin de l'onglet choisi par `category`.
Avec `group`, plusieurs objets partagent une case qui se déplie, comme les laines ou les
planches du jeu ; la case montre le premier objet créé du groupe.

```lua
server.items.create("monserveur:rubis", { name = "Rubis", icon = "emerald", category = "nature", group = "Gemmes" })
server.items.create("monserveur:saphir", { name = "Saphir", icon = "diamond", category = "nature", group = "Gemmes" })
server.items.create("monserveur:cle_admin", { name = "Clé", icon = "trial_key", creative = false })  -- hors du menu
```

Deux objets vont dans le même groupe s'ils ont le même `group` **et** la même `category`.

### Donner l'objet

```lua
joueur:give("monserveur:rubis", 3)
```

ou, dans la console et en jeu : `give Steve monserveur:rubis 3`.

### Réagir quand l'objet est utilisé

`PlayerItemUseEvent` est appelé quand un joueur fait un clic droit (ou un appui long, sur
écran tactile) avec un objet en main sans viser de bloc. C'est là qu'un plugin donne son
effet à un objet :

```lua
local Baguette = {}

function Baguette:onPlayerItemUse(event)
  local joueur = event:getPlayer()
  if event:getItem().name == "monserveur:baguette" then
    local x, y, z = joueur:getPosition()
    joueur:teleport(x, y + 10, z)
    joueur:sendMessage("§dZou !")
  end
end

server.registerEvents(Baguette)
```

Annuler l'événement empêche l'usage normal de l'objet (un aliment n'est pas mangé).
Tant que le joueur garde le clic appuyé, le jeu renvoie le geste toutes les 1,6 seconde :
l'événement est alors appelé de nouveau. Pour un clic sur un bloc, c'est
`PlayerInteractEvent` ; pour un coup porté, `EntityDamageByEntityEvent`.

### À savoir

- **Créez vos objets au chargement du plugin**, pas plus tard. Le jeu reçoit la liste des
  objets une seule fois, à la connexion : un objet créé pendant que des joueurs sont en
  jeu ne leur apparaît qu'à leur prochaine connexion (la console le signale).
- **L'image est celle d'un objet du jeu.** Aucun fichier n'est envoyé aux joueurs ; des
  images propres demanderont un pack de ressources, que Basalt n'envoie pas encore. Le nom
  à donner à `icon` est le nom de texture du jeu, en général le nom de l'objet
  (`emerald`, `apple`, `iron_ingot`, `rabbit_stew`). Si le jeu ne le connaît pas, l'objet
  s'affiche **vide**.
- **Outils et armures d'origine n'ont pas d'image à leur nom** : le jeu range celles du
  bois, de la pierre, du fer, de l'or, du diamant, du cuir et des mailles dans des planches
  communes. `diamond_sword` ou `iron_pickaxe` donnent donc un objet vide (la console le
  signale). Prenez la Netherite (`netherite_sword`, `netherite_pickaxe`,
  `netherite_helmet`…) ou un autre objet (`mace`, `trident`, `blaze_rod`, `stick`).
- Pour une image qui n'existe pas dans le jeu, ajoutez un pack de ressources : voir
  [PACKS.md](PACKS.md).
- Dans la sauvegarde d'un joueur, l'objet est rangé sous son nom. Si le plugin est retiré,
  l'objet disparaît de l'inventaire au chargement.
- Un objet créé se modifie ensuite comme un autre (`server.items.modify`), et répond aux
  mêmes événements (`PlayerItemConsumeEvent`…).
- 2 000 objets créés au plus.
- Blocs et entités nouveaux ne sont pas encore possibles.

## Écrire ses plugins dans VS Code

Pour que VS Code propose les fonctions de Basalt, affiche leur aide et signale les fautes :

1. Installez l'extension **Lua** (éditeur « sumneko », identifiant `sumneko.lua`).
2. Copiez `docs/vscode/luarc.json` dans le dossier de votre plugin (`plugins/MonPlugin/`) et
   renommez-le `.luarc.json`. Il indique à l'extension où trouver les définitions
   (`docs/vscode/basalt-api.lua`). Si votre plugin n'est pas dans `plugins/` du serveur,
   remplacez le chemin par celui du fichier sur votre machine.
3. Ouvrez le dossier du plugin dans VS Code : tapez `server.` et la liste apparaît.

`basalt-api.lua` ne s'exécute pas et ne se copie pas dans le plugin : il ne fait que décrire
l'API. Il suit ce document ; en cas d'écart, ce document fait foi.

## Référence de l'API

Tout passe par la table globale `server`.

### Fonctions

| Fonction | Rôle |
|---|---|
| `server.on(événement, fonction)` | s'abonner à un événement |
| `server.command(nom, description, fonction)` | déclarer une commande console |
| `server.log(texte)` | écrire une ligne d'information, préfixée du nom du plugin |
| `server.broadcast(texte)` | envoyer un message à tous les joueurs en jeu |
| `server.getPlayer(pseudo)` | le joueur en jeu (majuscules indifférentes), ou `nil` |
| `server.getOnlinePlayers()` | la liste des joueurs en jeu (objets joueur) |
| `server.afterSeconds(s, f)`, `server.everySeconds(s, f)` | minuteries en secondes (voir [Minuteries](#minuteries)) |
| `server.saveData(nom, valeur)`, `server.loadData(nom, défaut)` | données enregistrées (voir [Fichiers et configuration](#fichiers-et-configuration)) |
| `server.getConfig(défauts)` | réglages du `config.yml` du plugin |
| `server.warn(texte)` | écrire un avertissement |
| `server.getMotd()` | nom du serveur affiché dans la liste des serveurs |
| `server.setMotd(texte)` | changer ce nom (64 caractères au plus) |
| `server.stop()` | arrêter le serveur proprement |
| `server.items.modify(nom, valeurs)`, `server.items.reset(nom)` | changer un objet du jeu (voir « Modifier objets et blocs ») |
| `server.items.create(nom, réglages)` | créer un objet nouveau (voir « Créer des objets ») |
| `server.blocks.modify(nom, valeurs)`, `server.blocks.reset(nom)` | changer un bloc du jeu |

Précisions :

- `joueur:kick`, `server.setMotd` et `server.stop` prennent effet **quand votre
  fonction se termine**, pas au milieu de son exécution.
- `joueur:getPosition()` renvoie la dernière position acceptée par le serveur, mise à
  jour à chaque tick. Avec `movement-check=true` (par défaut), les déplacements
  impossibles sont refusés et n'apparaissent donc pas ici.

### Constantes

| Constante | Exemple | Contenu |
|---|---|---|
| `server.version` | `"0.1.0"` | version de Basalt |
| `server.game_version` | `"1.26.65"` | version du jeu annoncée par le serveur |
| `server.protocol` | `2225` | protocole annoncé par le serveur |
| `server.max_players` | `20` | nombre maximal de joueurs |
| `TextFormat.GREEN`, `RED`, `GOLD`… | `"§a"` | couleurs et styles du chat, comme dans PocketMine (liste dans [le guide du débutant](DEBUTER.md#5-parler-aux-joueurs-en-couleurs)) |
| `GameMode.SURVIVAL`, `CREATIVE`, `ADVENTURE`, `SPECTATOR` | `0` à `3` | modes de jeu |

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

**Données enregistrées.** Plus simple que `read` et `write` : `server.saveData(nom, valeur)`
garde une valeur (nombre, texte, booléen ou table, même imbriquée) dans `<nom>.data`, et
`server.loadData(nom, défaut)` la relit (ou renvoie `défaut` si rien n'a encore été
enregistré). Le nom ne contient que des lettres, des chiffres, `_` et `-`.

```lua
local scores = server.loadData("scores", {})
scores.Steve = (scores.Steve or 0) + 1
server.saveData("scores", scores)
```

**Réglages (`config.yml`).** `server.getConfig(défauts)` renvoie les réglages du fichier
`config.yml` du plugin, en le créant avec les valeurs par défaut s'il n'existe pas ; un
réglage ajouté plus tard dans le code est ajouté au fichier sans toucher aux autres. Forme
reconnue : `clé: valeur` par ligne (texte, nombre, `true`/`false`, liste `[a, b]`),
commentaires après `#`.

```lua
local config = server.getConfig({ message = "Bienvenue !", rayon = 30 })
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
  local cible = server.getPlayer(args[1])
  if not cible then
    server.warn(args[1] .. " n'est pas en jeu")
    return
  end
  local raison = table.concat(args, " ", 2)
  if raison == "" then raison = "Expulsé par un administrateur." end
  cible:kick(raison)
end)
```

`plugins/gardien/config.lua` :

```lua
return {
  bannis = { "Griefer123" },
  protocole_minimum = 2193,
}
```

## Générateurs de terrain

Un plugin peut fournir ses propres générateurs de monde : relief, biomes, décorations
(arbres, minerais, herbes…) et structures, ou retoucher le terrain du jeu.

### Mettre en place

Un générateur est un fichier `generators/<nom>.lua` dans le dossier d'un plugin complet,
c'est-à-dire avec sa fiche `plugin.yml` et son fichier principal, comme tout plugin :

```
plugins/
  MonPlugin/
    plugin.yml          (obligatoire)
    main.lua            (obligatoire, même presque vide)
    generators/
      iles.lua
```

Un dossier sans `plugin.yml` ni fichier principal n'est pas un plugin : son générateur est
ignoré (le serveur le signale), et les scripts d'installation et de mise à jour ne le
copient pas.

Dans `server.properties`, **pour un nouveau monde** (un autre `world-dir`) :

```
level-type=plugin:iles
```

Le type est enregistré avec le monde : il garde ce générateur aux démarrages suivants.
Comme pour le terrain du jeu, seuls les blocs modifiés par les joueurs sont enregistrés ;
le terrain est recalculé à partir de la graine. Corriger le générateur change donc le
terrain des zones que personne n'a touchées. Si le fichier manque ou ne se charge pas, le
serveur le signale et le terrain reste vide en attendant (rien n'est perdu).

Le plugin `Archipel`, livré avec Basalt (`plugins/Archipel/generators/archipel.lua`), est
un exemple complet et commenté : des îles, des palmiers et des phares.

### Le fichier

Il renvoie une table de fonctions ; seule `terrain` est obligatoire.

```lua
local STONE, GRASS = block("stone"), block("grass_block")

local gen = {}

-- Hauteur du sol : une fonction pure de (x, z), donc toujours le même monde
local function height(x, z)
  return 64 + math.floor(noise(x / 100, z / 100) * 20)
end
gen.height = height              -- facultatif : sert aux structures (voir plus bas)

function gen.terrain(chunk)      -- obligatoire : remplit le chunk
  for x = 0, 15 do
    for z = 0, 15 do
      local h = height(chunk.x * 16 + x, chunk.z * 16 + z)
      chunk:fill(x, -64, z, x, h - 1, z, STONE)
      chunk:set(x, h, z, GRASS)
    end
  end
  chunk:biome("plains")
end

function gen.decorate(chunk) end -- facultatif : appelée après terrain
gen.structures = { }             -- facultatif : voir plus bas
function gen.spawn() return 0, 80, 0 end -- facultatif : point d'apparition

return gen
```

Ordre pour chaque chunk : `terrain`, puis `decorate`, puis les structures.

### Outils disponibles

| Fonction | Rôle |
|---|---|
| `block(nom)` | identifiant d'un bloc : `block("oak_stairs[weirdo_direction=2]")` accepte les propriétés. À garder dans une variable en haut du fichier (plus rapide). Partout où un bloc est attendu, on peut aussi donner son nom. |
| `noise(x, z)`, `noise(x, y, z)` | bruit de Perlin de -1 à 1, propre à la graine du monde. Divisez les coordonnées pour des formes plus larges. |
| `seed` | la graine du monde (un nombre) |
| `math.random()` | hasard **déterministe** : le même pour un chunk donné (comme `chunk:random()`) |
| `vanillaHeight(x, z)` | hauteur du sol du terrain du jeu à cet endroit |
| `vanillaBiome(x, z)` | biome de surface du terrain du jeu (`"plains"`, `"desert"`…) |

### Le chunk

`chunk.x`, `chunk.z` : coordonnées du chunk (le bloc `x` du monde vaut `chunk.x * 16 + x`).
`chunk.minY` (-64) et `chunk.maxY` (319). Sauf `setAt`, les coordonnées `x` et `z` vont
de 0 à 15.

| Méthode | Rôle |
|---|---|
| `chunk:set(x, y, z, bloc)` | pose un bloc |
| `chunk:get(x, y, z)` | identifiant du bloc (à comparer avec `block(...)`) |
| `chunk:fill(x1, y1, z1, x2, y2, z2, bloc)` | remplit un pavé (bien plus rapide que des `set`) |
| `chunk:height(x, z)` | y du plus haut bloc qui n'est pas de l'air |
| `chunk:biome(nom)`, `chunk:biome(x, z, nom)` | biome du chunk, ou d'une colonne (noms du jeu : `"plains"`, `"cherry_grove"`…) |
| `chunk:random()`, `chunk:random(n)`, `chunk:random(a, b)` | hasard déterministe : de 0 à 1, de 1 à n, de a à b |
| `chunk:setAt(x, y, z, bloc)` | pose un bloc en coordonnées du monde ; ignoré hors du chunk (pour un arbre qui déborde un peu) |
| `chunk:vanilla()` | remplit le chunk avec le terrain du jeu (relief, grottes, arbres, villages…), à retoucher ensuite |

Avec `chunk:vanilla()`, un générateur devient un **décorateur** du terrain du jeu :

```lua
local GOLD = block("gold_ore")
return {
  terrain = function(chunk)
    chunk:vanilla()
    for i = 1, 4 do               -- 4 filons d'or en plus par chunk
      chunk:set(chunk:random(0, 15), chunk:random(-60, 30), chunk:random(0, 15), GOLD)
    end
  end,
}
```

### Structures

Une structure peut s'étendre sur plusieurs chunks. Elles sont placées sur une grille,
comme dans le jeu : une au plus par case de `spacing` chunks de côté, à `separation`
chunks au moins l'une de l'autre.

```lua
gen.structures = {
  {
    spacing = 16, separation = 4, -- une par case de 16 × 16 chunks
    salt = 1,                     -- un nombre propre à cette structure
    reach = 1,                    -- portée en chunks autour de son départ (taille ÷ 16)
    check = function(s)           -- facultatif : faux pour ne pas la construire ici
      return s:height(s.x, s.z) > 63
    end,
    build = function(s)           -- obligatoire
      local y = s:height(s.x, s.z)
      s:fill(s.x - 3, y + 1, s.z - 3, s.x + 3, y + 5, s.z + 3, "stone_bricks")
    end,
  },
}
```

`s.x`, `s.z` : centre du chunk de départ, en coordonnées du monde. Toutes les méthodes de
`s` emploient les coordonnées du monde :

| Méthode | Rôle |
|---|---|
| `s:set(x, y, z, bloc)`, `s:fill(x1, y1, z1, x2, y2, z2, bloc)` | pose des blocs |
| `s:get(x, y, z)` | bloc à cet endroit (seulement dans le chunk en cours, sinon `nil`) |
| `s:height(x, z)` | hauteur du sol n'importe où : appelle `height(x, z)` du générateur, à défaut le relief du jeu |
| `s:random(...)` | hasard propre à la structure : le même à chaque appel |

`build` est appelée **pour chaque chunk que la structure touche** ; seuls les blocs du
chunk en cours sont posés. Pour que les morceaux se raccordent, `build` doit faire
exactement la même chose à chaque appel : n'utiliser que `s:random`, `s:height` et le
bruit, jamais `chunk:height` ni `math.random`. Un `reach` trop petit coupe la structure ;
trop grand, il la fait recalculer pour rien.

### Limites

Un générateur tourne sur les fils de génération du terrain, en parallèle : chaque fil a sa
propre copie du fichier. Il n'a donc pas accès à `server`, aux joueurs, aux fichiers ni aux
variables d'un autre plugin ; seulement à son chunk et aux outils ci-dessus (`io`, `os`,
`require` et `load` n'existent pas). Les variables globales du fichier ne sont pas
partagées entre les fils : ne vous en servez pas pour compter ou mémoriser.

Chaque chunk dispose de 2 secondes au plus, et chaque copie de la mémoire réglée par
`plugin-memory-mb`. Une erreur est écrite une fois dans le journal ; le chunk concerné
reste vide. Côté vitesse, l'exemple `Archipel` (terrain, herbes, palmiers et phares)
prend environ 0,6 milliseconde par chunk, contre 3 pour le terrain du jeu : préférez
`fill` à de longues suites de `set`.

## Ce qui n'existe pas encore

L'API suit l'avancement du serveur. Ne sont pas encore accessibles aux plugins :

- la création de blocs et d'entités nouveaux (les objets, oui : voir « Créer des objets ») ;
- un événement de déplacement (`PlayerMoveEvent`), trop fréquent pour le fil des plugins ;
- le contenu des inventaires (seuls `joueur:give` et `joueur:getItemInHand` existent) ;
- les entités autres que les joueurs et les PNJ (animaux, monstres) ;
- les coffres remplis par un générateur (ses structures peuvent poser des coffres, vides) ;
- l'annulation d'une minuterie ;
- la communication directe entre plugins (en dehors des événements qu'ils écoutent tous).

Ces fonctions seront ajoutées à mesure que le serveur les prendra en charge ; ce
document sera mis à jour en même temps.
