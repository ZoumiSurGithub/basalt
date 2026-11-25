// Basalt, tableau de bord. Tout passe par l'API du serveur, sur la même adresse.
// Rien de ce que le serveur renvoie n'est inséré comme du HTML : uniquement comme du texte.
"use strict";

const $ = (id) => document.getElementById(id);
let jeton = sessionStorage.getItem("basalt-jeton") || "";
let admin = false;
let contenu = { items: null, blocks: null };
let veille = null;

function el(tag, texte, classe) {
  const e = document.createElement(tag);
  if (texte !== undefined && texte !== null) e.textContent = String(texte);
  if (classe) e.className = classe;
  return e;
}

async function api(methode, chemin, corps) {
  const reponse = await fetch(chemin, {
    method: methode,
    headers: { Authorization: "Bearer " + jeton },
    body: corps === undefined ? undefined : JSON.stringify(corps),
    cache: "no-store",
  });
  let donnees = null;
  try { donnees = await reponse.json(); } catch (e) { /* réponse vide */ }
  if (!reponse.ok) {
    const erreur = new Error((donnees && donnees.error) || "Le serveur a répondu " + reponse.status);
    erreur.code = reponse.status;
    throw erreur;
  }
  return donnees;
}

function avis(texte, alerte) {
  const a = $("avis");
  a.textContent = texte;
  a.classList.toggle("alerte", !!alerte);
  a.hidden = false;
  clearTimeout(avis.minuterie);
  avis.minuterie = setTimeout(() => { a.hidden = true; }, alerte ? 9000 : 5000);
}

function duree(secondes) {
  const j = Math.floor(secondes / 86400), h = Math.floor(secondes % 86400 / 3600), m = Math.floor(secondes % 3600 / 60);
  if (j > 0) return j + " j " + h + " h";
  if (h > 0) return h + " h " + m + " min";
  return m + " min " + Math.floor(secondes % 60) + " s";
}

function heureDuMonde(ticks) {
  const t = ((ticks % 24000) + 24000) % 24000;
  const h = Math.floor(t / 1000 + 6) % 24, m = Math.floor(t % 1000 * 60 / 1000);
  return String(h).padStart(2, "0") + " h " + String(m).padStart(2, "0");
}

// ---- La porte ----

async function ouvrir() {
  try {
    const moi = await api("GET", "/v1/me");
    admin = !!moi.admin;
  } catch (e) {
    fermer(e.code === 401 ? "Ce jeton n'est pas reconnu par le serveur." : "Le serveur ne répond pas : " + e.message);
    return;
  }
  sessionStorage.setItem("basalt-jeton", jeton);
  $("porte").hidden = true;
  $("tableau").hidden = false;
  $("droit").textContent = admin ? "Administration" : "Lecture seule";
  for (const b of [$("recharger"), $("commande-form").querySelector("button"), $("commande")]) b.disabled = !admin;
  montrer("etat");
}

function fermer(raison) {
  clearInterval(veille);
  jeton = "";
  admin = false;
  contenu = { items: null, blocks: null };
  sessionStorage.removeItem("basalt-jeton");
  $("tableau").hidden = true;
  $("porte").hidden = false;
  $("jeton").value = "";
  const e = $("porte-erreur");
  e.textContent = raison || "";
  e.hidden = !raison;
  $("jeton").focus();
}

$("porte-form").addEventListener("submit", (ev) => {
  ev.preventDefault();
  jeton = $("jeton").value.trim();
  if (jeton) ouvrir();
});

$("fermer").addEventListener("click", () => fermer());

// ---- Les onglets ----

const sections = { etat: lireEtat, joueurs: lireJoueurs, carte: lireCarte, contenu: lireContenu, analyse: lireAnalyse, packs: lirePacks, console: async () => {} };
const titres = { etat: "Aperçu", joueurs: "Joueurs", carte: "Carte", contenu: "Objets et blocs", analyse: "Analyse de la carte", packs: "Packs", console: "Console" };
let ongletCourant = "etat";

function montrer(nom) {
  clearInterval(veille);
  for (const b of document.querySelectorAll(".barre button")) {
    if (b.dataset.onglet === nom) b.setAttribute("aria-current", "page"); else b.removeAttribute("aria-current");
  }
  for (const s of Object.keys(sections)) $(s).hidden = s !== nom;
  ongletCourant = nom;
  $("titre").textContent = titres[nom];
  const lire = () => sections[nom]().catch((e) => {
    if (e.code === 401) fermer("Le jeton n'est plus accepté par le serveur."); else avis(e.message, true);
  });
  lire();
  // L'état et les joueurs se rafraîchissent d'eux-mêmes, tant que la page est regardée et
  // qu'aucune fiche n'est ouverte (inutile d'interroger le serveur pour un onglet caché)
  if (nom === "etat" || nom === "joueurs") {
    veille = setInterval(() => { if (!document.hidden && !$("fiche").open) lire(); }, nom === "etat" ? 3000 : 5000);
  }
  if (nom === "carte") veille = setInterval(() => { if (!document.hidden) lire(); }, 2000);
}

for (const b of document.querySelectorAll(".barre button")) b.addEventListener("click", () => montrer(b.dataset.onglet));

// ---- Le thème : sombre par défaut (celui du logo), clair au choix ; le choix est gardé ----

