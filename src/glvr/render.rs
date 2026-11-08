//! Draws the arena into the headset's images with OpenGL (as stealth-mission-89's
//! src/glvr/render.rs): both eyes in one pass (GL_OVR_multiview2), multisampled in tile memory
//! (GL_OVR_multiview_multisampled_render_to_texture) so no multisampled image is stored, depth
//! reversed and infinitely far, invalidated at the end. That one pass a frame, straight into
//! the runtime's image, is what Valve's density map layers foveate where the eyes look.

use super::extract::{DrawList, LitItem, NeonItem};
use bevy::asset::AssetId;
use bevy::camera::primitives::Aabb;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use glow::HasContext;
use openxr as xr;
use std::collections::HashMap;
use std::ffi::c_void;

const LIT_SHADER: &str = include_str!("../../assets/shaders/gl/lit.glsl");
const NEON_SHADER: &str = include_str!("../../assets/shaders/gl/neon.glsl");
const LENS_SHADER: &str = include_str!("../../assets/shaders/gl/lens.glsl");
/// Near plane (m), as `--vr`'s eye cameras (src/xr.rs).
pub const NEAR: f32 = 0.05;
/// The scope's view (src/scope.rs): its size (pixels, square) and mip levels.
const SCOPE_SIZE: i32 = 512;
const SCOPE_LEVELS: i32 = 10;
/// The lens disc's segments.
const DISC_SEGMENTS: u16 = 40;
/// GL_LOWER_LEFT / GL_ZERO_TO_ONE (glClipControl, for reversed depth).
const LOWER_LEFT: u32 = 0x8CA1;
const ZERO_TO_ONE: u32 = 0x935F;

type MultiviewFn = unsafe extern "system" fn(u32, u32, u32, i32, i32, i32);
type MultisampleMultiviewFn = unsafe extern "system" fn(u32, u32, u32, i32, i32, i32, i32);
type ClipControlFn = unsafe extern "system" fn(u32, u32);

struct GpuMesh {
    vao: glow::VertexArray,
    buffers: Vec<glow::Buffer>,
    count: i32,
    indexed: Option<u32>,
    mode: u32,
}

/// A shader program and its uniforms (those it lacks are None).
struct Program {
    program: glow::Program,
    view_proj: Option<glow::UniformLocation>,
    eye: Option<glow::UniformLocation>,
    model: Option<glow::UniformLocation>,
    normal: Option<glow::UniformLocation>,
    base: Option<glow::UniformLocation>,
    light_dir: Option<glow::UniformLocation>,
    light_colour: Option<glow::UniformLocation>,
    ambient: Option<glow::UniformLocation>,
    colour: Option<glow::UniformLocation>,
    params: Option<glow::UniformLocation>,
    time: Option<glow::UniformLocation>,
    red: Option<glow::UniformLocation>,
    blue: Option<glow::UniformLocation>,
    pulse_pos: Option<glow::UniformLocation>,
    pulse_colour: Option<glow::UniformLocation>,
    extra: Option<glow::UniformLocation>,
    shots: Option<glow::UniformLocation>,
    point_pos: Option<glow::UniformLocation>,
    point_colour: Option<glow::UniformLocation>,
    point_count: Option<glow::UniformLocation>,
    on: Option<glow::UniformLocation>,
}

/// The programs drawing the arena: lit surfaces, neon accents and their halos.
struct Programs {
    lit: Program,
    accent: Program,
    glow: Program,
}

impl Programs {
    /// For `views` views at once (2: both eyes; 1: the scope).
    fn new(gl: &glow::Context, views: u32) -> Result<Self, String> {
        Ok(Programs {
            lit: compile(gl, LIT_SHADER, "", views)?,
            accent: compile(gl, NEON_SHADER, "", views)?,
            glow: compile(gl, NEON_SHADER, "#define GLOW", views)?,
        })
    }
}

/// The scope's view: its image (mipmapped) and depth, a layer each, and the framebuffer.
struct ScopeTarget {
    colour: glow::Texture,
    depth: glow::Texture,
    fb: glow::Framebuffer,
    srgb: bool,
}

/// A mesh's data for the render thread.
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub uvs: Option<Vec<[f32; 2]>>,
    /// Vertex colours (the arena's colour zones), linear RGBA.
    pub colours: Option<Vec<[f32; 4]>>,
    pub indices: Option<Indices>,
    pub topology: PrimitiveTopology,
}

/// Meshes coming and going, from the main thread.
pub enum AssetMsg {
    Mesh(AssetId<Mesh>, MeshData),
    ForgetMesh(AssetId<Mesh>),
}

/// An eye in the world: world to clip and its position.
#[derive(Clone, Copy)]
pub struct EyeView {
    pub view_proj: Mat4,
    pub eye: Vec3,
}

