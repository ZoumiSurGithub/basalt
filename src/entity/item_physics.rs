//! Physique des objets posés au sol, d'après celle du jeu (par tick, 20 fois par seconde) :
//! gravité, frottement de l'air et du sol, collisions avec les blocs, flottaison dans l'eau.
//!
//! L'objet est traité comme un petit cube de 0,25 bloc de côté, dont la position est le centre
//! de la base.

/// Ce qu'il y a à un endroit, pour un objet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cell {
    /// Rien qui gêne (air, fleurs, torches…)
    Empty,
    /// Bloc plein jusqu'à cette hauteur dans le bloc (1 : bloc entier, 0,5 : dalle du bas…)
    Solid(f32),
    Water,
    /// Lave, feu, cactus : l'objet est détruit
    Deadly,
}

/// Gravité par tick
pub const GRAVITY: f32 = 0.04;
/// Frottement de l'air (vitesse gardée à chaque tick)
pub const AIR_DRAG: f32 = 0.98;
/// Frottement au sol pour un bloc ordinaire (glissance 0,6 × 0,98)
pub const GROUND_DRAG: f32 = 0.588;
/// Demi-largeur de l'objet
const HALF: f32 = 0.125;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub pos: (f32, f32, f32),
    pub vel: (f32, f32, f32),
    pub on_ground: bool,
}

/// Résultat d'un tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Moved,
    /// Immobile au sol : plus besoin de le calculer tant que rien ne change autour
    Resting,
    /// Tombé dans la lave, le feu ou sur un cactus
    Destroyed,
}

fn cell_at(at: &dyn Fn(i32, i32, i32) -> Cell, x: f32, y: f32, z: f32) -> Cell {
    at(x.floor() as i32, y.floor() as i32, z.floor() as i32)
}

/// Hauteur absolue du sol sous le point (x, y, z) dans son bloc, s'il y en a un.
fn floor_at(at: &dyn Fn(i32, i32, i32) -> Cell, x: f32, y: f32, z: f32) -> Option<f32> {
    let by = y.floor();
    match cell_at(at, x, y, z) {
        Cell::Solid(h) => Some(by + h),
        _ => None,
    }
}

/// Un point est-il dans un bloc plein ?
fn blocked(at: &dyn Fn(i32, i32, i32) -> Cell, x: f32, y: f32, z: f32) -> bool {
    match cell_at(at, x, y, z) {
        Cell::Solid(h) => y - y.floor() < h,
        _ => false,
    }
}

/// Fait avancer l'objet d'un tick.
pub fn step(b: &mut Body, at: &dyn Fn(i32, i32, i32) -> Cell) -> Step {
    let (x, y, z) = b.pos;
    // Lave, feu, cactus : à l'endroit de l'objet ou juste en dessous
    if matches!(cell_at(at, x, y + 0.05, z), Cell::Deadly) || matches!(cell_at(at, x, y - 0.05, z), Cell::Deadly) {
        return Step::Destroyed;
    }
    // Coincé dans un bloc (bloc posé dessus) : remonté au-dessus, comme dans le jeu
    if blocked(at, x, y + 0.01, z) {
        let top = y.floor() + 1.0;
        b.pos.1 = top;
        b.vel = (b.vel.0, 0.0, b.vel.2);
        return Step::Moved;
    }
    let in_water = matches!(cell_at(at, x, y + 0.1, z), Cell::Water);
    let (mut vx, mut vy, mut vz) = b.vel;
    if in_water {
        // Flotte : remonte doucement vers la surface
        vy = (vy * 0.8 + 0.02).min(0.06);
        vx *= 0.9;
        vz *= 0.9;
    } else {
        vy -= GRAVITY;
    }
    // Déplacement vertical, avec le sol et le plafond
    let mut ny = y + vy;
    let mut on_ground = false;
    if vy <= 0.0 {
        // Le sol le plus haut traversé pendant la chute (bloc d'arrivée ou bloc du départ)
        let mut floor = None;
        let mut by = y.floor();
        while by >= ny.floor() {
            if let Some(f) = floor_at(at, x, by, z) {
                if f <= y + 1e-4 && f >= ny {
                    floor = Some(f);
                    break;
                }
            }
            by -= 1.0;
        }
        if let Some(f) = floor {
            ny = f;
            vy = 0.0;
            on_ground = true;
        }
    } else if blocked(at, x, ny + 0.25, z) {
        ny = y;
        vy = 0.0;
    }
    // Déplacement horizontal, axe par axe : arrêté net par un bloc
    let mut nx = x + vx;
    let edge_x = nx + HALF * vx.signum();
    if vx != 0.0 && blocked(at, edge_x, ny + 0.05, z) {
        nx = x;
        vx = 0.0;
    }
    let mut nz = z + vz;
    let edge_z = nz + HALF * vz.signum();
    if vz != 0.0 && blocked(at, nx, ny + 0.05, edge_z) {
        nz = z;
        vz = 0.0;
    }
    let drag = if on_ground { GROUND_DRAG } else { AIR_DRAG };
    vx *= drag;
    vz *= drag;
    vy *= AIR_DRAG;
    if vx.abs() < 0.003 {
        vx = 0.0;
    }
    if vz.abs() < 0.003 {
        vz = 0.0;
    }
    b.pos = (nx, ny, nz);
    b.vel = (vx, vy, vz);
    b.on_ground = on_ground;
    if on_ground && vx == 0.0 && vz == 0.0 && !in_water {
        Step::Resting
    } else {
        Step::Moved
    }
}

