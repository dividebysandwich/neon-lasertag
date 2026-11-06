//! VR on any OpenXR runtime (`--vr`) through bevy_mod_openxr: Bevy's renderer draws the
//! eyes (with bloom); controllers and the head are read into `vr::XrInput`. The action set
//! and bindings follow stealth-mission-89's src/xr.rs.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::camera::Hdr;
use bevy_mod_openxr::action_binding::{OxrSendActionBindings, OxrSuggestActionBinding};
use bevy_mod_openxr::action_set_attaching::OxrAttachActionSet;
use bevy_mod_openxr::action_set_syncing::{OxrActionSetSyncSet, OxrSyncActionSet};
use bevy_mod_openxr::resources::OxrInstance;
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_openxr::spaces::OxrSpaceLocationFlags;
use bevy_mod_openxr::{add_xr_plugins, openxr_session_running};
use bevy_mod_xr::camera::XrCamera;
use bevy_mod_xr::session::{XrPreDestroySession, XrSessionCreated, XrTrackingRoot, session_available};
use bevy_mod_xr::spaces::XrSpaceSyncSet;
use openxr::{Haptic, Posef};

use crate::game::InputSet;
use crate::vr::{VrPlugin, XrInput, XrRig};

/// Whether an OpenXR runtime with Vulkan (what bevy_mod_openxr renders with) is there, without
/// creating an instance (which would start SteamVR).
pub fn runtime_available() -> bool {
    #[cfg(windows)]
    let entry = Some(openxr::Entry::linked());
    // SAFETY: loads the OpenXR loader library.
    #[cfg(not(windows))]
    let entry = unsafe { openxr::Entry::load() }.ok();
    let Some(entry) = entry else {
        info!("OpenXR: no loader");
        return false;
    };
    match entry.enumerate_extensions() {
        Ok(e) => e.khr_vulkan_enable2,
        Err(e) => {
            info!("OpenXR: no runtime ({e})");
            false
        }
    }
}

/// The app's plugins for `--vr`; false if there is no runtime (play on the flat screen then).
pub fn add_plugins(app: &mut App, asset_root: &str) -> bool {
    if !runtime_available() {
        return false;
    }
    app.add_plugins(add_xr_plugins(
        DefaultPlugins
            .set(AssetPlugin { file_path: asset_root.to_string(), ..default() })
            .set(WindowPlugin {
                primary_window: Some(Window { title: "Neon Lasertag (VR)".into(), ..default() }),
                ..default()
            }),
    ))
    .add_plugins((OxrInputPlugin, VrPlugin));
    true
}

struct OxrInputPlugin;

impl Plugin for OxrInputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, create_actions.run_if(session_available))
            .add_systems(OxrSendActionBindings, suggest_bindings)
            .add_systems(XrSessionCreated, (attach_set, spawn_hand_spaces))
            .add_systems(XrPreDestroySession, despawn_hand_spaces)
            .add_systems(
                PreUpdate,
                (
                    sync_actions.before(OxrActionSetSyncSet),
                    read.after(OxrActionSetSyncSet).after(XrSpaceSyncSet),
                )
                    .run_if(openxr_session_running)
                    .run_if(resource_exists::<XrActions>),
            )
            .add_systems(Update, (setup_cameras, find_rig.before(InputSet)))
            .add_systems(Update, head.before(InputSet))
            .add_systems(Update, eye_shots.run_if(openxr_session_running))
            .add_systems(
                PostUpdate,
                haptics.run_if(openxr_session_running).run_if(resource_exists::<XrActions>),
            );
    }
}

#[derive(Resource)]
struct XrActions {
    set: openxr::ActionSet,
    grip: openxr::Action<Posef>,
    aim: openxr::Action<Posef>,
    squeeze: openxr::Action<f32>,
    trigger: openxr::Action<f32>,
    primary: openxr::Action<bool>,
    secondary: openxr::Action<bool>,
    menu: openxr::Action<bool>,
    thumbstick: openxr::Action<openxr::Vector2f>,
    haptic: openxr::Action<Haptic>,
    hands: [openxr::Path; 2],
}

#[derive(Component)]
struct HandSpace {
    hand: usize,
    aim: bool,
}

