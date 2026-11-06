//! The game's models (assets/models/*.glb, built by tools/models/*.py) read directly with the
//! gltf crate at startup, split by material name ("body", "accent", "glow", ...) so the arena
//! can be merged into a few big meshes and the materials replaced by the game's own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// Plain triangle data of one material of one mesh.
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub pos: Vec<[f32; 3]>,
    pub nrm: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    /// Vertex colours (empty: none).
    pub col: Vec<[f32; 4]>,
    pub idx: Vec<u32>,
}

impl MeshData {
    /// Appends `other` turned by `rot` and moved by `offset`.
    pub fn append(&mut self, other: &MeshData, rot: Quat, offset: Vec3) {
        let base = self.pos.len() as u32;
        self.pos.extend(
            other
                .pos
                .iter()
                .map(|p| (rot * Vec3::from(*p) + offset).to_array()),
        );
        self.nrm
            .extend(other.nrm.iter().map(|n| (rot * Vec3::from(*n)).to_array()));
        if other.uv.len() == other.pos.len() {
            self.uv.extend_from_slice(&other.uv);
        } else {
            self.uv
                .extend(std::iter::repeat_n([0.0, 0.0], other.pos.len()));
        }
        if !self.col.is_empty() || !other.col.is_empty() {
            self.col.resize(base as usize, [0.0; 4]);
            if other.col.len() == other.pos.len() {
                self.col.extend_from_slice(&other.col);
            } else {
                self.col
                    .extend(std::iter::repeat_n([0.0; 4], other.pos.len()));
            }
        }
        self.idx.extend(other.idx.iter().map(|i| i + base));
    }

    /// Appends `other` (as `append`) with every vertex coloured `col`.
    pub fn append_tinted(&mut self, other: &MeshData, rot: Quat, offset: Vec3, col: [f32; 4]) {
        let base = self.pos.len();
        self.col.resize(base, [0.0; 4]);
        self.append(other, rot, offset);
        self.col.truncate(base);
        self.col.extend(std::iter::repeat_n(col, other.pos.len()));
    }

    pub fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }

    pub fn to_mesh(&self) -> Mesh {
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos.clone());
        let nrm = if self.nrm.len() == self.pos.len() {
            self.nrm.clone()
        } else {
            vec![[0.0, 1.0, 0.0]; self.pos.len()]
        };
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
        let uv = if self.uv.len() == self.pos.len() {
            self.uv.clone()
        } else {
            vec![[0.0, 0.0]; self.pos.len()]
        };
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        if !self.col.is_empty() {
            let mut col = self.col.clone();
            col.resize(self.pos.len(), [0.0; 4]);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
        }
        m.insert_indices(Indices::U32(self.idx.clone()));
        m
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.pos.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(a, b), p| (a.min(Vec3::from(*p)), b.max(Vec3::from(*p))),
        )
    }
}

/// A glb file: each named mesh split by material name, and the named nodes' positions.
#[derive(Clone, Debug, Default)]
pub struct GlbFile {
    pub meshes: HashMap<String, HashMap<String, MeshData>>,
    pub nodes: HashMap<String, Vec3>,
}

pub fn asset_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("BEVY_ASSET_ROOT") {
        return Path::new(&dir).join("assets");
    }
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        return Path::new(&dir).join("assets");
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
        && dir.join("assets").is_dir()
    {
        return dir.join("assets");
    }
    PathBuf::from("assets")
}

