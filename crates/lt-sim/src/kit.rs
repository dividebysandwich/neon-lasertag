//! The modular arena kit: every piece's dimensions and collision shapes, in the piece's own
//! frame (metres, Y up, origin at the centre of its footprint on the floor it stands on,
//! "forward" along +Z, mirror-symmetric in X so the arena can be mirrored across z = 0).
//!
//! This file is the single source of the kit's numbers: `tools/models/kit_dims.json` (read by
//! the Blender kit builder, tools/models/kit.py) is generated from it by the
//! `kit_dims_json_is_current` test (`LT_WRITE_KIT_DIMS=1 cargo test -p lt-sim kit` rewrites it).

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Grid cell (metres).
pub const CELL: f32 = 2.0;
/// Height of a storey: the upper floor's walking surface.
pub const STOREY: f32 = 3.0;
pub const WALL_T: f32 = 0.24;
pub const FLOOR_T: f32 = 0.2;
pub const HALF_H: f32 = 1.2;
pub const RAIL_H: f32 = 1.1;
pub const RAIL_T: f32 = 0.12;
pub const POST: f32 = 0.34;
pub const PILLAR: f32 = 0.7;
pub const BLOCK: f32 = 1.2;
pub const BORDER_H: f32 = 6.0;
pub const BORDER_T: f32 = 0.4;
pub const WINDOW_W: f32 = 1.0;
pub const WINDOW_Y0: f32 = 1.05;
pub const WINDOW_Y1: f32 = 1.75;
/// A ramp is three cells long and climbs one storey.
pub const RAMP_CELLS: usize = 3;
pub const ALCOVE_H: f32 = STOREY - FLOOR_T;
pub const TARGET_W: f32 = 1.0;
pub const TARGET_Y0: f32 = 1.0;
pub const TARGET_Y1: f32 = 2.0;
pub const TARGET_D: f32 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PieceKind {
    FloorTile,
    UpperTile,
    WallFull,
    WallWindow,
    WallHalf,
    Railing,
    PostFull,
    PostLow,
    Pillar,
    Block,
    Ramp,
    TargetAlcove,
    SpawnPad,
    BorderWall,
    BorderPost,
}

impl PieceKind {
    pub const ALL: [PieceKind; 15] = [
        PieceKind::FloorTile,
        PieceKind::UpperTile,
        PieceKind::WallFull,
        PieceKind::WallWindow,
        PieceKind::WallHalf,
        PieceKind::Railing,
        PieceKind::PostFull,
        PieceKind::PostLow,
        PieceKind::Pillar,
        PieceKind::Block,
        PieceKind::Ramp,
        PieceKind::TargetAlcove,
        PieceKind::SpawnPad,
        PieceKind::BorderWall,
        PieceKind::BorderPost,
    ];

