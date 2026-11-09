//! The player's settings: the flat screen's key bindings, mouse sensitivity, inverted mouse
//! look, field of view, crouching (held or toggled), head bob and reflections, and the volumes of the sound effects and the music (in VR too);
//! VR: the controller buttons of the actions that aren't gestures (`VrAction`).
//! Changed in the options screen (src/options.rs) and kept in `settings.ron` in the user's
//! config directory (`$XDG_CONFIG_HOME/lasertag`, `~/.config/lasertag`, `%APPDATA%\lasertag`).

use std::path::PathBuf;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Something the player does with a key or a mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Walk,
    Crouch,
    Jump,
    Dash,
    Fire,
    Grenade,
    Shield,
    Gadget,
    Zoom,
    Scores,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::Forward,
        Action::Back,
        Action::Left,
        Action::Right,
        Action::Walk,
        Action::Crouch,
        Action::Jump,
        Action::Dash,
        Action::Fire,
        Action::Grenade,
        Action::Shield,
        Action::Gadget,
        Action::Zoom,
        Action::Scores,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Action::Forward => "Move forward",
            Action::Back => "Move back",
            Action::Left => "Strafe left",
            Action::Right => "Strafe right",
            Action::Walk => "Walk slowly",
            Action::Crouch => "Crouch",
            Action::Jump => "Jump",
            Action::Dash => "Dash",
            Action::Fire => "Fire",
            Action::Grenade => "Grenade (hold, let go)",
            Action::Shield => "Shield (switch on)",
            Action::Gadget => "Gadget (use)",
            Action::Zoom => "Zoom (prism lance)",
            Action::Scores => "Scores (hold)",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Action::Forward => "forward",
            Action::Back => "back",
            Action::Left => "left",
            Action::Right => "right",
            Action::Walk => "walk",
            Action::Crouch => "crouch",
            Action::Jump => "jump",
            Action::Dash => "dash",
            Action::Fire => "fire",
            Action::Grenade => "grenade",
            Action::Shield => "shield",
            Action::Gadget => "barrier",
            Action::Zoom => "zoom",
            Action::Scores => "scores",
        }
    }

    fn defaults(self) -> [Option<Binding>; 2] {
        use Binding::{Key, Mouse, Wheel};
        match self {
            Action::Forward => [Some(Key(KeyCode::KeyW)), Some(Key(KeyCode::ArrowUp))],
            Action::Back => [Some(Key(KeyCode::KeyS)), Some(Key(KeyCode::ArrowDown))],
            Action::Left => [Some(Key(KeyCode::KeyA)), Some(Key(KeyCode::ArrowLeft))],
            Action::Right => [Some(Key(KeyCode::KeyD)), Some(Key(KeyCode::ArrowRight))],
            Action::Walk => [Some(Key(KeyCode::AltLeft)), None],
            Action::Crouch => [Some(Key(KeyCode::ControlLeft)), Some(Key(KeyCode::KeyC))],
            Action::Jump => [Some(Key(KeyCode::Space)), None],
            Action::Dash => [
                Some(Key(KeyCode::ShiftLeft)),
                Some(Mouse(MouseButton::Back)),
            ],
            Action::Fire => [Some(Mouse(MouseButton::Left)), None],
            Action::Grenade => [Some(Key(KeyCode::KeyG)), Some(Mouse(MouseButton::Right))],
            Action::Shield => [Some(Key(KeyCode::KeyQ)), None],
            Action::Gadget => [Some(Key(KeyCode::KeyE)), None],
            Action::Zoom => [Some(Key(KeyCode::KeyZ)), Some(Wheel)],
            Action::Scores => [Some(Key(KeyCode::Tab)), None],
        }
    }
}

/// A VR action on a controller button (the VR controls panel, src/vrui.rs). Firing (the
/// blaster hand's trigger), drawing (gripping at the holster), moving and turning (the sticks)
/// stay as they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VrAction {
    Jump,
    Dash,
    /// Crouching on (and off again), besides crouching for real.
    Crouch,
    /// Held to take a grenade into that hand, let go to throw.
    Grenade,
    Shield,
    Gadget,
    /// Held a second (without a grenade in that hand): the menu, for controllers without a
    /// right menu button.
    Menu,
}