function appliquerTheme(nom) {
  if (nom === "clair") document.documentElement.dataset.theme = "clair"; else delete document.documentElement.dataset.theme;
}

let theme = "sombre";
try { theme = localStorage.getItem("basalt-theme") || (matchMedia("(prefers-color-scheme: light)").matches ? "clair" : "sombre"); } catch (e) { /* stockage refusé */ }
appliquerTheme(theme);

$("theme").addEventListener("click", () => {
  theme = theme === "clair" ? "sombre" : "clair";
  appliquerTheme(theme);
  try { localStorage.setItem("basalt-theme", theme); } catch (e) { /* stockage refusé */ }
});

// ---- État ----

async function lireEtat() {
  const e = await api("GET", "/v1/status");
  const tps = Math.max(0, Math.min(20, e.ticks_per_second));
  $("tps").textContent = tps.toLocaleString("fr-FR", { maximumFractionDigits: 1 });
  const orgues = $("orgues");
  orgues.replaceChildren();
  // Vingt colonnes de hauteurs inégales, comme une chaussée de basalte ; pleines jusqu'au
  // nombre de ticks tenus ; la dernière tenue porte la lave, comme sur le logo
  for (let i = 0; i < 20; i++) {
    const colonne = el("span");
    colonne.style.height = (58 + ((i * 37 + 11) % 43)) + "%";
    if (i >= Math.round(tps)) colonne.className = "creuse";
    else if (i === Math.round(tps) - 1) colonne.className = "tete-de-file";
    orgues.append(colonne);
  }
  const faits = [
    ["Version", e.version],
    ["En marche depuis", duree(e.uptime_seconds)],
    ["Joueurs", e.players_online + " sur " + e.players_max],
    ["Créatures", e.creatures],
    ["Mémoire", e.memory_mb === null ? "inconnue" : e.memory_mb + " Mo"],
    ["Plus long retard d'un tick", e.worst_tick_delay_ms + " ms"],
    ["Heure du monde", heureDuMonde(e.world_time)],
    ["Blocs modifiés", e.world_edits.toLocaleString("fr-FR")],
    ["Plugins", e.plugins.length ? e.plugins.join(", ") : "aucun"],
  ];
  const liste = $("faits");
  liste.replaceChildren();
  for (const [nom, valeur] of faits) {
    const ligne = el("div");
    ligne.append(el("dt", nom), el("dd", valeur));
    liste.append(ligne);
  }
}

// ---- Joueurs ----
// La liste est demandée page par page, avec la recherche faite par le serveur : avec mille
// joueurs connectés, on ne rapatrie ni n'affiche jamais tout.

const modes = { survival: "survie", creative: "créatif", adventure: "aventure", spectator: "spectateur" };
const PAGE = 50;
let pageJoueurs = 0;
let ficheDe = null;

async function lireJoueurs() {
  const q = $("cherche-joueur").value.trim();
  const r = await api("GET", "/v1/players?limit=" + PAGE + "&offset=" + pageJoueurs * PAGE + "&q=" + encodeURIComponent(q));
  // (la page demandée n'existe plus : des joueurs sont partis)
  if (!r.players.length && pageJoueurs > 0) { pageJoueurs = 0; return lireJoueurs(); }
  const debut = r.offset + 1, fin = r.offset + r.players.length;
  $("compte-joueurs").textContent = r.online + " joueur(s) en jeu" + (q ? ", " + r.total + " pour cette recherche" : "") + (r.total > PAGE ? ", du " + debut + "e au " + fin + "e" : "");
  $("joueurs-avant").disabled = pageJoueurs === 0;
  $("joueurs-apres").disabled = fin >= r.total;
  document.querySelector(".pages").hidden = r.total <= PAGE;
  const liste = $("liste-joueurs");
  liste.replaceChildren();
  if (!r.players.length) {
    liste.append(el("li", q ? "Aucun joueur en jeu ne porte ce nom." : "Personne n'est connecté.", "vide"));
    return;
  }
  for (const j of r.players) {
    const ligne = el("li");
    const nom = el("span", j.name, "nom");
    if (j.operator) nom.append(" ", el("span", "opérateur", "marque-op"));
    ligne.append(nom);
    if (j.health !== null) {
      const jauge = el("span", null, "jauge");
      jauge.title = (j.health / 2).toLocaleString("fr-FR") + " cœurs";
      const plein = el("span");
      plein.style.width = Math.max(0, Math.min(100, j.health / (j.max_health || 20) * 100)) + "%";
      jauge.append(plein);
      ligne.append(jauge);
    }
    ligne.append(el("span", (modes[j.gamemode] || j.gamemode) + (j.dead ? ", mort" : "") + ", en " + Math.floor(j.x) + " " + Math.floor(j.y) + " " + Math.floor(j.z), "detail"));
    const ouvrirFiche = el("button", admin ? "Gérer" : "Voir");
    ouvrirFiche.type = "button";
    ouvrirFiche.addEventListener("click", () => montrerFiche(j));
    ligne.append(ouvrirFiche);
    liste.append(ligne);
  }
}

