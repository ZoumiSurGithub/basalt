---@meta
-- Définitions de l'API des plugins Basalt pour l'extension « Lua » de VS Code
-- (sumneko.lua). Ce fichier ne s'exécute pas : il décrit les fonctions, pour que l'éditeur
-- propose les noms, affiche l'aide et signale les fautes. Voir docs/PLUGINS.md, qui fait foi.

---@class Timer
local Timer = {}
--- Arrête la minuterie.
function Timer:stop() end
---@return boolean
function Timer:isRunning() end

---@class Block
local Block = {}
---@return string nom complet, par exemple "minecraft:stone"
function Block:getName() end
---@return integer état du bloc (avec toutes ses propriétés)
function Block:getState() end
---@return integer
function Block:getX() end
---@return integer
function Block:getY() end
---@return integer
function Block:getZ() end
---@return integer x, integer y, integer z
function Block:getPosition() end

---@class Item
---@field name string nom complet de l'objet
---@field count integer quantité

---@class World
local World = {}
---@param x integer
---@param y integer
---@param z integer
---@return Block
function World:getBlock(x, y, z) end
---@return integer état du bloc
function World:getBlockState(x, y, z) end
--- États de tout un volume (1 000 000 de blocs au plus), rangés par y, puis x, puis z.
---@return integer[]
function World:getBlocks(x1, y1, z1, x2, y2, z2) end
---@param bloc string|integer|Block un nom, un état ou un bloc
function World:setBlock(x, y, z, bloc) end
--- Pose beaucoup de blocs : { x, y, z, état, x, y, z, état, … }
---@param t integer[]
function World:setBlocks(t) end

---@class Player
---@field name string pseudo du compte
local Player = {}
---@return string
function Player:getName() end
---@return string
function Player:getDisplayName() end
---@param texte string
function Player:setDisplayName(texte) end
---@return string
function Player:getNameTag() end
---@param texte string
function Player:setNameTag(texte) end
---@return integer mode de jeu (GameMode.SURVIVAL, CREATIVE, ADVENTURE, SPECTATOR)
function Player:getGamemode() end
---@param mode integer
function Player:setGamemode(mode) end
---@return number
function Player:getHealth() end
---@param n number à 0, le joueur meurt
function Player:setHealth(n) end
---@return number
function Player:getMaxHealth() end
---@param n number
function Player:setMaxHealth(n) end
---@return number faim, de 0 à 20
function Player:getFood() end
---@param n number
function Player:setFood(n) end
---@return integer
function Player:getXpLevel() end
---@param n integer
function Player:setXpLevel(n) end
---@return number progression vers le niveau suivant, de 0 à 1
function Player:getXpProgress() end
---@param n number
function Player:setXpProgress(n) end
--- Position des pieds.
---@return number x, number y, number z
function Player:getPosition() end
---@param x number
---@param y number
---@param z number
function Player:teleport(x, y, z) end
---@return integer première visite, en secondes
function Player:getFirstPlayed() end
---@return boolean
function Player:isOnline() end
---@return boolean
function Player:isOp() end
---@param permission string
---@return boolean
function Player:hasPermission(permission) end
---@param texte string
function Player:sendMessage(texte) end
--- Prend effet quand la fonction en cours se termine.
---@param raison? string
function Player:kick(raison) end
function Player:save() end
---@param objet string nom de l'objet
---@param quantite? integer
function Player:give(objet, quantite) end
---@return string nom de l'objet tenu ("" si la main est vide)
function Player:getItemInHand() end
---@return number
function Player:getYaw() end
---@return number
function Player:getPitch() end
--- Direction regardée parmi les six.
---@return integer dx, integer dy, integer dz
function Player:getDirection() end

