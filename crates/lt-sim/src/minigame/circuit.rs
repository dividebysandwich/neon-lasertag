//! The circuit puzzle: a square of `N` x `N` wire tiles, each turned some quarter turns; a click
//! turns a tile a quarter turn clockwise. Current comes in from the square's left edge at one
//! row (`source`) and has to get out at its right edge at another (`sink`), through tiles whose
//! wires meet. Solved when it does.
//!
//! The wires are a random tree over every cell (`generate`), so every tile carries some and
//! only one way through exists; the branches off it lead nowhere. Each tile's state is how many
//! quarter turns it is off its wires' shape in the tree (0..4).
//!
//! Cells are (column, row): column 0 at the left, row 0 at the bottom; element `c + r * N`.
//! Directions are bits: up `UP`, right `RIGHT`, down `DOWN`, left `LEFT`.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

pub const N: usize = 4;
pub const UP: u8 = 1;
pub const RIGHT: u8 = 2;
pub const DOWN: u8 = 4;
pub const LEFT: u8 = 8;
const DIRS: [(u8, i8, i8); 4] = [(UP, 0, 1), (RIGHT, 1, 0), (DOWN, 0, -1), (LEFT, -1, 0)];

/// A wire shape `mask` turned `k` quarter turns clockwise (up to right, right to down, ...).
pub fn turned(mask: u8, k: u8) -> u8 {
    (0..k % 4).fold(mask & 0xF, |m, _| ((m << 1) | (m >> 3)) & 0xF)
}