impl VrAction {
    pub const ALL: [VrAction; 7] = [
        VrAction::Jump,
        VrAction::Dash,
        VrAction::Crouch,
        VrAction::Grenade,
        VrAction::Shield,
        VrAction::Gadget,
        VrAction::Menu,
    ];

    /// As the panel names it (capitals: VR's font).
    pub fn name(self) -> &'static str {
        match self {
            VrAction::Jump => "JUMP",
            VrAction::Dash => "DASH",
            VrAction::Crouch => "CROUCH (ON / OFF)",
            VrAction::Grenade => "GRENADE (HOLD)",
            VrAction::Shield => "SHIELD",
            VrAction::Gadget => "GADGET",
            VrAction::Menu => "MENU (HOLD)",
        }
    }

    fn id(self) -> &'static str {
        match self {
            VrAction::Jump => "jump",
            VrAction::Dash => "dash",
            VrAction::Crouch => "crouch",
            VrAction::Grenade => "grenade",
            VrAction::Shield => "shield",
            VrAction::Gadget => "gadget",
            VrAction::Menu => "menu",
        }
    }

    pub fn default_button(self) -> VrButton {
        match self {
            VrAction::Jump => VrButton::Primary,
            VrAction::Dash => VrButton::StickUp,
            VrAction::Crouch => VrButton::StickDown,
            VrAction::Grenade => VrButton::Secondary,
            VrAction::Shield => VrButton::FreeTrigger,
            VrAction::Gadget => VrButton::FreeGrip,
            VrAction::Menu => VrButton::Secondary,
        }
    }

    /// The buttons it can be put on, in the order the panel steps through them (the menu
    /// always on one: some controllers have no other way to it; it is held, so not on the
    /// free hand's trigger or grip, which other things use).
    pub fn choices(self) -> &'static [VrButton] {
        use VrButton::*;
        match self {
            VrAction::Menu => &[Primary, Secondary, LeftStick, RightStick],
            _ => &[
                Primary,
                Secondary,
                LeftStick,
                RightStick,
                StickUp,
                StickDown,
                FreeTrigger,
                FreeGrip,
                Unbound,
            ],
        }
    }

    /// Another action whose sharing a button with this one is meant (the menu is held on the
    /// grenade's button: with grenades, holding it takes one instead).
    fn shares_with(self, other: VrAction) -> bool {
        matches!(
            (self, other),
            (VrAction::Menu, VrAction::Grenade) | (VrAction::Grenade, VrAction::Menu)
        )
    }
}

/// A controller input a `VrAction` can be on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VrButton {
    /// A or X (either controller).
    Primary,
    /// B or Y.
    Secondary,
    /// Clicking the left stick (the left controller's trackpad), or the right one.
    LeftStick,
    RightStick,
    /// The right stick pushed forward, or pulled down.
    StickUp,
    StickDown,
    /// The trigger and grip of the hand without the blaster (gripping near the holster draws
    /// it instead).
    FreeTrigger,
    FreeGrip,
    Unbound,
}

impl VrButton {
    const ALL: [VrButton; 9] = [
        VrButton::Primary,
        VrButton::Secondary,
        VrButton::LeftStick,
        VrButton::RightStick,
        VrButton::StickUp,
        VrButton::StickDown,
        VrButton::FreeTrigger,
        VrButton::FreeGrip,
        VrButton::Unbound,
    ];

