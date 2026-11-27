# Console, commandes et couleurs

Les commandes du jeu se tapent aussi dans la console, sans `/` :

| Commande | Effet |
|---|---|
| `help` | liste des commandes, y compris celles des plugins |
| `list` | joueurs connectés |
| `say`, `me`, `tell` | messages |
| `gamemode`, `give`, `clear`, `kill`, `tp` | joueurs |
| `encryptpack <fichier>` | chiffre un pack de `resource_packs/` (console seulement ; voir [PACKS.md](PACKS.md)) |
| `apitoken reset` | crée deux nouveaux jetons pour l'API de gestion ; les anciens cessent de fonctionner (console seulement ; voir [API.md](API.md)) |
| `give <joueur> <objet> [quantité] [variante]` | la variante est un numéro ; pour une potion, son nom convient aussi : `give Alex splash_potion 1 poison` |
| `effect <joueur> <effet> [secondes] [niveau] [true]`, `effect <joueur> clear [effet]` | effets d'un joueur (30 s et niveau 0 par défaut ; le niveau commence à 0, comme dans le jeu ; `true` : sans particules) |
| `enchant <joueur> <enchantement> [niveau]` | enchante l'objet qu'il tient en main (niveau 1 par défaut, 0 pour retirer ; cinq enchantements au plus par objet). Les règles du jeu s'appliquent : niveau maximal, objet qui accepte l'enchantement (`sharpness` sur une épée ou une hache, pas sur une pioche), et enchantements qui s'excluent (`fortune` et `silk_touch`, `sharpness` et `smite`, les quatre protections, `mending` et `infinity`). Un objet créé par un plugin accepte tout. Noms du jeu : `sharpness`, `protection`, `unbreaking`, `efficiency`, `power`… |
| `setblock`, `fill`, `time` | monde |
| `weather clear\|rain\|thunder [secondes]`, `weather query` | la météo : beau temps, pluie, orage (sans durée : une durée naturelle) |
| `gamerule`, `gamerule dodaylightcycle\|doweathercycle [true\|false]` | règles du monde où l'on se trouve, enregistrées avec lui : l'heure avance, la météo change d'elle-même |
| `debug-crash` | console seulement : provoque une erreur interne volontaire, pour essayer le rapport de plantage (`crashdumps/`). Le serveur enregistre tout puis s'arrête |
| `world list`, `world load <nom> [vanilla\|flat\|void\|nether\|end] [graine]`, `world unload <nom>`, `world tp <nom> [joueur]` | plusieurs mondes ouverts à la fois : liste, ouverture (ou création) d'un monde de `worlds/`, fermeture, envoi d'un joueur |
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

## Potions

Trois objets : `potion` (on la boit, il reste la fiole vide), `splash_potion` (on la lance :
elle touche les joueurs à moins de quatre blocs de son point de chute, d'autant plus
longtemps qu'ils sont près) et `lingering_potion` (lancée aussi ; son nuage persistant
n'existe pas encore, elle agit comme une potion jetable plus brève).

Leur contenu est la variante de l'objet, par son nom ou son numéro :

| Nom | Effet | Durée |
|---|---|---|
| `night_vision` (5), `long_` (6) | vision nocturne | 3 min, 8 min |
| `invisibility` (7), `long_` (8) | invisibilité (affichage seulement) | 3 min, 8 min |
| `leaping` (9), `long_` (10), `strong_` (11) | saut | 3 min, 8 min, 1 min 30 (II) |
| `fire_resistance` (12), `long_` (13) | résistance au feu | 3 min, 8 min |
| `swiftness` (14), `long_` (15), `strong_` (16) | vitesse | 3 min, 8 min, 1 min 30 (II) |
| `slowness` (17), `long_` (18), `strong_` (42) | lenteur | 1 min 30, 4 min, 20 s (IV) |
| `water_breathing` (19), `long_` (20) | respiration aquatique (affichage seulement) | 3 min, 8 min |
| `healing` (21), `strong_` (22) | soin immédiat | 2 cœurs, 4 cœurs |
| `harming` (23), `strong_` (24) | dégâts immédiats | 3 cœurs, 6 cœurs |
| `poison` (25), `long_` (26), `strong_` (27) | poison | 45 s, 2 min, 22 s (II) |
| `regeneration` (28), `long_` (29), `strong_` (30) | régénération | 45 s, 2 min, 22 s (II) |
| `strength` (31), `long_` (32), `strong_` (33) | force | 3 min, 8 min, 1 min 30 (II) |
| `weakness` (34), `long_` (35) | faiblesse | 1 min 30, 4 min |
| `wither` (36) | flétrissement II | 40 s |
| `turtle_master` (37), `long_` (38), `strong_` (39) | lenteur et résistance | 20 s, 40 s, 20 s |
| `slow_falling` (40), `long_` (41) | chute lente | 1 min 30, 4 min |

`long_` et `strong_` se placent devant le nom : `long_poison`, `strong_healing`. Les
variantes 0 à 4 (eau, potions banale, épaisse, étrange) ne font rien.

Certains aliments ont aussi leurs effets, comme dans le jeu : la pomme dorée (régénération
et absorption), la chair putréfiée et le poulet cru (faim, parfois), l'œil d'araignée et la
pomme de terre empoisonnée (poison), le poisson-globe (faim, poison, nausée).

## Rapports de plantage

Si le serveur rencontre une erreur interne, il écrit un fichier `crashdumps/crash-<date>.txt` :
l'erreur, l'endroit du code, la pile d'appels, la version, les mondes ouverts, les joueurs,
les plugins et les 80 dernières lignes du journal. C'est le fichier à joindre pour signaler
un problème (relisez-le avant : il peut montrer des pseudos et des adresses IP).

Une erreur déclenchée par le paquet d'un joueur ne ferme que ce joueur : le rapport est écrit
et le serveur continue. Une erreur dans la boucle principale arrête le serveur, après un
enregistrement de secours des joueurs et des mondes.

