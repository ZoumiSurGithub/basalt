//! Images des objets et des blocs, pour le tableau de bord.
//!
//! Basalt ne livre aucune image du jeu. Il les lit là où elles se trouvent :
//! - dans **vos packs de ressources** (`resource_packs/`), pour vos objets et blocs — les
//!   packs chiffrés sont lus avec leur clé ;
//! - dans un **pack du jeu que vous fournissez**, déposé dans le dossier
//!   `vanilla_resource_pack/`, pour les objets et blocs du jeu.
//!
//! Le chemin d'une image se trouve comme le jeu le fait : `item_texture.json` relie un nom
//! d'icône à un fichier, `blocks.json` relie un bloc à ses textures, et
//! `terrain_texture.json` relie une texture de bloc à un fichier.

use super::*;
use crate::utils::packs::strip_comments;

/// Taille maximale d'un pack lu pour ses images, une fois décompressé
const MAX_PACK: usize = 96 * 1024 * 1024;
/// Taille maximale d'une image servie
const MAX_IMAGE: usize = 512 * 1024;

#[derive(Default)]
pub(super) struct Textures {
    /// Fichiers des packs de ressources chargés (chemin dans le pack → contenu)
    files: HashMap<String, Arc<Vec<u8>>>,
    /// Dossier du pack du jeu fourni par l'administrateur, s'il existe
    vanilla: Option<std::path::PathBuf>,
    /// Icône d'objet → fichier (sans extension)
    icons: HashMap<String, String>,
    /// Texture de bloc → fichier (sans extension)
    terrain: HashMap<String, String>,
    /// Bloc → sa texture (celle du côté, à défaut une autre)
    blocks: HashMap<String, String>,
    /// Bloc → la texture de son dessus, celle de l'inventaire d'abord (elle est déjà teinte)
    tops: HashMap<String, String>,
    /// Texture de bloc → la couleur que le jeu applique à sa partie marquée (`overlay_color`)
    overlays: HashMap<String, [u32; 3]>,
    /// Bloc → les textures de son dessus, de ses côtés et de son dessous
    faces: HashMap<String, [String; 3]>,
    /// Réponses déjà trouvées (ou introuvables)
    found: HashMap<String, Option<Arc<Vec<u8>>>>,
}

/// Première texture d'une entrée de `item_texture.json` ou `terrain_texture.json` : un
/// chemin, une liste de chemins, ou un objet qui en porte un.
fn first_path(v: &Json) -> Option<String> {
    match v {
        Json::Str(s) => Some(s.clone()),
        Json::Arr(list) => list.first().and_then(first_path),
        Json::Obj(_) => v.get("path").and_then(first_path).or_else(|| v.get("textures").and_then(first_path)),
        _ => None,
    }
}

/// Lit `texture_data` d'un `item_texture.json` ou d'un `terrain_texture.json`.
fn texture_data(text: &str, into: &mut HashMap<String, String>) {
    let Ok(doc) = crate::json::parse(&strip_comments(text.trim_start_matches('\u{feff}'))) else { return };
    let Some(Json::Obj(entries)) = doc.get("texture_data") else { return };
    for (key, value) in entries {
        if let Some(path) = value.get("textures").and_then(first_path) {
            into.entry(key.clone()).or_insert(path);
        }
    }
}

/// Lit les `overlay_color` d'un `terrain_texture.json` : le côté de l'herbe, par exemple, est
/// une image de terre dont la partie herbe, grise, est marquée pour recevoir cette couleur.
fn overlay_data(text: &str, into: &mut HashMap<String, [u32; 3]>) {
    let Ok(doc) = crate::json::parse(&strip_comments(text.trim_start_matches('\u{feff}'))) else { return };
    let Some(Json::Obj(entries)) = doc.get("texture_data") else { return };
    for (key, value) in entries {
        let first = match value.get("textures") {
            Some(Json::Arr(list)) => list.first(),
            other => other,
        };
        let Some(hex) = first.and_then(|t| t.get("overlay_color")).and_then(|c| c.as_str()) else { continue };
        let hex = hex.trim_start_matches('#');
        if let (true, Ok(n)) = (hex.len() == 6, u32::from_str_radix(hex, 16)) {
            into.entry(key.clone()).or_insert([n >> 16, (n >> 8) & 255, n & 255]);
        }
    }
}