    /// As the panel and the hints name it (capitals: VR's font).
    pub fn name(self) -> &'static str {
        match self {
            VrButton::Primary => "A OR X",
            VrButton::Secondary => "B OR Y",
            VrButton::LeftStick => "LEFT STICK CLICK",
            VrButton::RightStick => "RIGHT STICK CLICK",
            VrButton::StickUp => "RIGHT STICK FORWARD",
            VrButton::StickDown => "RIGHT STICK DOWN",
            VrButton::FreeTrigger => "FREE HAND TRIGGER",
            VrButton::FreeGrip => "FREE HAND GRIP",
            VrButton::Unbound => "NOT BOUND",
        }
    }

    fn id(self) -> &'static str {
        match self {
            VrButton::Primary => "primary",
            VrButton::Secondary => "secondary",
            VrButton::LeftStick => "left_stick",
            VrButton::RightStick => "right_stick",
            VrButton::StickUp => "stick_up",
            VrButton::StickDown => "stick_down",
            VrButton::FreeTrigger => "free_trigger",
            VrButton::FreeGrip => "free_grip",
            VrButton::Unbound => "none",
        }
    }

    /// On the free hand: with dual pistols both hands hold one.
    pub fn free_hand(self) -> bool {
        matches!(self, VrButton::FreeTrigger | VrButton::FreeGrip)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    Key(KeyCode),
    Mouse(MouseButton),
    /// The mouse wheel: rolled up, it holds the action until rolled down again.
    Wheel,
}

/// The name the mouse wheel is shown and saved by.
const WHEEL: &str = "Mouse Wheel";

/// Keys that can be bound, with the names shown and saved.
const KEYS: &[(KeyCode, &str)] = {
    use KeyCode::*;
    &[
        (KeyA, "A"),
        (KeyB, "B"),
        (KeyC, "C"),
        (KeyD, "D"),
        (KeyE, "E"),
        (KeyF, "F"),
        (KeyG, "G"),
        (KeyH, "H"),
        (KeyI, "I"),
        (KeyJ, "J"),
        (KeyK, "K"),
        (KeyL, "L"),
        (KeyM, "M"),
        (KeyN, "N"),
        (KeyO, "O"),
        (KeyP, "P"),
        (KeyQ, "Q"),
        (KeyR, "R"),
        (KeyS, "S"),
        (KeyT, "T"),
        (KeyU, "U"),
        (KeyV, "V"),
        (KeyW, "W"),
        (KeyX, "X"),
        (KeyY, "Y"),
        (KeyZ, "Z"),
        (Digit0, "0"),
        (Digit1, "1"),
        (Digit2, "2"),
        (Digit3, "3"),
        (Digit4, "4"),
        (Digit5, "5"),
        (Digit6, "6"),
        (Digit7, "7"),
        (Digit8, "8"),
        (Digit9, "9"),
        (ArrowUp, "Up"),
        (ArrowDown, "Down"),
        (ArrowLeft, "Left"),
        (ArrowRight, "Right"),
        (ShiftLeft, "Left Shift"),
        (ShiftRight, "Right Shift"),
        (ControlLeft, "Left Ctrl"),
        (ControlRight, "Right Ctrl"),
        (AltLeft, "Left Alt"),
        (AltRight, "Right Alt"),
        (Space, "Space"),
        (Tab, "Tab"),
        (Enter, "Enter"),
        (CapsLock, "Caps Lock"),
        (Backquote, "`"),
        (Minus, "-"),
        (Equal, "="),
        (BracketLeft, "["),
        (BracketRight, "]"),
        (Backslash, "\\"),
        (Semicolon, ";"),
        (Quote, "'"),
        (Comma, ","),
        (Period, "."),
        (Slash, "/"),
        (Insert, "Insert"),
        (Home, "Home"),
        (End, "End"),
        (PageUp, "Page Up"),
        (PageDown, "Page Down"),
        (Numpad0, "Num 0"),
        (Numpad1, "Num 1"),
        (Numpad2, "Num 2"),
        (Numpad3, "Num 3"),
        (Numpad4, "Num 4"),
        (Numpad5, "Num 5"),
        (Numpad6, "Num 6"),
        (Numpad7, "Num 7"),
        (Numpad8, "Num 8"),
        (Numpad9, "Num 9"),
        (F1, "F1"),
        (F2, "F2"),
        (F3, "F3"),
        (F4, "F4"),
        (F5, "F5"),
        (F6, "F6"),
        (F7, "F7"),
        (F8, "F8"),
        (F9, "F9"),
        (F10, "F10"),
        (F11, "F11"),
        (F12, "F12"),
    ]
};

