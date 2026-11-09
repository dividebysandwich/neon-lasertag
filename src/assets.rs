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

/// A glb file: each named mesh split by material name, and the named nodes' positions (and
/// their whole transforms, for nodes that place a mesh).
#[derive(Clone, Debug, Default)]
pub struct GlbFile {
    pub meshes: HashMap<String, HashMap<String, MeshData>>,
    pub nodes: HashMap<String, Vec3>,
    pub xforms: HashMap<String, Transform>,
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
            let (t, r, s) = node.transform().decomposed();
            out.nodes.insert(name.to_string(), Vec3::from(t));
            out.xforms.insert(
                name.to_string(),
                Transform {
                    translation: Vec3::from(t),
                    rotation: Quat::from_array(r).normalize(),
                    scale: Vec3::from(s),
                },
            );
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
    pub heads: Vec<HeadModel>,
    /// The eyeball and eyelid every head's eyes are made of (unit radius, looking along -Z;
    /// `HeadModel::eyes` places them): `crate::eyes`.
    pub eyeball: Model,
    pub eyelid: Model,
    /// The floating chest (origin at its top centre), with lights `hit_1`..`hit_3`.
    pub torso: Model,
    /// Left and right gloves (origin at the palm, fingers along -Z).
    pub hands: [Model; 2],
    /// Left and right gloves wrapped round the guns' grip, in the guns' frame (the hand's): shown
    /// instead of `hands` on a hand holding a gun.
    pub grips: [Model; 2],
    /// Left and right gloves pointing with the index finger (near a turret box's puzzle in VR).
    pub points: [Model; 2],
    /// Rigid body segments posed by `crate::rig`, one mesh for both sides; each hangs from its
    /// joint (the origin) down -Y, front -Z: the neck (from the head's pivot), the pelvis (top
    /// centre, tucked into the chest's hem), upper arm (shoulder), forearm (elbow), thigh
    /// (hip), shin (knee) and boot (ankle; toe along -Z).
    pub body: BodyModels,
    /// A reload station (origin on the floor in the middle of its cell).
    pub reload: Model,
    /// The powerups lying in the arena (origin at their centre, front towards -Z; the shotgun
    /// at its grip).
    pub powerups: HashMap<lt_sim::PickupKind, Model>,
    /// The shotgun in a player's hand (in the blaster's frame), and its muzzle.
    pub shotgun: Model,
    pub shotgun_muzzle: Vec3,
}

#[derive(Clone, Default)]
pub struct BodyModels {
    pub neck: Model,
    pub pelvis: Model,
    pub upper_arm: Model,
    pub forearm: Model,
    pub thigh: Model,
    pub shin: Model,
    pub boot: Model,
}

/// An animal head: its model, and where its eyes are.
#[derive(Clone, Default)]
pub struct HeadModel {
    pub model: Model,
    /// The left and right eye's frame in the head: origin at the eyeball's centre, -Z where it
    /// looks at rest, scaled by its radius (the `head_<animal>_eye_l/r` nodes); none in a
    /// model without them.
    pub eyes: Option<[Transform; 2]>,
    /// This animal's own eyeball (a cat's slit pupil: `eyeball_<animal>`), else the shared one.
    pub eyeball: Option<Model>,
    /// How wide open the eyes rest: the upper and the lower lid's edge, radians above and
    /// below the eye's forward point.
    pub lids: [f32; 2],
    /// The animal's tail (`tail_<animal>_<k>` and its joints), if the model has one.
    pub tail: Option<crate::tail::TailModel>,
}

/// The animals, with how wide open their eyes rest (the upper and the lower lid's edge, in
/// degrees above and below the eye's forward point; tools/models/avatars.py LIDS).
pub const ANIMALS: [(&str, f32, f32); 9] = [
    ("wolf", 40.0, 44.0),
    ("cat", 46.0, 48.0),
    ("deer", 54.0, 52.0),
    ("fox", 36.0, 44.0),
    ("bear", 48.0, 48.0),
    ("rabbit", 58.0, 54.0),
    ("owl", 62.0, 58.0),
    ("boar", 40.0, 42.0),
    ("dragon", 42.0, 46.0),
];