    /// The node and mesh name in assets/models/kit.glb.
    pub fn name(self) -> &'static str {
        match self {
            PieceKind::FloorTile => "floor_tile",
            PieceKind::UpperTile => "upper_tile",
            PieceKind::WallFull => "wall_full",
            PieceKind::WallWindow => "wall_window",
            PieceKind::WallHalf => "wall_half",
            PieceKind::Railing => "railing",
            PieceKind::PostFull => "post_full",
            PieceKind::PostLow => "post_low",
            PieceKind::Pillar => "pillar",
            PieceKind::Block => "block",
            PieceKind::Ramp => "ramp",
            PieceKind::TargetAlcove => "target_alcove",
            PieceKind::SpawnPad => "spawn_pad",
            PieceKind::BorderWall => "border_wall",
            PieceKind::BorderPost => "border_post",
        }
    }

    /// The piece's visual bounds in its own frame (the Blender model must fill exactly this box,
    /// apart from accents standing at most `ACCENT_PROUD` off its faces).
    pub fn bounds(self) -> (Vec3, Vec3) {
        let h = CELL / 2.0;
        let t = WALL_T / 2.0;
        match self {
            PieceKind::FloorTile => (Vec3::new(-h, -0.1, -h), Vec3::new(h, 0.0, h)),
            PieceKind::UpperTile => (Vec3::new(-h, -FLOOR_T, -h), Vec3::new(h, 0.0, h)),
            PieceKind::WallFull | PieceKind::WallWindow => {
                (Vec3::new(-h, 0.0, -t), Vec3::new(h, STOREY, t))
            }
            PieceKind::WallHalf => (Vec3::new(-h, 0.0, -t), Vec3::new(h, HALF_H, t)),
            PieceKind::Railing => (
                Vec3::new(-h, 0.0, -RAIL_T / 2.0),
                Vec3::new(h, RAIL_H, RAIL_T / 2.0),
            ),
            PieceKind::PostFull => (
                Vec3::new(-POST / 2.0, 0.0, -POST / 2.0),
                Vec3::new(POST / 2.0, STOREY, POST / 2.0),
            ),
            PieceKind::PostLow => (
                Vec3::new(-POST / 2.0, 0.0, -POST / 2.0),
                Vec3::new(POST / 2.0, HALF_H, POST / 2.0),
            ),
            PieceKind::Pillar => (
                Vec3::new(-PILLAR / 2.0, 0.0, -PILLAR / 2.0),
                Vec3::new(PILLAR / 2.0, STOREY, PILLAR / 2.0),
            ),
            PieceKind::Block => (
                Vec3::new(-BLOCK / 2.0, 0.0, -BLOCK / 2.0),
                Vec3::new(BLOCK / 2.0, BLOCK, BLOCK / 2.0),
            ),
            PieceKind::Ramp => {
                let l = CELL * RAMP_CELLS as f32 / 2.0;
                (Vec3::new(-h, 0.0, -l), Vec3::new(h, STOREY, l))
            }
            PieceKind::TargetAlcove => (Vec3::new(-h, 0.0, -h), Vec3::new(h, STOREY, h)),
            PieceKind::SpawnPad => (Vec3::new(-h, 0.0, -h), Vec3::new(h, 0.02, h)),
            PieceKind::BorderWall => (
                Vec3::new(-h, 0.0, -BORDER_T / 2.0),
                Vec3::new(h, BORDER_H, BORDER_T / 2.0),
            ),
            PieceKind::BorderPost => (
                Vec3::new(-BORDER_T / 2.0, 0.0, -BORDER_T / 2.0),
                Vec3::new(BORDER_T / 2.0, BORDER_H, BORDER_T / 2.0),
            ),
        }
    }

    /// Collision shapes in the piece's frame.
    pub fn shapes(self) -> Vec<Shape> {
        let h = CELL / 2.0;
        let t = WALL_T / 2.0;
        let b = |min: Vec3, max: Vec3| Shape::Box { min, max };
        match self {
            // The ground is one box under the whole arena (level.rs).
            PieceKind::FloorTile | PieceKind::SpawnPad => vec![],
            PieceKind::WallWindow => {
                let w = WINDOW_W / 2.0;
                vec![
                    b(Vec3::new(-h, 0.0, -t), Vec3::new(-w, STOREY, t)),
                    b(Vec3::new(w, 0.0, -t), Vec3::new(h, STOREY, t)),
                    b(Vec3::new(-w, 0.0, -t), Vec3::new(w, WINDOW_Y0, t)),
                    b(Vec3::new(-w, WINDOW_Y1, -t), Vec3::new(w, STOREY, t)),
                ]
            }
            PieceKind::Ramp => {
                let (min, max) = self.bounds();
                vec![Shape::Ramp { min, max }]
            }
            PieceKind::TargetAlcove => vec![
                // Back (towards the centre of the arena), sides and roof: open only at +Z.
                b(Vec3::new(-h, 0.0, -h), Vec3::new(h, ALCOVE_H, -h + WALL_T)),
                b(Vec3::new(-h, 0.0, -h), Vec3::new(-h + WALL_T, ALCOVE_H, h)),
                b(Vec3::new(h - WALL_T, 0.0, -h), Vec3::new(h, ALCOVE_H, h)),
                b(Vec3::new(-h, ALCOVE_H, -h), Vec3::new(h, STOREY, h)),
            ],
            k => {
                let (min, max) = k.bounds();
                vec![b(min, max)]
            }
        }
    }
}

/// The target panel inside a `TargetAlcove`, on the inside of its back wall, facing +Z.
pub fn target_panel() -> (Vec3, Vec3) {
    let z0 = -CELL / 2.0 + WALL_T;
    (
        Vec3::new(-TARGET_W / 2.0, TARGET_Y0, z0),
        Vec3::new(TARGET_W / 2.0, TARGET_Y1, z0 + TARGET_D),
    )
}

/// A team target's box (what bolts hit), hanging from the roof over its cell on the upper
/// level (the roof's underside at 2 * STOREY - FLOOR_T) near the cell's open side (+Z).
pub fn target_box() -> (Vec3, Vec3) {
    let c = Vec3::new(0.0, TARGET_HANG_Y, TARGET_HANG_Z);
    (c - TARGET_HALF, c + TARGET_HALF)
}

/// The target's middle above the cell's floor, and how far towards the open side it hangs.
pub const TARGET_HANG_Y: f32 = STOREY + 2.3;
pub const TARGET_HANG_Z: f32 = 0.55;
pub const TARGET_HALF: Vec3 = Vec3::new(0.3, 0.3, 0.3);

/// A collision shape. `Ramp` is a solid wedge whose top climbs from `min.y` at `min.z` to
/// `max.y` at `max.z`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Box { min: Vec3, max: Vec3 },
    Ramp { min: Vec3, max: Vec3 },
}

