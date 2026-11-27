-- Réglages de MultiWorld.
return {
  -- Mondes où les joueurs peuvent aller seuls avec /monde <nom>.
  -- Les opérateurs vont partout avec /mw tp.
  publics = { "lobby" },

  -- Mondes créés au démarrage s'ils n'existent pas : nom = type de terrain
  -- ("vanilla" : le terrain du jeu, "flat" : plat, "void" : vide, "nether", "end")
  creer = { lobby = "flat" },

  -- Monde où arrivent les joueurs à leur toute première connexion (false : le monde principal)
  premier_monde = false,

  -- Où mènent les portails. Le serveur allume les portails et prévient ce plugin quand un
  -- joueur en franchit un, mais ne téléporte personne : c'est décidé ici.
  -- Exemple : portails = { nether = "enfer", ["end"] = "fin" }, avec
  --           creer = { enfer = "nether", fin = "end" }
  -- Depuis le monde d'arrivée, le même portail ramène dans le monde principal.
  portails = {},
}