/// The eyes of `views` (tracking space) in the world, the tracking space placed by `rig`; their
/// fields of view `widen`ed.
pub fn eye_views(rig: Mat4, views: &[xr::View], widen: f32) -> Vec<EyeView> {
    views
        .iter()
        .take(2)
        .map(|v| {
            let (p, q) = pose(v);
            let world_from_eye = rig * Transform::from_translation(p).with_rotation(q).to_matrix();
            EyeView {
                view_proj: projection(v.fov, widen) * world_from_eye.inverse(),
                eye: world_from_eye.w_axis.truncate(),
            }
        })
        .collect()
}

/// A view's pose as position and rotation.
pub fn pose(v: &xr::View) -> (Vec3, Quat) {
    let p = v.pose.position;
    let o = v.pose.orientation;
    (
        Vec3::new(p.x, p.y, p.z),
        Quat::from_xyzw(o.x, o.y, o.z, o.w),
    )
}

/// Per-frame counts for the log.
#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub draws: u32,
    pub culled: u32,
}

pub struct Renderer {
    pub gl: glow::Context,
    eyes: Programs,
    scope: Programs,
    lens: Program,
    disc: GpuMesh,
    scope_target: Option<ScopeTarget>,
    meshes: HashMap<AssetId<Mesh>, GpuMesh>,
    depth: Option<(glow::Texture, (u32, u32))>,
    framebuffers: HashMap<u32, glow::Framebuffer>,
    multiview: MultiviewFn,
    multisample: Option<MultisampleMultiviewFn>,
    samples: i32,
    pub stats: Stats,
}

fn compile(gl: &glow::Context, shader: &str, defines: &str, views: u32) -> Result<Program, String> {
    // SAFETY: the context is current; the shaders are this module's.
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, stage) in [
            (glow::VERTEX_SHADER, "VERTEX"),
            (glow::FRAGMENT_SHADER, "FRAGMENT"),
        ] {
            let s = gl.create_shader(kind)?;
            let source = format!(
                "#version 450 core\n#extension GL_OVR_multiview2 : require\n#define VIEW gl_ViewID_OVR\n#define NUM_VIEWS {views}\n#define {stage}\n{defines}\n{shader}"
            );
            gl.shader_source(s, &source);
            gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) {
                return Err(format!("{stage} {defines}: {}", gl.get_shader_info_log(s)));
            }
            gl.attach_shader(program, s);
            shaders.push(s);
        }
        // (The shaders give their attributes' locations.)
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!(
                "linking {defines}: {}",
                gl.get_program_info_log(program)
            ));
        }
        for s in shaders {
            gl.delete_shader(s);
        }
        let at = |name: &str| gl.get_uniform_location(program, name);
        Ok(Program {
            program,
            view_proj: at("u_view_proj"),
            eye: at("u_eye"),
            model: at("u_model"),
            normal: at("u_normal"),
            base: at("u_base"),
            light_dir: at("u_light_dir"),
            light_colour: at("u_light_colour"),
            ambient: at("u_ambient"),
            colour: at("u_colour"),
            params: at("u_params"),
            time: at("u_time"),
            red: at("u_red"),
            blue: at("u_blue"),
            pulse_pos: at("u_pulse_pos"),
            pulse_colour: at("u_pulse_colour"),
            extra: at("u_extra"),
            shots: at("u_shots"),
            point_pos: at("u_point_pos"),
            point_colour: at("u_point_colour"),
            point_count: at("u_point_count"),
            on: at("u_on"),
        })
    }
}

/// The projection of an eye with field of view `fov` (its tangents times `widen`): reversed
/// depth, infinitely far.
fn projection(fov: xr::Fovf, widen: f32) -> Mat4 {
    let (l, r) = (fov.angle_left.tan() * widen, fov.angle_right.tan() * widen);
    let (d, u) = (fov.angle_down.tan() * widen, fov.angle_up.tan() * widen);
    projection_tan(l, r, d, u, NEAR)
}

/// The projection with the edges' tangents l, r, d, u: reversed depth, infinitely far.
pub fn projection_tan(l: f32, r: f32, d: f32, u: f32, near: f32) -> Mat4 {
    let (w, h) = (r - l, u - d);
    Mat4::from_cols(
        Vec4::new(2.0 / w, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / h, 0.0, 0.0),
        Vec4::new((r + l) / w, (u + d) / h, 0.0, -1.0),
        Vec4::new(0.0, 0.0, near, 0.0),
    )
}

/// lit.glsl's `aces`: the tone mapping of `--vr`'s eye cameras (for the clear colour).
fn aces(x: Vec3) -> Vec3 {
    (x * (2.51 * x + 0.03) / (x * (2.43 * x + 0.59) + 0.14)).clamp(Vec3::ZERO, Vec3::ONE)
}

