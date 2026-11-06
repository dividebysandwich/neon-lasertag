//! Atmosphere inside the arena, rebuilt with it (lib.rs `build_arena`):
//! - the reload stations' glow: a soft white column of light from the crown down to the pad,
//!   and a glow on the floor around it, breathing slowly;
//! - faint shafts of light falling through the holes in the upper floor, in the colour of the
//!   zone below, swelling a little with the music's bass;
//! - faint haze lying on the floor in slowly drifting patches (`NeonUniform::haze`);
//! - glowing motes floating in the air, in a few merged meshes drifting as a whole.
//!
//! Everything is made of the neon materials, so the foveated GL renderer draws it too, and kept
//! to a handful of draws.

use std::f32::consts::TAU;

use bevy::prelude::*;
use lt_sim::kit::STOREY;
use lt_sim::level::Level;

use crate::arena::{ArenaPart, MoverView, zone_color};
use crate::assets::MeshData;
use crate::game::{Game, alpha};
use crate::materials::{AccentMaterial, GlowMaterial, NeonUniform, WHITE, ZONE_COLORS};
use crate::music::MusicSpectrum;
use crate::scenery::cone;

pub struct AmbiencePlugin;

impl Plugin for AmbiencePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (animate, place_movers.run_if(resource_exists::<Game>)),
        );
    }
}

/// A material whose intensity breathes: `base` plus `swing` times a slow wave (`rate` Hz,
/// `phase`) plus `bass` times the music's bass pulse.
#[derive(Component)]
struct Breathe {
    glow: Handle<GlowMaterial>,
    base: f32,
    swing: f32,
    rate: f32,
    phase: f32,
    bass: f32,
}

/// A group of motes drifting: bobbing and turning slowly about the arena's middle.
#[derive(Component)]
struct Drift {
    phase: f32,
    spin: f32,
}

const STATION_COLUMN: f32 = 0.5;
const STATION_FLOOR: f32 = 0.9;
const SHAFT: f32 = 0.22;
const HAZE: f32 = 0.3;

fn glow(glows: &mut Assets<GlowMaterial>, c: Vec3, i: f32) -> Handle<GlowMaterial> {
    glows.add(GlowMaterial {
        neon: NeonUniform::fixed(c, i),
    })
}

/// A flat ring on the floor at height `y`: radius r0 (v 0) to r1 (v 1), facing up.
fn floor_ring(m: &mut MeshData, c: Vec3, r0: f32, r1: f32, seg: usize) {
    let base = m.pos.len() as u32;
    for i in 0..=seg {
        let a = i as f32 / seg as f32 * TAU;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        for (r, v) in [(r0, 0.0), (r1, 1.0)] {
            m.pos.push((c + d * r).to_array());
            m.nrm.push([0.0, 1.0, 0.0]);
            m.uv.push([i as f32 / seg as f32, v]);
        }
    }
    for i in 0..seg as u32 {
        let (a, b, c, d) = (
            base + i * 2,
            base + i * 2 + 1,
            base + i * 2 + 3,
            base + i * 2 + 2,
        );
        m.idx.extend([a, c, b, a, d, c]);
    }
}

/// A horizontal quad `half` in size at `c`, its halo fade fixed at `v`.
fn sheet(m: &mut MeshData, c: Vec3, half: Vec2, v: f32) {
    let base = m.pos.len() as u32;
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        m.pos
            .push((c + Vec3::new(x * half.x, 0.0, z * half.y)).to_array());
        m.nrm.push([0.0, 1.0, 0.0]);
        m.uv.push([0.0, v]);
    }
    m.idx
        .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
}