let frappe = null;
$("cherche-joueur").addEventListener("input", () => {
  // (on attend la fin de la frappe avant d'interroger le serveur)
  clearTimeout(frappe);
  frappe = setTimeout(() => { pageJoueurs = 0; lireJoueurs().catch((e) => avis(e.message, true)); }, 250);
});
$("joueurs-avant").addEventListener("click", () => { pageJoueurs = Math.max(0, pageJoueurs - 1); lireJoueurs().catch((e) => avis(e.message, true)); });
$("joueurs-apres").addEventListener("click", () => { pageJoueurs += 1; lireJoueurs().catch((e) => avis(e.message, true)); });

// ---- La fiche d'un joueur ----

const EFFETS = [["speed", "Vitesse"], ["slowness", "Lenteur"], ["haste", "Hâte"], ["strength", "Force"], ["weakness", "Faiblesse"], ["jump_boost", "Saut"], ["regeneration", "Régénération"], ["resistance", "Résistance"], ["fire_resistance", "Résistance au feu"], ["night_vision", "Vision nocturne"], ["invisibility", "Invisibilité"], ["slow_falling", "Chute lente"], ["levitation", "Lévitation"], ["poison", "Poison"], ["blindness", "Cécité"]];
for (const [valeur, nom] of EFFETS) {
  const o = el("option", nom);
  o.value = valeur;
  $("effet-nom").append(o);
}

function montrerFiche(j) {
  ficheDe = j.name;
  $("fiche-nom").textContent = j.name;
  const morceaux = [modes[j.gamemode] || j.gamemode];
  if (j.health !== null) morceaux.push((j.health / 2).toLocaleString("fr-FR") + " cœurs sur " + ((j.max_health || 20) / 2).toLocaleString("fr-FR"));
  if (j.food !== null) morceaux.push("faim " + Math.round(j.food) + " sur 20");
  morceaux.push("en " + Math.floor(j.x) + " " + Math.floor(j.y) + " " + Math.floor(j.z));
  if (j.operator) morceaux.push("opérateur");
  if (j.effects && j.effects.length) morceaux.push("effets : " + j.effects.join(", "));
  $("fiche-etat").textContent = morceaux.join(", ");
  $("fiche-actions").hidden = !admin;
  $("fiche-lecture").hidden = admin;
  $("tp-x").value = Math.floor(j.x);
  $("tp-y").value = Math.floor(j.y);
  $("tp-z").value = Math.floor(j.z);
  $("fiche").showModal();
}

async function agir(action, champs, garderOuverte) {
  if (!ficheDe) return;
  try {
    const r = await api("POST", "/v1/players/action", Object.assign({ name: ficheDe, action: action }, champs || {}));
    avis(ficheDe + " : " + r.done + ".");
    if (!garderOuverte) $("fiche").close();
    if (ongletCourant === "joueurs") lireJoueurs().catch(() => {});
  } catch (e) {
    $("fiche").close();
    avis(e.message, true);
  }
}

$("fiche-fermer").addEventListener("click", () => $("fiche").close());
$("fiche").addEventListener("close", () => { ficheDe = null; });
$("f-tp").addEventListener("submit", (ev) => { ev.preventDefault(); agir("teleport", { x: Number($("tp-x").value), y: Number($("tp-y").value), z: Number($("tp-z").value) }); });
$("f-tpvers").addEventListener("submit", (ev) => { ev.preventDefault(); agir("teleport_to", { target: $("tp-vers").value.trim() }); });
for (const b of $("f-mode").querySelectorAll("button")) b.addEventListener("click", () => agir("gamemode", { mode: b.dataset.mode }));
$("f-don").addEventListener("submit", (ev) => { ev.preventDefault(); agir("give", { item: $("don-objet").value.trim(), count: Number($("don-nombre").value) }, true); });
$("f-effet").addEventListener("submit", (ev) => { ev.preventDefault(); agir("effect", { effect: $("effet-nom").value, seconds: Number($("effet-secondes").value), level: Number($("effet-niveau").value) }, true); });
$("f-message").addEventListener("submit", (ev) => { ev.preventDefault(); agir("message", { text: $("message-texte").value.trim() }, true).then(() => { $("message-texte").value = ""; }); });
$("f-expulser").addEventListener("submit", (ev) => {
  ev.preventDefault();
  if (confirm("Expulser " + ficheDe + " du serveur ?")) agir("kick", { reason: $("expulser-raison").value.trim() });
});
for (const b of document.querySelectorAll("#fiche-actions button[data-action]")) {
  b.addEventListener("click", () => { if (!b.dataset.confirme || confirm(b.dataset.confirme)) agir(b.dataset.action); });
}

// ---- Les images des objets et des blocs ----
// Elles sont demandées une à une, avec le jeton, seulement pour les lignes qui arrivent à
// l'écran, quatre à la fois au plus ; ce qui a été trouvé (ou non) n'est pas redemandé.

const images = new Map();
const attente = [];
let enCours = 0;

function servirImages() {
  while (enCours < 4 && attente.length) {
    const { cle, cible } = attente.shift();
    if (images.has(cle)) { poserImage(cible, images.get(cle)); continue; }
    enCours += 1;
    const [genreImage, nom] = cle.split("|");
    fetch("/v1/texture?kind=" + genreImage + "&name=" + encodeURIComponent(nom), { headers: { Authorization: "Bearer " + jeton } })
      .then((r) => (r.ok ? r.blob() : null))
      .then((blob) => (blob ? URL.createObjectURL(blob) : null))
      .catch(() => null)
      .then((adresse) => {
        images.set(cle, adresse);
        poserImage(cible, adresse);
        enCours -= 1;
        servirImages();
      });
  }
}