/// The planes of the frustum of `m` (world to clip, reversed depth), each as (normal, distance)
/// with the inside positive; the far plane is left out.
pub fn planes(m: Mat4) -> [Vec4; 5] {
    let r = |i: usize| m.row(i);
    [
        r(3) + r(0),
        r(3) - r(0),
        r(3) + r(1),
        r(3) - r(1),
        r(3) - r(2),
    ]
}

/// Whether the box `aabb` in the space of `t` is at least partly inside `planes`.
pub fn inside(planes: &[Vec4; 5], t: &GlobalTransform, aabb: &Aabb) -> bool {
    let m = t.affine();
    let centre = m.transform_point3a(aabb.center);
    let he = aabb.half_extents;
    let (x, y, z) = (
        m.matrix3.x_axis.abs(),
        m.matrix3.y_axis.abs(),
        m.matrix3.z_axis.abs(),
    );
    let extent = x * he.x + y * he.y + z * he.z;
    planes.iter().all(|p| {
        let n = bevy::math::Vec3A::from(p.truncate());
        n.dot(centre) + n.abs().dot(extent) + p.w >= 0.0
    })
}

impl Renderer {
    pub fn new(
        gl: glow::Context,
        get: impl Fn(&str) -> *const c_void,
        samples: i32,
    ) -> Result<Self, String> {
        let exts = gl.supported_extensions();
        if !exts.contains("GL_OVR_multiview2") {
            return Err("OpenGL has no GL_OVR_multiview2 (Mesa: Zink has it, LT_GL_DRIVER=native may lack it)".into());
        }
        // SAFETY: the context is current.
        let (renderer, version) = unsafe {
            (
                gl.get_parameter_string(glow::RENDERER),
                gl.get_parameter_string(glow::VERSION),
            )
        };
        let msrtt = exts.contains("GL_OVR_multiview_multisampled_render_to_texture");
        info!("glvr: {renderer}, OpenGL {version}; multiview MSRTT {msrtt}");
        let load = |name: &str| {
            let f = get(name);
            (!f.is_null()).then_some(f)
        };
        // SAFETY: the extensions' entry points, with their signatures.
        let (multiview, multisample, clip_control) = unsafe {
            (
                std::mem::transmute::<*const c_void, MultiviewFn>(
                    load("glFramebufferTextureMultiviewOVR")
                        .ok_or("no glFramebufferTextureMultiviewOVR")?,
                ),
                load("glFramebufferTextureMultisampleMultiviewOVR")
                    .filter(|_| msrtt)
                    .map(|f| std::mem::transmute::<*const c_void, MultisampleMultiviewFn>(f)),
                std::mem::transmute::<*const c_void, ClipControlFn>(
                    load("glClipControl").ok_or("no glClipControl")?,
                ),
            )
        };
        // SAFETY: as above.
        unsafe { clip_control(LOWER_LEFT, ZERO_TO_ONE) };
        let eyes = Programs::new(&gl, 2)?;
        let scope = Programs::new(&gl, 1)?;
        let lens = compile(&gl, LENS_SHADER, "", 2)?;
        let disc = Self::upload(&gl, &disc()).ok_or("can't upload the scope's lens")?;
        let samples = if multisample.is_some() { samples } else { 0 };
        Ok(Renderer {
            gl,
            eyes,
            scope,
            lens,
            disc,
            scope_target: None,
            meshes: HashMap::new(),
            depth: None,
            framebuffers: HashMap::new(),
            multiview,
            multisample,
            samples,
            stats: Stats::default(),
        })
    }

    /// Takes a mesh from the main thread, or drops one.
    pub fn apply(&mut self, msg: AssetMsg) {
        match msg {
            AssetMsg::Mesh(id, data) => {
                self.drop_mesh(id);
                if let Some(gpu) = Self::upload(&self.gl, &data) {
                    self.meshes.insert(id, gpu);
                }
            }
            AssetMsg::ForgetMesh(id) => self.drop_mesh(id),
        }
    }

    fn drop_mesh(&mut self, id: AssetId<Mesh>) {
        if let Some(m) = self.meshes.remove(&id) {
            // SAFETY: the context is current; the objects are this renderer's.
            unsafe {
                self.gl.delete_vertex_array(m.vao);
                for b in m.buffers {
                    self.gl.delete_buffer(b);
                }
            }
        }
    }

