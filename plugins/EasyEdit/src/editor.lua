-- Opérations : une liste de blocs à poser, appliquée par tranches à chaque tick pour ne
-- jamais bloquer le serveur. Seuls les blocs qui changent vraiment sont gardés dans
-- l'historique, pour pouvoir annuler et refaire.
local config = require("src.config")
local positions = require("src.positions")

local editor = {}
local world = server.getWorld()
local queue = {}
local histories = {} -- pseudo -> { undo = {…}, redo = {…} }

function editor.history(name)
  histories[name] = histories[name] or { undo = {}, redo = {} }
  return histories[name]
end

-- Nouvelle opération : `keys` (positions rangées) et `news` (états).
-- `kind` : "do" (nouvelle, par défaut), "undo" ou "redo".
function editor.enqueue(player, label, keys, news, kind)
  if #keys == 0 then
    player:sendMessage("§70 bloc modifié.")
    return
  end
  if #keys > config.MAX_BLOCKS then
    player:sendMessage("§cTrop de blocs : " .. #keys .. " (maximum " .. config.MAX_BLOCKS .. ").")
    return
  end
  queue[#queue + 1] = {
    player = player:getName(), label = label, kind = kind or "do",
    keys = keys, news = news, i = 1,
    done_keys = {}, done_olds = {}, done_news = {},
  }
  if #queue > 1 or #keys > config.PER_TICK then
    player:sendMessage("§7" .. label .. " : " .. #keys .. " blocs, en cours…")
  end
end

local function remember(op)
  if #op.done_keys == 0 then return end
  local h = editor.history(op.player)
  local entry = { keys = op.done_keys, olds = op.done_olds, news = op.done_news, label = op.label }
  if op.kind == "undo" then
    -- Refaire, c'est rejouer l'opération d'origine : avant = après l'annulation, et inversement
    entry.olds, entry.news = entry.news, entry.olds
    h.redo[#h.redo + 1] = entry
  else
    if op.kind == "do" then h.redo = {} end
    h.undo[#h.undo + 1] = entry
    if #h.undo > config.HISTORY then table.remove(h.undo, 1) end
  end
  -- Historique borné en nombre de blocs, tous joueurs confondus : les plus anciens partent
  local total = 0
  for _, hh in pairs(histories) do
    for _, e in ipairs(hh.undo) do total = total + #e.keys end
    for _, e in ipairs(hh.redo) do total = total + #e.keys end
  end
  while total > config.HISTORY_BLOCKS and #h.undo > 1 do
    total = total - #h.undo[1].keys
    table.remove(h.undo, 1)
  end
end

-- Une tranche de la première opération en attente
local function tick()
  local op = queue[1]
  if not op then return end
  local writes, w = {}, 0
  local last = math.min(op.i + config.PER_TICK - 1, #op.keys)
  local news, keys = op.news, op.keys
  for i = op.i, last do
    local k = keys[i]
    local x, y, z = positions.unpack(k)
    local new = news[i]
    local old = world:getBlockState(x, y, z)
    if old and old ~= new then
      local n = #op.done_keys + 1
      op.done_keys[n], op.done_olds[n], op.done_news[n] = k, old, new
      writes[w + 1], writes[w + 2], writes[w + 3], writes[w + 4] = x, y, z, new
      w = w + 4
    end
  end
  if w > 0 then world:setBlocks(writes) end
  op.i = last + 1
  if op.i > #op.keys then
    table.remove(queue, 1)
    remember(op)
    local who = server.getPlayer(op.player)
    if who then
      local verb = op.kind == "undo" and "annulés" or "modifiés"
      who:sendMessage("§a" .. op.label .. " : " .. #op.done_keys .. " blocs " .. verb .. ".")
    end
  end
end

function editor.start()
  server.every(1, tick)
end

-- Liste des positions d'un volume (avec un filtre facultatif) et de leurs nouveaux états
function editor.fill(r, keep, choose)
  local keys, news, n = {}, {}, 0
  for y = r.y1, r.y2 do
    for x = r.x1, r.x2 do
      for z = r.z1, r.z2 do
        if not keep or keep(x, y, z) then
          n = n + 1
          keys[n], news[n] = positions.pack(x, y, z), choose(x, y, z)
        end
      end
    end
  end
  return keys, news
end

return editor
