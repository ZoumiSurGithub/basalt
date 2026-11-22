# Feuille de route

Ce que Basalt fait déjà et ce qu'il reste à faire, de la connexion jusqu'à une survie
complète. Une case cochée est écrite et testée ; le détail de chaque version est dans le
[journal des changements](../CHANGELOG.md).

## 1. Réseau et connexion

- [x] NetherNet (jeux 1.26.60 et plus)
- [x] RakNet et son chiffrement (ECDH, AES-256)
- [x] Connexion Xbox vérifiée
- [x] Compression des paquets
- [x] Refus propre des clients trop anciens
- [x] Nom du serveur dans la liste des serveurs
- [x] Plusieurs versions du jeu en même temps (1.26.60 à la préversion 1.26.70)
- [ ] Packs de ressources envoyés aux joueurs
- [ ] Transfert vers un autre serveur
- [ ] Query et RCON

## 2. Administration

- [x] Console, journaux, `status`, `timings`
- [x] Opérateurs, liste blanche, `kick`
- [x] Permissions
- [x] Installation et mise à jour par script (Linux, Windows)
- [ ] Bannissements (joueur, adresse)
- [ ] Sauvegardes planifiées
- [ ] Règles du jeu (`/gamerule`) et difficulté

## 3. Monde

- [x] Terrain à la manière du jeu : biomes, relief, grottes, minerais
- [x] Structures : villages, temples, mines, épaves, manoirs, cités antiques…
- [x] Enregistrement des blocs modifiés et des créatures
- [x] Cycle jour et nuit
- [x] Générateurs de terrain écrits en Lua
- [ ] Météo (pluie, orage, neige)
- [ ] Nether et End, portails
- [ ] Mondes multiples
- [ ] Chargement de vrais mondes Minecraft (LevelDB)
- [ ] Monuments océaniques, bastions, forteresses
- [ ] Lacs, aquifères, filons géants de fer et de cuivre

## 4. Blocs et physique

- [x] Pose et casse, orientation des blocs
- [x] Portes, trappes, portillons, leviers, boutons
- [x] Coffres, fours, établi, lits colorés, panneaux
- [x] Cloches
- [x] Eau et lave : écoulement, sources, obsidienne, seaux
- [ ] Cultures qui poussent
- [ ] Gravité du sable et du gravier
- [ ] Feu qui se propage, explosions, TNT
- [ ] Redstone, pistons, entonnoirs
- [ ] Temps de minage selon l'outil
- [ ] Ticks aléatoires (herbe, feuilles, glace)
- [ ] Générateurs d'épreuves, coffres-forts, sable et gravier suspects

## 5. Joueur

- [x] Déplacements vérifiés par le serveur
- [x] Survie, créatif, aventure, spectateur
- [x] Vie, dégâts de chute, mort et réapparition
- [x] Skins et affichage des autres joueurs
- [x] Chat, `/msg`, `/me`, couleurs
- [ ] Faim
- [ ] Expérience et niveaux
- [ ] Effets de potion
- [ ] Dormir pour passer la nuit, point de réapparition au lit

## 6. Objets et inventaires

- [x] Inventaire, objets au sol, `/give`, `/clear`
- [x] Fabrication et cuisson, vérifiées par le serveur
- [x] Coffres des structures avec leur butin
- [x] Seaux, cisailles, laisses, selles
- [ ] Armures qui protègent
- [ ] Durabilité des outils
- [ ] Enchantements et enclume
- [ ] Alchimie
- [ ] Arc, arbalète, trident, bouclier
- [ ] Pêche
- [ ] Ateliers : tailleur de pierre, métier à tisser, table de forgeron

## 7. Créatures

- [x] Animaux de ferme : élevage, croissance, tonte, traite
- [x] Montures : chevaux, ânes, mules, lamas, cochons
- [x] Loups et chats à apprivoiser
- [x] Villageois : métiers et postes de travail, portes, emploi du temps, lits, naissances
- [x] Échanges avec les villageois : offres par métier, niveaux, réassort
- [x] Golems de fer : gardiens des villages, à bâtir et à réparer
- [x] `/summon`
- [ ] Échanges avancés : objets enchantés, cartes au trésor, prix qui varient
- [ ] Fermiers qui récoltent, marchand ambulant
- [ ] Monstres : zombies, squelettes, creepers, araignées…
- [ ] Apparition selon la lumière et la nuit
- [ ] Créatures aquatiques et volantes
- [ ] Boss : Ender Dragon, Wither, Warden
- [ ] Raids de pillards
- [ ] Bateaux et wagonnets

## 8. Combat

- [x] Joueur contre joueur, recul
- [x] Loups qui défendent leur maître, golems qui défendent le village
- [ ] Coups critiques et temps de recharge
- [ ] Projectiles
- [ ] Messages de mort complets

## 9. Commandes

- [x] `/tp` `/gamemode` `/give` `/kill` `/time` `/summon`
- [x] `/setblock` `/fill` `/locate` `/list` `/say`
- [ ] `/effect` `/enchant` `/weather` `/xp`
- [ ] `/scoreboard` `/title` `/bossbar`
- [ ] Sélecteurs (`@a`, `@p`, `@e`…) complets
- [ ] Blocs de commande

## 10. Plugins (Lua)

- [x] Commandes et événements
- [x] Bac à sable et fil d'exécution dédié
- [x] Personnages non joueurs qui marchent seuls
- [x] Générateurs de terrain
- [x] Minuteries, données enregistrées, `config.yml`
- [x] Plugins fournis : EasyEdit, Archipel
- [ ] Inventaires et coffres personnalisés
- [ ] Formulaires (menus à boutons)
- [ ] Scoreboards, titres, barres de boss
- [ ] Créatures et objets personnalisés

## 11. Performances

- [x] Créatures simulées seulement près des joueurs
- [x] Terrain généré et compressé sur plusieurs cœurs
- [x] Outil de charge (joueurs simulés)
- [x] Mesures intégrées (`timings`)
- [ ] Mesure à 1000 joueurs sur une vraie machine, avec un vrai gameplay
- [ ] Distance de vue adaptée à la charge

## 12. Sécurité

- [x] Liaison chiffrée et identité vérifiée
- [x] Limites de débit et de taille par joueur
- [x] Paquets malformés, bombes de décompression, inondations
- [x] Contrôle des déplacements, fabrication et échanges vérifiés
- [ ] Anti-triche : portée des coups, vitesse de minage

## 13. Qualité et versions

- [x] Tests automatiques et faux client de bout en bout
- [x] Journal des changements et versions publiées sur GitHub
- [ ] Image Docker
- [ ] Site de documentation
- [ ] Version 1.0 : une survie complète, jouable de bout en bout
