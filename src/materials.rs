//! The neon look: `AccentMaterial` (glowing strips, opaque, HDR so bloom picks it up) and
//! `GlowMaterial` (soft additive halos beside the strips, which fake the bloom where there is
//! none: the foveated GL renderer draws straight into the headset's swapchain). Both share
//! `NeonGlobals` (time, team colours, event pulses), copied into them every frame.
//! Shader: assets/shaders/neon.wgsl (and its GL port assets/shaders/gl/neon.glsl).

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use lt_sim::{PickupKind, Team};

const SHADER: &str = "shaders/neon.wgsl";
pub const PULSES: usize = 8;
/// Shots lighting up the arena's dimmed lights around them (`NeonGlobals::shots`).
pub const SHOT_LIGHTS: usize = 8;

/// Team colours (linear, HDR multipliers come from the material's intensity).
pub const RED: Vec3 = Vec3::new(1.0, 0.06, 0.08);
pub const BLUE: Vec3 = Vec3::new(0.08, 0.35, 1.0);
pub const WHITE: Vec3 = Vec3::new(1.0, 1.0, 1.0);
/// The arena's middle and the scenery: purple, yellow, green, cyan, orange (white is kept for
/// reload stations, the team colours for the halves behind the targets).
pub const ZONE_COLORS: [Vec3; 5] = [
    Vec3::new(0.5, 0.08, 1.0),
    Vec3::new(1.0, 0.75, 0.04),
    Vec3::new(0.08, 1.0, 0.2),
    Vec3::new(0.04, 0.85, 0.9),
    Vec3::new(1.0, 0.35, 0.02),
];

/// A pickup's colour: white grenades, and a colour of its own for every powerup.
pub fn pickup_color(kind: PickupKind) -> Vec3 {
    match kind {
        PickupKind::Grenade => WHITE,
        PickupKind::Rapid => Vec3::new(1.0, 0.75, 0.04),
        PickupKind::Shield => Vec3::new(0.25, 0.9, 1.0),
        PickupKind::Shotgun => Vec3::new(1.0, 0.4, 0.04),
        PickupKind::Cloak => Vec3::new(0.6, 0.15, 1.0),
        PickupKind::Speed => Vec3::new(0.6, 1.0, 0.05),
        PickupKind::Charge => Vec3::new(1.0, 0.1, 0.6),
        PickupKind::Barrier => Vec3::new(0.0, 1.0, 0.45),
        PickupKind::Ricochet => Vec3::new(1.0, 1.0, 0.2),
        PickupKind::Lance => Vec3::new(0.55, 0.75, 1.0),
        PickupKind::Mirror => Vec3::new(0.8, 0.95, 1.0),
        PickupKind::Sensor => Vec3::new(0.2, 1.0, 0.8),
        PickupKind::Decoy => Vec3::new(1.0, 0.55, 0.85),
        PickupKind::Medbeam => Vec3::new(0.3, 1.0, 0.3),
        PickupKind::Dual => Vec3::new(1.0, 0.3, 0.2),
    }
}

pub fn team_color(team: Option<Team>) -> Vec3 {
    match team {
        Some(Team::Red) => RED,
        Some(Team::Blue) => BLUE,
        None => WHITE,
    }
}

#[derive(ShaderType, Clone, Copy, Debug)]
pub struct NeonUniform {
    pub color: Vec4,
    pub params: Vec4,
    pub red: Vec4,
    pub blue: Vec4,
    pub pulse_pos: [Vec4; PULSES],
    pub pulse_color: [Vec4; PULSES],
    /// x: halo shape (0 strip, 1 volume: fades where seen edge on, 2 haze, 3 forcefield, 4
    /// cloak), y: flow speed (m/s, mode 2), z: 1 for the arena's own lights (dimmed in a
    /// blackout), w: how bright those are now (`NeonGlobals::dim`).
    pub extra: Vec4,
    /// Shots near the eye (xyz, w strength; 0: none): the dimmed lights brighten around them.
    pub shots: [Vec4; SHOT_LIGHTS],
}

impl NeonUniform {
    /// Coloured by the half of the arena it's in.
    pub fn team_zoned(intensity: f32) -> Self {
        Self::new(Vec3::ONE, intensity, 0.0)
    }

    pub fn fixed(color: Vec3, intensity: f32) -> Self {
        Self::new(color, intensity, 1.0)
    }

