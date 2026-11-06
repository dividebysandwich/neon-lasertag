//! Text in the neon font (Orbitron, as the flat screen's menus: src/neonui.rs) as flat meshes,
//! for the VR panels (src/vrui.rs): each glyph's outline filled with triangles (lyon), facing
//! +Z, drawn with an `AccentMaterial`, so it draws in every renderer, the GL headset renderer
//! (no text or textures there) included. Glyphs are made once per weight and kept.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bevy::prelude::*;
use lyon_tessellation::path::Path;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex};
use lyon_tessellation::{VertexBuffers, geom::point};

use crate::assets::MeshData;

const FONT: &[u8] = include_bytes!("../assets/fonts/Orbitron.ttf");

/// A glyph: its triangles (in cap heights, the baseline at y = 0) and its advance.
#[derive(Clone, Default)]
struct Glyph {
    pos: Vec<Vec2>,
    idx: Vec<u32>,
    advance: f32,
}

struct Outline(lyon_tessellation::path::path::Builder, bool, f32);

impl Outline {
    fn p(&self, x: f32, y: f32) -> lyon_tessellation::math::Point {
        point(x * self.2, y * self.2)
    }
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.1 {
            self.0.end(true);
        }
        let p = self.p(x, y);
        self.0.begin(p);
        self.1 = true;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.p(x, y);
        self.0.line_to(p);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (c, p) = (self.p(x1, y1), self.p(x, y));
        self.0.quadratic_bezier_to(c, p);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b, p) = (self.p(x1, y1), self.p(x2, y2), self.p(x, y));
        self.0.cubic_bezier_to(a, b, p);
    }
    fn close(&mut self) {
        if self.1 {
            self.0.end(true);
            self.1 = false;
        }
    }
}

fn make_glyph(face: &ttf_parser::Face, c: char, scale: f32) -> Glyph {
    let Some(id) = face.glyph_index(c) else {
        return Glyph {
            advance: 0.6,
            ..default()
        };
    };
    let advance = face.glyph_hor_advance(id).unwrap_or(0) as f32 * scale;
    let mut o = Outline(Path::builder(), false, scale);
    if face.outline_glyph(id, &mut o).is_none() {
        return Glyph {
            advance,
            ..default()
        };
    }
    if o.1 {
        o.0.end(true);
    }
    let path = o.0.build();
    let mut buf: VertexBuffers<Vec2, u32> = VertexBuffers::new();
    // Variable fonts' outlines overlap: non-zero fills them whole.
    let opts = FillOptions::tolerance(0.004).with_fill_rule(FillRule::NonZero);
    let _ = FillTessellator::new().tessellate_path(
        &path,
        &opts,
        &mut BuffersBuilder::new(&mut buf, |v: FillVertex| {
            Vec2::new(v.position().x, v.position().y)
        }),
    );
    // Every triangle counter-clockwise, facing +Z (the other side is culled).
    let mut idx = buf.indices;
    for t in idx.chunks_mut(3) {
        let [a, b, c] = [0, 1, 2].map(|k| buf.vertices[t[k] as usize]);
        if (b - a).perp_dot(c - a) < 0.0 {
            t.swap(1, 2);
        }
    }
    Glyph {
        pos: buf.vertices,
        idx,
        advance,
    }
}

/// The glyphs made so far, by weight and character.
fn glyph(c: char, weight: u16) -> Glyph {
    static CACHE: OnceLock<Mutex<HashMap<(u16, char), Glyph>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(g) = cache.lock().ok().and_then(|m| m.get(&(weight, c)).cloned()) {
        return g;
    }
    let Ok(mut face) = ttf_parser::Face::parse(FONT, 0) else {
        return Glyph::default();
    };
    let _ = face.set_variation(ttf_parser::Tag::from_bytes(b"wght"), weight as f32);
    let cap = face
        .capital_height()
        .map_or(face.units_per_em() as f32 * 0.7, |h| h as f32);
    let g = make_glyph(&face, c, 1.0 / cap);
    if let Ok(mut m) = cache.lock() {
        m.insert((weight, c), g.clone());
    }
    g
}

/// Extra space between letters, in cap heights (the flat screen's look is airy).
const TRACKING: f32 = 0.08;

/// The width of `text` with capitals `h` high.
pub fn width(text: &str, h: f32, weight: u16) -> f32 {
    let n = text.chars().count();
    let w: f32 = text.chars().map(|c| glyph(c, weight).advance).sum();
    (w + TRACKING * n.saturating_sub(1) as f32) * h
}

/// `text` as one mesh: capitals `h` high, its left end at x = 0, the capitals' middle at y = 0.
pub fn mesh(text: &str, h: f32, weight: u16) -> MeshData {
    let mut m = MeshData::default();
    let mut x = 0.0;
    for c in text.chars() {
        let g = glyph(c, weight);
        let base = m.pos.len() as u32;
        for p in &g.pos {
            m.pos.push([(x + p.x) * h, (p.y - 0.5) * h, 0.0]);
            m.nrm.push([0.0, 0.0, 1.0]);
            m.uv.push([0.0, 0.0]);
        }
        m.idx.extend(g.idx.iter().map(|i| base + i));
        x += g.advance + TRACKING;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_are_filled_and_spaced() {
        let m = mesh("NEON 0", 1.0, 700);
        assert!(m.idx.len() > 30 && m.idx.len().is_multiple_of(3));
        assert_eq!(m.pos.len(), m.uv.len());
        // Capitals stand from -0.5 to 0.5.
        let top = m.pos.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert!((top - 0.5).abs() < 0.08, "{top}");
        assert!(width("WW", 1.0, 700) > width("I", 1.0, 700));
        // Facing +Z.
        for t in m.idx.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| Vec2::from_slice(&m.pos[t[k] as usize][..2]));
            assert!((b - a).perp_dot(c - a) >= 0.0);
        }
        assert_eq!(width("", 1.0, 400), 0.0);
    }
}
