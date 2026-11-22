# Console, commandes et couleurs

Les commandes du jeu se tapent aussi dans la console, sans `/` :

| Commande | Effet |
|---|---|
| `help` | liste des commandes, y compris celles des plugins |
| `list` | joueurs connectés |
| `say`, `me`, `tell` | messages |
| `gamemode`, `give`, `clear`, `kill`, `tp` | joueurs |
| `setblock`, `fill`, `time` | monde |
| `locate structure <nom>`, `locate biome <nom>` | structure ou biome le plus proche |
| `kick`, `op`, `deop`, `whitelist` | modération |
| `status` | charge du serveur : ticks par seconde, trafic, logins en attente |
| `timings on` / `timings off` | rapport détaillé du temps passé par le serveur (dossier `timings/`) |
| `plugins`, `reload` | plugins chargés, rechargement |
| `motd [texte]` | affiche ou change le nom du serveur |
| `version` | version du serveur et versions du jeu acceptées |
| `stop` | arrêt propre |

En jeu, les mêmes commandes sont réservées aux opérateurs (`ops.txt`), sauf `help`,
`list`, `me` et `tell`.

## Chat et couleurs

Les opérateurs écrivent en couleur, avec `&` suivi d'un code (ou `§`, plus difficile à
taper). Chez les autres joueurs, les `§` sont retirés et les `&` restent tels quels. Le
serveur ne filtre aucun mot, et les messages sont envoyés de façon à ce que le jeu ne masque
pas de mots par des `#`.

| Couleur | Opérateur dans le chat | Code | Plugin (`TextFormat.`) |
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

| Style | Opérateur dans le chat | Code | Plugin (`TextFormat.`) |
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
