//! Enregistrement des créatures par région, comme le jeu (qui les range avec leur chunk).
//!
//! Le monde est découpé en régions de 32 × 32 chunks ; chacune a son fichier
//! `entities/r.<x>.<z>.json` : les créatures qui s'y trouvent et les chunks déjà peuplés.
//! Seules les créatures proches d'un joueur sont en mémoire. Quand plus personne n'est près
//! de leur chunk, elles retournent dans leur région (figées, comme dans le jeu) ; quand un
//! joueur revient, elles en ressortent. Les fichiers sont lus et écrits sur le fil des
//! écritures, dans l'ordre : une région relue après un enregistrement est à jour.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use crate::json::Value as Json;

use super::mobs::MobRecord;

/// Région d'un chunk (32 × 32 chunks).
pub(super) fn region_of(chunk: (i32, i32)) -> (i32, i32) {
    (chunk.0 >> 5, chunk.1 >> 5)
}

/// Chunk d'une position.
pub(super) fn chunk_of(x: f32, z: f32) -> (i32, i32) {
    ((x.floor() as i32) >> 4, (z.floor() as i32) >> 4)
}

/// Une région en mémoire : les créatures rangées (par chunk) et les chunks déjà peuplés.
#[derive(Default, Debug)]
pub(super) struct Region {
    pub chunks: HashMap<(i32, i32), Vec<MobRecord>>,
    pub populated: HashSet<(i32, i32)>,
    /// Changée depuis le dernier enregistrement
    pub dirty: bool,
}

impl Region {
    /// Lit un fichier de région (ou l'ancien `mobs.json`, de même forme).
    pub fn parse(text: &str) -> Option<Region> {
        let doc = crate::json::parse(text).ok()?;
        let mut region = Region::default();
        // « populated » (régions) ou « chunks » (ancien fichier unique)
        for key in ["populated", "chunks"] {
            for c in doc.get(key).and_then(|c| c.as_arr()).unwrap_or(&[]) {
                if let Some((x, z)) = c.as_str().and_then(|s| s.split_once(',')) {
                    if let (Ok(x), Ok(z)) = (x.parse(), z.parse()) {
                        region.populated.insert((x, z));
                    }
                }
            }
        }
        for v in doc.get("mobs").and_then(|m| m.as_arr()).unwrap_or(&[]) {
            if let Some(r) = MobRecord::from_json(v) {
                region.chunks.entry(chunk_of(r.pos.0, r.pos.2)).or_default().push(r);
            }
        }
        Some(region)
    }

    /// Texte du fichier : les créatures rangées, plus celles en mémoire (`live`).
    pub fn encode(&self, live: &[MobRecord]) -> String {
        let mut mobs: Vec<Json> = self.chunks.values().flatten().map(|r| r.to_json()).collect();
        mobs.extend(live.iter().map(|r| r.to_json()));
        let mut populated: Vec<&(i32, i32)> = self.populated.iter().collect();
        populated.sort();
        let populated = populated.into_iter().map(|(x, z)| Json::Str(format!("{},{}", x, z))).collect();
        Json::Obj(vec![("mobs".into(), Json::Arr(mobs)), ("populated".into(), Json::Arr(populated))]).to_json()
    }

    /// Nombre de créatures rangées.
    pub fn stored(&self) -> usize {
        self.chunks.values().map(|v| v.len()).sum()
    }
}

/// Les régions en mémoire et celles en cours de lecture.
pub(super) struct MobStore {
    dir: PathBuf,
    pub regions: HashMap<(i32, i32), Region>,
    loading: HashSet<(i32, i32)>,
    tx: Sender<((i32, i32), Option<String>)>,
    rx: Receiver<((i32, i32), Option<String>)>,
    /// Copie au format du jeu dans la base du monde (voir `actors.rs`) : empreinte des entités
    /// de chaque chunk déjà écrit, régions déjà passées en revue, prochain numéro d'entité
    pub exported: HashMap<(i32, i32), u64>,
    pub exported_regions: HashSet<(i32, i32)>,
    pub next_actor: Option<u32>,
}