    /// A fixed colour with pulses of light flowing along the strip (uv.x: metres) at `speed`.
    pub fn flow(color: Vec3, intensity: f32, speed: f32) -> Self {
        let mut n = Self::new(color, intensity, 2.0);
        n.extra.y = speed;
        n
    }

    /// As a halo: a volume (light cones), fading where seen edge on.
    pub fn volume(mut self) -> Self {
        self.extra.x = 1.0;
        self
    }

    /// As a halo: haze, in slowly drifting patches.
    pub fn haze(mut self) -> Self {
        self.extra.x = 2.0;
        self
    }

    /// As a halo: a translucent forcefield (uv 0..1 across and up the sheet).
    pub fn shield(mut self) -> Self {
        self.extra.x = 3.0;
        self
    }

    /// One of the arena's own lights: dimmed in a blackout (`NeonGlobals::dim`).
    pub fn dimmable(mut self) -> Self {
        self.extra.z = 1.0;
        self.extra.w = 1.0;
        self
    }

    /// As a halo: a cloaked player's shimmer, brightest at the silhouette.
    pub fn cloak(mut self) -> Self {
        self.extra.x = 4.0;
        self
    }

    fn new(color: Vec3, intensity: f32, mode: f32) -> Self {
        NeonUniform {
            color: color.extend(intensity),
            params: Vec4::new(
                mode,
                0.0,
                0.0,
                lt_sim::level::ArenaSize::Small.dims().half_length(),
            ),
            red: RED.extend(1.0),
            blue: BLUE.extend(1.0),
            pulse_pos: [Vec4::new(0.0, 0.0, 0.0, -1.0); PULSES],
            pulse_color: [Vec4::ZERO; PULSES],
            extra: Vec4::ZERO,
            shots: [Vec4::ZERO; SHOT_LIGHTS],
        }
    }

    pub fn mode(&self) -> f32 {
        self.params.x
    }

    fn apply(&mut self, g: &NeonGlobals, glow: bool) {
        self.params.y = if glow { 1.0 } else { 0.0 };
        self.params.z = g.time;
        self.params.w = g.half_length;
        self.red = RED.extend(1.0);
        self.blue = BLUE.extend(1.0);
        self.extra.w = g.dim;
        self.shots = g.shots;
        for (i, p) in g.pulses.iter().enumerate() {
            self.pulse_pos[i] = p.0.extend(p.1);
            self.pulse_color[i] = p.2.extend(1.0);
        }
    }
}

/// How deep overlays draw: the world's depth from the near plane to this far is kept in front
/// of them (reversed depth, 1 at the near plane), in step with neon.wgsl's OVERLAY_DEPTH.
pub const OVERLAY_DEPTH: f32 = 0.01;

/// The variant of a neon material's pipeline: an overlay (the VR menus, scoreboards and the
/// pointers at them) is drawn over the world, never hidden behind it, and event pulses don't
/// light it up. The GL headset renderer draws overlays last, after clearing the depth.
#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct NeonKey {
    overlay: bool,
}

fn specialize_neon(descriptor: &mut RenderPipelineDescriptor, key: NeonKey) {
    if key.overlay
        && let Some(fragment) = descriptor.fragment.as_mut()
    {
        fragment.shader_defs.push("OVERLAY".into());
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(NeonKey)]
pub struct AccentMaterial {
    #[uniform(0)]
    pub neon: NeonUniform,
    /// Drawn over the world (`NeonKey`).
    pub overlay: bool,
}

impl From<&AccentMaterial> for NeonKey {
    fn from(m: &AccentMaterial) -> Self {
        NeonKey { overlay: m.overlay }
    }
}

impl Material for AccentMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_neon(descriptor, key.bind_group_data);
        Ok(())
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(NeonKey)]
pub struct GlowMaterial {
    #[uniform(0)]
    pub neon: NeonUniform,
    /// Drawn over the world (`NeonKey`).
    pub overlay: bool,
}

impl From<&GlowMaterial> for NeonKey {
    fn from(m: &GlowMaterial) -> Self {
        NeonKey { overlay: m.overlay }
    }
}

impl Material for GlowMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_neon(descriptor, key.bind_group_data);
        descriptor.primitive.cull_mode = None;
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                    alpha: BlendComponent::OVER,
                });
            }
        }
        Ok(())
    }
}