    fn upload(gl: &glow::Context, mesh: &MeshData) -> Option<GpuMesh> {
        let positions = &mesh.positions;
        let mode = match mesh.topology {
            PrimitiveTopology::TriangleList => glow::TRIANGLES,
            PrimitiveTopology::TriangleStrip => glow::TRIANGLE_STRIP,
            PrimitiveTopology::LineList => glow::LINES,
            PrimitiveTopology::LineStrip => glow::LINE_STRIP,
            PrimitiveTopology::PointList => glow::POINTS,
        };
        // SAFETY: the context is current; the data outlives the calls.
        unsafe {
            let vao = gl.create_vertex_array().ok()?;
            gl.bind_vertex_array(Some(vao));
            let mut buffers = Vec::new();
            let mut attribute = |location: u32, components: i32, bytes: &[u8]| -> Option<()> {
                let vbo = gl.create_buffer().ok()?;
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(location, components, glow::FLOAT, false, 0, 0);
                buffers.push(vbo);
                Some(())
            };
            let bytes =
                |v: &[f32]| std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 4);
            attribute(0, 3, bytes(positions.as_flattened()))?;
            match &mesh.normals {
                Some(n) => attribute(1, 3, bytes(n.as_flattened()))?,
                None => gl.vertex_attrib_3_f32(1, 0.0, 1.0, 0.0),
            }
            match &mesh.uvs {
                Some(uv) => attribute(2, 2, bytes(uv.as_flattened()))?,
                // (No UVs: a halo's fade is taken half way.)
                None => gl.vertex_attrib_2_f32(2, 0.5, 0.5),
            }
            match &mesh.colours {
                Some(c) => attribute(3, 4, bytes(c.as_flattened()))?,
                // (No colours: no tint.)
                None => gl.vertex_attrib_4_f32(3, 0.0, 0.0, 0.0, 0.0),
            }
            let (count, indexed) = match &mesh.indices {
                Some(indices) => {
                    let ebo = gl.create_buffer().ok()?;
                    gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
                    buffers.push(ebo);
                    let (b, n, kind) = match indices {
                        Indices::U16(i) => (
                            std::slice::from_raw_parts(i.as_ptr().cast::<u8>(), i.len() * 2),
                            i.len(),
                            glow::UNSIGNED_SHORT,
                        ),
                        Indices::U32(i) => (
                            std::slice::from_raw_parts(i.as_ptr().cast::<u8>(), i.len() * 4),
                            i.len(),
                            glow::UNSIGNED_INT,
                        ),
                    };
                    gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, b, glow::STATIC_DRAW);
                    (n as i32, Some(kind))
                }
                None => (positions.len() as i32, None),
            };
            gl.bind_vertex_array(None);
            Some(GpuMesh {
                vao,
                buffers,
                count,
                indexed,
                mode,
            })
        }
    }

    /// The framebuffer drawing into image `texture` (both layers), multisampled in tile memory.
    fn framebuffer(
        &mut self,
        texture: glow::Texture,
        size: (u32, u32),
    ) -> Result<glow::Framebuffer, String> {
        let gl = &self.gl;
        if self.depth.is_none_or(|(_, s)| s != size) {
            // SAFETY: the context is current.
            unsafe {
                if let Some((old, _)) = self.depth.take() {
                    gl.delete_texture(old);
                }
                for (_, fb) in self.framebuffers.drain() {
                    gl.delete_framebuffer(fb);
                }
                let depth = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(depth));
                gl.tex_storage_3d(
                    glow::TEXTURE_2D_ARRAY,
                    1,
                    glow::DEPTH_COMPONENT32F,
                    size.0 as i32,
                    size.1 as i32,
                    2,
                );
                gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
                self.depth = Some((depth, size));
            }
        }
        let key = texture.0.get();
        if let Some(&fb) = self.framebuffers.get(&key) {
            return Ok(fb);
        }
        let depth = self.depth.expect("the depth").0;
        // SAFETY: the context is current; both textures have two layers.
        unsafe {
            let fb = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
            for (attachment, tex) in [
                (glow::COLOR_ATTACHMENT0, texture),
                (glow::DEPTH_ATTACHMENT, depth),
            ] {
                match self.multisample {
                    Some(ms) if self.samples > 0 => ms(
                        glow::FRAMEBUFFER,
                        attachment,
                        tex.0.get(),
                        0,
                        self.samples,
                        0,
                        2,
                    ),
                    _ => (self.multiview)(glow::FRAMEBUFFER, attachment, tex.0.get(), 0, 0, 2),
                }
            }
            let complete =
                gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if !complete {
                gl.delete_framebuffer(fb);
                return Err("OpenGL can't draw into the headset's images".into());
            }
            self.framebuffers.insert(key, fb);
            Ok(fb)
        }
    }

    /// Draws `list` seen by the eyes `views` into image `texture`, `size` big, in one pass:
    /// the lit meshes and the accents (opaque), then the halos (added, depth tested only).
    /// The scope's view, if it is held up to an eye, is drawn before into its own image.
    pub fn draw(
        &mut self,
        list: &DrawList,
        views: &[xr::View],
        texture: glow::Texture,
        size: (u32, u32),
        srgb: bool,
    ) -> Result<(), String> {
        let mut stats = Stats {
            culled: list.culled,
            ..default()
        };
        let scoped = match list.scope.and_then(|s| s.view) {
            Some((view_proj, eye)) => {
                stats.draws += self.draw_scope(list, EyeView { view_proj, eye }, srgb)?;
                true
            }
            None => false,
        };
        let fb = self.framebuffer(texture, size)?;
        let eyes = eye_views(list.rig, views, 1.0);
        self.begin(fb, (size.0 as i32, size.1 as i32), srgb, list.clear);
        stats.draws += self.scene(&self.eyes, list, &eyes, Some(scoped));
        // SAFETY: the context is current.
        unsafe {
            let gl = &self.gl;
            gl.invalidate_framebuffer(glow::FRAMEBUFFER, &[glow::DEPTH_ATTACHMENT]);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.flush();
        }
        self.stats = stats;
        Ok(())
    }

    /// Binds `fb` (`size`) and clears it to `clear`, ready to draw the opaque meshes.
    fn begin(&self, fb: glow::Framebuffer, size: (i32, i32), srgb: bool, clear: Vec4) {
        let gl = &self.gl;
        // SAFETY: the context is current; the framebuffer is this renderer's.
        unsafe {
            if srgb {
                gl.enable(glow::FRAMEBUFFER_SRGB);
            } else {
                gl.disable(glow::FRAMEBUFFER_SRGB);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
            gl.viewport(0, 0, size.0, size.1);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::GREATER);
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.enable(glow::CULL_FACE);
            gl.cull_face(glow::BACK);
            gl.front_face(glow::CCW);
            let c = aces(clear.truncate());
            gl.clear_color(c.x, c.y, c.z, 1.0);
            gl.clear_depth_f32(0.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }
    }

    /// The scope's view of `list` from `eye` into its image (mipmapped for the lens): the
    /// draws.
    fn draw_scope(&mut self, list: &DrawList, eye: EyeView, srgb: bool) -> Result<u32, String> {
        let target = self.scope_target(srgb)?;
        self.begin(target.0, (SCOPE_SIZE, SCOPE_SIZE), srgb, list.clear);
        let draws = self.scene(&self.scope, list, &[eye], None);
        // SAFETY: the context is current; the texture is this renderer's.
        unsafe {
            let gl = &self.gl;
            gl.invalidate_framebuffer(glow::FRAMEBUFFER, &[glow::DEPTH_ATTACHMENT]);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(target.1));
            gl.generate_mipmap(glow::TEXTURE_2D_ARRAY);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
        }
        Ok(draws)
    }

    /// The scope's framebuffer and image, made (again) when first needed or the image's
    /// encoding changes.
    fn scope_target(&mut self, srgb: bool) -> Result<(glow::Framebuffer, glow::Texture), String> {
        if let Some(t) = &self.scope_target
            && t.srgb == srgb
        {
            return Ok((t.fb, t.colour));
        }
        let gl = &self.gl;
        // SAFETY: the context is current; the objects are this renderer's.
        unsafe {
            if let Some(old) = self.scope_target.take() {
                gl.delete_framebuffer(old.fb);
                gl.delete_texture(old.colour);
                gl.delete_texture(old.depth);
            }
            let colour = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(colour));
            let format = if srgb {
                glow::SRGB8_ALPHA8
            } else {
                glow::RGBA8
            };
            gl.tex_storage_3d(
                glow::TEXTURE_2D_ARRAY,
                SCOPE_LEVELS,
                format,
                SCOPE_SIZE,
                SCOPE_SIZE,
                1,
            );
            for (k, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, k, v as i32);
            }
            let depth = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(depth));
            gl.tex_storage_3d(
                glow::TEXTURE_2D_ARRAY,
                1,
                glow::DEPTH_COMPONENT32F,
                SCOPE_SIZE,
                SCOPE_SIZE,
                1,
            );
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, None);
            let fb = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
            // Not multisampled: rendering into a mipmapped image that way came out garbled
            // (Zink), and the mipmaps smooth the view, shrunk onto the lens, anyway.
            for (attachment, tex) in [
                (glow::COLOR_ATTACHMENT0, colour),
                (glow::DEPTH_ATTACHMENT, depth),
            ] {
                (self.multiview)(glow::FRAMEBUFFER, attachment, tex.0.get(), 0, 0, 1);
            }
            let complete =
                gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if !complete {
                gl.delete_framebuffer(fb);
                gl.delete_texture(colour);
                gl.delete_texture(depth);
                return Err("OpenGL can't draw the scope's view".into());
            }
            self.scope_target = Some(ScopeTarget {
                colour,
                depth,
                fb,
                srgb,
            });
            Ok((fb, colour))
        }
    }

    /// Draws `list` seen by `eyes` with `progs` into the framebuffer bound (cleared): the
    /// draws. `lens`: the eyes' pass, drawing the scope's lens (Some(true): its view is
    /// drawn) and the overlays; None: the scope's own view.
    fn scene(
        &self,
        progs: &Programs,
        list: &DrawList,
        eyes: &[EyeView],
        lens: Option<bool>,
    ) -> u32 {
        let gl = &self.gl;
        let mut draws = 0;
        // (What this view sees.)
        let view = if lens.is_some() {
            super::extract::IN_EYES
        } else {
            super::extract::IN_SCOPE
        };
        // SAFETY: the context is current; the programs and meshes are this renderer's.
        unsafe {
            let view_proj: Vec<f32> = eyes
                .iter()
                .flat_map(|e| e.view_proj.to_cols_array())
                .collect();
            let eye_pos: Vec<f32> = eyes.iter().flat_map(|e| e.eye.to_array()).collect();
            let n = &list.neon;
            let pulse_pos: Vec<f32> = n.pulse_pos.iter().flat_map(|v| v.to_array()).collect();
            let pulse_colour: Vec<f32> = n.pulse_colour.iter().flat_map(|v| v.to_array()).collect();
            let shots: Vec<f32> = n.shots.iter().flat_map(|v| v.to_array()).collect();
            for p in [&progs.lit, &progs.accent, &progs.glow] {
                gl.use_program(Some(p.program));
                gl.uniform_matrix_4_f32_slice(p.view_proj.as_ref(), false, &view_proj);
                gl.uniform_3_f32_slice(p.eye.as_ref(), &eye_pos);
                gl.uniform_3_f32_slice(p.light_dir.as_ref(), &list.light.0.to_array());
                gl.uniform_3_f32_slice(p.light_colour.as_ref(), &list.light.1.to_array());
                gl.uniform_3_f32_slice(p.ambient.as_ref(), &list.light.2.to_array());
                gl.uniform_1_f32(p.time.as_ref(), n.time);
                gl.uniform_3_f32_slice(p.red.as_ref(), &n.red.to_array());
                gl.uniform_3_f32_slice(p.blue.as_ref(), &n.blue.to_array());
                gl.uniform_4_f32_slice(p.pulse_pos.as_ref(), &pulse_pos);
                gl.uniform_4_f32_slice(p.pulse_colour.as_ref(), &pulse_colour);
                gl.uniform_4_f32_slice(p.shots.as_ref(), &shots);
            }

            gl.use_program(Some(progs.lit.program));
            let point_pos: Vec<f32> = list.points.iter().flat_map(|p| p.0.to_array()).collect();
            let point_colour: Vec<f32> = list.points.iter().flat_map(|p| p.1.to_array()).collect();
            gl.uniform_1_i32(progs.lit.point_count.as_ref(), list.points.len() as i32);
            if !list.points.is_empty() {
                gl.uniform_4_f32_slice(progs.lit.point_pos.as_ref(), &point_pos);
                gl.uniform_3_f32_slice(progs.lit.point_colour.as_ref(), &point_colour);
            }
            for item in list.lit.iter().filter(|i| i.views & view != 0) {
                draws += self.draw_lit(&progs.lit, item) as u32;
            }
            gl.use_program(Some(progs.accent.program));
            for item in list.accents.iter().filter(|i| i.views & view != 0) {
                draws += self.draw_neon(&progs.accent, item) as u32;
            }
            if let (Some(on), Some(scope)) = (lens, list.scope) {
                draws += self.draw_lens(&view_proj, &scope, on) as u32;
            }
            // The halos: added over everything, hidden behind what is opaque.
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.depth_mask(false);
            gl.disable(glow::CULL_FACE);
            gl.use_program(Some(progs.glow.program));
            for item in list.glows.iter().filter(|i| i.views & view != 0) {
                draws += self.draw_neon(&progs.glow, item) as u32;
            }
            // The overlays (the menus) over everything: depth cleared, and no event pulses on
            // them (as neon.wgsl's OVERLAY).
            if lens.is_some()
                && (!list.overlay_accents.is_empty() || !list.overlay_glows.is_empty())
            {
                let none: Vec<f32> = [0.0, 0.0, 0.0, -1.0].repeat(n.pulse_pos.len());
                for p in [&progs.accent, &progs.glow] {
                    gl.use_program(Some(p.program));
                    gl.uniform_4_f32_slice(p.pulse_pos.as_ref(), &none);
                }
                gl.depth_mask(true);
                gl.clear(glow::DEPTH_BUFFER_BIT);
                gl.disable(glow::BLEND);
                gl.enable(glow::CULL_FACE);
                gl.use_program(Some(progs.accent.program));
                for item in &list.overlay_accents {
                    draws += self.draw_neon(&progs.accent, item) as u32;
                }
                gl.enable(glow::BLEND);
                gl.depth_mask(false);
                gl.disable(glow::CULL_FACE);
                gl.use_program(Some(progs.glow.program));
                for item in &list.overlay_glows {
                    draws += self.draw_neon(&progs.glow, item) as u32;
                }
            }
            gl.disable(glow::BLEND);
            gl.depth_mask(true);
            gl.enable(glow::CULL_FACE);
            gl.bind_vertex_array(None);
        }
        draws
    }

    /// Draws the scope's lens (opaque) seen by the eyes `view_proj`: its view if `on` (drawn
    /// into the scope's image this frame), else dark glass.
    fn draw_lens(&self, view_proj: &[f32], scope: &super::extract::ScopeDraw, on: bool) -> bool {
        let gl = &self.gl;
        let p = &self.lens;
        let image = self.scope_target.as_ref().map(|t| t.colour).filter(|_| on);
        // SAFETY: the context is current; the program, texture and disc are this renderer's.
        unsafe {
            gl.use_program(Some(p.program));
            gl.uniform_matrix_4_f32_slice(p.view_proj.as_ref(), false, view_proj);
            gl.uniform_matrix_4_f32_slice(p.model.as_ref(), false, &scope.lens.to_cols_array());
            gl.uniform_4_f32_slice(p.colour.as_ref(), &scope.colour.extend(1.0).to_array());
            gl.uniform_1_f32(p.on.as_ref(), if image.is_some() { 1.0 } else { 0.0 });
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, image);
            gl.bind_vertex_array(Some(self.disc.vao));
        }
        self.submit(&self.disc);
        // SAFETY: as above.
        unsafe { gl.bind_texture(glow::TEXTURE_2D_ARRAY, None) };
        true
    }

    /// A fence after everything asked of the GPU so far: the image just drawn is released to
    /// the runtime only once it has passed (`wait`). The runtime may show an image as soon as
    /// it is released, and one half drawn showed its last contents.
    pub fn fence(&self) -> Option<glow::Fence> {
        // SAFETY: the context is current.
        unsafe {
            let fence = self.gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0).ok();
            // Submitted now: Zink's glFlush leaves the frame's work queued; a wait that flushes
            // and doesn't wait sends it.
            if let Some(f) = fence {
                self.gl
                    .client_wait_sync(f, glow::SYNC_FLUSH_COMMANDS_BIT, 0);
            }
            fence
        }
    }

    /// Waits for `fence` (`fence`).
    pub fn wait(&self, fence: glow::Fence) {
        // SAFETY: the context is current; the fence is this renderer's.
        unsafe {
            self.gl
                .client_wait_sync(fence, glow::SYNC_FLUSH_COMMANDS_BIT, 100_000_000);
            self.gl.delete_sync(fence);
        }
    }

    /// Saves both eyes of image `texture` as `xr-eye0.png` and `xr-eye1.png` (as `LT_XR_SHOT`
    /// does for Bevy's headset images): what the headset is given; and the scope's view, if
    /// it was drawn, as `xr-scope0.png`.
    pub fn save(&self, texture: glow::Texture, size: (u32, u32)) -> Result<(), String> {
        self.save_layers(texture, size, 2, "xr-eye")?;
        // (And the scope's view, if it was drawn: xr-scope0.png.)
        if let Some(t) = &self.scope_target {
            let s = SCOPE_SIZE as u32;
            self.save_layers(t.colour, (s, s), 1, "xr-scope")?;
        }
        Ok(())
    }

    /// Saves `layers` layers of `texture` as `<name><layer>.png`.
    fn save_layers(
        &self,
        texture: glow::Texture,
        size: (u32, u32),
        layers: i32,
        name: &str,
    ) -> Result<(), String> {
        let gl = &self.gl;
        let (w, h) = (size.0 as usize, size.1 as usize);
        for layer in 0..layers {
            let mut pixels = vec![0u8; w * h * 4];
            // SAFETY: the context is current; `pixels` holds the whole layer.
            unsafe {
                let fb = gl.create_framebuffer()?;
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fb));
                gl.framebuffer_texture_layer(
                    glow::READ_FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    Some(texture),
                    0,
                    layer,
                );
                gl.read_pixels(
                    0,
                    0,
                    size.0 as i32,
                    size.1 as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut pixels)),
                );
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
                gl.delete_framebuffer(fb);
            }
            // OpenGL's rows go bottom up.
            let rows: Vec<u8> = pixels
                .chunks_exact(w * 4)
                .rev()
                .flatten()
                .copied()
                .collect();
            let path = format!("{name}{layer}.png");
            image::save_buffer(&path, &rows, size.0, size.1, image::ColorType::Rgba8)
                .map_err(|e| format!("{path}: {e}"))?;
            info!("glvr: saved {path}");
        }
        Ok(())
    }

    fn bind_mesh(&self, id: AssetId<Mesh>) -> Option<&GpuMesh> {
        let m = self.meshes.get(&id)?;
        // SAFETY: the context is current; the vertex array is this renderer's.
        unsafe { self.gl.bind_vertex_array(Some(m.vao)) };
        Some(m)
    }

    fn submit(&self, m: &GpuMesh) {
        // SAFETY: the context is current; the mesh's vertex array is bound.
        unsafe {
            match m.indexed {
                Some(kind) => self.gl.draw_elements(m.mode, m.count, kind, 0),
                None => self.gl.draw_arrays(m.mode, 0, m.count),
            }
        }
    }

    /// Draws a lit mesh with `p` (in use): whether it was there to draw.
    fn draw_lit(&self, p: &Program, item: &LitItem) -> bool {
        let Some(m) = self.bind_mesh(item.mesh) else {
            return false;
        };
        let gl = &self.gl;
        let normal = Mat3::from_mat4(item.model).inverse().transpose();
        // SAFETY: the context is current.
        unsafe {
            gl.uniform_matrix_4_f32_slice(p.model.as_ref(), false, &item.model.to_cols_array());
            gl.uniform_matrix_3_f32_slice(p.normal.as_ref(), false, &normal.to_cols_array());
            gl.uniform_4_f32_slice(p.base.as_ref(), &item.base.to_array());
            if item.double_sided {
                gl.disable(glow::CULL_FACE);
            }
        }
        self.submit(m);
        if item.double_sided {
            // SAFETY: the context is current.
            unsafe { gl.enable(glow::CULL_FACE) };
        }
        true
    }

    /// Draws a neon mesh with `p` (in use): whether it was there to draw.
    fn draw_neon(&self, p: &Program, item: &NeonItem) -> bool {
        let Some(m) = self.bind_mesh(item.mesh) else {
            return false;
        };
        let gl = &self.gl;
        // SAFETY: the context is current.
        unsafe {
            gl.uniform_matrix_4_f32_slice(p.model.as_ref(), false, &item.model.to_cols_array());
            gl.uniform_4_f32_slice(p.colour.as_ref(), &item.colour.to_array());
            gl.uniform_4_f32_slice(p.params.as_ref(), &item.params.to_array());
            gl.uniform_4_f32_slice(p.extra.as_ref(), &item.extra.to_array());
        }
        self.submit(m);
        true
    }
}