/// Quarter turns about +Y; turn 1 takes +Z to +X.
pub fn turn(rot: u8) -> Quat {
    Quat::from_rotation_y(rot as f32 * std::f32::consts::FRAC_PI_2)
}

/// `rot` mirrored across the z = 0 plane (for X-symmetric pieces).
pub fn mirror_rot(rot: u8) -> u8 {
    (2 + 4 - rot % 4) % 4
}

/// Rotates a box by quarter turns about the origin.
pub fn rotate_box(min: Vec3, max: Vec3, rot: u8) -> (Vec3, Vec3) {
    let q = turn(rot);
    let a = q * min;
    let b = q * max;
    (a.min(b).round_to(1e-4), a.max(b).round_to(1e-4))
}

trait RoundTo {
    fn round_to(self, q: f32) -> Self;
}

impl RoundTo for Vec3 {
    fn round_to(self, q: f32) -> Self {
        (self / q).round() * q
    }
}

/// The JSON handed to the Blender kit builder.
pub fn dims_json() -> String {
    let mut s = String::from("{\n  \"constants\": {\n");
    let consts: [(&str, f32); 21] = [
        ("CELL", CELL),
        ("STOREY", STOREY),
        ("WALL_T", WALL_T),
        ("FLOOR_T", FLOOR_T),
        ("HALF_H", HALF_H),
        ("RAIL_H", RAIL_H),
        ("RAIL_T", RAIL_T),
        ("POST", POST),
        ("PILLAR", PILLAR),
        ("BLOCK", BLOCK),
        ("BORDER_H", BORDER_H),
        ("BORDER_T", BORDER_T),
        ("WINDOW_W", WINDOW_W),
        ("WINDOW_Y0", WINDOW_Y0),
        ("WINDOW_Y1", WINDOW_Y1),
        ("RAMP_CELLS", RAMP_CELLS as f32),
        ("ALCOVE_H", ALCOVE_H),
        ("TARGET_W", TARGET_W),
        ("TARGET_Y0", TARGET_Y0),
        ("TARGET_Y1", TARGET_Y1),
        ("TARGET_D", TARGET_D),
    ];
    for (i, (k, v)) in consts.iter().enumerate() {
        let comma = if i + 1 < consts.len() { "," } else { "" };
        s += &format!("    \"{k}\": {v}{comma}\n");
    }
    s += "  },\n  \"pieces\": {\n";
    let v = |p: Vec3| format!("[{}, {}, {}]", p.x, p.y, p.z);
    for (i, k) in PieceKind::ALL.iter().enumerate() {
        let (min, max) = k.bounds();
        let shapes: Vec<String> = k
            .shapes()
            .iter()
            .map(|sh| match sh {
                Shape::Box { min, max } => {
                    format!("{{\"box\": [{}, {}]}}", v(*min), v(*max))
                }
                Shape::Ramp { min, max } => {
                    format!("{{\"ramp\": [{}, {}]}}", v(*min), v(*max))
                }
            })
            .collect();
        let comma = if i + 1 < PieceKind::ALL.len() {
            ","
        } else {
            ""
        };
        s += &format!(
            "    \"{}\": {{\"bounds\": [{}, {}], \"shapes\": [{}]}}{comma}\n",
            k.name(),
            v(min),
            v(max),
            shapes.join(", ")
        );
    }
    let (pmin, pmax) = target_panel();
    s += &format!(
        "  }},\n  \"target_panel\": [{}, {}]\n}}\n",
        v(pmin),
        v(pmax)
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_rot_maps_directions() {
        for r in 0..4u8 {
            let f = turn(r) * Vec3::Z;
            let m = turn(mirror_rot(r)) * Vec3::Z;
            assert!((m - Vec3::new(f.x, f.y, -f.z)).length() < 1e-5, "rot {r}");
        }
    }

    #[test]
    fn shapes_inside_bounds() {
        for k in PieceKind::ALL {
            let (bmin, bmax) = k.bounds();
            for s in k.shapes() {
                let (min, max) = match s {
                    Shape::Box { min, max } | Shape::Ramp { min, max } => (min, max),
                };
                assert!(
                    min.cmpge(bmin - 1e-5).all() && max.cmple(bmax + 1e-5).all(),
                    "{k:?}"
                );
            }
        }
    }

    #[test]
    fn kit_dims_json_is_current() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/models/kit_dims.json"
        );
        let json = dims_json();
        let _: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        if std::env::var_os("LT_WRITE_KIT_DIMS").is_some() {
            std::fs::write(path, &json).unwrap();
        }
        // A Windows checkout may have turned the newlines into CRLF.
        let on_disk = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert_eq!(
            on_disk, json,
            "kit_dims.json is stale: LT_WRITE_KIT_DIMS=1 cargo test -p lt-sim kit"
        );
    }
}
