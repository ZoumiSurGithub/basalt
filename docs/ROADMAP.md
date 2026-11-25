# Feuille de route

Ce que Basalt fait déjà et ce qu'il reste à faire, de la connexion jusqu'à une survie
complète. Une case cochée est écrite et testée ; le détail de chaque version est dans le
[journal des changements](../CHANGELOG.md).

## 1. Réseau et connexion

- [x] NetherNet (jeux 1.26.60 et plus)
- [x] Connexion Xbox vérifiée
- [x] Compression des paquets
- [x] Refus propre des clients trop anciens
- [x] Nom du serveur dans la liste des serveurs
- [x] Plusieurs versions du jeu en même temps (1.26.60 à la préversion 1.26.70)
- [x] Packs de ressources envoyés aux joueurs, chiffrés ou non (`resource_packs.yml`)
- [ ] Packs de comportement (addons) : non prévus, la logique se fait dans les plugins
- [ ] Transfert vers un autre serveur
- [ ] Query et RCON

## 2. Administration

- [x] Console, journaux, `status`, `timings`
- [x] Opérateurs, liste blanche, `kick`
- [x] Permissions
- [x] Installation et mise à jour par script (Linux, Windows)
- [x] API de gestion à distance : lecture (état, joueurs, objets, blocs) et actions (désactiver un objet ou un bloc, recharger les plugins, commande de console), deux jetons
- [x] Chiffrement des packs de ressources (commande, console, API)
- [x] Tableau de bord web : navigation, thèmes sombre et clair, gestion des joueurs, images des objets et blocs
- [x] Analyse de la carte : compte des objets, piles impossibles, conteneurs jumeaux
- [x] API de gestion : accès de l'extérieur par un tunnel chiffré (Tailscale, Cloudflare, SSH), adresses écartées après des mauvais jetons en série, renouvellement des jetons
- [x] Carte en direct dans le tableau de bord, avec changement d'un bloc et téléportation d'un clic
- [x] Carte en relief : terrain en cubes, boîtes et noms des joueurs et des créatures
- [ ] Carte : outils d'édition (sélection, remplir, sphère, copier-coller, structures enregistrées)
- [ ] Site de Basalt à héberger (présentation, téléchargements, documentation, gestion des serveurs)
- [ ] Gardien : gérer le serveur quand le jeu est arrêté
- [ ] API de gestion : droits plus fins, plusieurs serveurs
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
- [x] Cultures qui poussent et se récoltent, houe, baies
- [x] Gravité du sable et du gravier
- [x] Blocs qui tiennent sur un support (fleurs, torches, rails…), feuilles qui dépérissent
- [x] Herbes marines et algues : elles gardent leur eau, qui s'écoule comme une autre
- [x] Explosions (creeper, boule de feu du ghast)
- [x] Feu qui se propage, lave qui brûle, TNT
- [ ] Redstone, pistons, entonnoirs
- [x] Temps de minage selon l'outil, fissures, bruits et miettes
- [x] Herbe qui s'étend, pousses qui deviennent des arbres
- [ ] Glace et neige qui fondent
- [ ] Générateurs d'épreuves, coffres-forts, sable et gravier suspects

## 5. Joueur

- [x] Déplacements vérifiés par le serveur
- [x] Survie, créatif, aventure, spectateur
- [x] Vie, dégâts de chute, mort et réapparition
- [x] Skins et affichage des autres joueurs
- [x] Chat, `/msg`, `/me`, couleurs
- [x] Faim et nourriture
- [x] Expérience et niveaux, orbes
- [x] Dormir pour passer la nuit, point de réapparition au lit
- [x] Effets de potion

## 6. Objets et inventaires

- [x] Inventaire, objets au sol, `/give`, `/clear`
- [x] Fabrication et cuisson, vérifiées par le serveur
- [x] Coffres des structures avec leur butin
- [x] Seaux, cisailles, laisses, selles
- [x] Durabilité des outils, variantes d'objets (couleurs)
- [x] Armures qui protègent (4 % par point d'armure) et s'usent
- [x] Armure visible sur les autres joueurs ; armure naturelle des zombies
- [x] Enchantements sur les objets, `/enchant`, premiers effets (protection, tranchant, solidité, puissance…)
- [ ] Table d'enchantement, enclume, livres enchantés, effets restants (fortune, toucher de soie, butin, recul…)
- [x] Arc
- [ ] Arbalète, trident, bouclier
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
- [x] Premiers monstres : zombie et araignée, qui poursuivent et frappent (`/summon`)
- [x] Monstres la nuit : apparition en surface autour des joueurs, disparition au loin, zombies qui brûlent au soleil
- [x] Autres monstres : squelette, vagabond, creeper, momie, noyé, zombie-villageois, araignée venimeuse, poisson d'argent, wither squelette, vindicateur, pillard
- [x] Projectiles (flèches, boules de neige, boules de feu) et explosions, ouverts aux plugins
- [x] Les créatures se poussent
- [x] Enderman (neutre, téléportation) et slime (trois tailles, division)
- [x] Arc pour les joueurs
- [x] Poison de l'araignée venimeuse
- [x] Créatures volantes : blaze, ghast, phantom
- [x] Phantoms pour les joueurs qui ne dorment pas ; joueur en feu ; boule de feu renvoyée
- [x] Potions (à boire, à lancer) et sorcière ; aliments à effets
- [ ] Alambic (préparer les potions), nuage des potions persistantes, effets sur les créatures
- [x] Feu sur les blocs et les créatures
- [x] Effets des joueurs : `/effect`, plugins, affichage chez le joueur
- [ ] Ralentissement de la flèche du vagabond
- [x] Apparition selon la lumière (grottes, torches qui protègent)
- [ ] Créatures aquatiques
- [ ] Boss : Ender Dragon, Wither, Warden
- [ ] Raids de pillards
- [ ] Bateaux et wagonnets

## 8. Combat

- [x] Joueur contre joueur, recul
- [x] Loups qui défendent leur maître, golems qui défendent le village
- [ ] Coups critiques et temps de recharge
- [x] Projectiles (flèches, boules de feu, potions)
- [ ] Messages de mort complets

## 9. Commandes

- [x] `/tp` `/gamemode` `/give` `/kill` `/time` `/summon`
- [x] `/setblock` `/fill` `/locate` `/list` `/say`
- [x] `/effect`
- [ ] `/enchant` `/weather` `/xp`
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
- [x] Modifier les objets et les blocs du jeu (butin, dégâts, durabilité, chute…)
- [x] Objets créés : aliments, armes, menu créatif et groupes
- [x] Blocs créés : images par face, formes, boîte, outil exigé, orientation, croissance
- [x] Créatures créées : valeurs, boîte, apparence du pack, hostiles, événements
- [x] Description de l'API pour Visual Studio Code
- [ ] Créatures créées qui apparaissent d'elles-mêmes, attaques à distance

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