/// The scope's lens: a unit disc facing +Z, its UVs across it (u right, v up).
fn disc() -> MeshData {
    let n = DISC_SEGMENTS;
    let mut positions = vec![[0.0, 0.0, 0.0]];
    let mut uvs = vec![[0.5, 0.5]];
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let (y, x) = a.sin_cos();
        positions.push([x, y, 0.0]);
        uvs.push([0.5 + 0.5 * x, 0.5 + 0.5 * y]);
    }
    let indices = (0..n).flat_map(|i| [0, 1 + i, 1 + (i + 1) % n]).collect();
    MeshData {
        positions,
        normals: Some(vec![[0.0, 0.0, 1.0]; n as usize + 1]),
        uvs: Some(uvs),
        colours: None,
        indices: Some(Indices::U16(indices)),
        topology: PrimitiveTopology::TriangleList,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reversed_infinite_projection() {
        let m = projection_tan(-1.0, 1.0, -1.0, 1.0, NEAR);
        // The near plane at depth 1, far away towards 0, straight ahead in the middle.
        let near = m * Vec4::new(0.0, 0.0, -NEAR, 1.0);
        assert!((near.z / near.w - 1.0).abs() < 1e-6);
        let far = m * Vec4::new(0.0, 0.0, -1.0e6, 1.0);
        assert!(far.z / far.w < 1e-6 && far.z / far.w >= 0.0);
        // Up is up (GL images go bottom up).
        let up = m * Vec4::new(0.0, 0.5, -1.0, 1.0);
        assert!(up.y / up.w > 0.0);
        let p = planes(m);
        assert!(
            p.iter()
                .all(|p| p.dot(Vec4::new(0.0, 0.0, -2.0, 1.0)) > 0.0)
        );
        assert!(p[1].dot(Vec4::new(5.0, 0.0, -1.0, 1.0)) < 0.0);
    }
}
