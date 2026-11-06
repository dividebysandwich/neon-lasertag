//! The controllers in the GL headset session: the action set and bindings of `crate::xr`
//! (`suggest_bindings`), read on the render thread each frame at the frame's display time and
//! handed to the main thread with the eyes, where they become `vr::XrInput`'s hands.

use crate::vr::HandInput;
use bevy::prelude::*;
use openxr as xr;

pub struct Controllers {
    set: xr::ActionSet,
    // (The pose actions stay alive while their spaces are used.)
    _grip: xr::Action<xr::Posef>,
    _aim: xr::Action<xr::Posef>,
    squeeze: xr::Action<f32>,
    trigger: xr::Action<f32>,
    primary: xr::Action<bool>,
    secondary: xr::Action<bool>,
    menu: xr::Action<bool>,
    thumbstick: xr::Action<xr::Vector2f>,
    haptic: xr::Action<xr::Haptic>,
    /// /user/hand/left, /user/hand/right
    hands: [xr::Path; 2],
    /// Grip and aim spaces of each hand.
    spaces: [[xr::Space; 2]; 2],
}

fn err(what: &'static str) -> impl Fn(xr::sys::Result) -> String {
    move |e| format!("{what}: {e}")
}

impl Controllers {
    /// The actions, their suggested bindings (as `crate::xr::suggest_bindings`), attached to
    /// `session`.
    pub fn new<G: xr::Graphics>(
        instance: &xr::Instance,
        session: &xr::Session<G>,
    ) -> Result<Self, String> {
        let path = |s: &str| instance.string_to_path(s).map_err(err("an OpenXR path"));
        let hands = [path("/user/hand/left")?, path("/user/hand/right")?];
        let set = instance
            .create_action_set("lasertag", "Lasertag", 0)
            .map_err(err("the action set"))?;
        let e = err("an action");
        let grip = set
            .create_action("grip_pose", "Grip pose", &hands)
            .map_err(&e)?;
        let aim = set
            .create_action("aim_pose", "Aim pose", &hands)
            .map_err(&e)?;
        let squeeze = set
            .create_action("grab", "Hold the blaster", &hands)
            .map_err(&e)?;
        let trigger = set.create_action("fire", "Fire", &hands).map_err(&e)?;
        let primary = set.create_action("primary", "A / X", &hands).map_err(&e)?;
        let secondary = set
            .create_action("grenade", "Grenade (B / Y)", &hands)
            .map_err(&e)?;
        let menu = set.create_action("menu", "Menu", &hands).map_err(&e)?;
        let thumbstick = set
            .create_action("thumbstick", "Move / turn", &hands)
            .map_err(&e)?;
        let haptic = set.create_action("haptic", "Haptics", &hands).map_err(&e)?;

        type B = Vec<String>;
        let both = |s: &str| -> B {
            vec![
                format!("/user/hand/left/{s}"),
                format!("/user/hand/right/{s}"),
            ]
        };
        let per_hand = |l: &str, r: &str| -> B {
            vec![
                format!("/user/hand/left/{l}"),
                format!("/user/hand/right/{r}"),
            ]
        };
        let left = |s: &str| -> B { vec![format!("/user/hand/left/{s}")] };
        // (profile, squeeze, trigger, primary, secondary, thumbstick, menu)
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
        for (profile, sq, tr, pr, se, st, me) in profiles {
            let mut bindings: Vec<xr::Binding> = Vec::new();
            for p in both("input/grip/pose") {
                bindings.push(xr::Binding::new(&grip, path(&p)?));
            }
            for p in both("input/aim/pose") {
                bindings.push(xr::Binding::new(&aim, path(&p)?));
            }
            for p in both("output/haptic") {
                bindings.push(xr::Binding::new(&haptic, path(&p)?));
            }
            for p in &sq {
                bindings.push(xr::Binding::new(&squeeze, path(p)?));
            }
            for p in &tr {
                bindings.push(xr::Binding::new(&trigger, path(p)?));
            }
            for p in &pr {
                bindings.push(xr::Binding::new(&primary, path(p)?));
            }
            for p in &se {
                bindings.push(xr::Binding::new(&secondary, path(p)?));
            }
            for p in &st {
                bindings.push(xr::Binding::new(&thumbstick, path(p)?));
            }
            for p in &me {
                bindings.push(xr::Binding::new(&menu, path(p)?));
            }
            // (A runtime without the profile refuses it: the others still bind.)
            if let Err(e) = instance.suggest_interaction_profile_bindings(path(profile)?, &bindings)
            {
                info!("glvr: no bindings for {profile}: {e}");
            }
        }
        session
            .attach_action_sets(&[&set])
            .map_err(err("attaching the actions"))?;
        let space = |a: &xr::Action<xr::Posef>, hand: xr::Path| {
            a.create_space(session, hand, xr::Posef::IDENTITY)
                .map_err(err("a hand's space"))
        };
        let spaces = [
            [space(&grip, hands[0])?, space(&aim, hands[0])?],
            [space(&grip, hands[1])?, space(&aim, hands[1])?],
        ];
        Ok(Controllers {
            set,
            _grip: grip,
            _aim: aim,
            squeeze,
            trigger,
            primary,
            secondary,
            menu,
            thumbstick,
            haptic,
            hands,
            spaces,
        })
    }

