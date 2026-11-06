//! Text from glowing strokes, as on a fourteen-segment display: upper case letters, digits and
//! a few signs, for the VR panels (src/vrui.rs). Each text is one mesh of flat strokes facing
//! +Z (drawn with an `AccentMaterial`) and optionally one of soft halos around them (with a
//! `GlowMaterial`), so it draws in every renderer, the GL headset renderer included.

use bevy::prelude::*;

use crate::assets::MeshData;
use crate::materials::{AccentMaterial, GlowMaterial};

/// Width of a character, in heights.
const WIDTH: f32 = 0.6;
/// Space between characters, in heights.
const GAP: f32 = 0.3;
/// Stroke width, in heights.
const STROKE: f32 = 0.11;

/// The strokes of `c`: a b c d e f (the outline, clockwise from the top), g G (the middle bar's
/// left and right halves), h i j (from the top left corner, the top middle and the top right
/// corner to the centre), k l m (from the centre to the bottom left, middle and right), p q r s
/// t (a rounded right side: the top bar short of the corner, down to the right edge, the right
/// edge, back in, the bottom bar), and the dots `.` (bottom) and `:` (two).
fn strokes(c: char) -> &'static str {
    match c.to_ascii_uppercase() {
        '0' => "abcdefjk",
        '1' => "bcj",
        '2' => "abgGed",
        '3' => "abcdG",
        '4' => "bcfgG",
        '5' => "acdfgG",
        '6' => "acdefgG",
        '7' => "abc",
        '8' => "abcdefgG",
        '9' => "abcdfgG",
        'A' => "abcefgG",
        'B' => "abcdiGl",
        'C' => "adef",
        'D' => "efpqrst",
        'E' => "adefg",
        'F' => "aefg",
        'G' => "acdefG",
        'H' => "bcefgG",
        'I' => "adil",
        'J' => "bcde",
        'K' => "efgjm",
        'L' => "def",
        'M' => "bcefhj",
        'N' => "bcefhm",
        'O' => "abcdef",
        'P' => "abefgG",
        'Q' => "abcdefm",
        'R' => "abefgGm",
        'S' => "acdfgG",
        'T' => "ail",
        'U' => "bcdef",
        'V' => "efjk",
        'W' => "bcefkm",
        'X' => "hjkm",
        'Y' => "hjl",
        'Z' => "adjk",
        '-' => "gG",
        '+' => "gGil",
        '/' => "jk",
        '%' => "cfjk",
        '(' => "jm",
        ')' => "hk",
        '_' => "d",
        '\'' => "i",
        '!' => "i.",
        '?' => "abGl",
        '.' => ".",
        ':' => ":",
        '>' => "hk",
        '<' => "jm",
        '=' => "gGd",
        _ => "",
    }
}

/// The strokes and halos of a text, its left end at x = 0, its middle at y = 0.
#[derive(Default)]
pub struct SegText {
    pub strokes: MeshData,
    pub halo: MeshData,
    pub width: f32,
}

pub fn width(text: &str, h: f32) -> f32 {
    let n = text.chars().count() as f32;
    (n * (WIDTH + GAP) - GAP).max(0.0) * h
}

/// A stroke from `a` to `b`, `half` wide each side; halos: three times as wide, fading out
/// from the middle (uv.y 0 in the middle, 1 at the edges).
fn stroke(out: &mut SegText, a: Vec2, b: Vec2, half: f32) {
    let d = (b - a).normalize_or(Vec2::X);
    let n = Vec2::new(-d.y, d.x);
    let len = (b - a).length();
    let base = out.strokes.pos.len() as u32;
    for (p, u) in [(a - n * half, 0.0), (b - n * half, len), (b + n * half, len), (a + n * half, 0.0)] {
        out.strokes.pos.push([p.x, p.y, 0.0]);
        out.strokes.nrm.push([0.0, 0.0, 1.0]);
        out.strokes.uv.push([u, 0.5]);
    }
    out.strokes.idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    // The halo: a band with a bright middle line, a little longer than the stroke.
    let w = half * 3.5;
    let (a, b) = (a - d * half, b + d * half);
    let base = out.halo.pos.len() as u32;
    for (p, v) in [(a - n * w, 1.0), (b - n * w, 1.0), (a, 0.0), (b, 0.0), (a + n * w, 1.0), (b + n * w, 1.0)] {
        out.halo.pos.push([p.x, p.y, -0.002]);
        out.halo.nrm.push([0.0, 0.0, 1.0]);
        out.halo.uv.push([0.0, v]);
    }
    out.halo.idx.extend([base, base + 1, base + 3, base, base + 3, base + 2, base + 2, base + 3, base + 5, base + 2, base + 5, base + 4]);
}