/// Élan d'un objet jeté par un joueur, d'après son regard (comme le jeu : 0,3 dans la
/// direction regardée, plus 0,1 vers le haut).
pub fn throw_velocity(yaw: f32, pitch: f32) -> (f32, f32, f32) {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    (-y.sin() * p.cos() * 0.3, -p.sin() * 0.3 + 0.1, y.cos() * p.cos() * 0.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monde plat : sol plein jusqu'à y = 0 (bloc -1), le reste vide.
    fn flat(x: i32, y: i32, z: i32) -> Cell {
        let _ = (x, z);
        if y < 0 {
            Cell::Solid(1.0)
        } else {
            Cell::Empty
        }
    }

    fn settle(b: &mut Body, at: &dyn Fn(i32, i32, i32) -> Cell) -> (Step, u32) {
        for t in 1..400 {
            match step(b, at) {
                Step::Moved => {}
                other => return (other, t),
            }
        }
        (Step::Moved, 400)
    }

    #[test]
    fn a_dropped_item_falls_and_rests_on_the_ground() {
        let mut b = Body { pos: (0.5, 5.0, 0.5), vel: (0.0, 0.0, 0.0), on_ground: false };
        let (end, ticks) = settle(&mut b, &flat);
        assert_eq!(end, Step::Resting);
        assert_eq!(b.pos.1, 0.0, "posé sur le sol");
        // 5 blocs de chute : un peu moins d'une seconde
        assert!(ticks > 10 && ticks < 25, "{} ticks", ticks);
    }

    #[test]
    fn a_thrown_item_flies_forward_then_stops() {
        // Regard vers le sud (+z), à l'horizontale, depuis les yeux d'un joueur
        let mut b = Body { pos: (0.5, 1.32, 0.5), vel: throw_velocity(0.0, 0.0), on_ground: false };
        let (end, _) = settle(&mut b, &flat);
        assert_eq!(end, Step::Resting);
        assert!(b.pos.2 > 2.0 && b.pos.2 < 4.0, "lancé à {} blocs", b.pos.2 - 0.5);
        assert!((b.pos.0 - 0.5).abs() < 0.01);
    }

    #[test]
    fn walls_stop_items_and_slabs_hold_them_half_way() {
        // Mur en z = 2
        let wall = |x: i32, y: i32, z: i32| if z == 2 { Cell::Solid(1.0) } else { flat(x, y, z) };
        let mut b = Body { pos: (0.5, 0.0, 0.5), vel: (0.0, 0.0, 0.5), on_ground: true };
        settle(&mut b, &wall);
        assert!(b.pos.2 < 2.0, "arrêté par le mur ({})", b.pos.2);
        // Dalle du bas en (0, 0, 0)
        let slab = |x: i32, y: i32, z: i32| if (x, y, z) == (0, 0, 0) { Cell::Solid(0.5) } else { flat(x, y, z) };
        let mut b = Body { pos: (0.5, 3.0, 0.5), vel: (0.0, 0.0, 0.0), on_ground: false };
        settle(&mut b, &slab);
        assert_eq!(b.pos.1, 0.5);
    }

    #[test]
    fn items_float_in_water_and_burn_in_lava() {
        // Eau de y = 0 à 3, sol dessous
        let pool = |x: i32, y: i32, z: i32| if (0..3).contains(&y) { Cell::Water } else { flat(x, y, z) };
        let mut b = Body { pos: (0.5, 0.5, 0.5), vel: (0.0, 0.0, 0.0), on_ground: false };
        for _ in 0..200 {
            step(&mut b, &pool);
        }
        assert!(b.pos.1 > 2.5, "remonté vers la surface ({})", b.pos.1);
        let lava = |x: i32, y: i32, z: i32| if y == 0 { Cell::Deadly } else { flat(x, y, z) };
        let mut b = Body { pos: (0.5, 3.0, 0.5), vel: (0.0, 0.0, 0.0), on_ground: false };
        assert_eq!(settle(&mut b, &lava).0, Step::Destroyed);
    }

    #[test]
    fn an_item_under_a_new_block_pops_out_on_top() {
        let covered = |x: i32, y: i32, z: i32| if (x, y, z) == (0, 0, 0) { Cell::Solid(1.0) } else { flat(x, y, z) };
        let mut b = Body { pos: (0.5, 0.0, 0.5), vel: (0.0, 0.0, 0.0), on_ground: true };
        step(&mut b, &covered);
        assert_eq!(b.pos.1, 1.0);
    }
}
