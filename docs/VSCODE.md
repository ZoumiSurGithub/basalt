# Écrire ses plugins dans Visual Studio Code

Ce tutoriel met en place VS Code pour que, pendant que vous écrivez un plugin, l'éditeur :

- **propose** les fonctions de Basalt dès que vous tapez `server.` ou `joueur:` ;
- **affiche l'aide** de chaque fonction (ses paramètres, ce qu'elle renvoie) ;
- **souligne les fautes** : une fonction mal écrite, un texte donné là où il faut un nombre.

Comptez cinq minutes. Rien n'est à installer dans Basalt lui-même.

## 1. Installer Visual Studio Code

Téléchargez-le sur <https://code.visualstudio.com> et installez-le (gratuit, Windows, macOS
et Linux).

## 2. Installer l'extension Lua

1. Ouvrez VS Code.
2. Cliquez sur l'icône **Extensions** dans la barre de gauche (les quatre carrés), ou faites
   `Ctrl+Maj+X`.
3. Cherchez **Lua**.
4. Installez celle dont l'éditeur est **sumneko** (identifiant `sumneko.lua`). C'est la plus
   installée, elle est en tête de liste.

## 3. Ouvrir le dossier de Basalt

Dans VS Code : **Fichier → Ouvrir le dossier…**, et choisissez le dossier du serveur, celui
qui contient `plugins/`, `docs/` et `server.properties`.

Si VS Code demande « Faites-vous confiance aux auteurs des fichiers de ce dossier ? »,
répondez **Oui** : sans cela, les extensions restent désactivées.

C'est tout. Le dossier de Basalt contient déjà un fichier `.luarc.json` qui dit à
l'extension où se trouve la description de l'API (`docs/vscode/basalt-api.lua`).

## 4. Vérifier que ça marche

Créez un fichier `plugins/essai.lua` et tapez :

```lua
server.
```

Une liste doit s'ouvrir avec `broadcast`, `command`, `items`, `blocks`, `getPlayer`…
Choisissez-en une : l'aide de la fonction s'affiche à côté.

### Dans un écouteur d'événement

L'éditeur ne peut pas deviner ce qu'est `event` dans une fonction que vous écrivez.
Dites-le-lui par une ligne de commentaire au-dessus, et tout le reste suit :

```lua
local Accueil = {}

---@param event Event
function Accueil:onPlayerJoin(event)
  local joueur = event:getPlayer()
  joueur:
end

server.registerEvents(Accueil)
```

Après `event:`, la liste propose `getPlayer`, `cancel`, `getBlock`… ; après `joueur:`,
`sendMessage`, `teleport`, `give`, `getPosition`…

Sans la ligne `---@param event Event`, le plugin marche tout aussi bien : c'est seulement
l'éditeur qui ne propose rien pour `event`.

### Une faute volontaire

```lua
server.brodcast("Bonjour")
server.broadcast(42)
```

La première ligne est soulignée parce que `brodcast` n'existe pas (il manque un `a`), la
seconde parce que `broadcast` attend un texte et reçoit un nombre. Passez la souris sur le
mot souligné pour lire l'explication.

Les types utiles à écrire après `---@param` : `Event`, `Player`, `Block`, `Item`, `World`.

## Raccourcis utiles

| Geste | Effet |
|---|---|
| `Ctrl+Espace` | rouvrir la liste des propositions |
| souris sur un nom | afficher son aide |
| `Ctrl+Maj+Espace` (entre les parenthèses) | rappeler les paramètres attendus |
| `F12` sur un nom de fonction | aller à sa description dans `basalt-api.lua` |
| `Ctrl+Maj+M` | la liste de toutes les fautes signalées |

## Autres installations

### Vous n'ouvrez que le dossier d'un plugin

Si vous ouvrez `plugins/MonPlugin/` plutôt que tout le serveur, copiez
`docs/vscode/luarc.json` dans ce dossier et renommez-le **`.luarc.json`** (avec le point
devant). Son contenu indique le chemin de la description de l'API depuis le dossier du
plugin :

```json
{
  "runtime.version": "Lua 5.4",
  "workspace.library": ["../../docs/vscode/basalt-api.lua"],
  "workspace.checkThirdParty": false,
  "diagnostics.globals": ["server", "GameMode", "EventPriority", "TextFormat", "PlayerInteractEvent", "EntityDamageEvent"]
}
```

### Votre plugin est ailleurs sur l'ordinateur

Le principe est le même, avec le chemin complet du fichier dans `workspace.library`. Sous
Windows, écrivez les barres dans ce sens :

```json
"workspace.library": ["C:/Users/moi/Desktop/basalt/docs/vscode/basalt-api.lua"]
```

Vous pouvez aussi copier `basalt-api.lua` à côté de votre plugin et indiquer simplement
`["basalt-api.lua"]`. Pensez alors à le recopier quand vous mettez Basalt à jour.

## Si ça ne marche pas

**Aucune liste ne s'ouvre après `server.`**

- L'extension Lua est-elle bien installée et activée ? (Extensions → Lua → pas de bouton
  « Activer » visible.)
- Avez-vous répondu **Oui** à la question de confiance ? Sinon : `Ctrl+Maj+P`, tapez
  « Gérer la confiance de l'espace de travail », et accordez-la.
- Le fichier s'appelle-t-il bien `.luarc.json`, avec le point ? Sous Windows, l'explorateur
  cache parfois l'extension : un fichier nommé `.luarc.json.txt` n'est pas lu.
- Rechargez la fenêtre : `Ctrl+Maj+P`, puis « Recharger la fenêtre ».

**`server` est souligné : « Undefined global »**

L'extension ne trouve pas `basalt-api.lua`. Le chemin de `workspace.library` part du
dossier ouvert dans VS Code : vérifiez qu'il mène bien au fichier.

**Une fonction récente n'est pas proposée**

`basalt-api.lua` date de votre version de Basalt. Après une mise à jour du serveur, le
fichier est à jour lui aussi ; si vous en aviez fait une copie ailleurs, recopiez-le.

**Une fonction est proposée mais le serveur la refuse**

`basalt-api.lua` décrit l'API, il ne la contient pas : il ne s'exécute jamais. En cas
d'écart, c'est [PLUGINS.md](PLUGINS.md) qui fait foi — et signalez-le, c'est une erreur du
fichier.

## À savoir

- `basalt-api.lua` et `.luarc.json` ne servent qu'à l'éditeur. Le serveur ne les lit pas,
  et il est inutile de les copier dans vos plugins.
- L'éditeur vérifie l'écriture, pas le comportement : un plugin sans faute soulignée peut
  encore se tromper en jeu. La console du serveur reste le juge.
- Pour l'API elle-même, voir [PLUGINS.md](PLUGINS.md).
