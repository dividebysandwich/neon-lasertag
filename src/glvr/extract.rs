//! What the render thread draws, taken from the game's world after each update (as
//! stealth-mission-89's src/glvr/extract.rs): the visible meshes seen by the eyes (frustum
//! culled) with their transforms and materials (StandardMaterial lit, the neon accents and
//! their halos), the neon's frame state, the light, and the rig placing the tracking space in
//! the arena. Meshes go to the render thread once (again when changed), on their own channel
//! so a draw list replaced before it was drawn loses none.

use super::render::{AssetMsg, MeshData, eye_views, inside, planes};
use crate::materials::{AccentMaterial, BLUE, GlowMaterial, NeonGlobals, PULSES, RED};
use crate::vr::{HandInput, XrRig};
use bevy::asset::AssetId;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::GlobalAmbientLight;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use openxr as xr;
use std::collections::HashSet;
use std::sync::mpsc::Sender;

/// The eyes' fields of view are widened this much for culling: the draw list is drawn a frame
/// after the poses it was culled with.
const CULL_MARGIN: f32 = 1.15;

/// The point lights drawn (the nearest to the eyes): the bolts' lights.
pub const POINT_LIGHTS: usize = 16;

/// Bevy's default camera exposure (EV100 9.7) as a factor: lux and cd/m² to the shader's units.
const EXPOSURE: f32 = 1.0 / 998.0;

/// A StandardMaterial mesh to draw.
pub struct LitItem {
    pub mesh: AssetId<Mesh>,
    pub model: Mat4,
    /// Linear RGBA.
    pub base: Vec4,
    pub double_sided: bool,
}

/// A neon mesh (accent or halo) to draw: its material's colour and params (`NeonUniform`).
pub struct NeonItem {
    pub mesh: AssetId<Mesh>,
    pub model: Mat4,
    pub colour: Vec4,
    pub params: Vec4,
    pub extra: Vec4,
}

/// The neon's state this update (`NeonGlobals`).
#[derive(Clone, Copy, Default)]
pub struct NeonFrame {
    pub time: f32,
    pub red: Vec3,
    pub blue: Vec3,
    pub pulse_pos: [Vec4; PULSES],
    pub pulse_colour: [Vec4; PULSES],
}

/// One update's picture.
#[derive(Default)]
pub struct DrawList {
    /// The tracking space in the world (`XrRig`).
    pub rig: Mat4,
    pub clear: Vec4,
    /// Towards the directional light, its colour times strength, the ambient light.
    pub light: (Vec3, Vec3, Vec3),
    pub neon: NeonFrame,
    pub lit: Vec<LitItem>,
    pub accents: Vec<NeonItem>,
    pub glows: Vec<NeonItem>,
    /// Point lights: position and range (w), colour times strength at 1 m (rgb).
    pub points: Vec<(Vec4, Vec3)>,
    pub culled: u32,
    /// Vibrations asked for (`XrInput::haptic`), per hand.
    pub haptics: [f32; 2],
}

/// The eyes and the controllers as the render thread last saw them (tracking space).
#[derive(Clone)]
pub struct Eyes {
    pub views: Vec<xr::View>,
    pub controllers: [HandInput; 2],
}

type Common = (
    &'static Mesh3d,
    &'static GlobalTransform,
    &'static InheritedVisibility,
    Option<&'static Aabb>,
    Has<NoFrustumCulling>,
);

/// A mesh's common components, as a query gives them.
type CommonItem<'a> = (
    &'a Mesh3d,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Option<&'a Aabb>,
    bool,
);

/// Whether `c` is seen: None if hidden, Some(false) if outside the eyes' frusta.
fn seen(frusta: &[[Vec4; 5]], c: &CommonItem) -> Option<bool> {
    let (_, transform, visible, aabb, no_cull) = *c;
    if !visible.get() {
        return None;
    }
    if !no_cull
        && let Some(aabb) = aabb
        && !frusta.is_empty()
        && !frusta.iter().any(|f| inside(f, transform, aabb))
    {
        return Some(false);
    }
    Some(true)
}

