# Packs de ressources

Un pack de ressources apporte aux joueurs des images, des sons et des textes que le jeu
n'a pas : l'image d'un objet créé par un plugin, par exemple. Basalt envoie aux joueurs,
à leur connexion, les packs rangés dans le dossier `resource_packs/`.

## Mettre un pack en place

Comme sur PocketMine, tout se règle dans `resource_packs/resource_packs.yml`. Le dossier
et le fichier sont créés au premier démarrage du serveur.

1. Déposez le pack dans `resource_packs/`, en fichier `.mcpack` ou `.zip`.
2. Ajoutez son nom à la liste `resource_stack` de `resource_packs.yml`.
3. Redémarrez le serveur. La console annonce chaque pack chargé :

```
Pack de ressources : Mon pack 1.0.0 (48 Ko)
```

Le joueur voit à sa connexion l'écran habituel du jeu, qui lui propose de télécharger le
pack. Le jeu garde le pack en mémoire : il n'est téléchargé qu'une fois par version.

## `resource_packs.yml`

```yaml
# true : un joueur qui refuse les packs ne peut pas entrer.
# false : il peut jouer sans (les images propres au serveur lui manqueront).
force_resources: false

# Les packs à envoyer, dans l'ordre.
resource_stack:
  - monpack.mcpack
  - sons.zip
```

| Réglage | Effet |
|---|---|
| `force_resources` | `true` : les packs sont obligatoires pour entrer. `false` (par défaut) : le joueur peut les refuser et jouer sans. |
| `resource_stack` | les fichiers à envoyer. L'ordre compte : si deux packs fournissent la même image, celui qui est le plus haut dans la liste l'emporte. |

À savoir :

- Un fichier déposé dans le dossier mais **absent de la liste n'est pas envoyé** ; la
  console le rappelle au démarrage.
- Un fichier listé mais introuvable ou illisible est signalé et laissé de côté ; les
  autres packs sont envoyés quand même.
- Un pack doit être un **fichier** : un dossier non compressé est refusé.
  `manifest.json` peut se trouver à la racine de l'archive ou dans un dossier à
  l'intérieur.
- Si `resource_packs.yml` est illisible (faute d'indentation, par exemple), aucun pack
  n'est envoyé et la console le dit.

## Pack chiffré

Un pack chiffré se pose de la même façon, avec sa **clé** (32 caractères) dans un fichier
texte du même nom, suivi de `.key` :

```
resource_packs/
  monpack.mcpack
  monpack.mcpack.key
```

(`monpack.key` est aussi accepté.) Le pack s'ajoute à `resource_stack` comme un autre ; la
clé ne s'écrit pas dans `resource_packs.yml`. La console confirme :

```
Pack de ressources : Mon pack 1.0.0 (48 Ko, chiffré)
```

Le serveur ne déchiffre rien : il envoie le pack tel quel et donne la clé au jeu du
joueur, qui s'en sert pour le lire. Sans le fichier `.key`, ou avec une mauvaise clé, le
jeu ne peut pas lire le pack et ses images n'apparaissent pas.

Basalt ne chiffre pas lui-même les packs : utilisez pour cela l'outil de votre choix, qui
vous donnera la clé.

## Donner une image à un objet créé par un plugin

Trois fichiers suffisent.

`manifest.json` — remplacez les deux identifiants par les vôtres (générateur d'UUID) :

```json
{
  "format_version": 2,
  "header": {
    "name": "Mon serveur",
    "description": "Images du serveur",
    "uuid": "0fba4063-dba1-4281-9b89-ff9390653530",
    "version": [1, 0, 0],
    "min_engine_version": [1, 20, 0]
  },
  "modules": [
    { "type": "resources", "uuid": "7f4bb4f1-52a5-4b8e-9f0b-0a8d5a3c6e11", "version": [1, 0, 0] }
  ]
}
```

`textures/item_texture.json` — donne un nom à chaque image :

```json
{
  "resource_pack_name": "monserveur",
  "texture_name": "atlas.items",
  "texture_data": {
    "rubis": { "textures": "textures/items/rubis" }
  }
}
```

`textures/items/rubis.png` — l'image, en 16 × 16 pixels.

Compressez ces fichiers en `.zip` (ou `.mcpack`), déposez l'archive dans
`resource_packs/`, ajoutez-la à `resource_stack`, puis utilisez le nom dans le plugin :

```lua
server.items.create("monserveur:rubis", { name = "Rubis", icon = "rubis" })
```

Quand vous modifiez le pack, **augmentez sa version** dans `manifest.json` : c'est ce qui
dit au jeu de le télécharger de nouveau.

## Pack d'exemple

`docs/exemples/pack-exemple.mcpack` contient deux images d'objets (`exemple_rubis`,
`exemple_lame`), celles des blocs (minerai, bloc de rubis, tronc, fourneau, trois stades d'une culture),
et une créature complète (modèle, image et définition du « Gardien de rubis »)
et `docs/exemples/rubis_pack.lua` les objets qui s'en servent. Pour essayer : copiez le
premier dans `resource_packs/`, ajoutez `- pack-exemple.mcpack` à `resource_stack`, copiez
le second dans `plugins/`, et redémarrez.

## Limites

- 256 Mo par pack.
- Les packs sont lus au démarrage : un pack ajouté ou changé demande de redémarrer.
- Les packs de comportement (scripts, entités) ne sont pas gérés : la logique se fait dans
  les plugins.
- Les packs de la liste sont proposés à tous les joueurs.
