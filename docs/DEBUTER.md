# Créer un plugin sans savoir programmer

Ce guide part de zéro. En une demi-heure, vous saurez écrire des plugins qui accueillent les
joueurs, ajoutent des commandes, protègent une zone ou comptent des points. Tous les exemples
ont été vérifiés sur le serveur.

Pour tout le détail ensuite : [la référence complète des plugins](PLUGINS.md).

- [1. Ce qu'il vous faut](#1-ce-quil-vous-faut)
- [2. Votre premier plugin](#2-votre-premier-plugin)
- [3. Lire et écrire du Lua : l'essentiel](#3-lire-et-écrire-du-lua--lessentiel)
- [4. Réagir à ce qui se passe](#4-réagir-à-ce-qui-se-passe)
- [5. Parler aux joueurs, en couleurs](#5-parler-aux-joueurs-en-couleurs)
- [6. Ajouter une commande](#6-ajouter-une-commande)
- [7. Plus tard, ou régulièrement](#7-plus-tard-ou-régulièrement)
- [8. Se souvenir des choses](#8-se-souvenir-des-choses)
- [9. Des réglages pour l'administrateur](#9-des-réglages-pour-ladministrateur)
- [10. Ce qu'on peut faire d'un joueur](#10-ce-quon-peut-faire-dun-joueur)
- [11. Les blocs du monde](#11-les-blocs-du-monde)
- [12. Des plugins prêts à l'emploi](#12-des-plugins-prêts-à-lemploi)
- [13. Quand ça ne marche pas](#13-quand-ça-ne-marche-pas)

## 1. Ce qu'il vous faut

- Un éditeur de texte : le Bloc-notes suffit. Notepad++ ou VS Code colorent le code et
  aident à repérer les fautes.
- Le dossier `plugins` du serveur, à côté de `basalt` (ou `basalt.exe`).

Un plugin est un simple fichier texte qui se termine par `.lua`.

## 2. Votre premier plugin

1. Dans le dossier `plugins`, créez un fichier `bonjour.lua`.
2. Écrivez dedans :

   ```lua
   server.on("join", function(joueur)
     joueur:sendMessage("Bonjour " .. joueur:getName() .. " !")
   end)
   ```

3. Enregistrez, puis tapez `reload` dans la console du serveur (ou redémarrez-le).
   La console affiche `Plugin chargé : bonjour`.
4. Connectez-vous : le serveur vous dit bonjour.

Ce que ce plugin dit, ligne par ligne :

- `server.on("join", …)` : « quand un joueur arrive dans le monde, fais ceci » ;
- `function(joueur) … end` : ce qu'il faut faire, avec le joueur concerné ;
- `joueur:sendMessage(…)` : envoyer un message à ce joueur ;
- `"Bonjour " .. joueur:getName() .. " !"` : un texte fait de trois morceaux collés par `..`.

## 3. Lire et écrire du Lua : l'essentiel

Six notions suffisent pour la plupart des plugins.

```lua
-- Un commentaire : tout ce qui suit deux tirets est ignoré. Notez-y vos explications.

-- Une variable garde une valeur. « local » la réserve à ce fichier.
local nom = "Steve"
local vies = 3
local actif = true            -- true (vrai) ou false (faux)

-- Coller des textes : deux points
local phrase = "Salut " .. nom    -- "Salut Steve"

-- Une condition
if vies > 0 then
  server.log("Encore en vie")
elseif vies == 0 then         -- deux signes = pour comparer
  server.log("Plus de vie")
else
  server.log("Bizarre")
end

-- Une fonction : un bloc d'actions réutilisable
local function saluer(qui)
  server.log("Bonjour " .. qui)
end
saluer("Alex")

-- Une table : une liste, ou des valeurs rangées sous un nom
local couleurs = { "rouge", "vert", "bleu" }      -- couleurs[1] vaut "rouge"
local scores = { Steve = 10, Alex = 7 }            -- scores.Steve vaut 10
scores.Lea = 3                                     -- ajoute Léa

-- Parcourir une table
for nom, score in pairs(scores) do
  server.log(nom .. " : " .. score)
end
```

À retenir :

- chaque `function`, `if` et `for` se termine par `end` ;
- les textes s'écrivent entre guillemets `"…"` ;
- `..` colle des textes, `+` additionne des nombres ;
- `==` compare, `=` range une valeur ; « différent » s'écrit `~=` ;
- `and`, `or` et `not` combinent des conditions : `if joueur:isOp() and vies > 0 then`.

## 4. Réagir à ce qui se passe

`server.on(événement, fonction)` appelle votre fonction quand l'événement arrive :

| Événement | Votre fonction reçoit | Quand |
|---|---|---|
| `join` | `joueur` | un joueur arrive dans le monde |
| `quit` | `joueur` | un joueur part |
| `load` | `joueur`, `nouveau` | ses données sont chargées ; `nouveau` vaut `true` à sa toute première visite |
| `chat` | `joueur`, `message` | il écrit dans le chat |
| `block_break` | `joueur`, `x`, `y`, `z`, `bloc` | il casse un bloc |
| `block_click` | `joueur`, `x`, `y`, `z`, `bloc` | il clique sur un bloc (pour poser, ouvrir…) |
| `death` | `joueur` | il meurt |
| `respawn` | `joueur` | il réapparaît |
| `enable` | — | le serveur a fini de charger les plugins |
| `disable` | — | le serveur s'arrête (ou `reload`) |

**Empêcher une action :** pour `block_break` et `block_click`, répondez `false` :

```lua
server.on("block_break", function(joueur, x, y, z, bloc)
  if bloc == "minecraft:diamond_ore" and not joueur:isOp() then
    joueur:sendMessage("Pas touche aux diamants !")
    return false
  end
end)
```

Un nom d'événement mal écrit (`"joint"` au lieu de `"join"`) est signalé dans la console,
avec le bon nom proposé.

## 5. Parler aux joueurs, en couleurs

```lua
joueur:sendMessage("Rien que pour vous")
server.broadcast("Pour tout le monde")
server.log("Dans la console seulement")
```

Les couleurs se mettent devant le texte avec `TextFormat` :

```lua
server.broadcast(TextFormat.GOLD .. "[Annonce] " .. TextFormat.WHITE .. "Fête au spawn à 18 h !")
```

Toutes les couleurs et tous les styles du jeu :

| Couleur | Tapé dans le chat | Code | Plugin (`TextFormat.`) |
|---|---|---|---|
| noir | `&0` | `§0` | `BLACK` |
| bleu foncé | `&1` | `§1` | `DARK_BLUE` |
| vert foncé | `&2` | `§2` | `DARK_GREEN` |
| turquoise foncé | `&3` | `§3` | `DARK_AQUA` |
| rouge foncé | `&4` | `§4` | `DARK_RED` |
| violet | `&5` | `§5` | `DARK_PURPLE` |
| or | `&6` | `§6` | `GOLD` |
| gris | `&7` | `§7` | `GRAY` |
| gris foncé | `&8` | `§8` | `DARK_GRAY` |
| bleu | `&9` | `§9` | `BLUE` |
| vert | `&a` | `§a` | `GREEN` |
| turquoise | `&b` | `§b` | `AQUA` |
| rouge | `&c` | `§c` | `RED` |
| rose | `&d` | `§d` | `LIGHT_PURPLE` |
| jaune | `&e` | `§e` | `YELLOW` |
| blanc | `&f` | `§f` | `WHITE` |
| or Minecoin | `&g` | `§g` | `MINECOIN_GOLD` |
| quartz | `&h` | `§h` | `MATERIAL_QUARTZ` |
| fer | `&i` | `§i` | `MATERIAL_IRON` |
| netherite | `&j` | `§j` | `MATERIAL_NETHERITE` |
| redstone | `&m` | `§m` | `MATERIAL_REDSTONE` |
| cuivre | `&n` | `§n` | `MATERIAL_COPPER` |
| or (matériau) | `&p` | `§p` | `MATERIAL_GOLD` |
| émeraude | `&q` | `§q` | `MATERIAL_EMERALD` |
| diamant | `&s` | `§s` | `MATERIAL_DIAMOND` |
| lapis-lazuli | `&t` | `§t` | `MATERIAL_LAPIS` |
| améthyste | `&u` | `§u` | `MATERIAL_AMETHYST` |
| résine | `&v` | `§v` | `MATERIAL_RESIN` |

| Style | Tapé dans le chat | Code | Plugin (`TextFormat.`) |
|---|---|---|---|
| texte brouillé (lettres qui changent) | `&k` | `§k` | `OBFUSCATED` |
| gras | `&l` | `§l` | `BOLD` |
| italique | `&o` | `§o` | `ITALIC` |
| retour au texte normal | `&r` | `§r` | `RESET` |

- Un code s'applique à tout ce qui suit, jusqu'au code suivant : `&aBonjour &ctoi` donne
  « Bonjour » en vert et « toi » en rouge.
- Mettez la couleur avant le style : `&c&lAttention` (rouge gras).
- Bedrock n'a ni souligné ni barré : `&m` et `&n` y sont des couleurs (redstone et cuivre).
- Les couleurs de matériaux (`&h` à `&v`) n'existent que sur Bedrock ; leurs teintes ont
  changé selon les versions du jeu.
- Dans le chat, seuls les opérateurs (`ops.txt`) peuvent utiliser ces codes, et seulement en
  minuscules (`R&D` reste tel quel). Les plugins peuvent les utiliser partout : messages,
  `setNameTag`, `setDisplayName`, nom du serveur.

Les codes peuvent aussi s'écrire directement dans le texte : `"§aVert §cRouge"`.

## 6. Ajouter une commande

```lua
server.command("soin", function(args, joueur)
  if not joueur then
    server.log("À taper en jeu.")       -- tapée dans la console : pas de joueur
    return
  end
  joueur:setHealth(joueur:getMaxHealth())
  joueur:sendMessage(TextFormat.GREEN .. "Vous voilà soigné.")
end)
```

Les joueurs tapent `/soin` dans le chat ; la console accepte `soin`. Les mots tapés après le
nom arrivent dans `args` : `/donner Steve 5` donne `args[1] = "Steve"` et `args[2] = "5"`.

```lua
server.command("donner", function(args, joueur)
  if not joueur or not joueur:isOp() then return end
  local cible = server.getPlayer(args[1] or "")       -- les majuscules ne comptent pas
  local nombre = tonumber(args[2]) or 1                -- tonumber : du texte au nombre
  if not cible then
    joueur:sendMessage(TextFormat.RED .. "Joueur introuvable.")
    return
  end
  cible:give("minecraft:diamond", nombre)
end)
```

Pour réserver une commande aux opérateurs, testez `joueur:isOp()` comme ci-dessus. Les
permissions plus fines et l'aide qui s'affiche pendant la frappe se règlent dans un fichier
`plugin.yml` (voir [la référence](PLUGINS.md#la-fiche-du-plugin--pluginyml)).

## 7. Plus tard, ou régulièrement

```lua
-- Une fois, dans 5 secondes
server.afterSeconds(5, function()
  server.broadcast("5 secondes plus tard…")
end)

-- Toutes les 10 minutes
local annonce = server.everySeconds(600, function()
  server.broadcast("Pensez à voter pour le serveur !")
end)

-- Et pour l'arrêter :
annonce:stop()
```

Les secondes peuvent avoir des décimales : `server.everySeconds(0.5, …)` toutes les demi-
secondes.

## 8. Se souvenir des choses

Les variables sont oubliées quand le serveur redémarre. Pour garder quelque chose, utilisez
`server.saveData` et `server.loadData` :

```lua
-- Au démarrage : les scores enregistrés, ou une table vide la première fois
local scores = server.loadData("scores", {})

server.on("block_break", function(joueur)
  local nom = joueur:getName()
  scores[nom] = (scores[nom] or 0) + 1      -- « or 0 » : 0 si le joueur n'a pas encore de score
end)

-- Enregistrer de temps en temps, et à l'arrêt
server.everySeconds(60, function() server.saveData("scores", scores) end)
server.on("disable", function() server.saveData("scores", scores) end)
```

On peut enregistrer des nombres, des textes, `true`/`false` et des tables (même des tables
dans des tables). Les fichiers sont rangés dans `plugins/<votre plugin>/`.

## 9. Des réglages pour l'administrateur

Plutôt que d'écrire un message ou un nombre dans le code, laissez l'administrateur le
choisir dans un fichier `config.yml` :

```lua
local config = server.getConfig({
  message = "Bienvenue !",
  rayon = 30,
  kit = true,
})

server.on("join", function(joueur)
  joueur:sendMessage(config.message)
end)
```

Au premier démarrage, le plugin crée `plugins/<votre plugin>/config.yml` :

```yaml
# Réglages du plugin : modifiez les valeurs, puis tapez « reload » dans la console.
kit: true
message: "Bienvenue !"
rayon: 30
```

L'administrateur modifie ce fichier et tape `reload`. Un réglage ajouté plus tard dans le code
est ajouté au fichier, sans effacer les autres. Une liste s'écrit entre crochets :
`annonces: ["Bonjour", "Bonne partie"]`.

## 10. Ce qu'on peut faire d'un joueur

| Pour… | Écrire |
|---|---|
| son pseudo | `joueur:getName()` |
| lui écrire | `joueur:sendMessage("texte")` |
| savoir s'il est opérateur | `joueur:isOp()` |
| sa position (aux pieds) | `local x, y, z = joueur:getPosition()` |
| le téléporter (aux pieds) | `joueur:teleport(x, y, z)` |
| lui donner des objets | `joueur:give("minecraft:diamond", 3)` |
| l'objet qu'il tient | `joueur:getItemInHand()` |
| sa vie / la changer | `joueur:getHealth()` / `joueur:setHealth(20)` |
| sa faim / la changer | `joueur:getFood()` / `joueur:setFood(20)` |
| son niveau d'expérience | `joueur:getXpLevel()` / `joueur:setXpLevel(10)` |
| son mode de jeu | `joueur:setGamemode(GameMode.CREATIVE)` (`SURVIVAL`, `ADVENTURE`, `SPECTATOR`) |
| le nom au-dessus de sa tête | `joueur:setNameTag("[VIP] Steve")` |
| l'expulser | `joueur:kick("raison")` |

Trouver un joueur par son pseudo : `server.getPlayer("steve")` (ou `nil` s'il n'est pas là).
Tous les joueurs en jeu : `server.getOnlinePlayers()`.

```lua
for _, j in ipairs(server.getOnlinePlayers()) do
  j:setFood(20)
end
```

## 11. Les blocs du monde

```lua
local monde = server.getWorld()

local bloc = monde:getBlock(10, 64, 20)
server.log(bloc:getName())                     -- "minecraft:grass_block"

monde:setBlock(10, 65, 20, "minecraft:gold_block")
```

Les noms des blocs et des objets sont ceux du jeu (`minecraft:stone`, `minecraft:oak_planks`,
`minecraft:diamond`…) ; `minecraft:` peut être omis.

## 12. Des plugins prêts à l'emploi

Le dossier [`docs/exemples`](exemples) contient des plugins complets et commentés. Copiez-en un
dans `plugins`, tapez `reload`, puis lisez-le et modifiez-le :

| Fichier | Ce qu'il fait |
|---|---|
| [`bienvenue.lua`](exemples/bienvenue.lua) | message de bienvenue, annonce des arrivées et départs, kit pour les nouveaux |
| [`spawn.lua`](exemples/spawn.lua) | `/setspawn` (opérateurs) et `/spawn` |
| [`annonces.lua`](exemples/annonces.lua) | annonces automatiques, réglables dans `config.yml` |
| [`protection.lua`](exemples/protection.lua) | personne ne construit près du centre, sauf les opérateurs |
| [`points.lua`](exemples/points.lua) | un point par bloc cassé, `/points` et le meilleur joueur |

## 13. Quand ça ne marche pas

Regardez la console : une erreur dit **quel fichier** et **quelle ligne**.

```
[ERREUR] [bonjour] plugins/bonjour.lua:3: ')' expected near 'joueur'
```

Les fautes les plus courantes :

| Message | Cause probable |
|---|---|
| `'end' expected` | il manque un `end` après un `function`, un `if` ou un `for` |
| `')' expected` | une parenthèse ou une virgule manque, ou un `..` entre deux textes |
| `unfinished string` | un guillemet n'est pas refermé |
| `attempt to concatenate a nil value` | un `..` colle une valeur vide : variable mal écrite ou joueur introuvable |
| `attempt to index a nil value` | on utilise `:` sur quelque chose de vide, souvent un joueur introuvable (`server.getPlayer` a renvoyé `nil`) |
| `attempt to perform arithmetic on a string value` | un calcul sur du texte : utilisez `tonumber(…)` |
| `événement inconnu « … »` | le nom de l'événement est mal écrit (le bon nom est proposé) |

Après chaque modification, enregistrez le fichier et tapez `reload`. Une erreur dans un
plugin n'arrête jamais le serveur ni les autres plugins.

Vous voulez aller plus loin (écouteurs à la PocketMine, permissions, PNJ, générateurs de
terrain) ? Tout est dans [la référence complète](PLUGINS.md).