function poserImage(cible, adresse) {
  if (!adresse || !cible.isConnected) return;
  const image = document.createElement("img");
  image.src = adresse;
  image.alt = "";
  cible.replaceChildren(image);
}

const guetteur = new IntersectionObserver((vues) => {
  for (const vue of vues) {
    if (!vue.isIntersecting) continue;
    guetteur.unobserve(vue.target);
    attente.push({ cle: vue.target.dataset.cle, cible: vue.target });
  }
  servirImages();
}, { rootMargin: "200px" });

function vignette(genreImage, nom) {
  const cadre = el("span", null, "vignette");
  cadre.dataset.cle = genreImage + "|" + nom;
  const connue = images.get(cadre.dataset.cle);
  if (connue) poserImage(cadre, connue); else if (connue === undefined) guetteur.observe(cadre);
  return cadre;
}

let imagesDuJeu = null;

// ---- Objets et blocs ----

function genre() {
  return document.querySelector('input[name="genre"]:checked').value;
}

async function lireContenu() {
  const g = genre();
  if (!contenu[g]) {
    $("compte").textContent = "Lecture de la liste…";
    contenu[g] = await api("GET", "/v1/" + g);
  }
  if (imagesDuJeu === null) {
    try { imagesDuJeu = (await api("GET", "/v1/textures")).vanilla; } catch (e) { imagesDuJeu = true; }
    $("sans-images").hidden = imagesDuJeu;
  }
  afficherContenu();
}

function detailDe(x, g) {
  const morceaux = [];
  if (x.custom) morceaux.push("plugin");
  if (g === "items") {
    morceaux.push("pile de " + x.max_stack);
    if (x.durability > 0) morceaux.push(x.durability + " utilisations");
    if (x.attack_damage > 1) morceaux.push(x.attack_damage + " de dégâts");
    if (x.armor_points > 0) morceaux.push(x.armor_points + " d'armure");
    if (x.food) morceaux.push("se mange");
  } else {
    if (x.hardness !== null) morceaux.push(x.hardness < 0 ? "incassable" : "dureté " + x.hardness);
    if (x.light > 0) morceaux.push("lumière " + x.light);
    if (x.states > 1) morceaux.push(x.states + " états");
  }
  return morceaux.join(", ");
}

function afficherContenu() {
  const g = genre();
  const tout = contenu[g] || [];
  const mot = $("recherche").value.trim().toLowerCase();
  const garde = tout.filter((x) => (!mot || x.name.toLowerCase().includes(mot)) && (!$("seul-plugins").checked || x.custom) && (!$("seul-coupes").checked || x.disabled));
  const montre = garde.slice(0, 150);
  $("compte").textContent = garde.length + (g === "items" ? " objet(s)" : " bloc(s)") + (garde.length > montre.length ? ", les " + montre.length + " premiers affichés : précisez la recherche" : "");
  const liste = $("liste-contenu");
  liste.replaceChildren();
  for (const x of montre) {
    const court = x.name.replace(/^minecraft:/, "");
    const ligne = el("li", null, x.disabled ? "coupe" : "");
    ligne.append(vignette(g === "items" ? "item" : "block", court), el("span", court, "nom"), el("span", (x.disabled ? "désactivé, " : "") + detailDe(x, g), "detail"));
    if (admin) {
      const bouton = el("button", x.disabled ? "Réactiver" : "Désactiver", x.disabled ? "plein" : "");
      bouton.type = "button";
      bouton.addEventListener("click", async () => {
        bouton.disabled = true;
        try {
          const r = await api("POST", x.disabled ? "/v1/enable" : "/v1/disable", { name: court });
          // Un objet et son bloc portent souvent le même nom : les deux listes suivent
          for (const cle of ["items", "blocks"]) for (const y of contenu[cle] || []) if (y.name.replace(/^minecraft:/, "") === court) y.disabled = r.disabled;
          avis(court + (r.disabled ? " est désactivé : on ne peut plus le poser ni s'en servir." : " est réactivé."));
          afficherContenu();
        } catch (e) { avis(e.message, true); bouton.disabled = false; }
      });
      ligne.append(bouton);
    }
    liste.append(ligne);
  }
  if (!montre.length) liste.append(el("li", "Rien ne correspond à cette recherche.", "vide"));
}

$("recherche").addEventListener("input", afficherContenu);
$("seul-plugins").addEventListener("change", afficherContenu);
$("seul-coupes").addEventListener("change", afficherContenu);
for (const r of document.querySelectorAll('input[name="genre"]')) r.addEventListener("change", () => lireContenu().catch((e) => avis(e.message, true)));

// ---- La carte en direct ----

// Le serveur envoie des tuiles de 64 blocs de côté : quatre octets par colonne (rouge, vert,
// bleu, connue ou non). Elles sont gardées ici et redemandées toutes les cinq secondes tant
// qu'elles sont à l'écran ; les joueurs, toutes les deux secondes.
const TUILE = 64;
const ZOOMS = [1, 2, 4, 8, 16];
const AIDES = { plan: "Glissez pour vous déplacer, cliquez un bloc pour le choisir. Les zones rayées ne sont pas chargées : aucun joueur n'est à proximité.", relief: "Glissez pour tourner, glissez à deux doigts (ou avec Maj) pour vous déplacer, cliquez un bloc pour le choisir. Seul le terrain autour des joueurs est chargé." };
const carte = { mode: "plan", x: 0, z: 0, zoom: 4, tuiles: new Map(), joueurs: [], choix: null, centree: false, demandes: 0, noms: false };

