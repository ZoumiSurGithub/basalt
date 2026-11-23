# Architecture : les gabarits

Basalt range ce qu'est un objet, un bloc ou une entité dans des **gabarits**, à la manière
de PocketMine : un fichier par idée (`Item`, `Durable`, `Food`, `Tool`…), et un serveur qui
pose des questions aux gabarits au lieu de connaître les choses par leur nom.

Cette page dit où se trouve chaque règle, comment en ajouter une, et ce qui est ouvert
aux plugins aujourd'hui.

## Le principe

Le serveur ne contient pas de phrase du genre « si c'est du pain, rendre 5 de faim ». Il
demande à l'objet tenu : *te manges-tu ?* L'objet répond par ses valeurs, et le serveur
les applique.

```
joueur qui mange ──► server/survival.rs ──► item.food() ──► item/consumable.rs
                     (applique la règle)    (la question)   (la réponse)
```

Changer ce que fait un aliment, c'est donc toucher à `consumable.rs`, jamais au serveur.
Il en va de même pour l'usure, la gravité, les cultures.

## Correspondance avec PocketMine

| PocketMine | Basalt | Fichier |
|---|---|---|
| `Item` | trait `Item` + `BaseItem` | `src/item/item.rs` |
| `Durable` | `max_durability()`, `wear()`, `apply_wear` | `src/item/durable.rs` |
| `Food`, `ConsumableItem` | `Food`, `FoodValues` | `src/item/consumable.rs` |
| `Tool`, `TieredTool` | `Tool`, `ToolType`, `ToolTier` | `src/item/tool.rs` |
| `Armor` | `Armor` | `src/item/armor.rs` |
| `ItemBlock` | `BlockItem` | `src/item/item.rs` |
| `VanillaItems` | `VanillaItems` | `src/item/vanilla_items.rs` |
| `Item` (la pile : nombre, méta, dégâts) | `ItemStack` | `src/item/stack.rs` |
| `Block` | trait `Block` + `BaseBlock` | `src/block/block.rs` |
| `Fallable`, `Flowable`, `Crops`, `Leaves`, `Wood` | `falls`, `needs_floor`, `is_crop`, `is_leaves`, `is_log`, `two_halves` | `src/block/behavior.rs` |
| `Transparent`, `Liquid`, `Door`… | `Air`, `Liquid`, `Fragile`, `Crop`, `Interactive`, `WithTile`… | `src/block/types.rs` |
| `VanillaBlocks` | `VanillaBlocks` | `src/block/vanilla_blocks.rs` |
| `Tile`, `Chest`, `Furnace`, `Sign` | `Tile`, `ContainerTile`, `FurnaceTile`, `SignTile`, `BedTile` | `src/tile/` |
| `Entity` → `Living` → `Human` → `Player` | même chaîne | `src/entity/`, `src/player/` |

Une différence à connaître : dans PocketMine, l'objet et la pile sont une seule classe.
Dans Basalt, `Item` décrit la *sorte* d'objet (une pioche en fer) et `ItemStack` la *pile*
qu'un joueur tient (combien, quelle variante, quelle usure). Deux pioches en fer sont le
même `Item`, usées différemment.

## Les objets

### `Item` : la base

Tout objet implémente le trait `Item`. Il n'a qu'une chose à fournir, sa fiche
(`base()`, un `BaseItem` : nom et taille de pile). Tout le reste a une réponse par
défaut, qu'un gabarit redéfinit quand il a quelque chose à dire :