/// A powerup's mesh in powerups.glb.
fn powerup_mesh(kind: lt_sim::PickupKind) -> &'static str {
    match kind {
        lt_sim::PickupKind::Grenade => "grenade",
        lt_sim::PickupKind::Rapid => "rapid",
        lt_sim::PickupKind::Shield => "shield",
        lt_sim::PickupKind::Shotgun => "shotgun",
        lt_sim::PickupKind::Cloak => "cloak",
        lt_sim::PickupKind::Speed => "speed",
        lt_sim::PickupKind::Charge => "charge",
        lt_sim::PickupKind::Barrier => "barrier",
        lt_sim::PickupKind::Ricochet => "ricochet",
        lt_sim::PickupKind::Lance => "lance",
        lt_sim::PickupKind::Mirror => "mirror",
        lt_sim::PickupKind::Sensor => "sensor",
        lt_sim::PickupKind::Decoy => "decoy",
        lt_sim::PickupKind::Medbeam => "medbeam",
        lt_sim::PickupKind::Dual => "dual",
    }
}

/// An animal's tail in avatars.glb: its pieces `tail_<animal>_<k>` and their joints
/// `tail_<animal>_j<k>` (one more, the tip).
fn load_tail(
    f: &GlbFile,
    animal: &str,
    meshes: &mut Assets<Mesh>,
) -> Option<crate::tail::TailModel> {
    let mut tail = crate::tail::TailModel {
        swing: crate::tail::swing(animal),
        ..default()
    };
    for k in 0.. {
        let piece = format!("tail_{animal}_{k}");
        let (Some(j), Some(_)) = (
            f.xforms.get(&format!("tail_{animal}_j{k}")),
            f.meshes.get(&piece),
        ) else {
            break;
        };
        tail.joints.push(*j);
        tail.pieces.push(Model::from_glb(f, Some(&piece), meshes));
    }
    let n = tail.pieces.len();
    let tip = f.xforms.get(&format!("tail_{animal}_j{n}"))?;
    tail.joints.push(*tip);
    Some(tail)
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
            for (a, top, bottom) in ANIMALS {
                let name = format!("head_{a}");
                if f.meshes.contains_key(&name) {
                    let eyes = match (
                        f.xforms.get(&format!("{name}_eye_l")),
                        f.xforms.get(&format!("{name}_eye_r")),
                    ) {
                        (Some(l), Some(r)) => Some([*l, *r]),
                        _ => None,
                    };
                    let own = format!("eyeball_{a}");
                    models.heads.push(HeadModel {
                        model: Model::from_glb(&f, Some(&name), meshes),
                        eyes,
                        eyeball: f
                            .meshes
                            .contains_key(&own)
                            .then(|| Model::from_glb(&f, Some(&own), meshes)),
                        lids: [top.to_radians(), bottom.to_radians()],
                        tail: load_tail(&f, a, meshes),
                    });
                }
            }
            for (n, m) in [
                ("eyeball", &mut models.eyeball),
                ("eyelid", &mut models.eyelid),
            ] {
                if f.meshes.contains_key(n) {
                    *m = Model::from_glb(&f, Some(n), meshes);
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
            for (i, n) in ["hand_grip_left", "hand_grip_right"].iter().enumerate() {
                if f.meshes.contains_key(*n) {
                    models.grips[i] = Model::from_glb(&f, Some(n), meshes);
                }
            }
            for (i, n) in ["hand_point_left", "hand_point_right"].iter().enumerate() {
                if f.meshes.contains_key(*n) {
                    models.points[i] = Model::from_glb(&f, Some(n), meshes);
                }
            }
            let b = &mut models.body;
            for (n, m) in [
                ("neck", &mut b.neck),
                ("pelvis", &mut b.pelvis),
                ("upper_arm", &mut b.upper_arm),
                ("forearm", &mut b.forearm),
                ("thigh", &mut b.thigh),
                ("shin", &mut b.shin),
                ("boot", &mut b.boot),
            ] {
                if f.meshes.contains_key(n) {
                    *m = Model::from_glb(&f, Some(n), meshes);
                }
            }
        }
        Err(e) => warn!("avatars: {e}"),
    }
    match load_glb("models/powerups.glb") {
        Ok(f) => {
            if f.meshes.contains_key("shotgun") {
                models.shotgun = Model::from_glb(&f, Some("shotgun"), meshes);
            }
            if let Some(m) = f.nodes.get("shotgun_muzzle") {
                models.shotgun_muzzle = *m;
            }
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
        models.heads.push(HeadModel {
            model: Model {
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
            },
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
    for (g, h) in models.grips.iter_mut().zip(&models.hands) {
        if g.parts.is_empty() {
            *g = h.clone();
        }
    }
    for (g, h) in models.points.iter_mut().zip(&models.hands) {
        if g.parts.is_empty() {
            *g = h.clone();
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
    if models.shotgun.parts.is_empty() {
        models.shotgun = models.blaster.clone();
        models.shotgun_muzzle = models.muzzle;
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
