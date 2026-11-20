-- Réglages d'EasyEdit
return {
  WAND = "minecraft:wooden_axe",
  PERMISSION = "easyedit.use",
  -- Blocs touchés par une opération au plus
  MAX_BLOCKS = 250000,
  -- Blocs traités par tick (20 ticks par seconde)
  PER_TICK = 20000,
  -- Opérations que l'on peut annuler, par joueur, et blocs gardés au total dans l'historique
  HISTORY = 20,
  HISTORY_BLOCKS = 400000,
}