const BUTTONS: &[(MouseButton, &str)] = &[
    (MouseButton::Left, "Left Mouse"),
    (MouseButton::Right, "Right Mouse"),
    (MouseButton::Middle, "Middle Mouse"),
    (MouseButton::Back, "Mouse 4"),
    (MouseButton::Forward, "Mouse 5"),
];

impl Binding {
    pub fn name(self) -> String {
        match self {
            Binding::Key(k) => KEYS
                .iter()
                .find(|(c, _)| *c == k)
                .map_or_else(|| format!("{k:?}"), |(_, n)| n.to_string()),
            Binding::Mouse(b) => BUTTONS
                .iter()
                .find(|(c, _)| *c == b)
                .map_or_else(|| format!("{b:?}"), |(_, n)| n.to_string()),
            Binding::Wheel => WHEEL.to_string(),
        }
    }

    fn parse(s: &str) -> Option<Binding> {
        KEYS.iter()
            .find(|(_, n)| *n == s)
            .map(|(k, _)| Binding::Key(*k))
            .or_else(|| {
                BUTTONS
                    .iter()
                    .find(|(_, n)| *n == s)
                    .map(|(b, _)| Binding::Mouse(*b))
            })
            .or_else(|| (s == WHEEL).then_some(Binding::Wheel))
    }

    /// Whether this key can be bound (Esc and the like are the game's own).
    pub fn bindable_key(k: KeyCode) -> bool {
        KEYS.iter().any(|(c, _)| *c == k)
    }
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Settings {
    /// Mouse look speed, 1 = the default.
    pub sensitivity: f32,
    pub invert_y: bool,
    /// The flat screen's field of view (degrees, vertical: `FOV_RANGE`).
    pub fov: f32,
    /// The crouch key toggles crouching (instead of crouching while held).
    pub toggle_crouch: bool,
    /// The view bobs while walking and the blaster sways (flat screen).
    pub head_bob: bool,
    /// The HUD marks the targets to go for (`hud::goals`), flat screen and VR.
    pub objective_markers: bool,
    /// Glossy floors, walls and models with screen-space reflections (flat screen,
    /// src/reflections.rs); off for weaker GPUs.
    pub reflections: bool,
    /// VR: the holster on the left hip (`--left-handed` too); the blaster turned nose down in
    /// the hand (degrees, `GUN_TILT_RANGE`); the holster raised (m, `HOLSTER_RANGE`); the
    /// vibration's strength (0..=1); the stick moving where the left hand points (else where
    /// the head looks); and how much higher the eyes are than tracked (m: "calibrate height",
    /// for playing seated or a floor tracked wrong).
    pub left_handed: bool,
    pub gun_tilt: f32,
    pub holster_dy: f32,
    pub haptics: f32,
    pub steer_hand: bool,
    pub eye_offset: f32,
    /// VR comfort: the right stick turning in steps of this many degrees (0: smoothly, else
    /// one of `SNAP_TURNS`), and the view's edges darkening while the stick moves, dashing or
    /// flying.
    pub snap_turn: f32,
    pub vignette: bool,
    /// Volumes, 0..=1 (1 the default): the sound effects and the music.
    pub sound_volume: f32,
    pub music_volume: f32,
    /// Per action (in `Action::ALL` order): two bindings.
    pub bindings: Vec<(Action, [Option<Binding>; 2])>,
    /// VR: each `VrAction`'s button (in `VrAction::ALL` order).
    pub vr_buttons: [VrButton; 7],
    /// How often each kind of pickup (by its `PickupKind::id`) was taken so far, counted up to
    /// `EXPLAINED`: its announcement says what it does until then.
    pub taken: Vec<(String, u32)>,
    /// The first-match hints shown already (by id).
    pub hints: Vec<String>,
}

/// The VR snap turn's steps (degrees).
pub const SNAP_TURNS: [f32; 2] = [30.0, 45.0];

/// The field of view's range and default (degrees).
pub const FOV_RANGE: (f32, f32) = (60.0, 110.0);
pub const FOV_DEFAULT: f32 = 80.0;

/// VR: the gun tilt's range and default (degrees), the holster's raise range (m), and the
/// height offset's range (m).
pub const GUN_TILT_RANGE: (f32, f32) = (0.0, 80.0);
pub const GUN_TILT_DEFAULT: f32 = 40.0;
pub const HOLSTER_RANGE: (f32, f32) = (-0.15, 0.15);
pub const EYE_OFFSET_RANGE: (f32, f32) = (-0.5, 1.2);

/// Pickups of a kind whose announcements explain it.
pub const EXPLAINED: u32 = 3;

/// How loud the sound effects are at the sound setting's 100 % (of how they were made): they
/// drowned out the music.
pub const SOUND_LEVEL: f32 = 0.2;

/// What every sound effect's volume is multiplied by: the sound setting, at its full level
/// `SOUND_LEVEL` (the default settings' without any).
pub fn sound_gain(settings: Option<&Settings>) -> f32 {
    SOUND_LEVEL * settings.map_or(1.0, |s| s.sound_volume)
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sensitivity: 1.0,
            invert_y: false,
            fov: FOV_DEFAULT,
            toggle_crouch: false,
            head_bob: true,
            objective_markers: true,
            reflections: true,
            left_handed: false,
            gun_tilt: GUN_TILT_DEFAULT,
            holster_dy: 0.0,
            haptics: 1.0,
            steer_hand: false,
            eye_offset: 0.0,
            snap_turn: 0.0,
            vignette: false,
            sound_volume: 1.0,
            music_volume: 1.0,
            bindings: Action::ALL.iter().map(|&a| (a, a.defaults())).collect(),
            vr_buttons: VrAction::ALL.map(VrAction::default_button),
            taken: Vec::new(),
            hints: Vec::new(),
        }
    }
}