    /// Both controllers at `time`, their poses in `base` (the tracking space).
    pub fn read<G: xr::Graphics>(
        &self,
        session: &xr::Session<G>,
        base: &xr::Space,
        time: xr::Time,
    ) -> [HandInput; 2] {
        if let Err(e) = session.sync_actions(&[xr::ActiveActionSet::new(&self.set)]) {
            debug!("glvr: syncing the actions: {e}");
            return [HandInput::default(); 2];
        }
        let mut out = [HandInput::default(); 2];
        for (i, c) in out.iter_mut().enumerate() {
            let path = self.hands[i];
            let f = |a: &xr::Action<f32>| a.state(session, path).map_or(0.0, |s| s.current_state);
            let b = |a: &xr::Action<bool>| a.state(session, path).is_ok_and(|s| s.current_state);
            c.squeeze = f(&self.squeeze);
            c.trigger = f(&self.trigger);
            c.primary = b(&self.primary);
            c.secondary = b(&self.secondary);
            c.menu = b(&self.menu);
            c.stick = self
                .thumbstick
                .state(session, path)
                .map_or(Vec2::ZERO, |s| {
                    Vec2::new(s.current_state.x, s.current_state.y)
                });
            let locate = |space: &xr::Space| {
                let l = space.locate(base, time).ok()?;
                let flags = xr::SpaceLocationFlags::POSITION_VALID
                    | xr::SpaceLocationFlags::ORIENTATION_VALID;
                l.location_flags.contains(flags).then(|| {
                    let (p, o) = (l.pose.position, l.pose.orientation);
                    Transform::from_xyz(p.x, p.y, p.z)
                        .with_rotation(Quat::from_xyzw(o.x, o.y, o.z, o.w))
                })
            };
            c.grip = locate(&self.spaces[i][0]);
            c.aim = locate(&self.spaces[i][1]);
        }
        out
    }

    /// A vibration of `amount` (0..1) on hand `i`, a little longer than a frame
    /// (`vr::HAPTIC_NANOS`: patterns set their amplitude every frame, see `vr::Rumble`).
    pub fn pulse<G: xr::Graphics>(&self, session: &xr::Session<G>, i: usize, amount: f32) {
        let v = xr::HapticVibration::new()
            .amplitude(amount.min(1.0))
            .duration(xr::Duration::from_nanos(crate::vr::HAPTIC_NANOS))
            .frequency(xr::FREQUENCY_UNSPECIFIED);
        if let Err(e) = self.haptic.apply_feedback(session, self.hands[i], &v) {
            debug!("glvr: haptics: {e}");
        }
    }
}