/// Le nom sous lequel `blocks.json` connaît un bloc, quand ce n'est pas le sien.
fn listed_as(short: &str) -> &str {
    match short {
        "grass_block" => "grass",
        other => other,
    }
}

/// Lit `blocks.json` : pour chaque bloc, la texture de son côté.
fn block_faces(text: &str, into: &mut HashMap<String, String>) {
    let Ok(Json::Obj(entries)) = crate::json::parse(&strip_comments(text.trim_start_matches('\u{feff}'))) else { return };
    for (name, value) in entries {
        let face = match value.get("textures") {
            Some(Json::Str(s)) => Some(s.clone()),
            Some(faces @ Json::Obj(_)) => ["side", "north", "south", "east", "west", "up", "down"].iter().find_map(|f| faces.get(f).and_then(|v| v.as_str()).map(|s| s.to_string())),
            _ => None,
        };
        if let Some(face) = face {
            into.entry(name.trim_start_matches("minecraft:").to_string()).or_insert(face);
        }
    }
}

/// Lit `blocks.json` : pour chaque bloc, la texture de son dessus. Celle que le jeu montre
/// dans l'inventaire (`carried_textures`) passe d'abord : pour l'herbe et les feuilles, elle
/// est déjà verte, alors que celle du monde est grise et teinte selon le biome.
fn block_tops(text: &str, into: &mut HashMap<String, String>) {
    let Ok(Json::Obj(entries)) = crate::json::parse(&strip_comments(text.trim_start_matches('\u{feff}'))) else { return };
    for (name, value) in entries {
        let top = |key: &str| match value.get(key) {
            Some(Json::Str(s)) => Some(s.clone()),
            Some(faces @ Json::Obj(_)) => ["up", "side", "north", "south", "east", "west", "down"].iter().find_map(|f| faces.get(f).and_then(|v| v.as_str()).map(|s| s.to_string())),
            _ => None,
        };
        if let Some(face) = top("carried_textures").or_else(|| top("textures")) {
            into.entry(name.trim_start_matches("minecraft:").to_string()).or_insert(face);
        }
    }
}

/// Lit `blocks.json` : pour chaque bloc, les textures de son dessus, de ses côtés et de son
/// dessous ; celles de l'inventaire d'abord, face par face (voir `block_tops`).
fn block_sides(text: &str, into: &mut HashMap<String, [String; 3]>) {
    let Ok(Json::Obj(entries)) = crate::json::parse(&strip_comments(text.trim_start_matches('\u{feff}'))) else { return };
    for (name, value) in entries {
        let face = |key: &str, wanted: &[&str]| match value.get(key) {
            Some(Json::Str(s)) => Some(s.clone()),
            Some(faces @ Json::Obj(_)) => wanted.iter().find_map(|f| faces.get(f).and_then(|v| v.as_str()).map(|s| s.to_string())),
            _ => None,
        };
        let pick = |wanted: &[&str]| face("carried_textures", wanted).or_else(|| face("textures", wanted));
        let (up, side, down) = (pick(&["up"]), pick(&["side", "north", "south", "east", "west"]), pick(&["down"]));
        if let Some(any) = side.clone().or_else(|| up.clone()).or_else(|| down.clone()) {
            into.entry(name.trim_start_matches("minecraft:").to_string()).or_insert([up.unwrap_or_else(|| any.clone()), side.unwrap_or_else(|| any.clone()), down.unwrap_or(any)]);
        }
    }
}