/// The input state, to ask about actions.
pub struct Inputs<'a> {
    pub keys: &'a ButtonInput<KeyCode>,
    pub mouse: &'a ButtonInput<MouseButton>,
    pub wheel: WheelState,
}

/// The mouse wheel as a button: `held` from rolling it up until rolling it down (kept by
/// whoever reads the input), `up` and `down` this frame's rolls.
#[derive(Clone, Copy, Default, Debug)]
pub struct WheelState {
    pub held: bool,
    pub up: bool,
    pub down: bool,
}

impl WheelState {
    /// This frame's wheel, rolled `dy` (up positive), latching `held`.
    pub fn roll(held: &mut bool, dy: f32) -> WheelState {
        let (up, down) = (dy > 0.0, dy < 0.0);
        if up {
            *held = true;
        } else if down {
            *held = false;
        }
        WheelState {
            held: *held,
            up,
            down,
        }
    }
}

impl Settings {
    /// Counts a pickup of `kind` taken; whether it should still be explained (saving the count
    /// meanwhile).
    pub fn take(&mut self, kind: lt_sim::PickupKind) -> bool {
        let id = kind.id();
        let n = match self.taken.iter_mut().find(|(k, _)| k == id) {
            Some((_, n)) => n,
            None => {
                self.taken.push((id.to_string(), 0));
                &mut self.taken.last_mut().unwrap().1
            }
        };
        if *n >= EXPLAINED {
            return false;
        }
        *n += 1;
        self.save();
        true
    }

    /// Whether hint `id` was shown already.
    pub fn hinted(&self, id: &str) -> bool {
        self.hints.iter().any(|h| h == id)
    }

    /// Hint `id` shown (saved).
    pub fn hint(&mut self, id: &str) {
        if !self.hinted(id) {
            self.hints.push(id.to_string());
            self.save();
        }
    }