function toile() {
  const t = $("carte-toile");
  const echelle = window.devicePixelRatio || 1;
  const [l, h] = [Math.round(t.clientWidth * echelle), Math.round(t.clientHeight * echelle)];
  if (t.width !== l || t.height !== h) { t.width = l; t.height = h; }
  return { t, echelle, l: t.clientWidth, h: t.clientHeight };
}

function dessinerCarte() {
  if (carte.mode === "relief") { Relief.dessiner(); return; }
  const { t, echelle, l, h } = toile();
  if (!l) return;
  const d = t.getContext("2d");
  const style = getComputedStyle(document.documentElement);
  const couleur = (nom) => style.getPropertyValue(nom).trim();
  d.setTransform(echelle, 0, 0, echelle, 0, 0);
  d.imageSmoothingEnabled = false;
  d.fillStyle = couleur("--surface");
  d.fillRect(0, 0, l, h);
  // Rayures : ce que le serveur n'a pas en mémoire
  d.strokeStyle = couleur("--filet");
  d.lineWidth = 1;
  d.beginPath();
  for (let i = -h; i < l; i += 12) { d.moveTo(i, 0); d.lineTo(i + h, h); }
  d.stroke();
  const z = carte.zoom;
  const [gauche, haut] = [carte.x - l / 2 / z, carte.z - h / 2 / z];
  const maintenant = Date.now();
  for (let tz = Math.floor(haut / TUILE); tz * TUILE < haut + h / z; tz++) {
    for (let tx = Math.floor(gauche / TUILE); tx * TUILE < gauche + l / z; tx++) {
      const cle = tx + "," + tz;
      let tuile = carte.tuiles.get(cle);
      if (!tuile) { tuile = { image: null, lue: 0, enRoute: false, vue: 0 }; carte.tuiles.set(cle, tuile); }
      tuile.vue = maintenant;
      if (tuile.image) d.drawImage(tuile.image, Math.round((tx * TUILE - gauche) * z), Math.round((tz * TUILE - haut) * z), TUILE * z, TUILE * z);
      if (!tuile.enRoute && maintenant - tuile.lue > 5000) demanderTuile(tx, tz, tuile);
    }
  }
  if (carte.choix) {
    d.strokeStyle = couleur("--lave-vive");
    d.lineWidth = 2;
    const c = Math.max(z, 6);
    d.strokeRect((carte.choix.x + 0.5 - gauche) * z - c / 2, (carte.choix.z + 0.5 - haut) * z - c / 2, c, c);
  }
  d.font = "600 13px " + style.getPropertyValue("--famille");
  d.textBaseline = "middle";
  for (const j of carte.joueurs) {
    const [px, pz] = [(j.x - gauche) * z, (j.z - haut) * z];
    if (px < -80 || pz < -20 || px > l + 20 || pz > h + 20) continue;
    d.fillStyle = "#14161a";
    d.beginPath(); d.arc(px, pz, 6.5, 0, 7); d.fill();
    d.fillStyle = couleur("--lave");
    d.beginPath(); d.arc(px, pz, 4.5, 0, 7); d.fill();
    const large = d.measureText(j.name).width;
    d.fillStyle = "rgb(20 22 26 / 0.8)";
    d.fillRect(px + 9, pz - 10, large + 10, 20);
    d.fillStyle = "#f4f1ea";
    d.fillText(j.name, px + 14, pz + 1);
  }
}

async function demanderTuile(tx, tz, tuile) {
  // (quatre demandes à la fois au plus : le reste attend le prochain dessin)
  if (carte.demandes >= 4) return;
  carte.demandes++;
  tuile.enRoute = true;
  try {
    const reponse = await fetch("/v1/map?cx=" + tx * 4 + "&cz=" + tz * 4, { headers: { Authorization: "Bearer " + jeton }, cache: "no-store" });
    if (!reponse.ok) throw new Error("carte : " + reponse.status);
    const octets = new Uint8Array(await reponse.arrayBuffer());
    if (octets.length !== TUILE * TUILE * 4) throw new Error("carte : tuile incomplète");
    const points = new Uint8ClampedArray(octets);
    for (let i = 3; i < points.length; i += 4) points[i] = points[i] ? 255 : 0;
    const image = document.createElement("canvas");
    image.width = image.height = TUILE;
    image.getContext("2d").putImageData(new ImageData(points, TUILE, TUILE), 0, 0);
    tuile.image = image;
  } catch (e) { /* la tuile sera redemandée */ }
  tuile.lue = Date.now();
  tuile.enRoute = false;
  carte.demandes--;
  // Les tuiles qu'on ne regarde plus depuis une minute sont oubliées
  if (carte.tuiles.size > 400) for (const [cle, t] of carte.tuiles) if (Date.now() - t.vue > 60000) carte.tuiles.delete(cle);
  if (ongletCourant === "carte") dessinerCarte();
}

