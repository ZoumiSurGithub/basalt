"use strict";

// La carte en relief : les chunks du serveur dessinés en cubes avec WebGL, sans bibliothèque.
// Le serveur envoie chaque chunk une fois, puis seulement quand il a changé. Les cubes
// cachés ne sont pas dessinés ; les plantes et autres petits blocs le sont en petit.

const Relief = (() => {
  const chunks = new Map();
  const cam = { lacet: 0.6, tangage: 0.7, recul: 90, y: 70, ySuivi: true };
  let toile = null, calque = null, gl = null, lieux = null, lignes = null;
  let entites = [], rayon = 6, enCours = false, image = 0, vue = null;
  // Les textures : la planche des blocs (64 cases de 16 points par ligne), un point blanc, la peau des joueurs
  let planche = null, lignesPlanche = 0, blanc = null, peau = null, peauHaute = true;

  const FACES = [
    [0, 1, 0, [0, 1, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0], 1.0],
    [0, -1, 0, [0, 0, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1], 0.5],
    [1, 0, 0, [1, 0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1], 0.68],
    [-1, 0, 0, [0, 0, 0, 0, 0, 1, 0, 1, 1, 0, 1, 0], 0.68],
    [0, 0, 1, [0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1], 0.82],
    [0, 0, -1, [0, 0, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0], 0.82],
  ];

  function demarrer(surface, etiquettes) {
    if (gl) return true;
    toile = surface; calque = etiquettes;
    gl = toile.getContext("webgl", { antialias: true, alpha: false });
    if (!gl) return false;
    const nuance = (type, source) => { const s = gl.createShader(type); gl.shaderSource(s, source); gl.compileShader(s); return s; };
    const prog = gl.createProgram();
    gl.attachShader(prog, nuance(gl.VERTEX_SHADER, "attribute vec3 p; attribute vec3 c; attribute vec2 t; uniform mat4 m; uniform vec3 o; uniform float k; varying vec3 vc; varying vec2 vt; varying float vd; void main() { vec4 q = m * vec4(p * k + o, 1.0); gl_Position = q; vc = c; vt = t; vd = q.w; }"));
    gl.attachShader(prog, nuance(gl.FRAGMENT_SHADER, "precision mediump float; uniform vec3 f; uniform float loin; uniform sampler2D s; varying vec3 vc; varying vec2 vt; varying float vd; void main() { vec4 x = texture2D(s, vt); if (x.a < 0.5) discard; gl_FragColor = vec4(mix(x.rgb * vc, f, clamp((vd - loin * 0.65) / (loin * 0.35), 0.0, 1.0)), 1.0); }"));
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) { gl = null; return false; }
    gl.useProgram(prog);
    lieux = { p: gl.getAttribLocation(prog, "p"), c: gl.getAttribLocation(prog, "c"), t: gl.getAttribLocation(prog, "t"), k: gl.getUniformLocation(prog, "k"), m: gl.getUniformLocation(prog, "m"), o: gl.getUniformLocation(prog, "o"), f: gl.getUniformLocation(prog, "f"), loin: gl.getUniformLocation(prog, "loin") };
    gl.enableVertexAttribArray(lieux.p);
    gl.enableVertexAttribArray(lieux.c);
    gl.enableVertexAttribArray(lieux.t);
    gl.enable(gl.DEPTH_TEST);
    lignes = gl.createBuffer();
    blanc = texture(new ImageData(new Uint8ClampedArray([255, 255, 255, 255]), 1, 1));
    peau = texture(peauUnie());
    // La planche des textures des blocs et la peau de Steve, si le serveur a le pack du jeu
    image_("/v1/world/atlas").then((img) => {
      if (!img) return;
      planche = texture(img);
      lignesPlanche = img.height / 16;
      for (const ch of chunks.values()) ch.sale = true;
      dessiner();
    });
    image_("/v1/world/skin").then((img) => {
      if (!img) return;
      peau = texture(img);
      peauHaute = img.height >= img.width;
      dessiner();
    });
    return true;
  }

  function texture(source) {
    const t = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  }

  async function image_(chemin) {
    try {
      const reponse = await fetch(chemin, { headers: { Authorization: "Bearer " + jeton }, cache: "no-store" });
      if (!reponse.ok) return null;
      return await createImageBitmap(await reponse.blob());
    } catch (e) { return null; }
  }

  // Sans le pack du jeu : un personnage aux couleurs unies, rangées comme sur une vraie peau
  function peauUnie() {
    const c = document.createElement("canvas");
    c.width = c.height = 64;
    const d = c.getContext("2d");
    for (const [couleur, x, y, l, h] of [["#b98a6b", 0, 0, 32, 16], ["#3a2a1c", 8, 0, 8, 8], ["#17a3a8", 16, 16, 24, 16], ["#b98a6b", 40, 16, 16, 16], ["#b98a6b", 32, 48, 16, 16], ["#3b3fa3", 0, 16, 16, 16], ["#3b3fa3", 16, 48, 16, 16]]) {
      d.fillStyle = couleur;
      d.fillRect(x, y, l, h);
    }
    return d.getImageData(0, 0, 64, 64);
  }

  // ---- Les chunks ----

  function lire(octets, cx, cz, version) {
    const d = new DataView(octets.buffer, octets.byteOffset, octets.byteLength);
    const base = d.getInt16(0, true), haut = d.getUint16(2, true), n = octets[4];
    const palette = octets.slice(5, 5 + n * 10);
    const blocs = new Uint8Array(256 * haut);
    let i = 5 + n * 10;
    for (let col = 0; col < 256; col++) {
      for (let y = 0; y < haut && i + 1 < octets.length;) {
        const [longueur, entree] = [octets[i], octets[i + 1]];
        i += 2;
        if (!longueur) return null;
        blocs.fill(entree, col * haut + y, col * haut + Math.min(haut, y + longueur));
        y += longueur;
      }
    }
    const ancien = chunks.get(cx + "," + cz);
    return { cx, cz, version, base, haut, palette, blocs, tampon: ancien ? ancien.tampon : null, sommets: ancien ? ancien.sommets : 0, sale: true };
  }

  // Entrée de palette d'un bloc du chunk : 0 pour l'air, -1 sous ce que le serveur a envoyé
  function entree(ch, lx, y, lz) {
    if (y < ch.base) return -1;
    if (y >= ch.base + ch.haut) return 0;
    return ch.blocs[(lz * 16 + lx) * ch.haut + (y - ch.base)];
  }

  // Ce bloc cache-t-il la face de son voisin ? (un chunk pas encore reçu cache : pas de mur au bord)
  function cache(ch, lx, y, lz) {
    if (!ch) return true;
    const e = entree(ch, lx, y, lz);
    return e < 0 || (e > 0 && !(ch.palette[e * 10 + 3] & 5));
  }

  // Les sommets d'un maillage : position (en seizièmes ou soixante-quatrièmes de bloc),
  // couleur, puis place dans la texture ; seize octets chacun
  function ranger(v) {
    const sommets = v.length / 8;
    const octets = new ArrayBuffer(sommets * 16);
    const [positions, couleurs, places] = [new Int16Array(octets), new Uint8Array(octets), new Uint16Array(octets)];
    for (let i = 0; i < sommets; i++) {
      for (let k = 0; k < 3; k++) { positions[i * 8 + k] = Math.max(-32000, Math.min(32000, Math.round(v[i * 8 + k]))); couleurs[i * 16 + 8 + k] = v[i * 8 + 3 + k]; }
      places[i * 8 + 6] = Math.max(0, Math.min(65535, Math.round(v[i * 8 + 6] * 65535)));
      places[i * 8 + 7] = Math.max(0, Math.min(65535, Math.round(v[i * 8 + 7] * 65535)));
    }
    return [octets, sommets];
  }

  function pointer() {
    gl.vertexAttribPointer(lieux.p, 3, gl.SHORT, false, 16, 0);
    gl.vertexAttribPointer(lieux.c, 3, gl.UNSIGNED_BYTE, true, 16, 8);
    gl.vertexAttribPointer(lieux.t, 2, gl.UNSIGNED_SHORT, true, 16, 12);
  }

  function mailler(ch) {
    const voisins = { "1,0": chunks.get((ch.cx + 1) + "," + ch.cz), "-1,0": chunks.get((ch.cx - 1) + "," + ch.cz), "0,1": chunks.get(ch.cx + "," + (ch.cz + 1)), "0,-1": chunks.get(ch.cx + "," + (ch.cz - 1)) };
    const v = [];
    // Une face : ses quatre coins, sa couleur, et la case de la planche qui la couvre
    const face = (x, y, z, coins, taille, r, g, b, caseDe, axe) => {
      const [cu, cv] = planche ? [caseDe % 64, Math.floor(caseDe / 64)] : [0, 0];
      for (const k of [0, 1, 2, 0, 2, 3]) {
        const [px, py, pz] = [coins[k * 3], coins[k * 3 + 1], coins[k * 3 + 2]];
        const [u, w] = axe === 1 ? [px, pz] : [axe === 0 ? pz : px, 1 - py];
        v.push(x + px * taille[0], y + py * taille[1], z + pz * taille[0], r, g, b, planche ? (cu * 16 + 0.05 + u * 15.9) / 1024 : 0, planche ? (cv * 16 + 0.05 + w * 15.9) / (lignesPlanche * 16) : 0);
      }
    };
    for (let lz = 0; lz < 16; lz++) {
      for (let lx = 0; lx < 16; lx++) {
        for (let dy = 0; dy < ch.haut; dy++) {
          const e = ch.blocs[(lz * 16 + lx) * ch.haut + dy];
          if (!e) continue;
          const pal = ch.palette, at = e * 10;
          const drapeaux = pal[at + 3];
          const cases = [pal[at + 4] | (pal[at + 5] << 8), pal[at + 6] | (pal[at + 7] << 8), pal[at + 8] | (pal[at + 9] << 8)];
          const y = ch.base + dy;
          // (un bloc qui a sa texture est blanc dessous : c'est elle qui donne la couleur)
          const teinte = (caseDe) => (planche && caseDe ? [255, 255, 255] : [pal[at], pal[at + 1], pal[at + 2]]);
          if (drapeaux & 1) {
            if (planche && cases[1]) {
              // Une plante : deux plans croisés qui portent sa texture
              for (const coins of [[0, 0, 0, 1, 0, 1, 1, 1, 1, 0, 1, 0], [1, 0, 0, 0, 0, 1, 0, 1, 1, 1, 1, 0]]) face(lx * 16, dy * 16, lz * 16, coins, [16, 16], 235, 235, 235, cases[1], 2);
            } else {
              for (const [, ay, , coins, ombre] of FACES) face(lx * 16 + 5, dy * 16, lz * 16 + 5, coins, [6, 10], pal[at] * ombre, pal[at + 1] * ombre, pal[at + 2] * ombre, 0, ay ? 1 : 2);
            }
            continue;
          }
          for (const [ax, ay, az, coins, ombre] of FACES) {
            const [nx, nz] = [lx + ax, lz + az];
            const dehors = nx < 0 || nx > 15 || nz < 0 || nz > 15;
            const voisin = dehors ? voisins[ax + "," + az] : ch;
            if (cache(voisin, nx & 15, y + ay, nz & 15)) continue;
            // (entre deux blocs à trous, comme deux feuilles, pas de face)
            if ((drapeaux & 4) && voisin) { const autre = entree(voisin, nx & 15, y + ay, nz & 15); if (autre > 0 && (voisin.palette[autre * 10 + 3] & 4)) continue; }
            const caseDe = cases[ay > 0 ? 0 : ay < 0 ? 2 : 1];
            const [r, g, b] = teinte(caseDe);
            face(lx * 16, dy * 16, lz * 16, coins, [16, 16], r * ombre, g * ombre, b * ombre, caseDe, ay ? 1 : ax ? 0 : 2);
          }
        }
      }
    }
    const [octets, sommets] = ranger(v);
    if (!ch.tampon) ch.tampon = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, ch.tampon);
    gl.bufferData(gl.ARRAY_BUFFER, octets, gl.STATIC_DRAW);
    ch.sommets = sommets;
    ch.sale = false;
  }

  // Un joueur : le personnage du jeu, à sa place et tourné comme lui. Chaque partie est
  // une boîte dont les six faces prennent leur morceau de la peau.
  const CORPS = [[-4, 24, -4, 8, 8, 8, 0, 0], [-4, 12, -2, 8, 12, 4, 16, 16], [-8, 12, -2, 4, 12, 4, 40, 16], [4, 12, -2, 4, 12, 4, 32, 48], [-4, 0, -2, 4, 12, 4, 0, 16], [0, 0, -2, 4, 12, 4, 16, 48]];

  function personnage(v, e, x, z) {
    const angle = (e.yaw || 0) * Math.PI / 180;
    const [co, si] = [Math.cos(angle), Math.sin(angle)];
    const haut = peauHaute ? 64 : 32;
    for (const [x0, y0, z0, w, h, d, U0, V0] of CORPS) {
      // (une peau d'ancien format n'a qu'un bras et qu'une jambe : les deux côtés la partagent)
      const [U, V] = !peauHaute && V0 === 48 ? [U0 === 32 ? 40 : 0, 16] : [U0, V0];
      const [x1, y1, z1] = [x0 + w, y0 + h, z0 + d];
      const faces = [
        [[x0, y0, z1, U + d, V + d + h], [x1, y0, z1, U + d + w, V + d + h], [x1, y1, z1, U + d + w, V + d], [x0, y1, z1, U + d, V + d], 0.9],
        [[x1, y0, z0, U + 2 * d + w, V + d + h], [x0, y0, z0, U + 2 * d + 2 * w, V + d + h], [x0, y1, z0, U + 2 * d + 2 * w, V + d], [x1, y1, z0, U + 2 * d + w, V + d], 0.75],
        [[x0, y0, z0, U, V + d + h], [x0, y0, z1, U + d, V + d + h], [x0, y1, z1, U + d, V + d], [x0, y1, z0, U, V + d], 0.7],
        [[x1, y0, z1, U + d + w, V + d + h], [x1, y0, z0, U + 2 * d + w, V + d + h], [x1, y1, z0, U + 2 * d + w, V + d], [x1, y1, z1, U + d + w, V + d], 0.7],
        [[x0, y1, z0, U + d, V], [x1, y1, z0, U + d + w, V], [x1, y1, z1, U + d + w, V + d], [x0, y1, z1, U + d, V + d], 1],
        [[x0, y0, z0, U + d + w, V], [x1, y0, z0, U + d + 2 * w, V], [x1, y0, z1, U + d + 2 * w, V + d], [x0, y0, z1, U + d + w, V + d], 0.5],
      ];
      for (const f of faces) {
        for (const k of [0, 1, 2, 0, 2, 3]) {
          const [mx, my, mz, u, w2] = f[k];
          const [rx, rz] = [(mx * co - mz * si) * 0.05625, (mx * si + mz * co) * 0.05625];
          v.push((e.x + rx - x) * 64, (e.y + my * 0.05625 - cam.y) * 64, (e.z + rz - z) * 64, 255 * f[4], 255 * f[4], 255 * f[4], u / 64, w2 / haut);
        }
      }
    }
  }

  function salir(cx, cz) {
    for (const [dx, dz] of [[0, 0], [1, 0], [-1, 0], [0, 1], [0, -1]]) {
      const ch = chunks.get((cx + dx) + "," + (cz + dz));
      if (ch) ch.sale = true;
    }
  }

  // Demande au serveur les chunks autour du point regardé : ceux qu'on n'a pas, et ceux
  // dont la version a changé depuis. Ceux qui sont loin sont oubliés.
  async function suivre(x, z) {
    if (!gl || enCours) return;
    enCours = true;
    try {
      const [ccx, ccz] = [Math.floor(x / 16), Math.floor(z / 16)];
      const voulus = [];
      for (let dz = -rayon; dz <= rayon; dz++) for (let dx = -rayon; dx <= rayon; dx++) voulus.push([ccx + dx, ccz + dz, dx * dx + dz * dz]);
      voulus.sort((a, b) => a[2] - b[2]);
      for (let debut = 0; debut < voulus.length; debut += 100) {
        const liste = voulus.slice(debut, debut + 100).map(([cx, cz]) => { const ch = chunks.get(cx + "," + cz); return cx + "." + cz + "." + (ch ? ch.version : -1); }).join(",");
        const reponse = await fetch("/v1/world/chunks?list=" + liste, { headers: { Authorization: "Bearer " + jeton }, cache: "no-store" });
        if (!reponse.ok) break;
        const octets = new Uint8Array(await reponse.arrayBuffer());
        const d = new DataView(octets.buffer);
        for (let i = 0; i + 17 <= octets.length;) {
          const [cx, cz, version, etat, taille] = [d.getInt32(i, true), d.getInt32(i + 4, true), d.getUint32(i + 8, true), octets[i + 12], d.getUint32(i + 13, true)];
          i += 17;
          if (etat === 2) {
            const ch = lire(octets.subarray(i, i + taille), cx, cz, version);
            if (ch) { chunks.set(cx + "," + cz, ch); salir(cx, cz); }
          } else if (etat === 0 && chunks.has(cx + "," + cz)) {
            oublier(cx + "," + cz);
            salir(cx, cz);
          }
          i += taille;
        }
        dessiner();
      }
      for (const [cle, ch] of chunks) if (Math.abs(ch.cx - ccx) > rayon + 2 || Math.abs(ch.cz - ccz) > rayon + 2) oublier(cle);
    } catch (e) { /* le prochain tour redemandera */ }
    enCours = false;
  }

  function oublier(cle) {
    const ch = chunks.get(cle);
    if (ch && ch.tampon) gl.deleteBuffer(ch.tampon);
    chunks.delete(cle);
  }

  function bloc(x, y, z) {
    const ch = chunks.get(Math.floor(x / 16) + "," + Math.floor(z / 16));
    return ch ? entree(ch, x & 15, y, z & 15) : 0;
  }

  // Hauteur du sol sous un point, si le chunk est là
  function sol(x, z) {
    const ch = chunks.get(Math.floor(x / 16) + "," + Math.floor(z / 16));
    if (!ch) return null;
    const col = ((Math.floor(z) & 15) * 16 + (Math.floor(x) & 15)) * ch.haut;
    for (let dy = ch.haut - 1; dy >= 0; dy--) if (ch.blocs[col + dy]) return ch.base + dy + 1;
    return ch.base;
  }

  // ---- La caméra ----

  function calculerVue(x, z) {
    const echelle = window.devicePixelRatio || 1;
    const [l, h] = [toile.clientWidth, toile.clientHeight];
    for (const t of [toile, calque]) {
      if (t.width !== Math.round(l * echelle) || t.height !== Math.round(h * echelle)) { t.width = Math.round(l * echelle); t.height = Math.round(h * echelle); }
    }
    const [ct, st] = [Math.cos(cam.tangage), Math.sin(cam.tangage)];
    // (l'œil : derrière le point regardé, au sud quand le lacet est nul — le nord est en haut)
    const zAxe = [ct * Math.sin(cam.lacet), st, ct * Math.cos(cam.lacet)];
    const oeil = zAxe.map((c) => c * cam.recul);
    let xAxe = [zAxe[2], 0, -zAxe[0]];
    const n = Math.hypot(xAxe[0], xAxe[2]) || 1;
    xAxe = [xAxe[0] / n, 0, xAxe[2] / n];
    const yAxe = [zAxe[1] * xAxe[2] - zAxe[2] * xAxe[1], zAxe[2] * xAxe[0] - zAxe[0] * xAxe[2], zAxe[0] * xAxe[1] - zAxe[1] * xAxe[0]];
    const produit = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    const regard = [xAxe[0], yAxe[0], zAxe[0], 0, xAxe[1], yAxe[1], zAxe[1], 0, xAxe[2], yAxe[2], zAxe[2], 0, -produit(xAxe, oeil), -produit(yAxe, oeil), -produit(zAxe, oeil), 1];
    const loin = cam.recul + rayon * 16 * 1.3;
    const [f, pres] = [1 / Math.tan(0.5), Math.max(0.5, cam.recul / 50)];
    const perspective = [f / (l / h), 0, 0, 0, 0, f, 0, 0, 0, 0, (loin + pres) / (pres - loin), -1, 0, 0, 2 * loin * pres / (pres - loin), 0];
    const m = new Float32Array(16);
    for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) m[c * 4 + r] = perspective[r] * regard[c * 4] + perspective[4 + r] * regard[c * 4 + 1] + perspective[8 + r] * regard[c * 4 + 2] + perspective[12 + r] * regard[c * 4 + 3];
    vue = { m, l, h, echelle, loin, oeil, xAxe, yAxe, zAxe, x, z };
  }

  function versEcran(wx, wy, wz) {
    const [x, y, z] = [wx - vue.x, wy - cam.y, wz - vue.z];
    const m = vue.m;
    const w = m[3] * x + m[7] * y + m[11] * z + m[15];
    if (w <= 0.1) return null;
    return [((m[0] * x + m[4] * y + m[8] * z + m[12]) / w * 0.5 + 0.5) * vue.l, (0.5 - (m[1] * x + m[5] * y + m[9] * z + m[13]) / w * 0.5) * vue.h, w];
  }

  // ---- Le dessin ----

  function couleurDe(nom) {
    const hexa = getComputedStyle(document.documentElement).getPropertyValue(nom).trim().replace("#", "");
    return [0, 2, 4].map((i) => parseInt(hexa.slice(i, i + 2), 16) || 0);
  }

  function boite(v, x0, y0, z0, x1, y1, z1, c) {
    const coins = [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1], [x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]];
    for (const [a, b] of [[0, 1], [1, 2], [2, 3], [3, 0], [4, 5], [5, 6], [6, 7], [7, 4], [0, 4], [1, 5], [2, 6], [3, 7]]) v.push(...coins[a].map((n) => n * 64), ...c, 0, 0, ...coins[b].map((n) => n * 64), ...c, 0, 0);
  }

  function dessiner() {
    if (!gl || image) return;
    image = requestAnimationFrame(() => { image = 0; tracer(); });
  }

  function tracer() {
    if (!toile.clientWidth) return;
    const [x, z] = [carte.x, carte.z];
    // La caméra suit le relief sous le point regardé
    const pied = sol(x, z);
    let encore = false;
    if (cam.ySuivi && pied !== null && Math.abs(pied - cam.y) > 0.05) { cam.y += (pied - cam.y) * 0.25; encore = true; }
    calculerVue(x, z);
    // Quelques chunks maillés par image, les plus proches d'abord
    const sales = [...chunks.values()].filter((ch) => ch.sale).sort((a, b) => Math.hypot(a.cx * 16 + 8 - x, a.cz * 16 + 8 - z) - Math.hypot(b.cx * 16 + 8 - x, b.cz * 16 + 8 - z));
    for (const ch of sales.slice(0, 6)) mailler(ch);
    if (sales.length > 6) encore = true;
    const fond = couleurDe("--fond");
    gl.viewport(0, 0, toile.width, toile.height);
    gl.clearColor(fond[0] / 255, fond[1] / 255, fond[2] / 255, 1);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.uniformMatrix4fv(lieux.m, false, vue.m);
    gl.uniform3f(lieux.f, fond[0] / 255, fond[1] / 255, fond[2] / 255);
    gl.uniform1f(lieux.loin, vue.loin);
    gl.uniform1f(lieux.k, 1 / 16);
    gl.bindTexture(gl.TEXTURE_2D, planche || blanc);
    for (const ch of chunks.values()) {
      if (!ch.sommets) continue;
      gl.uniform3f(lieux.o, ch.cx * 16 - x, ch.base - cam.y, ch.cz * 16 - z);
      gl.bindBuffer(gl.ARRAY_BUFFER, ch.tampon);
      pointer();
      gl.drawArrays(gl.TRIANGLES, 0, ch.sommets);
    }
    // Les joueurs, en personnages
    const envoyer = (v, texture_, mode) => {
      if (!v.length) return;
      const [octets, sommets] = ranger(v);
      gl.bindTexture(gl.TEXTURE_2D, texture_);
      gl.bindBuffer(gl.ARRAY_BUFFER, lignes);
      gl.bufferData(gl.ARRAY_BUFFER, octets, gl.DYNAMIC_DRAW);
      pointer();
      gl.drawArrays(mode, 0, sommets);
    };
    gl.uniform1f(lieux.k, 1 / 64);
    gl.uniform3f(lieux.o, 0, 0, 0);
    const corps = [];
    for (const e of entites) if (e.player) personnage(corps, e, x, z);
    envoyer(corps, peau, gl.TRIANGLES);
    // Les boîtes : chaque créature à sa vraie taille, et le bloc choisi
    const v = [];
    for (const e of entites) if (!e.player) boite(v, e.x - e.w / 2 - x, e.y - cam.y, e.z - e.w / 2 - z, e.x + e.w / 2 - x, e.y + e.h - cam.y, e.z + e.w / 2 - z, [244, 241, 234]);
    const c = carte.choix;
    if (c) boite(v, c.x - 0.02 - x, c.y - 0.02 - cam.y, c.z - 0.02 - z, c.x + 1.02 - x, c.y + 1.02 - cam.y, c.z + 1.02 - z, couleurDe("--lave-vive"));
    envoyer(v, blanc, gl.LINES);
    // Les noms, au-dessus des boîtes : les joueurs toujours, les créatures quand elles sont proches
    const d = calque.getContext("2d");
    d.setTransform(vue.echelle, 0, 0, vue.echelle, 0, 0);
    d.clearRect(0, 0, vue.l, vue.h);
    d.font = "600 12px " + getComputedStyle(document.documentElement).getPropertyValue("--famille");
    d.textBaseline = "middle";
    d.textAlign = "center";
    for (const e of entites) {
      const p = versEcran(e.x, e.y + e.h + 0.35, e.z);
      if (!p || p[0] < -60 || p[1] < -20 || p[0] > vue.l + 60 || p[1] > vue.h + 20 || (!e.player && p[2] > 70)) continue;
      const large = d.measureText(e.name).width + 10;
      d.fillStyle = e.player ? "rgb(20 22 26 / 0.85)" : "rgb(20 22 26 / 0.6)";
      d.fillRect(p[0] - large / 2, p[1] - 9, large, 18);
      d.fillStyle = e.player ? "#ffa15e" : "#f4f1ea";
      d.fillText(e.name, p[0], p[1] + 1);
    }
    if (encore) dessiner();
  }

  // ---- Viser un bloc ----

  // Le premier bloc touché par le regard qui passe par ce point de l'écran
  function viser(px, pz) {
    if (!vue) return null;
    const [nx, ny] = [(px / vue.l) * 2 - 1, 1 - (pz / vue.h) * 2];
    const t = Math.tan(0.5);
    const dir = [0, 1, 2].map((i) => -vue.zAxe[i] + vue.xAxe[i] * nx * t * (vue.l / vue.h) + vue.yAxe[i] * ny * t);
    const pos = [vue.x + vue.oeil[0], cam.y + vue.oeil[1], vue.z + vue.oeil[2]];
    const cas = pos.map(Math.floor);
    const pas = dir.map((c) => (c > 0 ? 1 : -1));
    const delta = dir.map((c) => (c ? Math.abs(1 / c) : Infinity));
    const prochain = [0, 1, 2].map((i) => (dir[i] > 0 ? cas[i] + 1 - pos[i] : pos[i] - cas[i]) * delta[i]);
    for (let n = 0; n < 900; n++) {
      const axe = prochain[0] < prochain[1] ? (prochain[0] < prochain[2] ? 0 : 2) : (prochain[1] < prochain[2] ? 1 : 2);
      cas[axe] += pas[axe];
      prochain[axe] += delta[axe];
      if (cas[1] < -64) return null;
      if (bloc(cas[0], cas[1], cas[2]) !== 0) return { x: cas[0], y: cas[1], z: cas[2] };
    }
    return null;
  }

  // ---- Les gestes ----

  function brancher(surTouche, changer) {
    const doigts = new Map();
    let bouge = false, ecart = 0;
    const glisser = (dx, dz) => {
      // (déplacer le point regardé, dans le plan du sol, selon l'orientation de la caméra)
      const k = cam.recul / vue.h * 1.3;
      carte.x -= (vue.xAxe[0] * dx - vue.xAxe[2] * dz) * k;
      carte.z -= (vue.xAxe[2] * dx + vue.xAxe[0] * dz) * k;
      cam.ySuivi = true;
      changer();
    };
    toile.addEventListener("contextmenu", (e) => e.preventDefault());
    toile.addEventListener("pointerdown", (e) => {
      doigts.set(e.pointerId, { x: e.clientX, z: e.clientY, bouton: e.button, maj: e.shiftKey });
      bouge = doigts.size > 1;
      ecart = 0;
      toile.setPointerCapture(e.pointerId);
    });
    toile.addEventListener("pointermove", (e) => {
      const doigt = doigts.get(e.pointerId);
      if (!doigt || !vue) return;
      const [dx, dz] = [e.clientX - doigt.x, e.clientY - doigt.z];
      ecart += Math.abs(dx) + Math.abs(dz);
      if (ecart > 6) bouge = true;
      doigt.x = e.clientX; doigt.z = e.clientY;
      if (!bouge) return;
      if (doigts.size === 2) {
        // Deux doigts : déplacer, et pincer pour s'approcher
        const [a, b] = [...doigts.values()];
        const distance = Math.hypot(a.x - b.x, a.z - b.z);
        if (doigt.pince) cam.recul = Math.max(12, Math.min(260, cam.recul * doigt.pince / distance));
        for (const autre of doigts.values()) autre.pince = distance;
        glisser(dx / 2, dz / 2);
      } else if (doigt.bouton === 2 || doigt.maj) {
        glisser(dx, dz);
      } else {
        cam.lacet -= dx * 0.006;
        cam.tangage = Math.max(0.12, Math.min(1.5, cam.tangage + dz * 0.006));
      }
      dessiner();
    });
    const lacher = (e) => {
      const etait = doigts.size;
      doigts.delete(e.pointerId);
      for (const autre of doigts.values()) autre.pince = 0;
      if (etait !== 1 || bouge || e.type === "pointercancel") return;
      const bord = toile.getBoundingClientRect();
      surTouche(viser(e.clientX - bord.left, e.clientY - bord.top));
    };
    toile.addEventListener("pointerup", lacher);
    toile.addEventListener("pointercancel", lacher);
    toile.addEventListener("wheel", (e) => { e.preventDefault(); approcher(e.deltaY < 0 ? 1 : -1); }, { passive: false });
    toile.addEventListener("keydown", (e) => {
      const gestes = { ArrowLeft: [40, 0], ArrowRight: [-40, 0], ArrowUp: [0, 40], ArrowDown: [0, -40] };
      if (gestes[e.key] && vue) { e.preventDefault(); glisser(gestes[e.key][0], gestes[e.key][1]); dessiner(); }
      if (e.key === "+" || e.key === "-") { e.preventDefault(); approcher(e.key === "+" ? 1 : -1); }
    });
  }

  function approcher(sens) {
    cam.recul = Math.max(12, Math.min(260, cam.recul * (sens > 0 ? 0.8 : 1.25)));
    dessiner();
  }

  return {
    demarrer, brancher, suivre, dessiner, approcher,
    regler(largeur) { rayon = largeur < 700 ? 4 : 6; },
    poserEntites(liste) { entites = liste; dessiner(); },
    recentrer() { cam.ySuivi = true; dessiner(); },
    // (après une modification : la version du chunk a changé, le prochain tour le renvoie)
    compte() { return chunks.size; },
  };
})();
