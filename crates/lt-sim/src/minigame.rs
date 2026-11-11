//! The click puzzles on the consoles (`crate::console`): one of a few kinds, all made from a
//! seed (the kind too: `MinigameKind::of`), so the sim only sends the seed and a byte per
//! element (a piece, a tile, a light). A click pokes one element; the puzzle is solved when
//! its elements are all where they belong.
//!
//! Every kind lies on a grid of cells on the console's face (`extent` along both axes), and
//! `element_at` tells which element a cell belongs to (for clicking).

use serde::{Deserialize, Serialize};

pub mod circuit;
pub mod lights;
use circuit::Circuit;
use lights::Lights;

use crate::puzzle::{Puzzle, Slide};
use crate::types::PuzzleSlide;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MinigameKind {
    /// The turret boxes' sliding puzzle (`crate::puzzle`).
    Slide,
    /// Wire tiles to turn until the current gets through (`circuit`).
    Circuit,
    /// Lights to switch off, each click switching its neighbours too (`lights`).
    Lights,
}

impl MinigameKind {
    pub const ALL: [MinigameKind; 3] = [
        MinigameKind::Slide,
        MinigameKind::Circuit,
        MinigameKind::Lights,
    ];

    /// The kind a seed makes.
    pub fn of(seed: u32) -> MinigameKind {
        let mut x = seed ^ 0x6A09_E667;
        x ^= x >> 16;
        x = x.wrapping_mul(0x7FEB_352D);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846C_A68B);
        x ^= x >> 16;
        Self::ALL[x as usize % Self::ALL.len()]
    }

    /// A short name (testing: `LT_DEV=console-game=circuit`).
    pub fn id(self) -> &'static str {
        match self {
            MinigameKind::Slide => "slide",
            MinigameKind::Circuit => "circuit",
            MinigameKind::Lights => "lights",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            MinigameKind::Slide => "sliding puzzle",
            MinigameKind::Circuit => "circuit",
            MinigameKind::Lights => "lights out",
        }
    }
}

/// The cells' size (m) of the puzzles on a plain grid (bigger than the sliding puzzle's, which
/// needs its strips beside the square).
pub const GRID_CELL: f32 = 0.17;

#[derive(Clone, Debug, PartialEq)]
pub enum Minigame {
    Slide(Puzzle),
    Circuit(Circuit),
    Lights(Lights),
}

impl Minigame {
    /// The puzzle for `seed` (the same everywhere).
    pub fn generate(seed: u32) -> Minigame {
        match MinigameKind::of(seed) {
            MinigameKind::Slide => Minigame::Slide(Puzzle::generate(seed)),
            MinigameKind::Circuit => Minigame::Circuit(Circuit::generate(seed)),
            MinigameKind::Lights => Minigame::Lights(Lights::generate(seed)),
        }
    }

    pub fn kind(&self) -> MinigameKind {
        match self {
            Minigame::Slide(_) => MinigameKind::Slide,
            Minigame::Circuit(_) => MinigameKind::Circuit,
            Minigame::Lights(_) => MinigameKind::Lights,
        }
    }

    /// How it starts.
    pub fn start(&self) -> Vec<u8> {
        match self {
            Minigame::Slide(p) => p.start(),
            Minigame::Circuit(c) => c.start(),
            Minigame::Lights(l) => l.start(),
        }
    }

    /// Element `i` clicked.
    pub fn poke(&self, at: &mut [u8], i: usize) -> PuzzleSlide {
        match self {
            Minigame::Slide(p) => match p.poke(at, i) {
                Slide::In => PuzzleSlide::In,
                Slide::Stuck => PuzzleSlide::Stuck,
                Slide::Out => PuzzleSlide::Out,
                Slide::Blocked => PuzzleSlide::Blocked,
            },
            Minigame::Circuit(c) => {
                c.poke(at, i);
                PuzzleSlide::In
            }
            Minigame::Lights(l) => {
                l.poke(at, i);
                // (A higher sound switching on.)
                if at.get(i) == Some(&1) {
                    PuzzleSlide::Out
                } else {
                    PuzzleSlide::In
                }
            }
        }
    }

    pub fn solved(&self, at: &[u8]) -> bool {
        match self {
            Minigame::Slide(p) => p.solved(at),
            Minigame::Circuit(c) => c.solved(at),
            Minigame::Lights(l) => l.solved(at),
        }
    }

    /// An element to poke next towards solving it (for the bots).
    pub fn hint(&self, at: &[u8]) -> Option<usize> {
        match self {
            Minigame::Slide(p) => p.hint(at),
            Minigame::Circuit(c) => c.hint(at),
            Minigame::Lights(l) => l.hint(at),
        }
    }

    /// How many elements it has (valid clicks are below this).
    pub fn elements(&self) -> usize {
        match self {
            Minigame::Slide(p) => p.pieces.len(),
            Minigame::Circuit(_) => circuit::N * circuit::N,
            Minigame::Lights(_) => lights::N * lights::N,
        }
    }

    /// The element on cell (column, row) with the elements where they are (`at`).
    pub fn element_at(&self, at: &[u8], c: i8, r: i8) -> Option<usize> {
        match self {
            Minigame::Slide(p) => p.piece_at(at, c, r),
            Minigame::Circuit(_) | Minigame::Lights(_) => {
                let n = self.n() as i8;
                ((0..n).contains(&c) && (0..n).contains(&r)).then(|| (c + r * n) as usize)
            }
        }
    }

    /// The cells it covers along both axes (the sliding puzzle's strips beside the square
    /// too).
    pub fn extent(&self) -> std::ops::Range<i8> {
        match self {
            Minigame::Slide(_) => {
                -(crate::puzzle::DEPTH as i8)..(crate::puzzle::N + crate::puzzle::DEPTH) as i8
            }
            Minigame::Circuit(_) | Minigame::Lights(_) => 0..self.n() as i8,
        }
    }

    /// Its cells' size on a console's face (m).
    pub fn cell(&self) -> f32 {
        match self {
            Minigame::Slide(_) => crate::level::PUZZLE_CELL,
            Minigame::Circuit(_) | Minigame::Lights(_) => GRID_CELL,
        }
    }

    /// The square's cells along each side (the elements' grid; the strips are outside it).
    pub fn n(&self) -> usize {
        match self {
            Minigame::Slide(_) => crate::puzzle::N,
            Minigame::Circuit(_) => circuit::N,
            Minigame::Lights(_) => lights::N,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_solve_every_kind() {
        let mut kinds = std::collections::HashSet::new();
        for seed in 0..300u32 {
            let g = Minigame::generate(seed);
            kinds.insert(g.kind());
            let mut at = g.start();
            assert!(!g.solved(&at), "seed {seed}: solved at the start");
            for _ in 0..200 {
                if g.solved(&at) {
                    break;
                }
                let i = g.hint(&at).expect("a hint");
                assert!(i < g.elements());
                g.poke(&mut at, i);
            }
            assert!(
                g.solved(&at),
                "seed {seed} {:?}: hints didn't solve it",
                g.kind()
            );
        }
        assert_eq!(kinds.len(), MinigameKind::ALL.len(), "every kind comes up");
    }
}