function remplirChoix(liste, noms, garder) {
  const avant = liste.value;
  while (liste.options.length > (garder ? 1 : 0)) liste.remove(liste.options.length - 1);
  for (const n of noms) liste.append(new Option(n, n));
  if (noms.includes(avant)) liste.value = avant;
}

async function lireCarte() {
  const r = await api("GET", "/v1/players?limit=200");
  carte.joueurs = r.players;
  const noms = r.players.map((j) => j.name);
  remplirChoix($("carte-joueur"), noms, true);
  remplirChoix($("carte-qui"), noms, false);
  if (!carte.centree && r.players.length) { carte.x = r.players[0].x; carte.z = r.players[0].z; }
  carte.centree = true;
  if (carte.vraies === undefined) {
    carte.vraies = null;
    api("GET", "/v1/textures").then((t) => { carte.vraies = !!t.vanilla; }).catch(() => { carte.vraies = undefined; });
  }
  $("carte-lieu").textContent = AIDES[carte.mode] + (carte.vraies === false ? " Couleurs approchées : avec le pack du jeu dans vanilla_resource_pack/, la carte prend les vraies textures." : "");
  if (carte.mode === "relief") {
    Relief.suivre(carte.x, carte.z);
    api("GET", "/v1/entities?x=" + carte.x.toFixed(1) + "&z=" + carte.z.toFixed(1) + "&r=96").then((e) => Relief.poserEntites(e.entities)).catch(() => {});
  }
  $("carte-actions").hidden = !admin;
  $("carte-lecture").hidden = admin;
  if (admin && !carte.noms) {
    carte.noms = true;
    api("GET", "/v1/blocks").then((b) => { for (const x of b.blocks) $("carte-noms").append(new Option(x.name, x.name)); }).catch(() => { carte.noms = false; });
  }
  dessinerCarte();
}

async function choisirBloc(x, z, y) {
  try {
    const b = await api("GET", "/v1/world/block?x=" + x + "&z=" + z + (y === undefined ? "" : "&y=" + y));
    carte.choix = b;
    $("carte-bloc").textContent = b.block + " en " + b.x + " " + b.y + " " + b.z;
    $("carte-choix").hidden = false;
  } catch (e) {
    carte.choix = null;
    $("carte-choix").hidden = true;
    avis(e.code === 409 ? "Cette zone n'est pas chargée : aucun joueur n'est à proximité." : e.message, true);
  }
  dessinerCarte();
}

async function changerBloc(nom) {
  const c = carte.choix;
  if (!c) return;
  try {
    const r = await api("POST", "/v1/world/block", { x: c.x, y: c.y, z: c.z, block: nom });
    avis(r.changed ? "Bloc changé en " + r.block + " en " + c.x + " " + c.y + " " + c.z + "." : "Ce bloc était déjà " + r.block + ".");
    const tuile = carte.tuiles.get(Math.floor(c.x / TUILE) + "," + Math.floor(c.z / TUILE));
    if (tuile) tuile.lue = 0;
    if (carte.mode === "relief") Relief.suivre(carte.x, carte.z);
    await choisirBloc(c.x, c.z, carte.mode === "relief" ? c.y : undefined);
  } catch (e) { avis(e.message, true); }
}