/// Shared animation state of every neon surface.
#[derive(Resource)]
pub struct NeonGlobals {
    pub time: f32,
    /// (origin, start time (negative: unused), colour).
    pub pulses: [(Vec3, f32, Vec3); PULSES],
    next: usize,
    /// Half the arena's length (m): the waves run from its ends.
    pub half_length: f32,
    /// How bright the arena's own lights are (1; less in a blackout).
    pub dim: f32,
    /// Shots lighting up those around them (xyz, w strength): set every frame by
    /// src/actors.rs (`shot_lights`).
    pub shots: [Vec4; SHOT_LIGHTS],
}

impl Default for NeonGlobals {
    fn default() -> Self {
        NeonGlobals {
            time: 0.0,
            pulses: [(Vec3::ZERO, -1.0, Vec3::ZERO); PULSES],
            next: 0,
            half_length: lt_sim::level::ArenaSize::Small.dims().half_length(),
            dim: 1.0,
            shots: [Vec4::ZERO; SHOT_LIGHTS],
        }
    }
}

impl NeonGlobals {
    /// A ripple of `color` running out from `origin` over the accents.
    pub fn pulse(&mut self, origin: Vec3, color: Vec3) {
        self.pulses[self.next] = (origin, self.time, color);
        self.next = (self.next + 1) % PULSES;
    }
}

/// Materials everything shares.
#[derive(Resource, Clone)]
pub struct Palette {
    pub body: Handle<StandardMaterial>,
    pub suit: Handle<StandardMaterial>,
    /// Lighter surfaces of the avatars (muzzles, patches).
    pub fur: Handle<StandardMaterial>,
    pub arena_accent: Handle<AccentMaterial>,
    pub arena_glow: Handle<GlowMaterial>,
    /// The border walls around the arena: a fixed colour, turned slowly through the rainbow and
    /// pulsing with the music's bass (`animate_border`).
    pub border_accent: Handle<AccentMaterial>,
    pub border_glow: Handle<GlowMaterial>,
    /// Fixed colours: red, blue, white.
    pub accent: [Handle<AccentMaterial>; 3],
    pub glow: [Handle<GlowMaterial>; 3],
    /// Brighter, for bolts.
    pub bolt: [Handle<AccentMaterial>; 3],
    /// Avatars: eyes (bright) and flowing stripes.
    pub eyes: [Handle<AccentMaterial>; 3],
    pub flow: [Handle<AccentMaterial>; 3],
    /// Pickups' accents and halos, per kind (`pickup_color`).
    pub pickup:
        std::collections::HashMap<PickupKind, (Handle<AccentMaterial>, Handle<GlowMaterial>)>,
}

impl Palette {
    pub fn accent_for(&self, team: Option<Team>) -> Handle<AccentMaterial> {
        self.accent[team.map_or(2, |t| t.index())].clone()
    }

    pub fn glow_for(&self, team: Option<Team>) -> Handle<GlowMaterial> {
        self.glow[team.map_or(2, |t| t.index())].clone()
    }

    pub fn bolt_for(&self, team: Option<Team>) -> Handle<AccentMaterial> {
        self.bolt[team.map_or(2, |t| t.index())].clone()
    }

    pub fn eyes_for(&self, team: Option<Team>) -> Handle<AccentMaterial> {
        self.eyes[team.map_or(2, |t| t.index())].clone()
    }

    pub fn flow_for(&self, team: Option<Team>) -> Handle<AccentMaterial> {
        self.flow[team.map_or(2, |t| t.index())].clone()
    }

    pub fn pickup_for(&self, kind: PickupKind) -> (Handle<AccentMaterial>, Handle<GlowMaterial>) {
        self.pickup[&kind].clone()
    }
}

pub struct MaterialsPlugin;

impl Plugin for MaterialsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<AccentMaterial>::default(),
            MaterialPlugin::<GlowMaterial>::default(),
        ))
        .init_resource::<NeonGlobals>()
        .add_systems(PreStartup, make_palette)
        .add_systems(Update, animate_border)
        .add_systems(PostUpdate, animate);
    }
}