fn create_actions(instance: Res<OxrInstance>, mut commands: Commands) {
    let result = (|| -> openxr::Result<XrActions> {
        let hands = [instance.string_to_path("/user/hand/left")?, instance.string_to_path("/user/hand/right")?];
        let set = instance.create_action_set("lasertag", "Lasertag", 0)?;
        Ok(XrActions {
            grip: set.create_action("grip_pose", "Grip pose", &hands)?,
            aim: set.create_action("aim_pose", "Aim pose", &hands)?,
            squeeze: set.create_action("grab", "Hold the blaster", &hands)?,
            trigger: set.create_action("fire", "Fire", &hands)?,
            primary: set.create_action("primary", "A / X", &hands)?,
            secondary: set.create_action("grenade", "Grenade (B / Y)", &hands)?,
            menu: set.create_action("menu", "Menu", &hands)?,
            thumbstick: set.create_action("thumbstick", "Move / turn", &hands)?,
            haptic: set.create_action("haptic", "Haptics", &hands)?,
            set,
            hands,
        })
    })();
    match result {
        Ok(a) => commands.insert_resource(a),
        Err(e) => error!("OpenXR actions: {e}"),
    }
}

fn suggest_bindings(actions: Option<Res<XrActions>>, mut out: MessageWriter<OxrSuggestActionBinding>) {
    let Some(a) = actions else { return };
    type B = Vec<std::borrow::Cow<'static, str>>;
    let both = |s: &str| -> B { vec![format!("/user/hand/left/{s}").into(), format!("/user/hand/right/{s}").into()] };
    let per_hand = |l: &str, r: &str| -> B { vec![format!("/user/hand/left/{l}").into(), format!("/user/hand/right/{r}").into()] };
    let left = |s: &str| -> B { vec![format!("/user/hand/left/{s}").into()] };
    // (profile, squeeze, trigger, primary, secondary, thumbstick, menu). Where the menu button
    // is the grenade's, holding it opens the menu (src/vr.rs).
    let profiles: Vec<(&str, B, B, B, B, B, B)> = vec![
        (
            "/interaction_profiles/oculus/touch_controller",
            both("input/squeeze/value"),
            both("input/trigger/value"),
            per_hand("input/x/click", "input/a/click"),
            per_hand("input/y/click", "input/b/click"),
            both("input/thumbstick"),
            left("input/menu/click"),
        ),
        (
            "/interaction_profiles/valve/index_controller",
            both("input/squeeze/value"),
            both("input/trigger/value"),
            both("input/a/click"),
            both("input/b/click"),
            both("input/thumbstick"),
            vec![],
        ),
        (
            "/interaction_profiles/htc/vive_controller",
            both("input/squeeze/click"),
            both("input/trigger/value"),
            vec![],
            both("input/menu/click"),
            both("input/trackpad"),
            vec![],
        ),
        (
            "/interaction_profiles/microsoft/motion_controller",
            both("input/squeeze/click"),
            both("input/trigger/value"),
            vec![],
            both("input/menu/click"),
            both("input/thumbstick"),
            vec![],
        ),
        (
            "/interaction_profiles/khr/simple_controller",
            vec![],
            both("input/select/click"),
            vec![],
            both("input/menu/click"),
            vec![],
            vec![],
        ),
    ];
    for (profile, squeeze, trigger, primary, secondary, stick, menu) in profiles {
        let mut add = |action: openxr::sys::Action, bindings: B| {
            if !bindings.is_empty() {
                out.write(OxrSuggestActionBinding { action, interaction_profile: profile.into(), bindings });
            }
        };
        add(a.grip.as_raw(), both("input/grip/pose"));
        add(a.aim.as_raw(), both("input/aim/pose"));
        add(a.haptic.as_raw(), both("output/haptic"));
        add(a.squeeze.as_raw(), squeeze);
        add(a.trigger.as_raw(), trigger);
        add(a.primary.as_raw(), primary);
        add(a.secondary.as_raw(), secondary);
        add(a.thumbstick.as_raw(), stick);
        add(a.menu.as_raw(), menu);
    }
}

fn attach_set(actions: Option<Res<XrActions>>, mut attach: MessageWriter<OxrAttachActionSet>) {
    if let Some(a) = actions {
        attach.write(OxrAttachActionSet(a.set.clone()));
    }
}

fn sync_actions(actions: Res<XrActions>, mut sync: MessageWriter<OxrSyncActionSet>) {
    sync.write(OxrSyncActionSet(actions.set.clone()));
}

fn spawn_hand_spaces(actions: Option<Res<XrActions>>, session: Res<OxrSession>, mut commands: Commands) {
    let Some(a) = actions else { return };
    for hand in 0..2 {
        for aim in [false, true] {
            let action = if aim { &a.aim } else { &a.grip };
            match session.create_action_space(action, a.hands[hand], Isometry3d::IDENTITY) {
                Ok(space) => {
                    commands.spawn((space, HandSpace { hand, aim }));
                }
                Err(e) => error!("OpenXR hand space: {e}"),
            }
        }
    }
}

