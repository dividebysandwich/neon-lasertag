//! The turret boxes' sliding puzzle: a square of `N` x `N` cells filled completely by pieces
//! that slide in from the square's four sides, each only along its own way in (a piece on the
//! left side slides right, one at the bottom slides up, ...).
//!
//! Clicking a piece that is all the way out slides it in until it meets another piece. If it
//! gets stuck partly in, clicking it again slides it all the way back out, so the player can
//! clear the blockage and try again. The same goes for a piece that is all the way in. The one
//! case where a piece can't come back out is when a piece slid in after it now sits in its way
//! out. That piece has to come out first. A click on a piece that can't come out does nothing:
//! it never stops halfway on its way out. Because of this the puzzle can never jam: the piece
//! that moved last can always slide back out (`something_can_always_slide_out` tests it).
//! The puzzle is solved when every piece is all the way in. Some pieces have to go in before
//! others: a piece whose way in crosses another piece's cells can't get past that piece once
//! it's in.
//!
//! Everything follows from a seed (`Puzzle::generate`), so the sim only sends the seed and how
//! far out each piece is. Cells are (column, row): column 0 at the left (-x in the box's
//! frame), row 0 at the bottom. Pieces are carved off a full square one by one, each sliding
//! out through the cells of those carved before it, so putting them back in the reverse order
//! always works; a piece is at most `DEPTH` cells deep along its way in, and no two pieces of
//! one side share a row (or column), so the pieces out lie side by side in a strip `DEPTH`
//! cells deep along each side of the square.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Cells along each side of the square.
pub const N: usize = 4;
/// The most cells a piece spans along its way in (the strips beside the square are this deep).
pub const DEPTH: usize = 2;

/// The sides of the square: the way in for each is the opposite direction.
pub const LEFT: u8 = 0;
pub const RIGHT: u8 = 1;
pub const BOTTOM: u8 = 2;
pub const TOP: u8 = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// The side it slides in from (`LEFT`, `RIGHT`, `BOTTOM`, `TOP`).
    pub side: u8,
    /// Its cells all the way in.
    pub cells: Vec<(i8, i8)>,
    /// How many cells it moves from all the way in to all the way out.
    pub out: u8,
}

impl Piece {
    /// One cell's step out (column, row).
    pub fn outward(&self) -> (i8, i8) {
        match self.side {
            LEFT => (-1, 0),
            RIGHT => (1, 0),
            BOTTOM => (0, -1),
            _ => (0, 1),
        }
    }

    /// Its cells `k` cells out from all the way in.
    pub fn cells_at(&self, k: u8) -> impl Iterator<Item = (i8, i8)> + '_ {
        let (dc, dr) = self.outward();
        let k = k as i8;
        self.cells
            .iter()
            .map(move |&(c, r)| (c + dc * k, r + dr * k))
    }
}

/// What a poke did to a piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slide {
    /// Slid all the way in.
    In,
    /// Slid in part of the way: another piece is in its way.
    Stuck,
    /// Slid all the way out.
    Out,
    /// Didn't move: blocked straight away.
    Blocked,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Puzzle {
    pub pieces: Vec<Piece>,
}

fn inside(c: i8, r: i8) -> bool {
    (0..N as i8).contains(&c) && (0..N as i8).contains(&r)
}

/// The row (left, right) or column (bottom, top) a cell is on for pieces of `side`.
fn line(side: u8, (c, r): (i8, i8)) -> u8 {
    if side < BOTTOM { r as u8 } else { c as u8 }
}

/// The cells between a cell and the square's `side` edge.
fn path(side: u8, (c, r): (i8, i8)) -> Vec<(i8, i8)> {
    let n = N as i8;
    match side {
        LEFT => (0..c).map(|x| (x, r)).collect(),
        RIGHT => (c + 1..n).map(|x| (x, r)).collect(),
        BOTTOM => (0..r).map(|y| (c, y)).collect(),
        _ => (r + 1..n).map(|y| (c, y)).collect(),
    }
}

