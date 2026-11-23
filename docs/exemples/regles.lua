-- Règles maison : change ce que valent quelques objets et blocs du jeu.
-- À copier dans plugins/ ; voir « Modifier objets et blocs » dans docs/PLUGINS.md.

-- Le pain rassasie entièrement
server.items.modify("minecraft:bread", { food = 20, saturation = 12 })

-- L'épée en bois frappe fort, mais casse au dixième coup
server.items.modify("minecraft:wooden_sword", { attackDamage = 9, durability = 10 })

-- Le gravier ne tombe plus et rend toujours du silex
server.blocks.modify("minecraft:gravel", { falls = false, drops = { ["minecraft:flint"] = 1 } })

-- Les planches de chêne tombent comme du sable
server.blocks.modify("minecraft:oak_planks", { falls = true })

-- L'obsidienne ne se casse plus en survie
server.blocks.modify("minecraft:obsidian", { breakable = false })

server.log("Règles maison en place")
