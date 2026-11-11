//! The pattern puzzle: a square of `N` x `N` lights, some on, and a pattern of cells marked to
//! be lit; clicking a light switches it (only it) on or off. Solved when the lights on are
//! exactly the marked ones.
//!
//! It starts as the pattern with a few lights switched over (`generate`), so it takes those
//! few clicks; quick to read and do in the middle of a fight.
//!
//! Cells are (column, row) as the circuit's: element `c + r * N`; each light's state 0 or 1.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

pub const N: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    /// Which cells are to be lit (element order).
    pub marked: Vec<bool>,
    start: Vec<u8>,
}

impl Pattern {
    /// The puzzle for `seed` (the same everywhere): five to eight cells marked, four to six
    /// lights off from them.
    pub fn generate(seed: u32) -> Pattern {
        let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0x9A77_E2A5);
        let pick = |rng: &mut ChaCha8Rng, n: usize| {
            let mut cells: Vec<usize> = (0..N * N).collect();
            for i in (1..cells.len()).rev() {
                cells.swap(i, rng.random_range(0..=i));
            }
            cells.truncate(n);
            cells
        };
        let mut marked = vec![false; N * N];
        let n = rng.random_range(5..=8);
        for i in pick(&mut rng, n) {
            marked[i] = true;
        }
        let mut start: Vec<u8> = marked.iter().map(|&m| m as u8).collect();
        let n = rng.random_range(4..=6);
        for i in pick(&mut rng, n) {
            start[i] ^= 1;
        }
        Pattern { marked, start }
    }

    pub fn start(&self) -> Vec<u8> {
        self.start.clone()
    }

    /// Light `i` clicked: switched over.
    pub fn poke(&self, at: &mut [u8], i: usize) {
        if let Some(v) = at.get_mut(i) {
            *v ^= 1;
        }
    }

    pub fn solved(&self, at: &[u8]) -> bool {
        at.len() == N * N && at.iter().zip(&self.marked).all(|(&v, &m)| (v != 0) == m)
    }

    /// A light to click next (for the bots): the first one that's wrong.
    pub fn hint(&self, at: &[u8]) -> Option<usize> {
        if at.len() != N * N {
            return None;
        }
        (0..N * N).find(|&i| (at[i] != 0) != self.marked[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_take_a_few_clicks() {
        for seed in 0..500u32 {
            let p = Pattern::generate(seed);
            let marked = p.marked.iter().filter(|&&m| m).count();
            assert!((5..=8).contains(&marked), "seed {seed}");
            let mut at = p.start();
            let mut clicks = 0;
            while !p.solved(&at) {
                let i = p.hint(&at).expect("a hint");
                p.poke(&mut at, i);
                clicks += 1;
            }
            assert!((4..=6).contains(&clicks), "seed {seed}: {clicks} clicks");
        }
    }
}
