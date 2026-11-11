//! The click puzzles on the consoles (`crate::console`): one of a few kinds, all made from a
//! seed (the kind too: `MinigameKind::of`), so the sim only sends the seed and a byte per
//! element (a piece, a tile, a light). A click pokes one element; the puzzle is solved when
//! its elements are all where they belong.
//!
//! Every kind lies on a grid of cells on the console's face (`extent` along both axes), and
//! `element_at` tells which element a cell belongs to (for clicking).

use serde::{Deserialize, Serialize};

use crate::puzzle::{Puzzle, Slide};
use crate::types::PuzzleSlide;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MinigameKind {
    /// The turret boxes' sliding puzzle (`crate::puzzle`).
    Slide,
}

impl MinigameKind {
    pub const ALL: [MinigameKind; 1] = [MinigameKind::Slide];

    /// The kind a seed makes.
    pub fn of(seed: u32) -> MinigameKind {
        let n = Self::ALL.len() as u32;
        Self::ALL[(seed.rotate_right(11).wrapping_mul(0x9E37_79B9) >> 16) as usize % n as usize]
    }

    pub fn name(self) -> &'static str {
        match self {
            MinigameKind::Slide => "sliding puzzle",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Minigame {
    Slide(Puzzle),
}

impl Minigame {
    /// The puzzle for `seed` (the same everywhere).
    pub fn generate(seed: u32) -> Minigame {
        match MinigameKind::of(seed) {
            MinigameKind::Slide => Minigame::Slide(Puzzle::generate(seed)),
        }
    }

    pub fn kind(&self) -> MinigameKind {
        match self {
            Minigame::Slide(_) => MinigameKind::Slide,
        }
    }

    /// How it starts.
    pub fn start(&self) -> Vec<u8> {
        match self {
            Minigame::Slide(p) => p.start(),
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
        }
    }

    pub fn solved(&self, at: &[u8]) -> bool {
        match self {
            Minigame::Slide(p) => p.solved(at),
        }
    }

    /// An element to poke next towards solving it (for the bots).
    pub fn hint(&self, at: &[u8]) -> Option<usize> {
        match self {
            Minigame::Slide(p) => p.hint(at),
        }
    }

    /// How many elements it has (valid clicks are below this).
    pub fn elements(&self) -> usize {
        match self {
            Minigame::Slide(p) => p.pieces.len(),
        }
    }

    /// The element on cell (column, row) with the elements where they are (`at`).
    pub fn element_at(&self, at: &[u8], c: i8, r: i8) -> Option<usize> {
        match self {
            Minigame::Slide(p) => p.piece_at(at, c, r),
        }
    }

    /// The cells it covers along both axes (the sliding puzzle's strips beside the square
    /// too).
    pub fn extent(&self) -> std::ops::Range<i8> {
        match self {
            Minigame::Slide(_) => {
                -(crate::puzzle::DEPTH as i8)..(crate::puzzle::N + crate::puzzle::DEPTH) as i8
            }
        }
    }

    /// Its cells' size on a console's face (m).
    pub fn cell(&self) -> f32 {
        match self {
            Minigame::Slide(_) => crate::level::PUZZLE_CELL,
        }
    }

    /// The square's cells along each side (the elements' grid; the strips are outside it).
    pub fn n(&self) -> usize {
        match self {
            Minigame::Slide(_) => crate::puzzle::N,
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