/// Ce que la carte sait de l'apparence des blocs, tiré des textures des packs.
#[derive(Default)]
pub(super) struct Look {
    /// Bloc (sans `minecraft:`) → couleur moyenne de son dessus
    pub colours: HashMap<String, [u8; 3]>,
    /// Bloc → cases de la planche pour son dessus, ses côtés, son dessous ; puis vrai si
    /// ses côtés ont des trous (verre, feuilles)
    pub faces: HashMap<String, ([u16; 3], bool)>,
    /// La planche : toutes les textures côte à côte, en PNG (cases de 16 points, 64 par ligne)
    pub atlas: Vec<u8>,
}

/// Côté d'une case de la planche, et cases par ligne
const CELL: usize = 16;
const PER_ROW: usize = 64;

/// La teinte que le jeu applique à une texture grise, d'après le nom du bloc.
fn tint_for(short: &str, grey: bool) -> Option<[u32; 3]> {
    if short == "water" || short == "flowing_water" {
        Some([63, 118, 228])
    } else if grey && ["leaves", "vine"].iter().any(|k| short.contains(k)) {
        Some([119, 171, 47])
    } else if grey && ["grass", "fern", "lily", "stem", "bush"].iter().any(|k| short.contains(k)) {
        Some([145, 189, 89])
    } else {
        None
    }
}

