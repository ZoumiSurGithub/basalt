# Développement

Organisation du code, tests et publication des versions. Les gabarits d'objets et de blocs
(où se trouve chaque règle, comment en ajouter une) ont leur page :
[ARCHITECTURE.md](ARCHITECTURE.md).

## Organisation du code

| Dossier ou fichier | Contenu |
|---|---|
| `src/main.rs` | démarrage : configuration, signaux, console |
| `src/server.rs` | `Server` : boucle principale, joueurs connectés, console |
| `src/player/` | `Player` : un joueur, sa sauvegarde et son chargement (hérite de `Human`) |
| `src/entity/` | `Entity` → `Living` → `Human` : la hiérarchie des entités ; physique des objets au sol (`item_physics.rs`) |
| `src/inventory/` | `Inventory` (base commune), `BaseInventory`, puis `PlayerInventory`, `ArmorInventory`, `ChestInventory`, `FurnaceInventory`, `CraftingGrid` ; `PlayerWindows` (tous les inventaires d'un joueur) ; `transaction.rs` (demandes du jeu) |
| `src/block/` | `Block` (base de tous les blocs), types de blocs (`types.rs`), comportements dans le monde (`behavior.rs` : chute, appui, cultures, feuilles), registre `VanillaBlocks` couvrant tous les blocs du jeu |
| `src/tile/` | `Tile` (base des données attachées aux blocs), `ContainerTile`, `FurnaceTile`, `SignTile`, `TileMap` et sa sauvegarde |
| `src/item/` | `Item` (base de tous les objets), `Tool`, `Armor`, `Food` (`consumable.rs`), usure (`durable.rs`), `VanillaItems` ; `ItemStack` (une pile) |
| `src/world/` | `World` : le monde, ses blocs modifiés et leur sauvegarde ; `vanilla.rs` : terrain à la manière du jeu ; `structures.rs` : villages, temples, mines, avant-postes, cités antiques, ruines de piste, chambres des épreuves, butin, `/locate` ; `liquid.rs` : écoulement de l'eau et de la lave ; `generator.rs` : terrain plat, premier terrain, choix du générateur |
| `src/permission/` | `Permission`, règles par défaut, liste des opérateurs |
| `src/utils/` | journal (`log.rs`), `server.properties` (`config.rs`), JSON et base64 (`json.rs`), YAML (`yaml.rs`), compression et décompression (`deflate.rs`, `inflate.rs`) |
| `src/security/` | authentification Xbox (`auth.rs`), SHA-2 et signatures (`crypto.rs`), clé du serveur (`identity.rs`) |
| `src/network/` | NetherNet : signalisation HTTP (`signaling.rs`), liaison WebRTC (`nethernet.rs`, sur son propre fil : `nethernet_thread.rs`), côté joueur pour les outils (`nethernet_client.rs`) ; transport de test (`transport.rs`) ; fragments (`framing.rs`) |
| `src/network/protocol/versions.rs` | versions acceptées et différences entre protocoles |
| `src/network/protocol/bedrock/` | paquets Bedrock, un fichier par thème : `codec`, `login`, `spawn`, `chunk`, `player`, `state`, `chat`, `blocks`, `items` |
| `src/data/` | chargement des données du jeu : objets, blocs et états, recettes (`recipes.rs`), NBT, gzip |
| `data/` | fichiers de données du jeu (projet BedrockData, licence CC0) ; `data/<protocole>/packets/` : paquets relevés auprès du serveur officiel par l'outil de capture |
| `src/plugin/` | fiche `plugin.yml` (`manifest.rs`), moteur Lua et API (`lua.rs`), préambule et bac à sable (`sandbox.lua`), fil d'exécution dédié (`host.rs`), générateurs de terrain en Lua (`generator.rs`) |
| `examples/stress.rs` | outil de charge : simule des centaines ou milliers de joueurs |
| `tests/e2e.rs` | faux client : connexion, login, entrée en jeu, combat, objets, fabrication, coffres, plugins, terrain, `/locate`, générateur d'un plugin, PNJ |
| `plugins/EasyEdit/` | plugin fourni : édition du monde en volume, exemple de plugin en plusieurs fichiers |
| `plugins/Archipel/` | plugin fourni en exemple, avec un générateur de terrain : îles, palmiers, phares (`level-type=plugin:archipel`) |

Tests : `cargo test`

## Publier une version

Les changements sont notés au fil de l'eau dans [CHANGELOG.md](../CHANGELOG.md), section
« Non publié » (catégories Ajouté, Modifié, Corrigé, Retiré). Pour publier, une seule
commande, une fois tout enregistré (commit) :

```sh
./release.sh 0.0.7        # sous Windows : depuis « Git Bash »
./release.sh -n 0.0.7     # pour voir ce qui serait fait, sans rien toucher
```

Le script vérifie d'abord que tout est prêt (rien en attente d'enregistrement, branche à jour
avec GitHub, numéro pas encore utilisé, journal rempli), montre les notes, demande
confirmation, puis date la section « Non publié » sous ce numéro, inscrit le numéro dans
`Cargo.toml`, crée le commit « Version 0.0.7 » et l'étiquette `v0.0.7`, et envoie les deux.
Un numéro avec un tiret (`0.1.0-beta.1`) donne une préversion.

**Le numéro vient de l'étiquette.** À la compilation, GitHub inscrit dans le programme le
numéro de l'étiquette (`v0.0.7` → `0.0.7`) et vérifie que `basalt --version` l'annonce bien :
le programme publié porte toujours le numéro de sa page de version, même si `Cargo.toml` a
été oublié (une étiquette posée à la main reste donc possible ; les notes sont alors prises
dans « Non publié », avec un avertissement). La branche `dev` et les autres branches donnent
le numéro de `Cargo.toml` suivi de `-dev.<commit>` (par exemple `0.0.6-dev.ab12cd3`), et
l'étiquette `dev-latest` suit le dernier envoi. Une compilation locale prend le numéro de
`Cargo.toml`.

Si la compilation d'une version échoue : corrigez, enregistrez, et publiez le numéro suivant.

Tests sur GitHub : à chaque envoi sur une branche, les tests rapides (sans connexion) ;
pour une étiquette `v…`, tous les tests, y compris ceux de bout en bout avec de vrais
joueurs simulés. Les tests et les deux compilations (Linux, Windows) tournent en même
temps ; rien n'est publié, ni pour Linux ni pour Windows, si un test échoue.

GitHub compile Basalt pour Linux et Windows et publie les archives ; `get.sh` et
`update.sh` installent toujours la dernière. Chaque envoi sur la branche `dev` publie la
préversion `dev-latest` (`./update.sh -v dev`).

## Ajouter une version du jeu

Les versions acceptées sont décrites dans `src/network/protocol/versions.rs`. Quand une
version est ajoutée, mettez aussi à jour le tableau et les deux badges « Minecraft Bedrock »
et « protocole » en tête du `README.md`, ainsi que le tableau de `docs/RESEAU.md` : ils sont
écrits à la main.
