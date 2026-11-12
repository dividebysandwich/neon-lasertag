//! Reflections on the flat screen (`Settings::reflections`): the floors, walls and models turn
//! glossy and the camera gets screen-space reflections, so the neon shows in them. That needs
//! deferred rendering (the neon accents too: assets/shaders/neon.wgsl writes them into the
//! G-buffer), which can't multisample, so SMAA stands in for MSAA while they're on.
//! Off, everything is drawn forward with MSAA and the surfaces are matte as before.
//!
//! VR keeps forward rendering: the headsets' frame budgets have no room for it, and so does
//! the browser's WebGL2 build (`crate::WEBGL2`), which has no deferred rendering.

use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::core_pipeline::prepass::{DeferredPrepass, DepthPrepass, MotionVectorPrepass};
use bevy::pbr::{DefaultOpaqueRendererMethod, ScreenSpaceReflections};
use bevy::prelude::*;
use bevy::render::camera::{MipBias, TemporalJitter};

use crate::desktop::MainCamera;
use crate::materials::Palette;
use crate::settings::Settings;

pub struct ReflectionsPlugin;

impl Plugin for ReflectionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, apply);
    }
}

/// A surface's (perceptual roughness, reflectance) without and with reflections.
struct Gloss {
    matte: (f32, f32),
    glossy: (f32, f32),
}

/// The arena's floors and walls: a polished black floor, mirroring the neon softly.
const BODY: Gloss = Gloss {
    matte: (0.45, 0.35),
    glossy: (0.03, 0.6),
};
/// The players' suits and blasters: a satin sheen.
const SUIT: Gloss = Gloss {
    matte: (0.6, 0.5),
    glossy: (0.1, 0.55),
};
/// Their lighter patches stay mostly matte.
const FUR: Gloss = Gloss {
    matte: (0.8, 0.5),
    glossy: (0.55, 0.5),
};

fn ssr() -> ScreenSpaceReflections {
    ScreenSpaceReflections {
        // Every surface up to fairly rough reflects: the arena's are all glossy or matte.
        min_perceptual_roughness: 0.0..0.0,
        max_perceptual_roughness: 0.35..0.45,
        linear_steps: 16,
        thickness: 0.3,
        edge_fadeout: 0.0..0.08,
        ..default()
    }
}

/// Switches the reflections on or off when the setting (or the camera) changes.
fn apply(
    mut commands: Commands,
    settings: Res<Settings>,
    palette: Option<Res<Palette>>,
    cams: Query<(Entity, Has<ScreenSpaceReflections>), With<MainCamera>>,
    mut method: ResMut<DefaultOpaqueRendererMethod>,
    mut std: ResMut<Assets<StandardMaterial>>,
    mut done: Local<Option<bool>>,
) {
    let on = settings.reflections && !crate::WEBGL2;
    for (e, has) in &cams {
        if has == on {
            continue;
        }
        let mut c = commands.entity(e);
        if on {
            c.insert((
                ssr(),
                DeferredPrepass,
                Msaa::Off,
                TemporalAntiAliasing::default(),
            ));
        } else {
            c.remove::<(
                ScreenSpaceReflections,
                TemporalAntiAliasing,
                TemporalJitter,
                MipBias,
                DeferredPrepass,
                DepthPrepass,
                MotionVectorPrepass,
            )>()
            .insert(Msaa::Sample4);
        }
    }
    let Some(palette) = palette else { return };
    if *done == Some(on) {
        return;
    }
    *done = Some(on);
    if on {
        method.set_to_deferred();
    } else {
        method.set_to_forward();
    }
    for (handle, g) in [
        (&palette.body, BODY),
        (&palette.suit, SUIT),
        (&palette.avatar, SUIT),
        (&palette.fur, FUR),
    ] {
        if let Some(mut m) = std.get_mut(handle) {
            (m.perceptual_roughness, m.reflectance) = if on { g.glossy } else { g.matte };
        }
    }
    // The render method is read when a material is prepared: iterating mutably marks every
    // one changed, so they're all prepared anew.
    for _ in std.iter_mut() {}
}