{
  const t = $("carte-toile");
  let prise = null;
  t.addEventListener("pointerdown", (e) => { prise = { x: e.clientX, z: e.clientY, bouge: false }; t.setPointerCapture(e.pointerId); });
  t.addEventListener("pointermove", (e) => {
    const bord = t.getBoundingClientRect();
    const [bx, bz] = [Math.floor(carte.x + (e.clientX - bord.left - bord.width / 2) / carte.zoom), Math.floor(carte.z + (e.clientY - bord.top - bord.height / 2) / carte.zoom)];
    if (!prise) { t.title = bx + " " + bz; return; }
    if (Math.abs(e.clientX - prise.x) + Math.abs(e.clientY - prise.z) > 5) prise.bouge = true;
    if (!prise.bouge) return;
    carte.x -= e.movementX / carte.zoom;
    carte.z -= e.movementY / carte.zoom;
    dessinerCarte();
  });
  t.addEventListener("pointerup", (e) => {
    const clic = prise && !prise.bouge;
    prise = null;
    if (!clic) return;
    const bord = t.getBoundingClientRect();
    choisirBloc(Math.floor(carte.x + (e.clientX - bord.left - bord.width / 2) / carte.zoom), Math.floor(carte.z + (e.clientY - bord.top - bord.height / 2) / carte.zoom));
  });
  t.addEventListener("pointercancel", () => { prise = null; });
  const zoomer = (pas) => {
    if (carte.mode === "relief") { Relief.approcher(pas); return; }
    carte.zoom = ZOOMS[Math.max(0, Math.min(ZOOMS.length - 1, ZOOMS.indexOf(carte.zoom) + pas))];
    $("carte-moins").disabled = carte.zoom === ZOOMS[0];
    $("carte-plus").disabled = carte.zoom === ZOOMS[ZOOMS.length - 1];
    dessinerCarte();
  };
  t.addEventListener("wheel", (e) => { e.preventDefault(); zoomer(e.deltaY < 0 ? 1 : -1); }, { passive: false });
  // Au clavier : les flèches déplacent, + et − changent l'échelle
  t.addEventListener("keydown", (e) => {
    const pas = 48 / carte.zoom;
    const gestes = { ArrowLeft: () => { carte.x -= pas; }, ArrowRight: () => { carte.x += pas; }, ArrowUp: () => { carte.z -= pas; }, ArrowDown: () => { carte.z += pas; }, "+": () => zoomer(1), "-": () => zoomer(-1) };
    if (!gestes[e.key]) return;
    e.preventDefault();
    gestes[e.key]();
    dessinerCarte();
  });
  $("carte-moins").addEventListener("click", () => zoomer(-1));
  $("carte-plus").addEventListener("click", () => zoomer(1));
  $("carte-joueur").addEventListener("change", () => {
    const j = carte.joueurs.find((x) => x.name === $("carte-joueur").value);
    if (j) { carte.x = j.x; carte.z = j.z; if (carte.mode === "relief") { Relief.recentrer(); Relief.suivre(carte.x, carte.z); } dessinerCarte(); }
  });
  $("carte-remplacer").addEventListener("submit", (e) => { e.preventDefault(); changerBloc($("carte-nom").value.trim()); });
  $("carte-retirer").addEventListener("click", () => changerBloc("air"));
  $("carte-amener").addEventListener("submit", async (e) => {
    e.preventDefault();
    const c = carte.choix;
    if (!c) return;
    try {
      await api("POST", "/v1/players/action", { name: $("carte-qui").value, action: "teleport", x: c.x + 0.5, y: c.y + 1, z: c.z + 0.5 });
      avis($("carte-qui").value + " est téléporté en " + c.x + " " + (c.y + 1) + " " + c.z + ".");
      lireCarte().catch(() => {});
    } catch (err) { avis(err.message, true); }
  });
  window.addEventListener("resize", () => { if (ongletCourant === "carte") dessinerCarte(); });
  // Plan ou relief : la même position, les mêmes outils
  let branche = false;
  const choisirVue = (mode) => {
    if (mode === "relief") {
      $("carte-volume").hidden = false;
      if (!Relief.demarrer($("carte-toile3d"), $("carte-noms3d"))) {
        $("carte-volume").hidden = true;
        avis("Ce navigateur n'affiche pas la vue en relief (WebGL indisponible).", true);
        return;
      }
      if (!branche) {
        branche = true;
        Relief.brancher((b) => { if (b) choisirBloc(b.x, b.z, b.y); }, () => {});
      }
      Relief.regler(window.innerWidth);
    }
    carte.mode = mode;
    carte.choix = null;
    $("carte-choix").hidden = true;
    $("carte-volume").hidden = mode !== "relief";
    t.hidden = mode === "relief";
    $("carte-plan").setAttribute("aria-pressed", String(mode === "plan"));
    $("carte-relief").setAttribute("aria-pressed", String(mode === "relief"));
    $("carte-moins").disabled = $("carte-plus").disabled = false;
    lireCarte().catch(() => {});
  };
  $("carte-plan").addEventListener("click", () => choisirVue("plan"));
  $("carte-relief").addEventListener("click", () => choisirVue("relief"));
}

// ---- Analyse de la carte ----

let rapport = null;
let suiviAnalyse = null;

function ligneObjet(t) {
  const ligne = el("li");
  const ou = t.top.map((h) => h.where + " (" + h.count.toLocaleString("fr-FR") + ")").join(", ");
  ligne.append(vignette("item", t.item), el("span", t.item, "nom"), el("span", t.total.toLocaleString("fr-FR") + " au total, dont " + t.containers.toLocaleString("fr-FR") + " en conteneurs et " + t.players.toLocaleString("fr-FR") + " chez les joueurs. Le plus : " + ou, "detail"));
  return ligne;
}

function afficherTotaux() {
  const mot = $("analyse-cherche").value.trim().toLowerCase();
  const liste = $("analyse-totaux");
  liste.replaceChildren();
  const garde = (rapport ? rapport.totals : []).filter((t) => !mot || t.item.includes(mot)).slice(0, 100);
  for (const t of garde) liste.append(ligneObjet(t));
  if (!garde.length) liste.append(el("li", "Aucun objet de ce nom n'a été trouvé.", "vide"));
}

function afficherRapport(r) {
  rapport = r;
  $("analyse-rapport").hidden = false;
  const faits = [
    ["Conteneurs non vides", r.containers.toLocaleString("fr-FR")],
    ["Joueurs en jeu", r.players_online],
    ["Joueurs absents", r.players_offline.toLocaleString("fr-FR")],
    ["Objets comptés", r.items.toLocaleString("fr-FR")],
    ["Sortes d'objets", r.kinds],
    ["Piles impossibles", r.impossible_stacks],
    ["Groupes de conteneurs jumeaux", r.twin_groups],
    ["Durée de l'analyse", r.seconds.toLocaleString("fr-FR") + " s"],
  ];
  const dl = $("analyse-faits");
  dl.replaceChildren();
  for (const [nom, valeur] of faits) {
    const ligne = el("div");
    ligne.append(el("dt", nom), el("dd", valeur));
    dl.append(ligne);
  }
  const alertes = $("analyse-alertes");
  alertes.replaceChildren();
  for (const a of r.alerts) {
    const ligne = el("li");
    if (a.kind === "stack") {
      ligne.append(vignette("item", a.item), el("span", "Pile impossible : " + a.item, "nom"), el("span", a.count + " dans une case qui en tient " + a.max + " au plus, dans " + a.where, "detail"));
    } else if (a.kind === "twins") {
      ligne.append(el("span", a.count + " conteneurs au contenu identique", "nom"), el("span", a.items.toLocaleString("fr-FR") + " objets chacun, rangés case pour case de la même façon : " + a.where.join(", ") + (a.count > a.where.length ? "…" : ""), "detail"));
    } else {
      ligne.append(vignette("item", a.item), el("span", "Objet désactivé encore présent : " + a.item, "nom"), el("span", a.count.toLocaleString("fr-FR") + " au total", "detail"));
    }
    alertes.append(ligne);
  }
  if (!r.alerts.length) alertes.append(el("li", "Rien d'anormal : aucune pile impossible, aucun conteneur jumeau, aucun objet désactivé en circulation.", "vide"));
  const rares = $("analyse-rares");
  rares.replaceChildren();
  for (const t of r.rare) rares.append(ligneObjet(t));
  if (!r.rare.length) rares.append(el("li", "Aucun objet rare n'est rangé nulle part.", "vide"));
  afficherTotaux();
}