    pub fn get(&self, a: Action) -> [Option<Binding>; 2] {
        self.bindings
            .iter()
            .find(|(b, _)| *b == a)
            .map_or([None, None], |(_, b)| *b)
    }

    pub fn set(&mut self, a: Action, slot: usize, b: Option<Binding>) {
        // A key does one thing: taken away from wherever else it was.
        if let Some(b) = b {
            for (_, bs) in &mut self.bindings {
                for s in bs.iter_mut() {
                    if *s == Some(b) {
                        *s = None;
                    }
                }
            }
        }
        if let Some((_, bs)) = self.bindings.iter_mut().find(|(x, _)| *x == a) {
            bs[slot] = b;
        }
    }

    /// The button `a` is on in VR.
    pub fn vr(&self, a: VrAction) -> VrButton {
        self.vr_buttons[VrAction::ALL.iter().position(|&x| x == a).unwrap_or(0)]
    }

    pub fn set_vr(&mut self, a: VrAction, b: VrButton) {
        if let Some(k) = VrAction::ALL.iter().position(|&x| x == a) {
            self.vr_buttons[k] = b;
        }
    }

    /// `a`'s button one step on (or back) among its choices.
    pub fn step_vr(&mut self, a: VrAction, forward: bool) {
        let c = a.choices();
        let k = c.iter().position(|&b| b == self.vr(a)).unwrap_or(0);
        let n = c.len();
        let k = if forward {
            (k + 1) % n
        } else {
            (k + n - 1) % n
        };
        self.set_vr(a, c[k]);
    }

    /// Another VR action on `a`'s button that isn't meant to share it.
    pub fn vr_clash(&self, a: VrAction) -> Option<VrAction> {
        let b = self.vr(a);
        (b != VrButton::Unbound)
            .then(|| {
                VrAction::ALL
                    .into_iter()
                    .find(|&o| o != a && !a.shares_with(o) && self.vr(o) == b)
            })
            .flatten()
    }

    /// The button `a` is on with dual pistols, where both hands hold one: the shield on the
    /// dash's button, the gadget on the grenade's (no grenades then), if they were on the free
    /// hand (the dash moves to a quick second jump then).
    pub fn vr_dual(&self, a: VrAction) -> VrButton {
        let own = self.vr(a);
        let instead = match a {
            VrAction::Shield => VrAction::Dash,
            VrAction::Gadget => VrAction::Grenade,
            _ => return own,
        };
        if !own.free_hand() {
            return own;
        }
        match self.vr(instead) {
            b if b.free_hand() || b == VrButton::Unbound => instead.default_button(),
            b => b,
        }
    }

    fn any(&self, a: Action, f: impl Fn(Binding) -> bool) -> bool {
        self.get(a).iter().flatten().any(|&b| f(b))
    }

    pub fn pressed(&self, a: Action, i: &Inputs) -> bool {
        self.any(a, |b| match b {
            Binding::Key(k) => i.keys.pressed(k),
            Binding::Mouse(m) => i.mouse.pressed(m),
            Binding::Wheel => i.wheel.held,
        })
    }

    pub fn just_pressed(&self, a: Action, i: &Inputs) -> bool {
        self.any(a, |b| match b {
            Binding::Key(k) => i.keys.just_pressed(k),
            Binding::Mouse(m) => i.mouse.just_pressed(m),
            Binding::Wheel => i.wheel.up,
        })
    }

    pub fn just_released(&self, a: Action, i: &Inputs) -> bool {
        self.any(a, |b| match b {
            Binding::Key(k) => i.keys.just_released(k),
            Binding::Mouse(m) => i.mouse.just_released(m),
            Binding::Wheel => i.wheel.down,
        })
    }

    /// The names of an action's bindings ("W / Up"), or "-" if it has none.
    pub fn describe(&self, a: Action) -> String {
        let names: Vec<String> = self.get(a).iter().flatten().map(|b| b.name()).collect();
        if names.is_empty() {
            "-".into()
        } else {
            names.join(" / ")
        }
    }

    fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("lasertag").join("settings.ron"))
    }

    /// A volume one step (10 %) up or down.
    pub fn step_volume(v: f32, up: bool) -> f32 {
        ((v * 10.0).round() + if up { 1.0 } else { -1.0 }).clamp(0.0, 10.0) / 10.0
    }

    pub fn load() -> Settings {
        let mut s = Settings::default();
        let Some(text) = Self::path().and_then(|p| std::fs::read_to_string(p).ok()) else {
            return s;
        };
        match ron::from_str::<Saved>(&text) {
            Ok(saved) => {
                s.sensitivity = saved.sensitivity.clamp(0.1, 5.0);
                s.invert_y = saved.invert_y;
                s.fov = saved.fov.clamp(FOV_RANGE.0, FOV_RANGE.1);
                s.toggle_crouch = saved.toggle_crouch;
                s.head_bob = saved.head_bob;
                s.objective_markers = saved.objective_markers;
                s.reflections = saved.reflections;
                s.left_handed = saved.left_handed;
                s.gun_tilt = saved.gun_tilt.clamp(GUN_TILT_RANGE.0, GUN_TILT_RANGE.1);
                s.holster_dy = saved.holster_dy.clamp(HOLSTER_RANGE.0, HOLSTER_RANGE.1);
                s.haptics = saved.haptics.clamp(0.0, 1.0);
                s.steer_hand = saved.steer_hand;
                s.snap_turn = if SNAP_TURNS.contains(&saved.snap_turn) {
                    saved.snap_turn
                } else {
                    0.0
                };
                s.vignette = saved.vignette;
                s.eye_offset = saved
                    .eye_offset
                    .clamp(EYE_OFFSET_RANGE.0, EYE_OFFSET_RANGE.1);
                s.sound_volume = saved.sound_volume.clamp(0.0, 1.0);
                s.music_volume = saved.music_volume.clamp(0.0, 1.0);
                s.taken = saved.taken;
                s.hints = saved.hints;
                for (id, name) in saved.vr_buttons {
                    if let Some(&a) = VrAction::ALL.iter().find(|a| a.id() == id)
                        && let Some(&b) = VrButton::ALL.iter().find(|b| b.id() == name)
                        && a.choices().contains(&b)
                    {
                        s.set_vr(a, b);
                    }
                }
                for (id, names) in saved.bindings {
                    if let Some(a) = Action::ALL.iter().find(|a| a.id() == id)
                        && let Some((_, bs)) = s.bindings.iter_mut().find(|(x, _)| x == a)
                    {
                        for (slot, n) in names.iter().enumerate().take(2) {
                            bs[slot] = n.as_deref().and_then(Binding::parse);
                        }
                    }
                }
            }
            Err(e) => warn!("settings: {e}"),
        }
        s
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let saved = Saved {
            sensitivity: self.sensitivity,
            invert_y: self.invert_y,
            fov: self.fov,
            toggle_crouch: self.toggle_crouch,
            head_bob: self.head_bob,
            objective_markers: self.objective_markers,
            reflections: self.reflections,
            left_handed: self.left_handed,
            gun_tilt: self.gun_tilt,
            holster_dy: self.holster_dy,
            haptics: self.haptics,
            steer_hand: self.steer_hand,
            snap_turn: self.snap_turn,
            vignette: self.vignette,
            eye_offset: self.eye_offset,
            sound_volume: self.sound_volume,
            music_volume: self.music_volume,
            bindings: self
                .bindings
                .iter()
                .map(|(a, bs)| {
                    (
                        a.id().to_string(),
                        bs.iter().map(|b| b.map(|b| b.name())).collect(),
                    )
                })
                .collect(),
            vr_buttons: VrAction::ALL
                .iter()
                .map(|&a| (a.id().to_string(), self.vr(a).id().to_string()))
                .collect(),
            taken: self.taken.clone(),
            hints: self.hints.clone(),
        };
        let text = match ron::ser::to_string_pretty(&saved, ron::ser::PrettyConfig::default()) {
            Ok(t) => t,
            Err(e) => return warn!("settings: {e}"),
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, text) {
            warn!("can't save {}: {e}", path.display());
        }
    }
}