fn opposite(d: u8) -> u8 {
    turned(d, 2)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Circuit {
    /// Each cell's wires as they lie in the tree (element order).
    pub shapes: Vec<u8>,
    /// The rows where the current comes in (left edge) and goes out (right edge).
    pub source: usize,
    pub sink: usize,
    /// The cells of the one way through, from the source to the sink.
    pub route: Vec<usize>,
    /// How the puzzle starts: each tile's quarter turns off.
    start: Vec<u8>,
}

fn idx(c: usize, r: usize) -> usize {
    c + r * N
}

fn cell(i: usize) -> (usize, usize) {
    (i % N, i / N)
}

fn step(i: usize, d: u8) -> Option<usize> {
    let (c, r) = cell(i);
    let &(_, dc, dr) = DIRS.iter().find(|x| x.0 == d)?;
    let (c, r) = (c as i8 + dc, r as i8 + dr);
    ((0..N as i8).contains(&c) && (0..N as i8).contains(&r)).then(|| idx(c as usize, r as usize))
}

impl Circuit {
    /// The puzzle for `seed` (the same everywhere).
    pub fn generate(seed: u32) -> Circuit {
        let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0xC1C0_17E5);
        loop {
            let c = Self::try_generate(&mut rng);
            if let Some(c) = c {
                return c;
            }
        }
    }

    fn try_generate(rng: &mut ChaCha8Rng) -> Option<Circuit> {
        // A random tree over the cells (growing from a random cell, each time joining a cell
        // next to the tree at random).
        let mut shapes = vec![0u8; N * N];
        let mut inside = vec![false; N * N];
        inside[rng.random_range(0..N * N)] = true;
        for _ in 1..N * N {
            let mut edges = Vec::new();
            for i in (0..N * N).filter(|&i| inside[i]) {
                for (d, _, _) in DIRS {
                    if let Some(j) = step(i, d)
                        && !inside[j]
                    {
                        edges.push((i, j, d));
                    }
                }
            }
            let (i, j, d) = edges[rng.random_range(0..edges.len())];
            shapes[i] |= d;
            shapes[j] |= opposite(d);
            inside[j] = true;
        }
        let source = rng.random_range(0..N);
        let sink = rng.random_range(0..N);
        let (a, b) = (idx(0, source), idx(N - 1, sink));
        shapes[a] |= LEFT;
        shapes[b] |= RIGHT;
        // The way through: the tree's path from a to b.
        let route = Self::path(&shapes, a, b)?;
        // Long enough to need some thought.
        if route.len() < 6 {
            return None;
        }
        let mut c = Circuit {
            shapes,
            source,
            sink,
            route,
            start: Vec::new(),
        };
        // Scrambled: at least four tiles of the way through off (and not solved).
        for _ in 0..50 {
            let start: Vec<u8> = (0..N * N).map(|_| rng.random_range(0..4u8)).collect();
            let off = c.route.iter().filter(|&&i| !c.fits(&start, i)).count();
            if off >= 4 && !c.solved(&start) {
                c.start = start;
                return Some(c);
            }
        }
        None
    }

    /// The cells from `a` to `b` along the tree's wires.
    fn path(shapes: &[u8], a: usize, b: usize) -> Option<Vec<usize>> {
        let mut from = vec![usize::MAX; N * N];
        let mut todo = vec![a];
        from[a] = a;
        while let Some(i) = todo.pop() {
            for (d, _, _) in DIRS {
                if shapes[i] & d != 0
                    && let Some(j) = step(i, d)
                    && from[j] == usize::MAX
                {
                    from[j] = i;
                    todo.push(j);
                }
            }
        }
        if from[b] == usize::MAX {
            return None;
        }
        let mut p = vec![b];
        while *p.last().unwrap() != a {
            p.push(from[*p.last().unwrap()]);
        }
        p.reverse();
        Some(p)
    }

    pub fn start(&self) -> Vec<u8> {
        self.start.clone()
    }

    /// Tile `i`'s wires as it lies now.
    pub fn wires(&self, at: &[u8], i: usize) -> u8 {
        turned(self.shapes[i], at.get(i).copied().unwrap_or(0))
    }

    /// Whether route tile `i` lies so its wires join the route's tiles before and after it.
    fn fits(&self, at: &[u8], i: usize) -> bool {
        let Some(k) = self.route.iter().position(|&x| x == i) else {
            return true;
        };
        let w = self.wires(at, i);
        let towards = |j: usize| {
            DIRS.iter()
                .find(|(d, _, _)| step(i, *d) == Some(j))
                .map(|x| x.0)
                .unwrap_or(0)
        };
        let before = if k == 0 {
            LEFT
        } else {
            towards(self.route[k - 1])
        };
        let after = if k + 1 == self.route.len() {
            RIGHT
        } else {
            towards(self.route[k + 1])
        };
        w & before != 0 && w & after != 0
    }

    /// The tiles the current reaches, from the source along wires that meet.
    pub fn powered(&self, at: &[u8]) -> Vec<bool> {
        let mut on = vec![false; N * N];
        let a = idx(0, self.source);
        if self.wires(at, a) & LEFT == 0 {
            return on;
        }
        on[a] = true;
        let mut todo = vec![a];
        while let Some(i) = todo.pop() {
            let w = self.wires(at, i);
            for (d, _, _) in DIRS {
                if w & d != 0
                    && let Some(j) = step(i, d)
                    && !on[j]
                    && self.wires(at, j) & opposite(d) != 0
                {
                    on[j] = true;
                    todo.push(j);
                }
            }
        }
        on
    }

    pub fn solved(&self, at: &[u8]) -> bool {
        let b = idx(N - 1, self.sink);
        at.len() == N * N && self.powered(at)[b] && self.wires(at, b) & RIGHT != 0
    }

    /// Tile `i` clicked: a quarter turn clockwise.
    pub fn poke(&self, at: &mut [u8], i: usize) {
        if let Some(k) = at.get_mut(i) {
            *k = (*k + 1) % 4;
        }
    }

    /// A tile to turn next (for the bots): the first on the way through that doesn't fit.
    pub fn hint(&self, at: &[u8]) -> Option<usize> {
        if at.len() != N * N {
            return None;
        }
        self.route.iter().copied().find(|&i| !self.fits(at, i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuits_are_trees_with_a_long_way_through() {
        for seed in 0..500u32 {
            let c = Circuit::generate(seed);
            // Every cell carries wires, every wire meets one coming the other way (but the
            // ends at the edges).
            for i in 0..N * N {
                assert_ne!(c.shapes[i], 0, "seed {seed}");
                for (d, _, _) in DIRS {
                    if c.shapes[i] & d != 0 {
                        match step(i, d) {
                            Some(j) => assert!(c.shapes[j] & opposite(d) != 0, "seed {seed}"),
                            None => assert!(
                                i == idx(0, c.source) && d == LEFT
                                    || i == idx(N - 1, c.sink) && d == RIGHT,
                                "seed {seed}: a wire off the edge"
                            ),
                        }
                    }
                }
            }
            assert!(c.route.len() >= 6);
            assert!(!c.solved(&c.start()), "seed {seed}");
            // As the tree has it: solved.
            assert!(c.solved(&vec![0; N * N]), "seed {seed}");
        }
    }

    #[test]
    fn hints_solve_circuits_in_few_turns() {
        for seed in 0..500u32 {
            let c = Circuit::generate(seed);
            let mut at = c.start();
            let mut turns = 0;
            while !c.solved(&at) {
                let i = c.hint(&at).expect("a hint");
                c.poke(&mut at, i);
                turns += 1;
                assert!(turns <= 3 * c.route.len(), "seed {seed}");
            }
            // (Turned tiles can meet off the tree's wires too: a shortcut is fine.)
            assert!(turns >= 2, "seed {seed}: too easy");
        }
    }

    #[test]
    fn turning_goes_round() {
        assert_eq!(turned(UP, 1), RIGHT);
        assert_eq!(turned(LEFT, 1), UP);
        assert_eq!(turned(UP | RIGHT, 3), UP | LEFT);
    }
}