pub fn load_glb(rel: &str) -> anyhow::Result<GlbFile> {
    let path = asset_dir().join(rel);
    let (doc, buffers, _images) =
        gltf::import(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let mut out = GlbFile::default();
    for mesh in doc.meshes() {
        let name = mesh.name().unwrap_or("").to_string();
        let parts = out.meshes.entry(name).or_default();
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let mat = prim.material().name().unwrap_or("body").to_string();
            let r = prim.reader(|b| Some(&buffers[b.index()]));
            let Some(pos) = r.read_positions() else {
                continue;
            };
            let pos: Vec<[f32; 3]> = pos.collect();
            let nrm: Vec<[f32; 3]> = r.read_normals().map(|n| n.collect()).unwrap_or_default();
            let uv: Vec<[f32; 2]> = r
                .read_tex_coords(0)
                .map(|t| t.into_f32().collect())
                .unwrap_or_default();
            let idx: Vec<u32> = match r.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..pos.len() as u32).collect(),
            };
            let part = MeshData {
                pos,
                nrm,
                uv,
                idx,
                ..default()
            };
            parts
                .entry(mat)
                .or_default()
                .append(&part, Quat::IDENTITY, Vec3::ZERO);
        }
    }
    for node in doc.nodes() {
        if let Some(name) = node.name() {
            let (t, _, _) = node.transform().decomposed();
            out.nodes.insert(name.to_string(), Vec3::from(t));
        }
    }
    Ok(out)
}

/// A model ready to spawn: one mesh per material name.
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub parts: Vec<(String, Handle<Mesh>)>,
    pub nodes: HashMap<String, Vec3>,
}

impl Model {
    pub fn from_glb(file: &GlbFile, mesh: Option<&str>, meshes: &mut Assets<Mesh>) -> Model {
        let mut parts = Vec::new();
        for (name, mats) in &file.meshes {
            if mesh.is_some_and(|m| m != name) {
                continue;
            }
            for (mat, data) in mats {
                if !data.is_empty() {
                    parts.push((mat.clone(), meshes.add(data.to_mesh())));
                }
            }
        }
        parts.sort_by(|a, b| a.0.cmp(&b.0));
        Model {
            parts,
            nodes: file.nodes.clone(),
        }
    }
}

/// Models used by players.
#[derive(Resource, Clone, Default)]
pub struct Models {
    pub blaster: Model,
    pub grenade: Model,
    /// The blaster's muzzle in its own frame.
    pub muzzle: Vec3,
    /// Animal heads (origin between the eyes, looking along -Z).
    pub heads: Vec<Model>,
    /// The floating chest (origin at its top centre), with lights `hit_1`..`hit_3`.
    pub torso: Model,
    /// Left and right gloves (origin at the palm, fingers along -Z).
    pub hands: [Model; 2],
    /// A reload station (origin on the floor in the middle of its cell).
    pub reload: Model,
    /// The powerups lying in the arena (origin at their centre, front towards -Z).
    pub powerups: HashMap<lt_sim::PickupKind, Model>,
}

pub const ANIMALS: [&str; 8] = [
    "wolf", "cat", "deer", "fox", "bear", "rabbit", "owl", "boar",
];

/// A powerup's mesh in powerups.glb.
fn powerup_mesh(kind: lt_sim::PickupKind) -> &'static str {
    match kind {
        lt_sim::PickupKind::Grenade => "grenade",
        lt_sim::PickupKind::Rapid => "rapid",
    }
}

