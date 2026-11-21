-- /undo, /redo
local editor = require("src.editor")
local command = require("src.commands.util").command

-- Rejoue la dernière opération de la pile `which` (undo ou redo).
local function replay(player, which, kind, label)
  local h = editor.history(player:getName())
  local entry = table.remove(h[which])
  if not entry then
    player:sendMessage("§c" .. (kind == "undo" and "Rien à annuler." or "Rien à refaire."))
    return
  end
  -- Dans l'ordre inverse pour annuler (anciens états), comme à l'origine pour refaire
  local undo = kind == "undo"
  local keys, news, n = {}, {}, #entry.keys
  for i = 1, n do
    local j = undo and (n - i + 1) or i
    keys[i], news[i] = entry.keys[j], undo and entry.olds[j] or entry.news[j]
  end
  editor.enqueue(player, label, keys, news, kind)
end

command("undo", "/undo", function(player)
  replay(player, "undo", "undo", "Annulation")
end)

command("redo", "/redo", function(player)
  replay(player, "redo", "redo", "Rétablissement")
end)