impl MobStore {
    pub fn new(dir: PathBuf) -> MobStore {
        let (tx, rx) = channel();
        MobStore { dir, regions: HashMap::new(), loading: HashSet::new(), tx, rx, exported: HashMap::new(), exported_regions: HashSet::new(), next_actor: None }
    }

    fn path(dir: &Path, r: (i32, i32)) -> PathBuf {
        dir.join(format!("r.{}.{}.json", r.0, r.1))
    }

    /// La région si elle est en mémoire ; sinon sa lecture est demandée (None pour l'instant).
    pub fn region(&mut self, r: (i32, i32)) -> Option<&mut Region> {
        if !self.regions.contains_key(&r) {
            self.request(r);
            return None;
        }
        self.regions.get_mut(&r)
    }

    /// Demande la lecture d'une région au fil des écritures (après les écritures en attente).
    fn request(&mut self, r: (i32, i32)) {
        if !self.loading.insert(r) {
            return;
        }
        let (path, tx) = (Self::path(&self.dir, r), self.tx.clone());
        crate::background::DISK.run(move || {
            let _ = tx.send((r, std::fs::read_to_string(&path).ok()));
        });
    }

    /// Range les régions lues depuis le dernier appel.
    pub fn receive(&mut self) {
        while let Ok((r, text)) = self.rx.try_recv() {
            self.loading.remove(&r);
            if self.regions.contains_key(&r) {
                continue;
            }
            let region = match text {
                None => Region::default(),
                Some(t) => Region::parse(&t).unwrap_or_else(|| {
                    warn!("{} illisible : animaux de cette région perdus", Self::path(&self.dir, r).display());
                    Region::default()
                }),
            };
            self.regions.insert(r, region);
        }
    }

    /// Lit une région tout de suite (arrêt du serveur, après les écritures en attente).
    pub fn load_now(&mut self, r: (i32, i32)) {
        if self.regions.contains_key(&r) {
            return;
        }
        let region = std::fs::read_to_string(Self::path(&self.dir, r)).ok().and_then(|t| Region::parse(&t)).unwrap_or_default();
        self.regions.insert(r, region);
        self.loading.remove(&r);
    }

    /// Écrit une région sur le fil des écritures (fichier temporaire puis renommé).
    pub fn write(&self, r: (i32, i32), text: String) {
        let (dir, path) = (self.dir.clone(), Self::path(&self.dir, r));
        crate::background::DISK.run(move || {
            let tmp = path.with_extension("json.tmp");
            let done = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&tmp, text.as_bytes())).and_then(|_| std::fs::rename(&tmp, &path));
            if let Err(e) = done {
                warn!("Animaux non enregistrés dans {} : {}", path.display(), e);
            }
        });
    }

    /// Écrit une région tout de suite (conversion de l'ancien fichier).
    pub fn write_now(&self, r: (i32, i32), text: &str) -> std::io::Result<()> {
        let path = Self::path(&self.dir, r);
        std::fs::create_dir_all(&self.dir)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_round_trip() {
        assert_eq!(region_of((31, -1)), (0, -1));
        assert_eq!(region_of((32, -32)), (1, -1));
        assert_eq!(region_of((-33, 0)), (-2, 0));
        assert_eq!(chunk_of(-0.5, 17.0), (-1, 1));
        let text = r#"{"mobs":[{"kind":"minecraft:cow","x":1.5,"y":64,"z":-3.5,"age":-100},{"kind":"minecraft:inconnu","x":0,"y":0,"z":0}],"chunks":["0,-1","2,3"]}"#;
        let region = Region::parse(text).unwrap();
        assert_eq!(region.stored(), 1, "l'espèce inconnue est ignorée");
        assert_eq!(region.chunks[&(0, -1)][0].age, -100);
        assert!(region.populated.contains(&(2, 3)));
        let again = Region::parse(&region.encode(&[])).unwrap();
        assert_eq!(again.stored(), 1);
        assert_eq!(again.populated, region.populated);
    }
}
