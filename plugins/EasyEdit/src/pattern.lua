-- Motifs : « stone », « stone,dirt », « 70%stone,30%cobblestone ».
-- Renvoie une fonction qui donne l'état à poser, ou nil (avec un message au joueur).
return function(player, text)
  local choices, total = {}, 0
  for part in tostring(text):gmatch("[^,]+") do
    local weight, name = part:match("^%s*(%d+)%%(.-)%s*$")
    if not weight then weight, name = 1, part:match("^%s*(.-)%s*$") end
    local state = server.blockState(name)
    if not state then
      player:sendMessage("§cBloc inconnu : " .. name)
      return nil
    end
    total = total + tonumber(weight)
    choices[#choices + 1] = { total, state }
    if #choices >= 16 then break end
  end
  if #choices == 0 then
    player:sendMessage("§cAucun bloc indiqué.")
    return nil
  end
  if #choices == 1 then
    local only = choices[1][2]
    return function() return only end
  end
  return function()
    local r = math.random() * total
    for _, c in ipairs(choices) do
      if r < c[1] then return c[2] end
    end
    return choices[#choices][2]
  end
end