async function lireAnalyse() {
  $("analyser").hidden = !admin;
  const s = await api("GET", "/v1/scan");
  clearTimeout(suiviAnalyse);
  if (s.state === "running") {
    $("analyse-etat").textContent = "Analyse en cours…";
    $("analyser").disabled = true;
    suiviAnalyse = setTimeout(() => { if (ongletCourant === "analyse") lireAnalyse().catch((e) => avis(e.message, true)); }, 1000);
  } else {
    $("analyser").disabled = false;
    $("analyse-etat").textContent = s.state === "done" ? "Dernière analyse ci-dessous." : (admin ? "Aucune analyse n'a encore été faite." : "Aucune analyse n'a encore été faite. La lancer demande le jeton d'administration.");
    if (s.state === "done") afficherRapport(s.report);
  }
}

$("analyser").addEventListener("click", async () => {
  try {
    await api("POST", "/v1/scan");
    await lireAnalyse();
  } catch (e) { avis(e.message, true); }
});
$("analyse-cherche").addEventListener("input", afficherTotaux);

// ---- Packs ----

function taille(octets) {
  return octets >= 1048576 ? (octets / 1048576).toLocaleString("fr-FR", { maximumFractionDigits: 1 }) + " Mo" : Math.max(1, Math.round(octets / 1024)) + " Ko";
}

async function lirePacks() {
  const p = await api("GET", "/v1/packs");
  const charges = $("liste-packs");
  charges.replaceChildren();
  for (const pack of p.loaded) {
    const ligne = el("li");
    ligne.append(el("span", pack.name + " " + pack.version, "nom"), el("span", taille(pack.size) + (pack.encrypted ? ", chiffré" : ", en clair"), "detail"));
    charges.append(ligne);
  }
  if (!p.loaded.length) charges.append(el("li", "Aucun pack n'est envoyé aux joueurs.", "vide"));
  const fichiers = $("liste-fichiers");
  fichiers.replaceChildren();
  for (const f of p.files) {
    const ligne = el("li");
    ligne.append(el("span", f.file, "nom"), el("span", taille(f.size) + (f.has_key ? ", avec sa clé" : ", sans clé"), "detail"));
    if (admin && !f.has_key) {
      const bouton = el("button", "Chiffrer");
      bouton.type = "button";
      bouton.addEventListener("click", async () => {
        bouton.disabled = true;
        try {
          await api("POST", "/v1/packs/encrypt", { file: f.file });
          avis("Chiffrement de " + f.file + " lancé. Le fichier chiffré apparaîtra dans cette liste.");
          setTimeout(() => lirePacks().catch(() => {}), 4000);
        } catch (e) { avis(e.message, true); bouton.disabled = false; }
      });
      ligne.append(bouton);
    }
    fichiers.append(ligne);
  }
  if (!p.files.length) fichiers.append(el("li", "Le dossier resource_packs ne contient aucun pack.", "vide"));
}

// ---- Console ----

$("commande-form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const champ = $("commande");
  const ligne = champ.value.trim();
  if (!ligne) return;
  try {
    await api("POST", "/v1/command", { command: ligne });
    avis("Commande lancée : " + ligne);
    champ.value = "";
    const trace = el("li");
    trace.append(el("span", ligne, "nom"), el("span", new Date().toLocaleTimeString("fr-FR"), "detail"));
    $("historique").prepend(trace);
    while ($("historique").children.length > 30) $("historique").lastChild.remove();
  } catch (e) { avis(e.message, true); }
});

$("recharger").addEventListener("click", async () => {
  if (!confirm("Recharger tous les plugins ?")) return;
  try {
    await api("POST", "/v1/plugins/reload");
    contenu = { items: null, blocks: null };
    images.clear();
    imagesDuJeu = null;
    avis("Plugins rechargés.");
  } catch (e) { avis(e.message, true); }
});

// ---- Départ ----

// Hors de la machine du serveur, la page doit être servie en https : sinon le jeton
// voyagerait en clair
if (location.protocol !== "https:" && !["127.0.0.1", "localhost", "[::1]"].includes(location.hostname)) $("en-clair").hidden = false;

if (jeton) ouvrir(); else { $("porte").hidden = false; $("jeton").focus(); }
