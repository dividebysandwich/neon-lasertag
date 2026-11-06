//! The player's settings: the flat screen's key bindings, mouse sensitivity and inverted mouse
//! look, and the volumes of the sound effects and the music (in VR too).
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
    Fire,
    Grenade,
    Shield,
}

impl Action {
    pub const ALL: [Action; 9] = [
        Action::Forward,
        Action::Back,
        Action::Left,
        Action::Right,
        Action::Walk,
        Action::Crouch,
        Action::Fire,
        Action::Grenade,
        Action::Shield,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Action::Forward => "Move forward",
            Action::Back => "Move back",
            Action::Left => "Strafe left",
            Action::Right => "Strafe right",
            Action::Walk => "Walk slowly",
            Action::Crouch => "Crouch",
            Action::Fire => "Fire",
            Action::Grenade => "Grenade (hold, let go)",
            Action::Shield => "Shield (switch on)",
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
            Action::Fire => "fire",
            Action::Grenade => "grenade",
            Action::Shield => "shield",
        }
    }

    fn defaults(self) -> [Option<Binding>; 2] {
        use Binding::{Key, Mouse};
        match self {
            Action::Forward => [Some(Key(KeyCode::KeyW)), Some(Key(KeyCode::ArrowUp))],
            Action::Back => [Some(Key(KeyCode::KeyS)), Some(Key(KeyCode::ArrowDown))],
            Action::Left => [Some(Key(KeyCode::KeyA)), Some(Key(KeyCode::ArrowLeft))],
            Action::Right => [Some(Key(KeyCode::KeyD)), Some(Key(KeyCode::ArrowRight))],
            Action::Walk => [Some(Key(KeyCode::ShiftLeft)), None],
            Action::Crouch => [Some(Key(KeyCode::ControlLeft)), Some(Key(KeyCode::KeyC))],
            Action::Fire => [Some(Mouse(MouseButton::Left)), None],
            Action::Grenade => [Some(Key(KeyCode::KeyG)), Some(Mouse(MouseButton::Right))],
            Action::Shield => [Some(Key(KeyCode::KeyQ)), None],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    Key(KeyCode),
    Mouse(MouseButton),
}

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
    /// Volumes, 0..=1 (1 the default): the sound effects and the music.
    pub sound_volume: f32,
    pub music_volume: f32,
    /// Per action (in `Action::ALL` order): two bindings.
    pub bindings: Vec<(Action, [Option<Binding>; 2])>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sensitivity: 1.0,
            invert_y: false,
            sound_volume: 1.0,
            music_volume: 1.0,
            bindings: Action::ALL.iter().map(|&a| (a, a.defaults())).collect(),
        }
    }
}

/// The input state, to ask about actions.
pub struct Inputs<'a> {
    pub keys: &'a ButtonInput<KeyCode>,
    pub mouse: &'a ButtonInput<MouseButton>,
}

impl Settings {
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

    fn any(&self, a: Action, f: impl Fn(Binding) -> bool) -> bool {
        self.get(a).iter().flatten().any(|&b| f(b))
    }

    pub fn pressed(&self, a: Action, i: &Inputs) -> bool {
        self.any(a, |b| match b {
            Binding::Key(k) => i.keys.pressed(k),
            Binding::Mouse(m) => i.mouse.pressed(m),
        })
    }

    pub fn just_pressed(&self, a: Action, i: &Inputs) -> bool {
        self.any(a, |b| match b {
            Binding::Key(k) => i.keys.just_pressed(k),
            Binding::Mouse(m) => i.mouse.just_pressed(m),
        })
    }

    pub fn just_released(&self, a: Action, i: &Inputs) -> bool {
        self.any(a, |b| match b {
            Binding::Key(k) => i.keys.just_released(k),
            Binding::Mouse(m) => i.mouse.just_released(m),
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
                s.sound_volume = saved.sound_volume.clamp(0.0, 1.0);
                s.music_volume = saved.music_volume.clamp(0.0, 1.0);
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
    sound_volume: f32,
    music_volume: f32,
    bindings: Vec<(String, Vec<Option<String>>)>,
}

impl Default for Saved {
    fn default() -> Self {
        Saved {
            sensitivity: 1.0,
            invert_y: false,
            sound_volume: 1.0,
            music_volume: 1.0,
            bindings: Vec::new(),
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
    }

    #[test]
    fn a_key_does_one_thing() {
        let mut s = Settings::default();
        s.set(Action::Fire, 1, Some(Binding::Key(KeyCode::KeyW)));
        assert_eq!(s.get(Action::Forward)[0], None);
        assert_eq!(s.get(Action::Fire)[1], Some(Binding::Key(KeyCode::KeyW)));
    }
}