/// The queries of the meshes drawn, by material.
struct Queries {
    standard: QueryState<(Common, &'static MeshMaterial3d<StandardMaterial>)>,
    accents: QueryState<(Common, &'static MeshMaterial3d<AccentMaterial>)>,
    glows: QueryState<(Common, &'static MeshMaterial3d<GlowMaterial>)>,
    lights: QueryState<(&'static DirectionalLight, &'static GlobalTransform)>,
    points: QueryState<(
        &'static PointLight,
        &'static GlobalTransform,
        &'static InheritedVisibility,
    )>,
    transforms: QueryState<&'static GlobalTransform>,
}

pub struct Extractor {
    assets: Sender<AssetMsg>,
    /// Meshes the render thread has.
    sent: HashSet<AssetId<Mesh>>,
    queries: Option<Queries>,
}

impl Extractor {
    pub fn new(assets: Sender<AssetMsg>) -> Self {
        Extractor {
            assets,
            sent: HashSet::new(),
            queries: None,
        }
    }

    /// Meshes changed or gone: the render thread drops them, they are sent again when drawn.
    pub fn forget(&mut self, ids: impl Iterator<Item = AssetId<Mesh>>) {
        for id in ids {
            if self.sent.remove(&id) {
                let _ = self.assets.send(AssetMsg::ForgetMesh(id));
            }
        }
    }

    /// Sends mesh `id` if the render thread doesn't have it: whether it has it now.
    fn send_mesh(&mut self, meshes: &Assets<Mesh>, id: AssetId<Mesh>) -> bool {
        if self.sent.contains(&id) {
            return true;
        }
        let Some(data) = meshes.get(id).and_then(mesh_data) else {
            return false;
        };
        let _ = self.assets.send(AssetMsg::Mesh(id, data));
        self.sent.insert(id);
        true
    }

    /// Whether `c` is drawn (its mesh sent if need be).
    fn take(
        &mut self,
        frusta: &[[Vec4; 5]],
        list: &mut DrawList,
        meshes: &Assets<Mesh>,
        c: &CommonItem,
    ) -> bool {
        match seen(frusta, c) {
            None => false,
            Some(false) => {
                list.culled += 1;
                false
            }
            Some(true) => self.send_mesh(meshes, c.0.0.id()),
        }
    }

    /// The draw list of `world`, culled to `eyes` (everything while there are none yet).
    pub fn extract(&mut self, world: &mut World, eyes: Option<&Eyes>) -> DrawList {
        let mut q = self.queries.take().unwrap_or_else(|| Queries {
            standard: world.query(),
            accents: world.query(),
            glows: world.query(),
            lights: world.query(),
            points: world.query(),
            transforms: world.query(),
        });
        let mut list = DrawList::default();
        {
            let mut input = world.resource_mut::<crate::vr::XrInput>();
            list.haptics = std::mem::take(&mut input.haptic);
        }
        let world: &World = world;
        let rig = world.get_resource::<XrRig>().map(|r| r.0);
        list.rig = rig
            .and_then(|r| q.transforms.get(world, r).ok())
            .map_or(Mat4::IDENTITY, |t| t.to_matrix());
        let frusta: Vec<[Vec4; 5]> = eyes
            .map(|e| {
                eye_views(list.rig, &e.views, CULL_MARGIN)
                    .iter()
                    .map(|v| planes(v.view_proj))
                    .collect()
            })
            .unwrap_or_default();
        list.clear = world
            .get_resource::<ClearColor>()
            .map_or(Vec4::new(0.0, 0.0, 0.0, 1.0), |c| {
                Vec4::from_array(c.0.to_linear().to_f32_array())
            });
        let (dir, colour) = q
            .lights
            .iter(world)
            .next()
            .map_or((Vec3::Y, Vec3::ZERO), |(l, t)| {
                let c = l.color.to_linear();
                (
                    t.back().as_vec3(),
                    Vec3::new(c.red, c.green, c.blue) * l.illuminance * EXPOSURE,
                )
            });
        let ambient = world
            .get_resource::<GlobalAmbientLight>()
            .map_or(Vec3::ZERO, |a| {
                let c = a.color.to_linear();
                Vec3::new(c.red, c.green, c.blue) * a.brightness * EXPOSURE
            });
        list.light = (dir, colour, ambient);
        // The point lights nearest the eyes (lumens to candela at 1 m, then exposure).
        let centre = eyes
            .and_then(|e| eye_views(list.rig, &e.views, 1.0).first().map(|v| v.eye))
            .unwrap_or(list.rig.w_axis.truncate());
        let mut points: Vec<(f32, Vec4, Vec3)> = q
            .points
            .iter(world)
            .filter(|(_, _, v)| v.get())
            .map(|(l, t, _)| {
                let p = t.translation();
                let c = l.color.to_linear();
                let strength = l.intensity / (4.0 * std::f32::consts::PI) * EXPOSURE;
                (
                    (p - centre).length_squared(),
                    p.extend(l.range),
                    Vec3::new(c.red, c.green, c.blue) * strength,
                )
            })
            .collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        list.points = points
            .into_iter()
            .take(POINT_LIGHTS)
            .map(|(_, p, c)| (p, c))
            .collect();
        if let Some(g) = world.get_resource::<NeonGlobals>() {
            let mut n = NeonFrame {
                time: g.time,
                red: RED,
                blue: BLUE,
                ..default()
            };
            for (i, p) in g.pulses.iter().enumerate() {
                n.pulse_pos[i] = p.0.extend(p.1);
                n.pulse_colour[i] = p.2.extend(1.0);
            }
            list.neon = n;
        }
        let meshes = world.resource::<Assets<Mesh>>();
        let materials = world.resource::<Assets<StandardMaterial>>();
        for (common, material) in q.standard.iter(world) {
            let Some(m) = materials.get(&material.0) else {
                continue;
            };
            if !self.take(&frusta, &mut list, meshes, &common) {
                continue;
            }
            list.lit.push(LitItem {
                mesh: common.0.0.id(),
                model: common.1.to_matrix(),
                base: Vec4::from_array(m.base_color.to_linear().to_f32_array()),
                double_sided: m.double_sided || m.cull_mode.is_none(),
            });
        }
        let materials = world.resource::<Assets<AccentMaterial>>();
        for (common, material) in q.accents.iter(world) {
            let Some(m) = materials.get(&material.0) else {
                continue;
            };
            if !self.take(&frusta, &mut list, meshes, &common) {
                continue;
            }
            list.accents.push(NeonItem {
                mesh: common.0.0.id(),
                model: common.1.to_matrix(),
                colour: m.neon.color,
                params: m.neon.params,
                extra: m.neon.extra,
            });
        }
        let materials = world.resource::<Assets<GlowMaterial>>();
        for (common, material) in q.glows.iter(world) {
            let Some(m) = materials.get(&material.0) else {
                continue;
            };
            if !self.take(&frusta, &mut list, meshes, &common) {
                continue;
            }
            list.glows.push(NeonItem {
                mesh: common.0.0.id(),
                model: common.1.to_matrix(),
                colour: m.neon.color,
                params: m.neon.params,
                extra: m.neon.extra,
            });
        }
        self.queries = Some(q);
        list
    }
}

/// What the render thread needs of `mesh`: positions, normals, UVs, indices, topology.
fn mesh_data(mesh: &Mesh) -> Option<MeshData> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return None;
    };
    let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(n)) => Some(n.clone()),
        _ => None,
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(uv)) => Some(uv.clone()),
        _ => None,
    };
    let colours = match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(c)) => Some(c.clone()),
        _ => None,
    };
    Some(MeshData {
        positions: positions.clone(),
        normals,
        uvs,
        colours,
        indices: mesh.indices().cloned(),
        topology: mesh.primitive_topology(),
    })
}