---@class Event
local Event = {}
---@return string
function Event:getEventName() end
---@return boolean
function Event:isCancelled() end
function Event:cancel() end
function Event:uncancel() end
---@param annule boolean
function Event:setCancelled(annule) end
---@return boolean
function Event:isCancellable() end
---@return Player
function Event:getPlayer() end
---@return Player
function Event:getEntity() end
---@return Block
function Event:getBlock() end
---@return Block
function Event:getBed() end
---@return Item
function Event:getItem() end
---@return Item[]
function Event:getDrops() end
---@param butin { name: string, count: integer }[]
function Event:setDrops(butin) end
---@return string
function Event:getMessage() end
---@param texte string
function Event:setMessage(texte) end
---@return string
function Event:getJoinMessage() end
---@param texte string "" : aucun message
function Event:setJoinMessage(texte) end
---@return string
function Event:getQuitMessage() end
---@param texte string
function Event:setQuitMessage(texte) end
---@return string
function Event:getQuitReason() end
---@param texte string
function Event:setKickMessage(texte) end
---@return string
function Event:getCommand() end
---@param texte string
function Event:setCommand(texte) end
---@return integer
function Event:getAction() end
---@return string
function Event:getCause() end
---@return number
function Event:getBaseDamage() end
---@param n number
function Event:setBaseDamage(n) end
---@return number
function Event:getDamage() end
---@return Player?
function Event:getDamager() end
---@return string
function Event:getDeathMessage() end
---@param texte string
function Event:setDeathMessage(texte) end
---@return boolean
function Event:getKeepInventory() end
---@param garder boolean
function Event:setKeepInventory(garder) end
---@return number x, number y, number z
function Event:getRespawnPosition() end
function Event:setRespawnPosition(x, y, z) end
---@return integer
function Event:getNewGamemode() end
---@return number
function Event:getAmount() end
---@param n number
function Event:setAmount(n) end

---@alias EventName
---| "PlayerLoginEvent" | "PlayerJoinEvent" | "PlayerQuitEvent" | "PlayerChatEvent"
---| "PlayerCommandPreprocessEvent" | "ServerCommandEvent" | "BlockBreakEvent" | "BlockPlaceEvent"
---| "PlayerInteractEvent" | "EntityDamageEvent" | "EntityDamageByEntityEvent" | "PlayerDeathEvent"
---| "PlayerRespawnEvent" | "PlayerDropItemEvent" | "EntityItemPickupEvent" | "PlayerGameModeChangeEvent"
---| "PlayerItemUseEvent" | "PlayerItemConsumeEvent" | "PlayerExhaustEvent" | "PlayerExperienceChangeEvent"
---| "PlayerBedEnterEvent" | "PlayerBedLeaveEvent" | "LeavesDecayEvent" | "BlockFallEvent" | "BlockGrowEvent"

---@alias SimpleEvent
---| "enable" | "disable" | "connect" | "login" | "load" | "join" | "chat" | "save"
---| "block_break" | "block_click" | "death" | "respawn" | "quit" | "disconnect"

---@class ItemChanges
---@field food? number points de faim rendus (0 à 20)
---@field saturation? number satiété rendue (0 à 20)
---@field leftover? string objet qui reste en main après le repas ("" : rien)
---@field durability? number utilisations avant de casser (0 : incassable)
---@field attackDamage? number dégâts d'un coup

---@class BlockChanges
---@field drops? table butin : { ["minecraft:diamond"] = 2 } ou { "minecraft:stick", { "minecraft:flint", 3 } }
---@field falls? boolean tombe quand rien ne le porte
---@field needsFloor? boolean se détache si le bloc du dessous disparaît
---@field breakable? boolean peut être cassé en survie

---@class NewItem
---@field name? string nom affiché au joueur
---@field icon? string image : celle d'un objet du jeu ("emerald", "diamond_spear"…) ou d'un pack de ressources
---@field maxStack? number taille de pile (1 à 64)
---@field glint? boolean brille comme un objet enchanté
---@field category? "construction"|"nature"|"equipment"|"items" onglet du menu créatif
---@field group? string groupe du menu créatif (case qui se déplie)
---@field creative? boolean présent dans le menu créatif (vrai par défaut)
---@field food? number points de faim rendus : l'objet se mange
---@field saturation? number satiété rendue
---@field alwaysEdible? boolean se mange le ventre plein
---@field leftover? string objet qui reste en main après le repas
---@field durability? number utilisations avant de casser
---@field attackDamage? number dégâts d'un coup
---@field handEquipped? boolean tenu en main comme un outil

---@class ItemModifier
local ItemModifier = {}
--- Crée un objet nouveau, déclaré aux joueurs à leur connexion. À appeler au chargement du plugin.
---@param nom string "espace:objet", par exemple "monserveur:rubis"
---@param reglages? NewItem
function ItemModifier.create(nom, reglages) end
--- Change ce que vaut un objet du jeu.
---@param nom string nom complet, par exemple "minecraft:bread"
---@param valeurs ItemChanges
function ItemModifier.modify(nom, valeurs) end
--- Rend l'objet à la règle du jeu.
---@param nom string
function ItemModifier.reset(nom) end