/// Un chemin de fichier venu d'un pack est-il sans danger ? (pas de retour en arrière)
fn safe(path: &str) -> bool {
    !path.is_empty() && path.len() < 200 && !path.starts_with('/') && !path.contains('\\') && !path.contains(':') && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

impl Textures {
    /// Prépare la lecture : les fichiers des packs chargés, et les tables du pack du jeu.
    pub(super) fn load(packs: &[crate::utils::packs::ResourcePack], root: &std::path::Path) -> Textures {
        let mut t = Textures::default();
        for pack in packs {
            let files = if pack.key.is_empty() { crate::utils::pack_crypt::zip_entries(&pack.data) } else { crate::utils::pack_crypt::decrypt_pack(&pack.data, &pack.key) };
            let Ok(files) = files else { continue };
            if files.iter().map(|(_, d)| d.len()).sum::<usize>() > MAX_PACK {
                continue;
            }
            // (un pack zippé avec son dossier : les chemins repartent de sa racine)
            let root_len = files.iter().map(|(n, _)| n.as_str()).filter(|n| *n == "manifest.json" || n.ends_with("/manifest.json")).min_by_key(|n| n.len()).map(|n| n.len() - "manifest.json".len()).unwrap_or(0);
            for (name, data) in files {
                if let Some(short) = name.get(root_len..) {
                    t.files.entry(short.to_string()).or_insert_with(|| Arc::new(data));
                }
            }
        }
        let vanilla = root.join("vanilla_resource_pack");
        if vanilla.join("textures").is_dir() {
            t.vanilla = Some(vanilla);
        }
        // Les tables : celles de vos packs d'abord, puis celles du jeu
        for (file, kind) in [("textures/item_texture.json", 0), ("textures/terrain_texture.json", 1), ("blocks.json", 2)] {
            let mut texts: Vec<String> = t.files.get(file).map(|d| String::from_utf8_lossy(d).into_owned()).into_iter().collect();
            if let Some(dir) = &t.vanilla {
                texts.extend(std::fs::read_to_string(dir.join(file)).ok());
            }
            for text in texts {
                match kind {
                    0 => texture_data(&text, &mut t.icons),
                    1 => {
                        texture_data(&text, &mut t.terrain);
                        overlay_data(&text, &mut t.overlays);
                    }
                    _ => {
                        block_faces(&text, &mut t.blocks);
                        block_tops(&text, &mut t.tops);
                        block_sides(&text, &mut t.faces);
                    }
                }
            }
        }
        t
    }

    /// Le pack du jeu a-t-il été fourni ?
    pub(super) fn has_vanilla(&self) -> bool {
        self.vanilla.is_some()
    }

    /// Lit une image PNG par son chemin sans extension, dans vos packs puis dans celui du jeu.
    fn read(&self, base: &str) -> Option<Arc<Vec<u8>>> {
        let path = format!("{}.png", base.trim_end_matches(".png"));
        if !safe(&path) {
            return None;
        }
        let is_png = |d: &[u8]| d.len() <= MAX_IMAGE && d.starts_with(&[0x89, b'P', b'N', b'G']);
        if let Some(data) = self.files.get(&path).filter(|d| is_png(d)) {
            return Some(data.clone());
        }
        let dir = self.vanilla.as_ref()?;
        std::fs::read(dir.join(&path)).ok().filter(|d| is_png(d)).map(Arc::new)
    }

    /// Lit une texture par son chemin sans extension, PNG ou TGA, pour en tirer une couleur.
    fn read_any(&self, base: &str) -> Option<Arc<Vec<u8>>> {
        self.read(base).or_else(|| {
            let path = format!("{}.tga", base);
            if !safe(&path) {
                return None;
            }
            self.files.get(&path).cloned().or_else(|| std::fs::read(self.vanilla.as_ref()?.join(&path)).ok().map(Arc::new)).filter(|d| d.len() <= MAX_IMAGE)
        })
    }

    /// La couleur de chaque bloc vu du dessus : la moyenne de sa texture, quand un pack la
    /// fournit. Les blocs sans texture lisible sont absents (la carte garde sa couleur à elle).
    pub(super) fn block_colours(&self, names: &[String]) -> HashMap<String, [u8; 3]> {
        let mut out = HashMap::new();
        // (une même texture sert à plusieurs blocs : elle n'est lue qu'une fois)
        let mut seen: HashMap<String, Option<[u8; 3]>> = HashMap::new();
        for name in names {
            let short = name.trim_start_matches("minecraft:");
            let path = self.tops.get(listed_as(short)).or_else(|| self.tops.get(short)).and_then(|face| self.terrain.get(face)).cloned().unwrap_or_else(|| format!("textures/blocks/{}", short));
            let mean = *seen.entry(path.clone()).or_insert_with(|| self.read_any(&path).and_then(|d| crate::utils::image::average(&d)));
            let Some(mut c) = mean else { continue };
            // Une texture grise est teinte par le jeu : l'eau en bleu, la verdure en vert
            if short == "water" || short == "flowing_water" {
                c = [52, 98, 190];
            } else if let Some(t) = tint_for(short, c[0].abs_diff(c[1]) < 12 && c[1].abs_diff(c[2]) < 12) {
                c = [(c[0] as u32 * t[0] / 255) as u8, (c[1] as u32 * t[1] / 255) as u8, (c[2] as u32 * t[2] / 255) as u8];
            }
            out.insert(short.to_string(), c);
        }
        out
    }

    /// La peau de Steve du pack du jeu, pour dessiner les joueurs sur la carte.
    pub(super) fn steve(&self) -> Option<Arc<Vec<u8>>> {
        self.read("textures/entity/steve")
    }

    /// Une texture ramenée à une case de 16 points (la première image d'une texture animée),
    /// telle qu'elle est dans le fichier. Rend aussi : a-t-elle des trous ? est-elle grise ?
    fn cell(&self, path: &str, overlay: Option<[u32; 3]>) -> Option<(Vec<u8>, bool, bool)> {
        let image = crate::utils::image::decode(&self.read_any(path)?)?;
        let side = image.w.min(image.h);
        let mut out = Vec::with_capacity(CELL * CELL * 4);
        for y in 0..CELL {
            for x in 0..CELL {
                let at = ((y * side / CELL) * image.w + x * side / CELL) * 4;
                out.extend_from_slice(&image.rgba[at..at + 4]);
            }
        }
        // Une couleur à appliquer : les points marqués (opaques) la reçoivent, les autres
        // restent tels quels, et l'image entière est pleine
        if let Some(colour) = overlay {
            for p in out.chunks_exact_mut(4) {
                if p[3] >= 128 {
                    for c in 0..3 {
                        p[c] = (p[c] as u32 * colour[c] / 255) as u8;
                    }
                }
                p[3] = 255;
            }
        }
        let (mut sum, mut seen, mut gaps) = ([0u32; 3], 0u32, 0u32);
        for p in out.chunks_exact(4) {
            if p[3] < 128 {
                gaps += 1;
            } else {
                seen += 1;
                for c in 0..3 {
                    sum[c] += p[c] as u32;
                }
            }
        }
        if seen == 0 {
            return None;
        }
        let mean = [sum[0] / seen, sum[1] / seen, sum[2] / seen];
        // (des trous : plus d'un point sur douze ; un bord transparent ne compte pas)
        let holes = gaps * 12 > (CELL * CELL) as u32;
        Some((out, holes, mean[0].abs_diff(mean[1]) < 12 && mean[1].abs_diff(mean[2]) < 12))
    }

    /// L'apparence de chaque bloc pour la carte : couleur, et textures rangées sur une planche.
    pub(super) fn look(&self, names: &[String]) -> Look {
        let mut look = Look { colours: self.block_colours(names), ..Default::default() };
        // (la case 0 est blanche : c'est elle que prennent les blocs sans texture)
        let mut cells: Vec<Vec<u8>> = vec![vec![255; CELL * CELL * 4]];
        let mut read: HashMap<(String, Option<[u32; 3]>), Option<(Vec<u8>, bool, bool)>> = HashMap::new();
        let mut placed: HashMap<(String, Option<[u32; 3]>), u16> = HashMap::new();
        for name in names {
            let short = name.trim_start_matches("minecraft:");
            let keys = match self.faces.get(listed_as(short)).or_else(|| self.faces.get(short)) {
                Some(keys) => keys.clone(),
                None => [short.to_string(), short.to_string(), short.to_string()],
            };
            let mut found = [0u16; 3];
            let (mut any, mut holes) = (false, false);
            for (i, key) in keys.iter().enumerate() {
                let path = self.terrain.get(key).cloned().unwrap_or_else(|| format!("textures/blocks/{}", key));
                // (une même texture sert à plusieurs blocs ; teinte ou non, elle n'est rangée qu'une fois)
                let overlay = self.overlays.get(key).copied();
                let Some((raw, holed, grey)) = read.entry((path.clone(), overlay)).or_insert_with(|| self.cell(&path, overlay)).clone() else { continue };
                let tint = if overlay.is_some() { None } else { tint_for(short, grey) };
                let path = format!("{}{}", path, overlay.map(|o| format!("#{:?}", o)).unwrap_or_default());
                let index = match placed.get(&(path.clone(), tint)) {
                    Some(index) => *index,
                    None if cells.len() < PER_ROW * PER_ROW => {
                        let mut cell = raw;
                        if let Some(t) = tint {
                            for p in cell.chunks_exact_mut(4) {
                                for c in 0..3 {
                                    p[c] = (p[c] as u32 * t[c] / 255) as u8;
                                }
                            }
                        }
                        cells.push(cell);
                        placed.insert((path, tint), (cells.len() - 1) as u16);
                        (cells.len() - 1) as u16
                    }
                    None => continue,
                };
                found[i] = index;
                any = true;
                holes |= i == 1 && holed;
            }
            if any {
                look.faces.insert(short.to_string(), (found, holes));
            }
        }
        if cells.len() > 1 {
            let rows = (cells.len() + PER_ROW - 1) / PER_ROW;
            let (w, h) = (PER_ROW * CELL, rows * CELL);
            let mut rgba = vec![0u8; w * h * 4];
            for (i, cell) in cells.iter().enumerate() {
                let (cx, cy) = (i % PER_ROW * CELL, i / PER_ROW * CELL);
                for y in 0..CELL {
                    rgba[((cy + y) * w + cx) * 4..((cy + y) * w + cx + CELL) * 4].copy_from_slice(&cell[y * CELL * 4..(y + 1) * CELL * 4]);
                }
            }
            look.atlas = crate::utils::image::to_png(&crate::utils::image::Image { w, h, rgba });
        }
        look
    }

    fn block_image(&self, short: &str, custom_face: Option<&str>) -> Option<Arc<Vec<u8>>> {
        let face = custom_face.map(|f| f.to_string()).or_else(|| self.blocks.get(short).cloned());
        face.as_ref().and_then(|f| self.terrain.get(f)).and_then(|p| self.read(p)).or_else(|| self.read(&format!("textures/blocks/{}", short)))
    }

    /// L'image d'un objet (`kind` = "item") ou d'un bloc ("block"). `icon` et `face` : ce
    /// que la fiche d'un plugin a choisi, pour un objet ou un bloc créé.
    pub(super) fn image(&mut self, kind: &str, name: &str, icon: Option<&str>, face: Option<&str>) -> Option<Arc<Vec<u8>>> {
        let short = name.trim_start_matches("minecraft:");
        let key = format!("{}:{}", kind, short);
        if let Some(known) = self.found.get(&key) {
            return known.clone();
        }
        let image = if kind == "block" {
            self.block_image(short, face)
        } else {
            let wanted = icon.unwrap_or(short);
            self.icons
                .get(wanted)
                .and_then(|p| self.read(p))
                .or_else(|| self.read(&format!("textures/items/{}", wanted)))
                // (un objet qui est un bloc n'a pas d'icône à lui : on montre le bloc)
                .or_else(|| self.block_image(short, face))
        };
        self.found.insert(key, image.clone());
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::packs::{from_bytes, zip_store};

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2, 3];
    const MANIFEST: &str = r#"{"format_version":2,"header":{"name":"Essai","uuid":"11111111-2222-3333-4444-555555555555","version":[1,0,0]},"modules":[]}"#;

    fn pack() -> crate::utils::packs::ResourcePack {
        let items = r#"{ // commentaire
            "texture_data": { "rubis_icone": { "textures": "textures/items/rubis" }, "liste": { "textures": ["textures/items/rubis", "x"] } } }"#;
        let terrain = r#"{"texture_data": {"minerai_cote": {"textures": "textures/blocks/minerai"}, "objet": {"textures": {"path": "textures/blocks/minerai"}}}}"#;
        let blocks = r#"{"format_version":[1,1,0], "essai:minerai": {"textures": {"up": "autre", "side": "minerai_cote"}}, "minecraft:pierre": {"textures": "objet"}}"#;
        let zip = zip_store(&[
            ("Dossier/manifest.json", MANIFEST.as_bytes()),
            ("Dossier/textures/item_texture.json", items.as_bytes()),
            ("Dossier/textures/terrain_texture.json", terrain.as_bytes()),
            ("Dossier/blocks.json", blocks.as_bytes()),
            ("Dossier/textures/items/rubis.png", PNG),
            ("Dossier/textures/blocks/minerai.png", PNG),
            ("Dossier/textures/blocks/pas_une_image.png", b"texte"),
        ]);
        from_bytes(zip, "", "essai.mcpack").unwrap()
    }

    /// Un PNG d'un seul point, sans compression.
    fn dot(r: u8, g: u8, b: u8) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
        for (name, body) in [(b"IHDR", vec![0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]), (b"IDAT", vec![0x78, 0x01, 1, 4, 0, 0xfb, 0xff, 0, r, g, b]), (b"IEND", vec![])] {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&body);
            out.extend_from_slice(&[0, 0, 0, 0]);
        }
        out
    }

    #[test]
    fn block_colours_come_from_the_top_texture() {
        let terrain = r#"{"texture_data": {"herbe_dessus": {"textures": "textures/blocks/herbe_grise"}, "herbe_inventaire": {"textures": "textures/blocks/herbe_verte"}, "terre": {"textures": "textures/blocks/terre"}, "feuilles": {"textures": "textures/blocks/feuilles"}}}"#;
        let blocks = r#"{"grass_block": {"textures": {"up": "herbe_dessus", "side": "terre"}, "carried_textures": {"up": "herbe_inventaire", "side": "terre"}}, "dirt": {"textures": "terre"}, "oak_leaves": {"textures": "feuilles"}}"#;
        let (green, brown, grey, red) = (dot(100, 150, 60), dot(130, 90, 60), dot(200, 200, 200), dot(200, 10, 10));
        let zip = zip_store(&[
            ("manifest.json", MANIFEST.as_bytes()),
            ("textures/terrain_texture.json", terrain.as_bytes()),
            ("blocks.json", blocks.as_bytes()),
            ("textures/blocks/herbe_verte.png", &green),
            ("textures/blocks/herbe_grise.png", &grey),
            ("textures/blocks/terre.png", &brown),
            ("textures/blocks/feuilles.png", &grey),
            ("textures/blocks/essai_rubis.png", &red),
        ]);
        let t = Textures::load(&[from_bytes(zip, "", "couleurs.mcpack").unwrap()], &std::env::temp_dir().join("basalt-pas-de-pack-du-jeu"));
        let names: Vec<String> = ["minecraft:grass_block", "minecraft:dirt", "minecraft:oak_leaves", "essai_rubis", "minecraft:inconnu"].iter().map(|n| n.to_string()).collect();
        let c = t.block_colours(&names);
        // L'herbe : la texture de l'inventaire, déjà verte, plutôt que celle du monde, grise
        assert_eq!(c.get("grass_block"), Some(&[100, 150, 60]));
        assert_eq!(c.get("dirt"), Some(&[130, 90, 60]));
        // Des feuilles grises sont teintes en vert, comme le fait le jeu
        assert_eq!(c.get("oak_leaves"), Some(&[93, 134, 36]));
        // Un bloc sans entrée dans blocks.json : la texture qui porte son nom
        assert_eq!(c.get("essai_rubis"), Some(&[200, 10, 10]));
        assert_eq!(c.get("inconnu"), None);
        // La planche des textures : une case blanche, puis une par texture ; l'herbe a un
        // dessus et des côtés différents
        let look = t.look(&names);
        let (grass, holes) = look.faces["grass_block"];
        assert!(grass[0] != grass[1] && grass[0] > 0 && grass[1] == look.faces["dirt"].0[1] && !holes);
        let atlas = crate::utils::image::decode(&look.atlas).unwrap();
        assert_eq!((atlas.w, atlas.h, &atlas.rgba[0..4]), (1024, 16, &[255u8, 255, 255, 255][..]));
        let at = grass[0] as usize * 16 * 4;
        assert_eq!(&atlas.rgba[at..at + 4], &[100, 150, 60, 255]);
        assert!(!look.faces.contains_key("inconnu"));
    }

    /// Avec le vrai pack du jeu (`BASALT_PACK_ROOT` : le dossier qui contient
    /// `vanilla_resource_pack`) : écrit la planche dans /tmp et dit ce qui manque.
    #[test]
    #[ignore]
    fn the_real_pack_gives_most_blocks_a_texture() {
        let Ok(root) = std::env::var("BASALT_PACK_ROOT") else { return };
        let t = Textures::load(&[], std::path::Path::new(&root));
        let data = crate::data::GameData::load(std::path::Path::new("data")).expect("données du jeu");
        let names = data.block_name_list();
        let look = t.look(&names);
        std::fs::write("/tmp/planche.png", &look.atlas).unwrap();
        let missing: Vec<&str> = names.iter().map(|n| n.trim_start_matches("minecraft:")).filter(|n| !look.faces.contains_key(*n)).collect();
        println!("{} blocs, {} avec textures, {} couleurs ; sans : {:?}", names.len(), look.faces.len(), look.colours.len(), missing);
        for n in ["waterlily", "grass_block", "oak_leaves", "water", "stone", "oak_planks", "short_grass", "glass", "dirt", "sand", "oak_log", "poppy", "birch_leaves", "fern", "vine", "waterlily", "grass_path", "snow", "lava", "cobblestone", "red_wool"] {
            println!("{:14} {:?} {:?}", n, look.colours.get(n), look.faces.get(n));
        }
        assert!(look.faces.len() * 10 > names.len() * 8);
    }

    #[test]
    fn images_are_found_the_way_the_game_finds_them() {
        let nowhere = std::env::temp_dir().join("basalt-pas-de-pack-du-jeu");
        let mut t = Textures::load(&[pack()], &nowhere);
        assert!(!t.has_vanilla());
        // Objet d'un plugin : par l'icône de sa fiche
        assert_eq!(t.image("item", "essai:rubis", Some("rubis_icone"), None).as_deref().map(|d| d.as_slice()), Some(PNG));
        assert!(t.image("item", "essai:autre", Some("liste"), None).is_some());
        // Bloc : par blocks.json puis terrain_texture.json, le côté de préférence
        assert!(t.image("block", "essai:minerai", None, None).is_some());
        assert!(t.image("block", "minecraft:pierre", None, None).is_some());
        // Bloc d'un plugin : par la texture de sa fiche
        assert!(t.image("block", "essai:neuf", None, Some("minerai_cote")).is_some());
        // Un objet qui est un bloc montre le bloc ; un fichier nommé d'après lui suffit aussi
        assert!(t.image("item", "essai:minerai", None, None).is_some());
        assert!(t.image("block", "minerai", None, None).is_some());
        // Rien d'inventé : un nom inconnu, un fichier qui n'est pas une image
        assert!(t.image("item", "minecraft:diamond", None, None).is_none());
        assert!(t.image("block", "pas_une_image", None, None).is_none());
        assert!(!safe("../secret") && !safe("textures/../../x") && !safe("C:/x") && safe("textures/items/a"));
    }

    #[test]
    fn encrypted_packs_are_read_with_their_key() {
        let (sealed, key, _) = crate::utils::pack_crypt::encrypt_pack(&pack().data).unwrap();
        let mut t = Textures::load(&[from_bytes(sealed, &key, "essai-chiffre.mcpack").unwrap()], &std::env::temp_dir().join("basalt-pas-de-pack-du-jeu"));
        assert_eq!(t.image("item", "essai:rubis", Some("rubis_icone"), None).as_deref().map(|d| d.as_slice()), Some(PNG));
    }

    #[test]
    fn the_game_pack_is_read_from_its_folder() {
        let root = std::env::temp_dir().join(format!("basalt-textures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("vanilla_resource_pack");
        std::fs::create_dir_all(dir.join("textures/items")).unwrap();
        std::fs::write(dir.join("textures/item_texture.json"), r#"{"texture_data":{"diamond":{"textures":"textures/items/diamond"}}}"#).unwrap();
        std::fs::write(dir.join("textures/items/diamond.png"), PNG).unwrap();
        std::fs::write(dir.join("textures/items/apple.png"), PNG).unwrap();
        let mut t = Textures::load(&[], &root);
        assert!(t.has_vanilla());
        assert!(t.image("item", "minecraft:diamond", None, None).is_some());
        // Sans entrée dans la table, le fichier du même nom
        assert!(t.image("item", "minecraft:apple", None, None).is_some());
        assert!(t.image("item", "minecraft:emerald", None, None).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