fn make_palette(
    mut commands: Commands,
    mut std: ResMut<Assets<StandardMaterial>>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
) {
    let body = std.add(StandardMaterial {
        base_color: Color::linear_rgb(0.012, 0.012, 0.016),
        perceptual_roughness: 0.45,
        reflectance: 0.35,
        ..default()
    });
    let suit = std.add(StandardMaterial {
        base_color: Color::linear_rgb(0.03, 0.03, 0.035),
        perceptual_roughness: 0.6,
        ..default()
    });
    let fur = std.add(StandardMaterial {
        base_color: Color::linear_rgb(0.14, 0.14, 0.16),
        perceptual_roughness: 0.8,
        ..default()
    });
    let fixed = |c: Vec3, i: f32| NeonUniform::fixed(c, i);
    let colors = [RED, BLUE, WHITE];
    commands.insert_resource(Palette {
        body,
        suit,
        fur,
        arena_accent: accents.add(AccentMaterial {
            overlay: false,
            neon: NeonUniform::team_zoned(3.0).dimmable(),
        }),
        arena_glow: glows.add(GlowMaterial {
            overlay: false,
            neon: NeonUniform::team_zoned(2.0).dimmable(),
        }),
        border_accent: accents.add(AccentMaterial {
            overlay: false,
            neon: fixed(RED, BORDER_ACCENT).dimmable(),
        }),
        border_glow: glows.add(GlowMaterial {
            overlay: false,
            neon: fixed(RED, BORDER_GLOW).dimmable(),
        }),
        accent: colors.map(|c| {
            accents.add(AccentMaterial {
                overlay: false,
                neon: fixed(c, 3.0),
            })
        }),
        glow: colors.map(|c| {
            glows.add(GlowMaterial {
                overlay: false,
                neon: fixed(c, 2.0),
            })
        }),
        bolt: colors.map(|c| {
            accents.add(AccentMaterial {
                overlay: false,
                neon: fixed(c, 9.0),
            })
        }),
        eyes: colors.map(|c| {
            accents.add(AccentMaterial {
                overlay: false,
                neon: fixed(c, 7.0),
            })
        }),
        flow: colors.map(|c| {
            accents.add(AccentMaterial {
                overlay: false,
                neon: NeonUniform::flow(c, 3.0, 0.8),
            })
        }),
        pickup: PickupKind::ALL
            .iter()
            .map(|&k| {
                let c = pickup_color(k);
                let accent = accents.add(AccentMaterial {
                    overlay: false,
                    neon: fixed(c, 4.0),
                });
                let glow = glows.add(GlowMaterial {
                    overlay: false,
                    neon: fixed(c, 2.0),
                });
                (k, (accent, glow))
            })
            .collect(),
    });
}

/// How bright the arena's own lights are in a blackout.
const BLACKOUT_DIM: f32 = 0.015;

/// The border walls' resting intensities (accents, halos), and the seconds they take to go
/// once round the rainbow.
const BORDER_ACCENT: f32 = 2.6;
const BORDER_GLOW: f32 = 1.6;
const RAINBOW_PERIOD: f32 = 45.0;

/// Turns the border walls slowly through the rainbow, brighter on each beat of the bass.
fn animate_border(
    time: Res<Time>,
    spectrum: Option<Res<crate::music::MusicSpectrum>>,
    palette: Res<Palette>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
) {
    let hue = (time.elapsed_secs_wrapped() / RAINBOW_PERIOD).fract() * 360.0;
    let c = Color::hsv(hue, 0.9, 1.0).to_linear();
    let rgb = Vec3::new(c.red, c.green, c.blue);
    let bass = spectrum.map_or(0.0, |s| s.bass);
    if let Some(mut m) = accents.get_mut(&palette.border_accent) {
        m.neon.color = rgb.extend(BORDER_ACCENT * (0.65 + 0.9 * bass));
    }
    if let Some(mut m) = glows.get_mut(&palette.border_glow) {
        m.neon.color = rgb.extend(BORDER_GLOW * (0.5 + 1.3 * bass));
    }
}

fn animate(
    time: Res<Time>,
    game: Option<Res<crate::game::Game>>,
    mut globals: ResMut<NeonGlobals>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
) {
    globals.time = time.elapsed_secs_wrapped();
    // A blackout dims the arena's own lights (eased, a little quicker going dark).
    let dark = game
        .as_ref()
        .is_some_and(|g| matches!(g.snap.event, Some((lt_sim::ArenaEvent::Blackout, _))));
    let (want, tau) = if dark {
        (BLACKOUT_DIM, 0.35)
    } else {
        (1.0, 0.8)
    };
    let k = 1.0 - (-time.delta_secs() / tau).exp();
    globals.dim += (want - globals.dim) * k;
    if let Some(game) = game {
        globals.half_length = game.level.dims().half_length();
    }
    for (_, m) in accents.iter_mut() {
        m.neon.apply(&globals, false);
    }
    for (_, m) in glows.iter_mut() {
        m.neon.apply(&globals, true);
    }
}