| Méthode | Par défaut | Redéfinie par |
|---|---|---|
| `max_stack_size()` | celle de la fiche | — |
| `max_durability()` | 0 (ne s'use pas) | `Tool`, `Armor` |
| `wear(usage)` | 0 | `Tool` |
| `tool_type()`, `tool_tier()`, `mining_efficiency()` | la main | `Tool` |
| `attack_damage()` | 1 | `Tool` |
| `armor_points()` | 0 | `Armor` |
| `food()` | `None` | `Food` |
| `block()` | `None` | `BlockItem` |

### `Durable` : ce qui s'use

`src/item/durable.rs`. Un objet est durable dès que `max_durability()` est positif.
`wear(usage)` dit ce que coûte un usage : `Usage::Mine` (un bloc cassé) ou
`Usage::Strike` (un coup porté). Une pioche répond 1 et 2, une épée 2 et 1.

Le serveur appelle `apply_wear(pile, objet, usage)`, qui ajoute l'usure à la pile et la
vide si l'objet casse.

### `Food` : ce qui se mange

`src/item/consumable.rs`. `food()` renvoie des `FoodValues` :

| Champ | Sens |
|---|---|
| `hunger` | points de faim rendus |
| `saturation` | satiété rendue |
| `always_edible` | se mange le ventre plein (pomme dorée) |
| `eat_ticks` | durée du repas (32 ; 16 pour l'algue séchée) |
| `leftover` | ce qui reste en main (`bowl` pour un ragoût) |

### Ajouter un aliment

Une ligne dans `Food::from_name`, dans `src/item/consumable.rs` :

```rust
"cooked_rabbit" => (5, 6.0),
"mon_gateau" => (7, 8.4),   // 7 de faim, 8,4 de satiété
```

### Donner un comportement propre à un objet

On écrit une structure qui porte un `BaseItem`, et on ne redéfinit que ce qui diffère.
Exemple, une épée en or qui frappe très fort mais casse au premier coup :

```rust
// src/item/golden_sword.rs
use super::durable::Usage;
use super::item::{BaseItem, Item};

pub struct FragileSword {
    pub base: BaseItem,
}

impl Item for FragileSword {
    fn base(&self) -> &BaseItem {
        &self.base
    }
    fn max_durability(&self) -> u32 {
        1
    }
    fn wear(&self, usage: Usage) -> u16 {
        if usage == Usage::Strike { 1 } else { 0 }
    }
    fn attack_damage(&self) -> f32 {
        12.0
    }
}
```

Puis on la fait reconnaître dans `VanillaItems::plain` (`src/item/vanilla_items.rs`), en
tête, avant que l'épée ne soit reconnue comme un outil ordinaire :

```rust
if short == "golden_sword" {
    return Box::new(FragileSword { base: BaseItem::new(short, 1) });
}
```

et on déclare le fichier dans `src/item/mod.rs` (`mod golden_sword;`). Le serveur n'a pas
à changer : il usera et cassera l'épée tout seul.

À savoir : le jeu des joueurs ne connaît que les objets qu'on lui déclare. Un gabarit
écrit ainsi en Rust vaut pour changer un objet existant, comme ici l'épée en or. Pour un objet au
nom nouveau, passez par `item/custom.rs` (voir « Comment un objet créé atteint le jeu »),
qui le déclare aux joueurs.

## Les blocs

### `Block` : la base

Même principe : le trait `Block` (`src/block/block.rs`), une fiche `BaseBlock` (nom,
dureté, outil, niveau d'outil exigé, propriétés physiques), et des réponses par défaut :
`hardness()`, `tool_type()`, `required_tier()`, `is_breakable()`, `blast_resistance()`,
`friction()`, `is_flammable()`, `is_solid()`, `is_transparent()`, `is_replaceable()`,
`light_level()`, `interaction()`, `tile()`, `can_harvest(outil)`, `drops(outil)`,
`break_time(outil)`.

Les familles de blocs sont dans `src/block/types.rs` : `GenericBlock` (ordinaire), `Air`,
`Liquid`, `DropsOther` (rend autre chose que lui-même), `Fragile` (ne rend rien sans le
bon outil), `Cobweb`, `Crop`, `Interactive` (porte, levier, coffre), `WithTile` (porte des
données). `VanillaBlocks::with_props` choisit la famille d'après le nom.

### Comportements dans le monde

`src/block/behavior.rs` répond aux questions que pose le monde :

| Méthode de `Block` | Question | Exemples |
|---|---|---|
| `falls()` | tombe sans appui ? | sable, gravier, enclume, béton en poudre |
| `needs_floor()` | lui faut-il un bloc dessous ? | fleurs, rails, tapis, portes, cultures |
| `two_halves()` | deux moitiés qui partent ensemble ? | portes, tournesol, lilas |
| `is_log()` | nourrit les feuilles ? | troncs et bois |
| `is_leaves()` | dépérit sans tronc ? | feuillages |
| `is_crop()` | culture sur terre labourée ? | blé, carottes, pommes de terre, betteraves |

`src/server/nature.rs` applique les règles (chute, perte d'appui, feuilles qui
dépérissent, croissance) sans aucune liste de blocs.

### Faire tomber un bloc de plus

Une entrée dans `falls`, dans `src/block/behavior.rs` :

```rust
matches!(n, "sand" | "red_sand" | "gravel" | … | "mon_bloc")
```

### Ce qui n'est pas encore rangé

Trois règles de blocs dépendent de l'*état* du bloc (son orientation, son stade) et non
de son seul nom ; elles sont restées dans `src/server/nature.rs` :

- `support_of` : de quel côté une torche, une échelle ou une lanterne s'appuie ;
- `random_tick_at` : la croissance, stade par stade ;
- `crop_loot` : la récolte selon la maturité.

Elles rejoindront le trait `Block` à l'étape suivante, quand il recevra l'état en plus
du nom.

## Les entités

`Entity` → `Living` → `Human` → `Player` (`src/entity/`, `src/player/`) suivent déjà la
chaîne de PocketMine. Les animaux et villageois sont décrits par `Species` dans
`src/server/mobs.rs` (taille, vie, vitesse, butin, nourriture d'élevage) : c'est une
table, pas encore un gabarit par espèce. À ranger de la même manière.

## Et les plugins ?

| | État |
|---|---|
| Réagir à ce qui arrive et l'annuler (manger, casser, tomber, pousser…) | **disponible** : événements |
| Modifier un objet ou un bloc existant (ce que nourrit un aliment, durabilité, dégâts, butin, chute, appui, incassable) | **disponible** : `server.items.modify`, `server.blocks.modify` |
| Créer un objet nouveau, rangé dans le menu créatif | **disponible** : `server.items.create` |
| Lui donner sa propre image | **disponible** : pack de ressources, voir [PACKS.md](PACKS.md) |
| Créer un bloc ou une entité | **plus tard** : pour les blocs, il faut renuméroter toute la palette |

Le mode d'emploi est dans [PLUGINS.md](PLUGINS.md), « Modifier objets et blocs ».

### Comment une modification atteint le jeu

```
plugin : server.items.modify("minecraft:bread", { food = 20 })
   │
   ▼
item/overrides.rs      range la modification, par nom d'objet
   │
   ▼
VanillaItems::get      enveloppe le gabarit dans `Modified`
   │
   ▼
server/survival.rs     demande item.food(), reçoit 20 — sans rien savoir du plugin
```

`Modified` répond à la place du gabarit pour ce qui a été changé et lui laisse le reste.
Les blocs suivent le même chemin (`block/overrides.rs`, `VanillaBlocks::with_props`) ;
`falls` et `needs_floor` de `behavior.rs` regardent d'abord la modification.

C'est pour cela que tout doit passer par les gabarits : une règle écrite en dur dans le
serveur échapperait aux plugins.

### Comment un objet créé atteint le jeu

```
plugin : server.items.create("monserveur:rubis", { icon = "emerald", maxStack = 16 })
   │
   ▼
item/custom.rs         CustomDef : la fiche ; CustomItem : le gabarit (implémente `Item`)
   │
   ├──► VanillaItems::get("monserveur:rubis")   le serveur l'interroge comme un autre
   │
   └──► CustomDef::components()                 les composants, au format du jeu
           │
           ▼
        liste des objets envoyée à la connexion (celle du serveur officiel + les nôtres)
```

Les composants sont calqués sur ceux que le serveur officiel envoie pour ses propres
objets à composants : la pomme pour un aliment, la lance pour un objet qui s'use. Ajouter
un réglage, c'est l'ajouter à `CustomDef` (lecture dans `parse`, écriture dans
`components`), le faire répondre dans `impl Item for CustomItem`, puis à `NEW_ITEM_KEYS`
dans `src/plugin/sandbox.lua`.

### Ouvrir une valeur de plus aux plugins

1. Ajouter le champ à `ItemOverride` (ou `BlockOverride`) et sa lecture dans `set`.
2. Le faire répondre dans `impl Item for Modified` (ou `impl Block`).
3. L'ajouter à `ITEM_KEYS` (ou `BLOCK_KEYS`) dans `src/plugin/sandbox.lua`, avec son type.
4. Le documenter dans `docs/PLUGINS.md` et `docs/vscode/basalt-api.lua`.

Ne sont ouvertes que les valeurs dont le serveur décide seul. La dureté ou la taille de
pile sont calculées par le jeu du joueur : les changer ici ferait dire à son écran autre
chose que le serveur.

## Ce qui manque

- L'état du bloc dans le trait `Block`, pour y ranger appuis, croissance et récoltes.
- Un gabarit par espèce d'entité.
- La création de blocs et d'entités.