fn despawn_hand_spaces(mut commands: Commands, q: Query<Entity, With<HandSpace>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

fn read(
    actions: Res<XrActions>,
    session: Res<OxrSession>,
    spaces: Query<(&HandSpace, &Transform, &OxrSpaceLocationFlags)>,
    mut input: ResMut<XrInput>,
) {
    for (i, hand) in input.hands.iter_mut().enumerate() {
        let path = actions.hands[i];
        let f = |a: &openxr::Action<f32>| a.state(&session, path).map(|s| s.current_state).unwrap_or(0.0);
        let b = |a: &openxr::Action<bool>| a.state(&session, path).is_ok_and(|s| s.current_state);
        hand.squeeze = f(&actions.squeeze);
        hand.trigger = f(&actions.trigger);
        hand.primary = b(&actions.primary);
        hand.secondary = b(&actions.secondary);
        hand.menu = b(&actions.menu);
        hand.stick = actions.thumbstick.state(&session, path).map_or(Vec2::ZERO, |s| Vec2::new(s.current_state.x, s.current_state.y));
        hand.grip = None;
        hand.aim = None;
        for (s, tf, flags) in &spaces {
            if s.hand == i && flags.pos_valid() && flags.rot_valid() {
                if s.aim {
                    hand.aim = Some(*tf);
                } else {
                    hand.grip = Some(*tf);
                }
            }
        }
    }
}

/// The head: between the two eye cameras (their transforms are in the tracking space).
fn head(cams: Query<&Transform, With<XrCamera>>, mut input: ResMut<XrInput>) {
    let n = cams.iter().len();
    if n == 0 {
        return;
    }
    let pos = cams.iter().map(|t| t.translation).sum::<Vec3>() / n as f32;
    if pos == Vec3::ZERO {
        return;
    }
    let rot = cams.iter().next().unwrap().rotation;
    input.head = Some(Transform { translation: pos, rotation: rot, ..default() });
}

fn find_rig(mut commands: Commands, root: Query<Entity, With<XrTrackingRoot>>, rig: Option<Res<XrRig>>) {
    if rig.is_none()
        && let Ok(r) = root.single()
    {
        commands.insert_resource(XrRig(r));
    }
}

/// The eye cameras: HDR with bloom (`LT_XR_BLOOM=0` turns it off); one of them listens.
fn setup_cameras(mut commands: Commands, cams: Query<Entity, Added<XrCamera>>, mut first: Local<bool>) {
    let bloom = std::env::var("LT_XR_BLOOM").map_or(true, |v| v != "0");
    for e in &cams {
        let mut c = commands.entity(e);
        if bloom {
            c.insert((Hdr, Tonemapping::AcesFitted, Bloom { intensity: 0.2, ..Bloom::NATURAL }));
        } else {
            c.insert(Tonemapping::None);
        }
        if !*first {
            *first = true;
            c.insert(SpatialListener::new(0.2));
        }
    }
}

/// This frame's vibrations (`XrInput::haptic`, set every frame while a pattern plays: each
/// lasts a little longer than a frame).
fn haptics(actions: Res<XrActions>, session: Res<OxrSession>, mut input: ResMut<XrInput>) {
    for i in 0..2 {
        let a = input.haptic[i];
        if a > 0.0 {
            let v = openxr::HapticVibration::new()
                .amplitude(a.min(1.0))
                .duration(openxr::Duration::from_nanos(crate::vr::HAPTIC_NANOS))
                .frequency(openxr::FREQUENCY_UNSPECIFIED);
            if let Err(e) = actions.haptic.apply_feedback(&session, actions.hands[i], &v) {
                debug!("haptics: {e}");
            }
            input.haptic[i] = 0.0;
        }
    }
}

/// `LT_XR_SHOT=<secs>`: saves both eyes' images that long into the run (xr-eye0.png, xr-eye1.png).
fn eye_shots(mut commands: Commands, time: Res<Time>, mut done: Local<bool>) {
    use bevy::render::view::screenshot::{Screenshot, save_to_disk};
    if *done {
        return;
    }
    let Some(at) = std::env::var("LT_XR_SHOT").ok().and_then(|v| v.parse::<f32>().ok()) else {
        *done = true;
        return;
    };
    if time.elapsed_secs() < at {
        return;
    }
    *done = true;
    for eye in 0..2 {
        let handle = bevy::camera::ManualTextureViewHandle(bevy_mod_openxr::render::XR_TEXTURE_INDEX + eye);
        commands.spawn(Screenshot::texture_view(handle)).observe(save_to_disk(format!("xr-eye{eye}.png")));
    }
}