/// A tiny octahedron (a mote) at `c`.
fn mote(m: &mut MeshData, c: Vec3, r: f32) {
    let base = m.pos.len() as u32;
    let pts = [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    for p in pts {
        m.pos.push((c + p * r).to_array());
        m.nrm.push(p.to_array());
        m.uv.push([0.0, 0.0]);
    }
    for (a, b, c) in [
        (0, 2, 4),
        (4, 2, 1),
        (1, 2, 5),
        (5, 2, 0),
        (4, 3, 0),
        (1, 3, 4),
        (5, 3, 1),
        (0, 3, 5),
    ] {
        m.idx.extend([base + a, base + b, base + c]);
    }
}

/// Holes in the upper floor: open cells with floor on two or more sides.
fn holes(level: &Level) -> Vec<(usize, usize)> {
    let g = &level.grid;
    let dm = level.dims();
    let (w, h) = (dm.w as i32, dm.h as i32);
    let mut out = Vec::new();
    for iz in 0..dm.h {
        for ix in 0..dm.w {
            if g.upper[dm.idx(ix, iz)] {
                continue;
            }
            let around = [(0i32, 1i32), (1, 0), (0, -1), (-1, 0)]
                .iter()
                .filter(|(dx, dz)| {
                    let (x, z) = (ix as i32 + dx, iz as i32 + dz);
                    x >= 0 && z >= 0 && x < w && z < h && g.upper[dm.idx(x as usize, z as usize)]
                })
                .count();
            if around >= 2 {
                out.push((ix, iz));
            }
        }
    }
    out
}

/// Spawns the arena's atmosphere (tagged `ArenaPart`, so it goes with the arena).
pub fn spawn(
    commands: &mut Commands,
    level: &Level,
    meshes: &mut Assets<Mesh>,
    glows: &mut Assets<GlowMaterial>,
    accents: &mut Assets<AccentMaterial>,
) {
    let mut add = |commands: &mut Commands,
                   m: MeshData,
                   mat: Handle<GlowMaterial>,
                   breathe: Option<Breathe>| {
        if m.is_empty() {
            return;
        }
        let mut e = commands.spawn((
            ArenaPart,
            Mesh3d(meshes.add(m.to_mesh())),
            MeshMaterial3d(mat),
            Transform::IDENTITY,
        ));
        if let Some(b) = breathe {
            e.insert(b);
        }
    };

    // Reload stations: a column of light under the crown, a glow on the floor.
    let mut column = MeshData::default();
    let mut floor = MeshData::default();
    for r in &level.reloads {
        cone(
            &mut column,
            *r + Vec3::Y * 2.38,
            Vec3::NEG_Y,
            2.36,
            0.22,
            0.72,
            16,
        );
        floor_ring(&mut floor, *r + Vec3::Y * 0.045, 0.78, 1.7, 24);
    }
    let col_mat = glows.add(GlowMaterial {
        neon: NeonUniform::fixed(WHITE, STATION_COLUMN).volume(),
    });
    let floor_mat = glow(glows, WHITE, STATION_FLOOR);
    add(
        commands,
        column,
        col_mat.clone(),
        Some(Breathe {
            glow: col_mat,
            base: STATION_COLUMN,
            swing: 0.15,
            rate: 0.5,
            phase: 0.0,
            bass: 0.0,
        }),
    );
    add(
        commands,
        floor,
        floor_mat.clone(),
        Some(Breathe {
            glow: floor_mat,
            base: STATION_FLOOR,
            swing: 0.25,
            rate: 0.5,
            phase: 0.3,
            bass: 0.0,
        }),
    );

    // Shafts through the upper floor's holes, grouped by colour.
    let mut shafts: Vec<(Vec3, MeshData)> = Vec::new();
    for (ix, iz) in holes(level) {
        let c = level.dims().cell_centre(ix, iz);
        let colour = zone_color(level, c);
        let i = match shafts
            .iter()
            .position(|(k, _)| (*k - colour).length() < 1e-3)
        {
            Some(i) => i,
            None => {
                shafts.push((colour, MeshData::default()));
                shafts.len() - 1
            }
        };
        cone(
            &mut shafts[i].1,
            c + Vec3::Y * (STOREY + 2.5),
            Vec3::NEG_Y,
            STOREY + 2.5,
            0.7,
            1.05,
            12,
        );
    }
    for (k, (colour, m)) in shafts.into_iter().enumerate() {
        let mat = glows.add(GlowMaterial {
            neon: NeonUniform::fixed(colour, SHAFT).volume().dimmable(),
        });
        let b = Breathe {
            glow: mat.clone(),
            base: SHAFT,
            swing: 0.06,
            rate: 0.13,
            phase: k as f32 * 1.3,
            bass: 0.18,
        };
        add(commands, m, mat, Some(b));
    }

    // Haze: two thin layers over the floor, in drifting patches.
    for (k, (y, colour)) in [(0.08, ZONE_COLORS[0]), (0.25, ZONE_COLORS[3])]
        .into_iter()
        .enumerate()
    {
        let mut m = MeshData::default();
        sheet(
            &mut m,
            Vec3::new(0.0, y, 0.0),
            Vec2::new(level.dims().half_width(), level.dims().half_length()),
            0.55,
        );
        let mat = glows.add(GlowMaterial {
            neon: NeonUniform::fixed(colour, HAZE).haze().dimmable(),
        });
        let b = Breathe {
            glow: mat.clone(),
            base: HAZE,
            swing: 0.08,
            rate: 0.07,
            phase: k as f32 * 2.0,
            bass: 0.0,
        };
        add(commands, m, mat, Some(b));
    }

    // Motes: a few groups scattered through the air, each drifting as a whole.
    let mut rng = level.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 40) as f32 / (1u64 << 24) as f32
    };
    for (k, colour) in [ZONE_COLORS[0], ZONE_COLORS[3], ZONE_COLORS[1]]
        .into_iter()
        .enumerate()
    {
        let mut m = MeshData::default();
        for _ in 0..110 {
            let p = Vec3::new(
                (rand() * 2.0 - 1.0) * (level.dims().half_width() - 0.5),
                0.4 + rand() * 5.5,
                (rand() * 2.0 - 1.0) * (level.dims().half_length() - 0.5),
            );
            mote(&mut m, p, 0.012 + rand() * 0.014);
        }
        let mat = accents.add(AccentMaterial {
            neon: NeonUniform::fixed(colour, 2.2).dimmable(),
        });
        commands.spawn((
            ArenaPart,
            Mesh3d(meshes.add(m.to_mesh())),
            MeshMaterial3d(mat),
            Transform::IDENTITY,
            Drift {
                phase: k as f32 * 2.1,
                spin: if k % 2 == 0 { 0.006 } else { -0.005 },
            },
        ));
    }
}