/// The file's contents (names rather than key codes, so it reads well and survives updates).
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    sensitivity: f32,
    invert_y: bool,
    fov: f32,
    toggle_crouch: bool,
    head_bob: bool,
    objective_markers: bool,
    reflections: bool,
    left_handed: bool,
    gun_tilt: f32,
    holster_dy: f32,
    haptics: f32,
    steer_hand: bool,
    eye_offset: f32,
    snap_turn: f32,
    vignette: bool,
    sound_volume: f32,
    music_volume: f32,
    bindings: Vec<(String, Vec<Option<String>>)>,
    vr_buttons: Vec<(String, String)>,
    taken: Vec<(String, u32)>,
    hints: Vec<String>,
}

impl Default for Saved {
    fn default() -> Self {
        Saved {
            sensitivity: 1.0,
            invert_y: false,
            fov: FOV_DEFAULT,
            toggle_crouch: false,
            head_bob: true,
            objective_markers: true,
            reflections: true,
            left_handed: false,
            gun_tilt: GUN_TILT_DEFAULT,
            holster_dy: 0.0,
            haptics: 1.0,
            steer_hand: false,
            eye_offset: 0.0,
            snap_turn: 0.0,
            vignette: false,
            sound_volume: 1.0,
            music_volume: 1.0,
            bindings: Vec::new(),
            vr_buttons: Vec::new(),
            taken: Vec::new(),
            hints: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_round_trip_by_name() {
        for (k, n) in KEYS {
            assert_eq!(Binding::parse(n), Some(Binding::Key(*k)));
        }
        for (b, n) in BUTTONS {
            assert_eq!(Binding::parse(n), Some(Binding::Mouse(*b)));
        }
        assert_eq!(Binding::parse(WHEEL), Some(Binding::Wheel));
    }

    #[test]
    fn a_key_does_one_thing() {
        let mut s = Settings::default();
        s.set(Action::Fire, 1, Some(Binding::Key(KeyCode::KeyW)));
        assert_eq!(s.get(Action::Forward)[0], None);
        assert_eq!(s.get(Action::Fire)[1], Some(Binding::Key(KeyCode::KeyW)));
    }

    #[test]
    fn vr_buttons_step_and_clash() {
        let mut s = Settings::default();
        // The menu shares the grenade's button by default: no clash.
        assert!(VrAction::ALL.iter().all(|&a| s.vr_clash(a).is_none()));
        s.step_vr(VrAction::Jump, true);
        assert_eq!(s.vr(VrAction::Jump), VrButton::Secondary);
        assert_eq!(s.vr_clash(VrAction::Jump), Some(VrAction::Grenade));
        s.step_vr(VrAction::Jump, false);
        assert_eq!(s.vr(VrAction::Jump), VrButton::Primary);
        // Stepping back from the first choice wraps round; the menu is never unbound.
        s.step_vr(VrAction::Menu, false);
        assert_eq!(s.vr(VrAction::Menu), VrButton::Primary);
        s.step_vr(VrAction::Menu, false);
        assert_eq!(s.vr(VrAction::Menu), VrButton::RightStick);
        assert!(!VrAction::Menu.choices().contains(&VrButton::Unbound));
    }

    #[test]
    fn dual_pistols_move_the_free_hand_actions() {
        let mut s = Settings::default();
        assert_eq!(s.vr_dual(VrAction::Shield), VrButton::StickUp);
        assert_eq!(s.vr_dual(VrAction::Gadget), VrButton::Secondary);
        s.set_vr(VrAction::Dash, VrButton::RightStick);
        assert_eq!(s.vr_dual(VrAction::Shield), VrButton::RightStick);
        s.set_vr(VrAction::Shield, VrButton::Primary);
        assert_eq!(s.vr_dual(VrAction::Shield), VrButton::Primary);
    }
}
