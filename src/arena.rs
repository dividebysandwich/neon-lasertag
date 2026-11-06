//! The arena on screen: the level's kit pieces merged into a few big meshes per 8 m chunk
//! (one per material: black body, team accents, glow halos), so it costs a few dozen draws.

use std::collections::HashMap;

use bevy::prelude::*;
use lt_sim::kit::{self, PieceKind, Shape};
use lt_sim::level::Level;

use crate::assets::{MeshData, load_glb};
use crate::materials::{BLUE, Palette, RED, WHITE, ZONE_COLORS};

/// Chunk size (metres) for merging.
const CHUNK: f32 = 8.0;

#[derive(Component)]
pub struct ArenaPart;

/// A door or sliding panel (`level.movers[i]`), placed each frame (ambience.rs `animate`).
#[derive(Component)]
pub struct MoverView(pub usize);

/// Kit pieces by kind: material name → triangles; and each piece's accents split into their
/// separate shapes, with the halos that go with them, so a placed piece can leave some out.
#[derive(Resource, Default)]
pub struct Kit {
    pub pieces: HashMap<PieceKind, HashMap<String, MeshData>>,
    pub shapes: HashMap<PieceKind, Vec<Shape3>>,
}

/// One connected accent shape of a kit piece and its halos.
#[derive(Default)]
pub struct Shape3 {
    pub accent: MeshData,
    pub glow: MeshData,
    /// Its bounding box's diagonal (the biggest is the piece's outline, always kept).
    pub size: f32,
}