fn animate(
    time: Res<Time>,
    spectrum: Option<Res<MusicSpectrum>>,
    breathing: Query<&Breathe>,
    mut drifting: Query<(&Drift, &mut Transform)>,
    mut glows: ResMut<Assets<GlowMaterial>>,
) {
    let t = time.elapsed_secs();
    let bass = spectrum.map_or(0.0, |s| s.bass);
    for b in &breathing {
        if let Some(mut m) = glows.get_mut(&b.glow) {
            m.neon.color.w =
                (b.base + b.swing * (t * b.rate * TAU + b.phase).sin() + b.bass * bass).max(0.0);
        }
    }
    for (d, mut tr) in &mut drifting {
        tr.translation = Vec3::new(
            (t * 0.05 + d.phase).sin() * 0.6,
            (t * 0.17 + d.phase).sin() * 0.35,
            (t * 0.04 + d.phase).cos() * 0.8,
        );
        tr.rotation = Quat::from_rotation_y(t * d.spin + d.phase);
    }
}

/// Doors and panels where the shown tick has them, between ticks smoothly.
fn place_movers(
    game: Res<Game>,
    time: Res<Time<Fixed>>,
    mut q: Query<(&MoverView, &mut Transform)>,
) {
    let a = alpha(&time);
    let tick = game.snap.tick;
    for (m, mut t) in &mut q {
        let Some(mv) = game.level.movers.get(m.0) else {
            continue;
        };
        let off = game
            .level
            .mover_offset(m.0, tick)
            .lerp(game.level.mover_offset(m.0, tick + 1), a);
        t.translation = mv.pos + off;
    }
}