/// A square dot at `c`.
fn dot(out: &mut SegText, c: Vec2, half: f32) {
    stroke(out, c - Vec2::X * half, c + Vec2::X * half, half);
}

pub fn layout(text: &str, h: f32) -> SegText {
    let mut out = SegText { width: width(text, h), ..default() };
    let w = WIDTH * h;
    let t = STROKE * h;
    let half = t / 2.0;
    for (i, c) in text.chars().enumerate() {
        let x0 = i as f32 * (WIDTH + GAP) * h;
        let p = |x: f32, y: f32| Vec2::new(x0 + x, y - h / 2.0);
        let (tl, tr, ml, mr, bl, br) = (p(0.0, h), p(w, h), p(0.0, h / 2.0), p(w, h / 2.0), p(0.0, 0.0), p(w, 0.0));
        let (tc, mc, bc) = (p(w / 2.0, h), p(w / 2.0, h / 2.0), p(w / 2.0, 0.0));
        for s in strokes(c).chars() {
            let (a, b) = match s {
                'a' => (tl, tr),
                'b' => (tr, mr),
                'c' => (mr, br),
                'd' => (bl, br),
                'e' => (ml, bl),
                'f' => (tl, ml),
                'g' => (ml, mc),
                'G' => (mc, mr),
                'h' => (tl, mc),
                'i' => (tc, mc),
                'j' => (tr, mc),
                'k' => (mc, bl),
                'l' => (mc, bc),
                'm' => (mc, br),
                'p' => (tl, p(w * 0.72, h)),
                'q' => (p(w * 0.72, h), p(w, h * 0.78)),
                'r' => (p(w, h * 0.78), p(w, h * 0.22)),
                's' => (p(w, h * 0.22), p(w * 0.72, 0.0)),
                't' => (bl, p(w * 0.72, 0.0)),
                '.' => {
                    dot(&mut out, bc + Vec2::Y * half, half * 1.2);
                    continue;
                }
                ':' => {
                    dot(&mut out, p(w / 2.0, h * 0.25), half * 1.2);
                    dot(&mut out, p(w / 2.0, h * 0.75), half * 1.2);
                    continue;
                }
                _ => continue,
            };
            // Diagonals to the centre end short of the corners, other strokes overlap at them.
            let diagonal = (b - a).x.abs() > 1e-4 && (b - a).y.abs() > 1e-4 && (a == mc || b == mc);
            let inset = if diagonal { t * 0.9 } else { 0.0 };
            let d = (b - a).normalize_or(Vec2::X);
            stroke(&mut out, a + d * inset, b - d * inset, half);
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Spawns `text` (height `h`) as a child of `parent` at `at` (its middle at the left end, the
/// middle or the right end, by `align`), in `accent` with `glow` halos; returns its entity.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    parent: Entity,
    text: &str,
    h: f32,
    at: Vec3,
    align: Align,
    accent: &Handle<AccentMaterial>,
    glow: Option<&Handle<GlowMaterial>>,
) -> Entity {
    let t = layout(text, h);
    let x = match align {
        Align::Left => 0.0,
        Align::Center => -t.width / 2.0,
        Align::Right => -t.width,
    };
    let root = commands.spawn((Transform::from_translation(at + Vec3::X * x), Visibility::default(), ChildOf(parent))).id();
    if !t.strokes.is_empty() {
        commands.spawn((Mesh3d(meshes.add(t.strokes.to_mesh())), MeshMaterial3d(accent.clone()), Transform::IDENTITY, ChildOf(root)));
        if let Some(g) = glow {
            commands.spawn((Mesh3d(meshes.add(t.halo.to_mesh())), MeshMaterial3d(g.clone()), Transform::IDENTITY, ChildOf(root)));
        }
    }
    root
}