/// `m` split into its connected parts (vertices at the same place joined).
fn islands(m: &MeshData) -> Vec<MeshData> {
    let n = m.pos.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let key = |v: [f32; 3]| v.map(|c| (c * 2000.0).round() as i64);
    let mut same: HashMap<[i64; 3], usize> = HashMap::new();
    for i in 0..n {
        if let Some(&j) = same.get(&key(m.pos[i])) {
            let (a, b) = (find(&mut parent, i), find(&mut parent, j));
            parent[a] = b;
        } else {
            same.insert(key(m.pos[i]), i);
        }
    }
    for t in m.idx.chunks(3) {
        let a = find(&mut parent, t[0] as usize);
        for &v in &t[1..] {
            let b = find(&mut parent, v as usize);
            parent[b] = a;
        }
    }
    let mut groups: HashMap<usize, MeshData> = HashMap::new();
    let mut remap: HashMap<usize, u32> = HashMap::new();
    for t in m.idx.chunks(3) {
        let root = find(&mut parent, t[0] as usize);
        let g = groups.entry(root).or_default();
        for &v in t {
            let v = v as usize;
            let k = *remap.entry(v).or_insert_with(|| {
                g.pos.push(m.pos[v]);
                g.nrm.push(m.nrm.get(v).copied().unwrap_or([0.0, 1.0, 0.0]));
                g.uv.push(m.uv.get(v).copied().unwrap_or([0.0, 0.0]));
                (g.pos.len() - 1) as u32
            });
            g.idx.push(k);
        }
    }
    let mut out: Vec<MeshData> = groups.into_values().collect();
    out.sort_by(|a, b| {
        a.bounds().0.to_array().partial_cmp(&b.bounds().0.to_array()).unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// A piece's accents as shapes, each with the halo parts nearest to it.
fn split_shapes(parts: &HashMap<String, MeshData>) -> Vec<Shape3> {
    let mut shapes: Vec<Shape3> = Vec::new();
    for (mat, data) in parts {
        if mat != "body" && mat != "glow" {
            for a in islands(data) {
                let (lo, hi) = a.bounds();
                shapes.push(Shape3 { size: (hi - lo).length(), accent: a, glow: MeshData::default() });
            }
        }
    }
    let boxes: Vec<(Vec3, Vec3)> = shapes.iter().map(|s| s.accent.bounds()).collect();
    if let Some(glow) = parts.get("glow") {
        for g in islands(glow) {
            let (lo, hi) = g.bounds();
            let gap = |(a, b): &(Vec3, Vec3)| (lo - *b).max(*a - hi).max(Vec3::ZERO).length();
            if let Some(i) = (0..boxes.len()).min_by(|&i, &j| gap(&boxes[i]).total_cmp(&gap(&boxes[j]))) {
                shapes[i].glow.append(&g, Quat::IDENTITY, Vec3::ZERO);
            }
        }
    }
    shapes.sort_by(|a, b| b.size.total_cmp(&a.size));
    shapes
}

impl Kit {
    pub fn load() -> Kit {
        let mut kit = Kit::default();
        match load_glb("models/kit.glb") {
            Ok(f) => {
                for k in PieceKind::ALL {
                    if let Some(m) = f.meshes.get(k.name()) {
                        kit.pieces.insert(k, m.clone());
                    } else {
                        warn!("kit.glb has no {}", k.name());
                    }
                }
            }
            Err(e) => warn!("kit: {e}; using plain boxes"),
        }
        for k in PieceKind::ALL {
            kit.pieces.entry(k).or_insert_with(|| fallback_piece(k));
            let shapes = split_shapes(&kit.pieces[&k]);
            kit.shapes.insert(k, shapes);
        }
        kit
    }
}

/// A stand-in piece built from the collision shapes, with accent strips along its top edges.
fn fallback_piece(k: PieceKind) -> HashMap<String, MeshData> {
    let mut body = MeshData::default();
    let mut accent = MeshData::default();
    let shapes = k.shapes();
    let (bmin, bmax) = k.bounds();
    let shapes = if shapes.is_empty() { vec![Shape::Box { min: bmin, max: bmax }] } else { shapes };
    for s in shapes {
        match s {
            Shape::Box { min, max } => add_box(&mut body, min, max),
            Shape::Ramp { min, max } => add_ramp(&mut body, min, max),
        }
    }
    // A light line along the top of the piece on both long faces, and along the floor tile edges.
    let s = 0.03;
    if k == PieceKind::FloorTile {
        for (a, b) in [
            (Vec3::new(bmin.x, 0.0, bmin.z), Vec3::new(bmax.x, 0.004, bmin.z + s)),
            (Vec3::new(bmin.x, 0.0, bmin.z), Vec3::new(bmin.x + s, 0.004, bmax.z)),
        ] {
            add_box(&mut accent, a, b);
        }
    } else if bmax.y > 0.1 {
        let y = bmax.y - 0.12;
        add_box(&mut accent, Vec3::new(bmin.x, y, bmin.z - 0.005), Vec3::new(bmax.x, y + 0.05, bmax.z + 0.005));
    }
    HashMap::from([("body".to_string(), body), ("accent".to_string(), accent)])
}

pub fn add_box(m: &mut MeshData, min: Vec3, max: Vec3) {
    let c = [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(max.x, max.y, max.z),
        Vec3::new(min.x, max.y, max.z),
    ];
    let faces: [([usize; 4], Vec3); 6] = [
        ([4, 5, 6, 7], Vec3::Z),
        ([1, 0, 3, 2], Vec3::NEG_Z),
        ([5, 1, 2, 6], Vec3::X),
        ([0, 4, 7, 3], Vec3::NEG_X),
        ([7, 6, 2, 3], Vec3::Y),
        ([0, 1, 5, 4], Vec3::NEG_Y),
    ];
    for (f, n) in faces {
        quad(m, [c[f[0]], c[f[1]], c[f[2]], c[f[3]]], n);
    }
}

fn quad(m: &mut MeshData, v: [Vec3; 4], n: Vec3) {
    let base = m.pos.len() as u32;
    for (i, p) in v.iter().enumerate() {
        m.pos.push(p.to_array());
        m.nrm.push(n.to_array());
        m.uv.push([(i == 1 || i == 2) as u8 as f32, (i >= 2) as u8 as f32]);
    }
    m.idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// A wedge climbing along +Z.
fn add_ramp(m: &mut MeshData, min: Vec3, max: Vec3) {
    let (x0, x1, y0, y1, z0, z1) = (min.x, max.x, min.y, max.y, min.z, max.z);
    let n_top = Vec3::new(0.0, z1 - z0, -(y1 - y0)).normalize();
    quad(m, [Vec3::new(x0, y0, z0), Vec3::new(x1, y0, z0), Vec3::new(x1, y1, z1), Vec3::new(x0, y1, z1)], n_top);
    quad(m, [Vec3::new(x1, y0, z1), Vec3::new(x0, y0, z1), Vec3::new(x0, y1, z1), Vec3::new(x1, y1, z1)], Vec3::Z);
    // Sides as degenerate quads (triangles).
    quad(m, [Vec3::new(x1, y0, z0), Vec3::new(x1, y0, z1), Vec3::new(x1, y1, z1), Vec3::new(x1, y0, z0)], Vec3::X);
    quad(m, [Vec3::new(x0, y0, z1), Vec3::new(x0, y0, z0), Vec3::new(x0, y0, z0), Vec3::new(x0, y1, z1)], Vec3::NEG_X);
}

/// Accents nearer the centre line than this (m) take their zone's colour; further out (the
/// targets and the bases) the team colour of their half.
const ZONE_EDGE: f32 = 12.0;
/// Accents within this of a reload station (horizontally, on its floor) are white.
const RELOAD_RADIUS: f32 = 2.3;

/// Brightness of the ordinary arena accents, against 1.0 for the ones that matter (the spawn
/// areas, the team targets' alcoves and the reload stations).
const DIM: f32 = 0.5;
/// Accents within this of a target's centre (horizontally, any height) are bright.
const TARGET_RADIUS: f32 = 2.6;
/// Accents within this of a spawn pad (horizontally) belong to a base (its walls included).
const BASE_RADIUS: f32 = 2.6;

/// The vertex tint of a piece's accents (`neon.wgsl` mixes the team colour towards rgb by a;
/// rgb may exceed 1 or be dimmed): white around reload stations, a zone colour in the middle of
/// the arena (zones of a few cells, mirrored like the arena), the team colour of the half behind
/// the targets. Everything at half brightness except the bases, the targets and the stations.
fn zone_tint(level: &Level, kind: PieceKind, pos: Vec3) -> ([f32; 4], bool) {
    let near = |c: Vec3, r: f32| (pos.y - c.y).abs() < 0.5 && Vec2::new(pos.x - c.x, pos.z - c.z).length() < r;
    let border = matches!(kind, PieceKind::BorderWall | PieceKind::BorderPost);
    if !border && level.reloads.iter().any(|r| near(*r, RELOAD_RADIUS)) {
        return (WHITE.extend(1.0).to_array(), true);
    }
    let base = level.spawns.iter().flatten().any(|s| Vec2::new(pos.x - s.pos.x, pos.z - s.pos.z).length() < BASE_RADIUS);
    let bright = base
        || kind == PieceKind::TargetAlcove
        // (On both floors and the roof: the target's walls go all the way up.)
        || level.targets.iter().any(|t| Vec2::new(pos.x - t.centre().x, pos.z - t.centre().z).length() < TARGET_RADIUS);
    // Ordinary accents: dimmed, each piece a little brighter or darker than the next, a few
    // nearly dark; the floor's grid fainter still.
    let r = piece_random(level, kind, pos, 1);
    let k = if bright {
        1.0
    } else if matches!(kind, PieceKind::FloorTile | PieceKind::UpperTile) {
        DIM * (0.45 + 0.25 * r)
    } else if r < 0.1 {
        DIM * 0.18
    } else {
        DIM * (0.5 + 0.9 * r)
    };
    let c = zone_color(level, pos);
    ((c * k).extend(1.0).to_array(), bright)
}

/// The colour of the arena at `pos`: the team colour behind the targets, a zone colour in the
/// middle (zones of a few cells, mirrored like the arena, the upper floor apart from the ground).
pub fn zone_color(level: &Level, pos: Vec3) -> Vec3 {
    if pos.z.abs() >= ZONE_EDGE - 0.01 {
        if pos.z >= 0.0 { RED } else { BLUE }
    } else {
        // Zones: four bands across, two along each half, upper floor apart from the ground.
        // Blue's half is Red's turned round the centre: so are the zones.
        let x = if pos.z >= 0.0 { pos.x } else { -pos.x };
        let hw = level.dims().half_width();
        let zx = ((x + hw) / (hw * 2.0 / 4.0)).floor().clamp(0.0, 3.0) as u64;
        let zz = (pos.z.abs() / (ZONE_EDGE / 2.0)).floor().min(1.0) as u64;
        let floor = (pos.y > 1.5) as u64;
        let h = (level.seed ^ (zx * 0x9E37_79B9) ^ (zz * 0x85EB_CA6B) ^ (floor * 0xC2B2_AE35))
            .wrapping_mul(0x2545_F491_4F6C_DD1D);
        ZONE_COLORS[((h >> 33) % ZONE_COLORS.len() as u64) as usize]
    }
}

/// A random number in 0..1 for a placed piece (the same every time for a seed).
fn piece_random(level: &Level, kind: PieceKind, pos: Vec3, salt: u64) -> f32 {
    let q = (pos * 4.0).round();
    let h = (level.seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add((q.x as i64 as u64).wrapping_mul(0x85EB_CA6B))
        .wrapping_add((q.y as i64 as u64).wrapping_mul(0xC2B2_AE35))
        .wrapping_add((q.z as i64 as u64).wrapping_mul(0x27D4_EB2F))
        .wrapping_add(kind as u64 * 0x1656_67B1);
    let h = (h ^ (h >> 31)).wrapping_mul(0x2545_F491_4F6C_DD1D);
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// The share of an ordinary piece's accent shapes it keeps (its outline always stays).
const KEEP: f32 = 0.6;

/// Spawns the doors and sliding panels: their kit piece in full colour (moving parts stand
/// out), as entities of their own.
fn spawn_movers(commands: &mut Commands, level: &Level, kit: &Kit, palette: &Palette, meshes: &mut Assets<Mesh>) {
    for (i, m) in level.movers.iter().enumerate() {
        let Some(parts) = kit.pieces.get(&m.piece) else { continue };
        let rot = kit::turn(m.rot);
        let tint = zone_color(level, m.pos).extend(1.0).to_array();
        let root = commands
            .spawn((ArenaPart, MoverView(i), Transform::from_translation(m.pos), Visibility::default(), Name::new("mover")))
            .id();
        for (mat, data) in parts {
            let mut d = MeshData::default();
            if mat == "body" {
                d.append(data, rot, Vec3::ZERO);
            } else {
                d.append_tinted(data, rot, Vec3::ZERO, tint);
            }
            let mut e = commands.spawn((Mesh3d(meshes.add(d.to_mesh())), Transform::IDENTITY, ChildOf(root)));
            match mat.as_str() {
                "body" => e.insert(MeshMaterial3d(palette.body.clone())),
                "glow" => e.insert(MeshMaterial3d(palette.arena_glow.clone())),
                "border accent" => e.insert(MeshMaterial3d(palette.border_accent.clone())),
                "border glow" => e.insert(MeshMaterial3d(palette.border_glow.clone())),
                _ => e.insert(MeshMaterial3d(palette.arena_accent.clone())),
            };
        }
    }
}

/// Spawns the arena's merged meshes.
pub fn spawn_arena(
    commands: &mut Commands,
    level: &Level,
    kit: &Kit,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
) {
    spawn_movers(commands, level, kit, palette, meshes);
    let (hw, hl) = (level.dims().half_width(), level.dims().half_length());
    let mut chunks: HashMap<(i32, i32), HashMap<&str, MeshData>> = HashMap::new();
    for p in &level.pieces {
        let Some(parts) = kit.pieces.get(&p.kind) else {
            continue;
        };
        let key = (((p.pos.x + hw + 2.0) / CHUNK).floor() as i32, ((p.pos.z + hl + 2.0) / CHUNK).floor() as i32);
        let chunk = chunks.entry(key).or_default();
        let rot = kit::turn(p.rot);
        let (tint, important) = zone_tint(level, p.kind, p.pos);
        if let Some(body) = parts.get("body") {
            chunk.entry("body").or_default().append(body, rot, p.pos);
        }
        // Fewer accents on ordinary pieces: a random share of their shapes, the outline kept.
        let thin = !important
            && !matches!(p.kind, PieceKind::FloorTile | PieceKind::UpperTile | PieceKind::BorderWall | PieceKind::BorderPost);
        // The border walls' accents have their own rainbow material.
        let border = matches!(p.kind, PieceKind::BorderWall | PieceKind::BorderPost);
        let (accent_slot, glow_slot) = if border { ("border accent", "border glow") } else { ("accent", "glow") };
        for (i, shape) in kit.shapes.get(&p.kind).into_iter().flatten().enumerate() {
            if thin && i > 0 && piece_random(level, p.kind, p.pos, 10 + i as u64) > KEEP {
                continue;
            }
            chunk.entry(accent_slot).or_default().append_tinted(&shape.accent, rot, p.pos, tint);
            if !shape.glow.is_empty() {
                chunk.entry(glow_slot).or_default().append_tinted(&shape.glow, rot, p.pos, tint);
            }
        }
    }
    for (_, parts) in chunks {
        for (slot, data) in parts {
            if data.is_empty() {
                continue;
            }
            let mesh = meshes.add(data.to_mesh());
            let mut e = commands.spawn((ArenaPart, Mesh3d(mesh), Transform::IDENTITY, Name::new(format!("arena {slot}"))));
            match slot {
                "body" => e.insert(MeshMaterial3d(palette.body.clone())),
                "glow" => e.insert(MeshMaterial3d(palette.arena_glow.clone())),
                "border accent" => e.insert(MeshMaterial3d(palette.border_accent.clone())),
                "border glow" => e.insert(MeshMaterial3d(palette.border_glow.clone())),
                _ => e.insert(MeshMaterial3d(palette.arena_accent.clone())),
            };
        }
    }
}