pub fn load_models(meshes: &mut Assets<Mesh>) -> Models {
    let mut models = Models {
        muzzle: Vec3::new(0.0, 0.06, -0.2),
        ..default()
    };
    match load_glb("models/blaster.glb") {
        Ok(f) => {
            models.blaster = Model::from_glb(&f, None, meshes);
            if let Some(m) = f.nodes.get("muzzle") {
                models.muzzle = *m;
            }
        }
        Err(e) => warn!("blaster model: {e}"),
    }
    match load_glb("models/grenade.glb") {
        Ok(f) => models.grenade = Model::from_glb(&f, None, meshes),
        Err(e) => warn!("grenade model: {e}"),
    }
    match load_glb("models/avatars.glb") {
        Ok(f) => {
            for a in ANIMALS {
                let name = format!("head_{a}");
                if f.meshes.contains_key(&name) {
                    models.heads.push(Model::from_glb(&f, Some(&name), meshes));
                }
            }
            if f.meshes.contains_key("torso") {
                models.torso = Model::from_glb(&f, Some("torso"), meshes);
            }
            for (i, n) in ["hand_left", "hand_right"].iter().enumerate() {
                if f.meshes.contains_key(*n) {
                    models.hands[i] = Model::from_glb(&f, Some(n), meshes);
                }
            }
        }
        Err(e) => warn!("avatars: {e}"),
    }
    match load_glb("models/powerups.glb") {
        Ok(f) => {
            for kind in lt_sim::PickupKind::POWERUPS {
                let name = powerup_mesh(kind);
                if f.meshes.contains_key(name) {
                    models
                        .powerups
                        .insert(kind, Model::from_glb(&f, Some(name), meshes));
                }
            }
        }
        Err(e) => warn!("powerups: {e}"),
    }
    for kind in lt_sim::PickupKind::POWERUPS {
        models.powerups.entry(kind).or_insert_with(|| Model {
            parts: vec![
                ("body".into(), meshes.add(Cuboid::new(0.25, 0.25, 0.06))),
                ("accent".into(), meshes.add(Cuboid::new(0.15, 0.15, 0.08))),
            ],
            ..default()
        });
    }
    match load_glb("models/reload.glb") {
        Ok(f) => models.reload = Model::from_glb(&f, Some("reload_station"), meshes),
        Err(e) => warn!("reload station: {e}"),
    }
    // Stand-ins for missing models.
    if models.reload.parts.is_empty() {
        models.reload.parts = vec![
            (
                "body".into(),
                meshes.add(
                    Cylinder::new(0.8, 0.03)
                        .mesh()
                        .resolution(8)
                        .build()
                        .translated_by(Vec3::Y * 0.015),
                ),
            ),
            (
                "glow_white".into(),
                meshes.add(
                    Torus::new(0.6, 0.66)
                        .mesh()
                        .build()
                        .translated_by(Vec3::Y * 0.035),
                ),
            ),
            (
                "glow_white".into(),
                meshes.add(
                    Torus::new(0.55, 0.7)
                        .mesh()
                        .build()
                        .translated_by(Vec3::Y * 2.6),
                ),
            ),
        ];
    }
    if models.heads.is_empty() {
        models.heads.push(Model {
            parts: vec![
                (
                    "body".into(),
                    meshes.add(Sphere::new(0.13).mesh().ico(1).unwrap()),
                ),
                (
                    "accent".into(),
                    meshes.add(
                        Cuboid::new(0.2, 0.045, 0.03)
                            .mesh()
                            .build()
                            .translated_by(Vec3::new(0.0, 0.0, -0.12)),
                    ),
                ),
            ],
            ..default()
        });
    }
    if models.torso.parts.is_empty() {
        let bar = |y: f32| {
            Cuboid::new(0.3, 0.045, 0.26)
                .mesh()
                .build()
                .translated_by(Vec3::new(0.0, y, 0.0))
        };
        models.torso.parts = vec![
            (
                "body".into(),
                meshes.add(
                    Cuboid::new(0.4, 0.48, 0.22)
                        .mesh()
                        .build()
                        .translated_by(Vec3::new(0.0, -0.25, 0.0)),
                ),
            ),
            ("hit_1".into(), meshes.add(bar(-0.12))),
            ("hit_2".into(), meshes.add(bar(-0.22))),
            ("hit_3".into(), meshes.add(bar(-0.32))),
        ];
    }
    for h in &mut models.hands {
        if h.parts.is_empty() {
            h.parts = vec![("body".into(), meshes.add(Cuboid::new(0.08, 0.04, 0.11)))];
        }
    }
    if models.blaster.parts.is_empty() {
        models
            .blaster
            .parts
            .push(("body".into(), meshes.add(Cuboid::new(0.05, 0.08, 0.26))));
        models
            .blaster
            .parts
            .push(("accent".into(), meshes.add(Cuboid::new(0.055, 0.015, 0.2))));
    }
    if models.grenade.parts.is_empty() {
        models
            .grenade
            .parts
            .push(("body".into(), meshes.add(Sphere::new(0.045))));
        models
            .grenade
            .parts
            .push(("glow_white".into(), meshes.add(Torus::new(0.04, 0.05))));
    }
    models
}