impl Puzzle {
    /// The puzzle for `seed` (the same everywhere).
    pub fn generate(seed: u32) -> Puzzle {
        let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0x5112_DE00);
        let mut best: Option<(i32, Puzzle)> = None;
        for _ in 0..60 {
            let Some(p) = Self::carve(&mut rng) else {
                continue;
            };
            // Not too easy, not too hard: five to seven pieces, a few that have to wait for
            // others, most of them bigger than a cell, from three sides or more.
            let n = p.pieces.len() as i32;
            let deps = p.dependencies() as i32;
            let singles = p.pieces.iter().filter(|q| q.cells.len() == 1).count() as i32;
            let mut sides = [false; 4];
            for q in &p.pieces {
                sides[q.side as usize] = true;
            }
            let sides = sides.iter().filter(|s| **s).count() as i32;
            let miss = (5 - n).max(0) * 3
                + (n - 7).max(0) * 3
                + (2 - deps).max(0) * 4
                + (deps - 4).max(0) * 2
                + singles.saturating_sub(1) * 2
                + (3 - sides).max(0) * 2;
            if miss == 0 {
                return p;
            }
            if best.as_ref().is_none_or(|b| miss < b.0) {
                best = Some((miss, p));
            }
        }
        best.map(|b| b.1).unwrap_or_else(Puzzle::fallback)
    }

    /// One try at carving the square into pieces (`None`: a cell was left that no piece can
    /// take).
    fn carve(rng: &mut ChaCha8Rng) -> Option<Puzzle> {
        let mut gone = [[false; N]; N];
        let mut used = [0u8; 4];
        let mut pieces = Vec::new();
        let is_gone = |gone: &[[bool; N]; N], (c, r): (i8, i8)| gone[c as usize][r as usize];
        while gone.iter().flatten().any(|g| !g) {
            // A first cell: one with nothing left between it and a side whose line is free.
            let mut seeds = Vec::new();
            for side in 0..4u8 {
                for c in 0..N as i8 {
                    for r in 0..N as i8 {
                        let cell = (c, r);
                        if !is_gone(&gone, cell)
                            && used[side as usize] & (1 << line(side, cell)) == 0
                            && path(side, cell).iter().all(|&p| is_gone(&gone, p))
                        {
                            seeds.push((side, cell));
                        }
                    }
                }
            }
            if seeds.is_empty() {
                return None;
            }
            let (side, first) = seeds[rng.random_range(0..seeds.len())];
            let want = match rng.random_range(0..20) {
                0..6 => 2,
                6..15 => 3,
                _ => 4,
            };
            let mut cells = vec![first];
            while cells.len() < want {
                let mut next = Vec::new();
                for &(c, r) in &cells {
                    for (dc, dr) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let n = (c + dc, r + dr);
                        if !inside(n.0, n.1)
                            || is_gone(&gone, n)
                            || cells.contains(&n)
                            || next.contains(&n)
                            || used[side as usize] & (1 << line(side, n)) != 0
                        {
                            continue;
                        }
                        if !path(side, n)
                            .iter()
                            .all(|p| is_gone(&gone, *p) || cells.contains(p))
                        {
                            continue;
                        }
                        // (Not deeper than `DEPTH` along the way in.)
                        let along = |p: &(i8, i8)| if side < BOTTOM { p.0 } else { p.1 };
                        let lo = cells.iter().map(along).min().unwrap_or(0).min(along(&n));
                        let hi = cells.iter().map(along).max().unwrap_or(0).max(along(&n));
                        if (hi - lo) as usize >= DEPTH {
                            continue;
                        }
                        next.push(n);
                    }
                }
                if next.is_empty() {
                    break;
                }
                cells.push(next[rng.random_range(0..next.len())]);
            }
            for &cell in &cells {
                gone[cell.0 as usize][cell.1 as usize] = true;
                used[side as usize] |= 1 << line(side, cell);
            }
            pieces.push(Self::piece(side, cells));
        }
        Some(Puzzle { pieces })
    }

    fn piece(side: u8, cells: Vec<(i8, i8)>) -> Piece {
        let n = N as i8;
        let out = match side {
            LEFT => cells.iter().map(|c| c.0).max().unwrap_or(0) + 1,
            RIGHT => n - cells.iter().map(|c| c.0).min().unwrap_or(0),
            BOTTOM => cells.iter().map(|c| c.1).max().unwrap_or(0) + 1,
            _ => n - cells.iter().map(|c| c.1).min().unwrap_or(0),
        };
        Piece {
            side,
            cells,
            out: out as u8,
        }
    }

    /// A plain puzzle (never needed in practice: carving nearly always works first time).
    fn fallback() -> Puzzle {
        let mut pieces = Vec::new();
        for r in 0..N as i8 {
            pieces.push(Self::piece(LEFT, vec![(0, r), (1, r)]));
            pieces.push(Self::piece(RIGHT, vec![(2, r), (3, r)]));
        }
        Puzzle { pieces }
    }

    /// How many (piece, piece) pairs have to go in in a given order: one's cells on the
    /// other's way in.
    pub fn dependencies(&self) -> usize {
        let mut n = 0;
        for (i, a) in self.pieces.iter().enumerate() {
            let way: Vec<(i8, i8)> = (1..=a.out)
                .flat_map(|k| a.cells_at(k))
                .filter(|&(c, r)| inside(c, r))
                .collect();
            for (j, b) in self.pieces.iter().enumerate() {
                if i != j && b.cells.iter().any(|c| way.contains(c)) {
                    n += 1;
                }
            }
        }
        n
    }

    /// Every piece all the way out (how a puzzle starts).
    pub fn start(&self) -> Vec<u8> {
        self.pieces.iter().map(|p| p.out).collect()
    }

    pub fn solved(&self, at: &[u8]) -> bool {
        at.len() == self.pieces.len() && at.iter().all(|&k| k == 0)
    }

    /// The square's cells taken by the pieces other than `but` where they are (`at`).
    fn taken(&self, at: &[u8], but: usize) -> [[bool; N]; N] {
        let mut t = [[false; N]; N];
        for (j, p) in self.pieces.iter().enumerate() {
            if j == but {
                continue;
            }
            for (c, r) in p.cells_at(at.get(j).copied().unwrap_or(p.out)) {
                if inside(c, r) {
                    t[c as usize][r as usize] = true;
                }
            }
        }
        t
    }

    /// Piece `i` clicked: all the way out, it slides in as far as it can; otherwise out, if
    /// nothing is in its way.
    pub fn poke(&self, at: &mut [u8], i: usize) -> Slide {
        let (Some(p), Some(&k)) = (self.pieces.get(i), at.get(i)) else {
            return Slide::Blocked;
        };
        let taken = self.taken(at, i);
        let free = |k: u8| {
            p.cells_at(k)
                .all(|(c, r)| !inside(c, r) || !taken[c as usize][r as usize])
        };
        if k >= p.out {
            let mut k = p.out;
            while k > 0 && free(k - 1) {
                k -= 1;
            }
            at[i] = k;
            if k == 0 {
                Slide::In
            } else if k == p.out {
                Slide::Blocked
            } else {
                Slide::Stuck
            }
        } else if (k + 1..=p.out).all(free) {
            at[i] = p.out;
            Slide::Out
        } else {
            Slide::Blocked
        }
    }

    /// The piece on the cell (column, row) with the pieces where they are (`at`): cells
    /// beside the square too, for the pieces out.
    pub fn piece_at(&self, at: &[u8], c: i8, r: i8) -> Option<usize> {
        self.pieces.iter().enumerate().find_map(|(j, p)| {
            p.cells_at(at.get(j).copied().unwrap_or(p.out))
                .any(|x| x == (c, r))
                .then_some(j)
        })
    }

    /// A piece to poke next towards solving it (for the bots): one that's in the way out of
    /// a piece stuck or in too early first, else one that goes all the way in now.
    pub fn hint(&self, at: &[u8]) -> Option<usize> {
        if at.len() != self.pieces.len() {
            return None;
        }
        let order = self.order();
        let slides_out = |i: usize| {
            let mut a = at.to_vec();
            at[i] < self.pieces[i].out && self.poke(&mut a, i) == Slide::Out
        };
        // In the solving order: every piece before the first one that isn't in yet is in;
        // anything in (or stuck) that comes later may be in the way: out with it, the last
        // first.
        let first = order.iter().position(|&i| at[i] != 0)?;
        if let Some(&i) = order[first + 1..]
            .iter()
            .rev()
            .find(|&&i| at[i] < self.pieces[i].out && slides_out(i))
        {
            return Some(i);
        }
        let i = order[first];
        if at[i] >= self.pieces[i].out || slides_out(i) {
            return Some(i);
        }
        (0..self.pieces.len()).find(|&j| slides_out(j))
    }

    /// An order the pieces go in in: built from the end, each piece one that slides all the
    /// way in with every piece before it in.
    fn order(&self) -> Vec<usize> {
        let n = self.pieces.len();
        let mut left: Vec<usize> = (0..n).collect();
        let mut rev = Vec::new();
        while !left.is_empty() {
            let mut at = vec![0u8; n];
            for (j, p) in self.pieces.iter().enumerate() {
                if !left.contains(&j) {
                    at[j] = p.out;
                }
            }
            let Some(pos) = left.iter().position(|&i| {
                let mut a = at.clone();
                a[i] = self.pieces[i].out;
                self.poke(&mut a, i) == Slide::In
            }) else {
                break;
            };
            rev.push(left.remove(pos));
        }
        rev.extend(left);
        rev.reverse();
        rev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puzzles_fill_the_square_and_can_be_solved() {
        for seed in 0..300u32 {
            let p = Puzzle::generate(seed);
            let mut seen = [[0u8; N]; N];
            for q in &p.pieces {
                assert!(q.cells.len() <= 4, "seed {seed}");
                for &(c, r) in &q.cells {
                    seen[c as usize][r as usize] += 1;
                }
                // Out, it lies in the strip beside its side, `DEPTH` cells deep at most.
                for (c, r) in q.cells_at(q.out) {
                    assert!(!inside(c, r), "seed {seed}: piece out still in the square");
                    let d = (-c).max(c - N as i8 + 1).max(-r).max(r - N as i8 + 1);
                    assert!(d >= 1 && d as usize <= DEPTH, "seed {seed}: {d} out");
                }
            }
            assert!(
                seen.iter().flatten().all(|&n| n == 1),
                "seed {seed}: {seen:?}"
            );
            // The pieces out beside the square don't overlap.
            let mut outs = Vec::new();
            for q in &p.pieces {
                for cell in q.cells_at(q.out) {
                    assert!(!outs.contains(&cell), "seed {seed}: pieces out overlap");
                    outs.push(cell);
                }
            }
            // Following the hints solves it.
            let mut at = p.start();
            for _ in 0..60 {
                if p.solved(&at) {
                    break;
                }
                let i = p.hint(&at).expect("a hint");
                p.poke(&mut at, i);
            }
            assert!(p.solved(&at), "seed {seed}: hints didn't solve it");
        }
    }

    #[test]
    fn most_puzzles_need_some_pieces_first() {
        let n = (0..200u32)
            .filter(|&s| {
                let p = Puzzle::generate(s);
                (5..=7).contains(&p.pieces.len()) && p.dependencies() >= 2
            })
            .count();
        assert!(n > 180, "{n} of 200");
    }

    #[test]
    fn blocked_pieces_stick_and_slide_back_out() {
        // Find a pair where one blocks the other, put the blocker in first.
        for seed in 0..100u32 {
            let p = Puzzle::generate(seed);
            let mut at = p.start();
            let order = p.order();
            let (first, later) = (order[0], order[order.len() - 1]);
            if p.poke(&mut at, later) != Slide::In {
                continue;
            }
            match p.poke(&mut at, first) {
                Slide::Stuck | Slide::Blocked => {}
                Slide::In => continue,
                Slide::Out => panic!("was out"),
            }
            // The blocker out again, and the first one too: then in the right order.
            if at[first] < p.pieces[first].out {
                assert_eq!(p.poke(&mut at, first), Slide::Out);
            }
            assert_eq!(p.poke(&mut at, later), Slide::Out);
            assert_eq!(p.poke(&mut at, first), Slide::In);
            return;
        }
        panic!("no puzzle with a blocking pair");
    }

    #[test]
    fn something_can_always_slide_out() {
        // Random clicking never jams the puzzle: unless solved or everything is out, some
        // piece in or stuck can come out.
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        for seed in 0..60u32 {
            let p = Puzzle::generate(seed);
            let mut at = p.start();
            for _ in 0..200 {
                let i = rng.random_range(0..p.pieces.len());
                p.poke(&mut at, i);
                let moved = (0..p.pieces.len()).any(|j| {
                    at[j] < p.pieces[j].out && {
                        let mut a = at.clone();
                        p.poke(&mut a, j) == Slide::Out
                    }
                });
                assert!(
                    moved || at.iter().zip(&p.pieces).all(|(k, q)| *k == q.out),
                    "seed {seed}: jammed at {at:?}"
                );
            }
        }
    }
}