---@class BlockModifier
local BlockModifier = {}
--- Change ce que répond un bloc du jeu.
---@param nom string nom complet, par exemple "minecraft:gravel"
---@param valeurs BlockChanges
function BlockModifier.modify(nom, valeurs) end
--- Rend le bloc à la règle du jeu.
---@param nom string
function BlockModifier.reset(nom) end

---@class Server
---@field items ItemModifier
---@field blocks BlockModifier
---@field version string version de Basalt
---@field game_version string version du jeu annoncée
---@field protocol integer protocole annoncé
---@field max_players integer
server = {}

--- S'abonner à un événement simple.
---@param evenement SimpleEvent
---@param fonction function
function server.on(evenement, fonction) end
--- Enregistrer un écouteur à la manière de PocketMine : une table de méthodes
--- `onBlockBreak(event)`, `onPlayerJoin(event)`… avec, au besoin, `priorities` et `handleCancelled`.
---@param ecouteur table
function server.registerEvents(ecouteur) end
---@param evenement EventName
---@param fonction fun(event: Event)
---@param priorite? string EventPriority.LOWEST … MONITOR
---@param handleCancelled? boolean
function server.registerEvent(evenement, fonction, priorite, handleCancelled) end
--- Déclarer une commande (console et jeu). La fonction reçoit les arguments puis le joueur
--- (nil depuis la console).
---@param nom string
---@param description string|fun(args: string[], joueur: Player?)
---@param fonction? fun(args: string[], joueur: Player?)
function server.command(nom, description, fonction) end
---@param texte string
function server.log(texte) end
---@param texte string
function server.warn(texte) end
---@param texte string
function server.broadcast(texte) end
---@param pseudo string
---@return Player?
function server.getPlayer(pseudo) end
---@return Player[]
function server.getOnlinePlayers() end
---@return World
function server.getWorld() end
---@param nom string
---@return integer? état par défaut du bloc, ou nil
function server.blockState(nom) end
---@param etat integer
---@return string
function server.blockName(etat) end
---@param secondes number
---@param fonction fun()
---@return Timer
function server.afterSeconds(secondes, fonction) end
---@param secondes number
---@param fonction fun(minuterie: Timer)
---@return Timer
function server.everySeconds(secondes, fonction) end
---@param nom string
---@param valeur any
function server.saveData(nom, valeur) end
---@param nom string
---@param defaut any
---@return any
function server.loadData(nom, defaut) end
---@param defauts table
---@return table
function server.getConfig(defauts) end
---@param nom string
---@return string?
function server.read(nom) end
---@param nom string
---@param texte string
---@return boolean
function server.write(nom, texte) end
---@param nom string
---@param texte string
function server.append(nom, texte) end
---@return integer
function server.time() end
---@param format? string
---@return string
function server.date(format) end
---@return number
function server.clock() end
---@return string
function server.getMotd() end
---@param texte string
function server.setMotd(texte) end
--- Arrête le serveur proprement (quand la fonction en cours se termine).
function server.stop() end
--- Personnages non joueurs.
---@param nom string
function server.spawnNpc(nom, x, y, z) end
---@param nom string
---@param vx number blocs par seconde sur x
---@param vz number blocs par seconde sur z
function server.walkNpc(nom, vx, vz) end
---@param nom string
function server.teleportNpc(nom, x, y, z) end
---@param nom string
function server.removeNpc(nom) end
function server.removeNpcs() end

GameMode = { SURVIVAL = 0, CREATIVE = 1, ADVENTURE = 2, SPECTATOR = 3 }
EventPriority = { LOWEST = "LOWEST", LOW = "LOW", NORMAL = "NORMAL", HIGH = "HIGH", HIGHEST = "HIGHEST", MONITOR = "MONITOR" }
PlayerInteractEvent = { LEFT_CLICK_BLOCK = 0, RIGHT_CLICK_BLOCK = 1 }
EntityDamageEvent = { CAUSE_FALL = "FALL", CAUSE_VOID = "VOID", CAUSE_ENTITY_ATTACK = "ENTITY_ATTACK", CAUSE_SUICIDE = "SUICIDE", CAUSE_CUSTOM = "CUSTOM" }
---@type table<string, string> couleurs et styles du chat (TextFormat.GREEN, RED, GOLD…)
TextFormat = {}
