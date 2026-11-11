//! The lights puzzle: a square of `N` x `N` lights, some on; clicking one switches it and the
//! (up to four) lights next to it over. Solved when they're all off.
//!
//! A puzzle starts as some presses on a dark square (`generate`), so it can always be solved;
//! `solve` works out the fewest presses from any state (over GF(2): pressing a light twice is
//! as not pressing it, and the order doesn't matter), which the bots' hint follows.
//!
//! Cells are (column, row) as the circuit's: element `c + r * N`; each light's state 0 or 1.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

pub const N: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Lights {
    start: Vec<u8>,
}

/// The lights pressing light `i` switches over (bits).
fn press_mask(i: usize) -> u16 {
    let (c, r) = (i % N, i / N);
    let mut m = 1u16 << i;
    if c > 0 {
        m |= 1 << (i - 1);
    }
    if c + 1 < N {
        m |= 1 << (i + 1);
    }
    if r > 0 {
        m |= 1 << (i - N);
    }
    if r + 1 < N {
        m |= 1 << (i + N);
    }
    m
}

fn bits(at: &[u8]) -> u16 {
    at.iter()
        .take(N * N)
        .enumerate()
        .filter(|(_, v)| **v != 0)
        .fold(0, |m, (i, _)| m | 1 << i)
}

/// The fewest presses (bits) that switch every light in `state` off, if any do.
pub fn solve(state: u16) -> Option<u16> {
    // Rows: one equation per light (which presses switch it, bit 16: whether it's on).
    let n = N * N;
    let mut rows: Vec<u32> = (0..n)
        .map(|i| press_mask(i) as u32 | ((state as u32 >> i) & 1) << 16)
        .collect();
    let mut pivots: Vec<(usize, usize)> = Vec::new(); // (column, row)
    let mut r = 0;
    for col in 0..n {
        let Some(p) = (r..n).find(|&k| rows[k] >> col & 1 != 0) else {
            continue;
        };
        rows.swap(r, p);
        for k in 0..n {
            if k != r && rows[k] >> col & 1 != 0 {
                rows[k] ^= rows[r];
            }
        }
        pivots.push((col, r));
        r += 1;
    }
    // Inconsistent: a row with no presses left but a light on.
    if rows[r..]
        .iter()
        .any(|row| row & 0xFFFF == 0 && row >> 16 & 1 != 0)
    {
        return None;
    }
    let free: Vec<usize> = (0..n)
        .filter(|c| !pivots.iter().any(|p| p.0 == *c))
        .collect();
    let mut best: Option<u16> = None;
    for pick in 0u32..1 << free.len() {
        let mut x: u16 = 0;
        for (k, &c) in free.iter().enumerate() {
            if pick >> k & 1 != 0 {
                x |= 1 << c;
            }
        }
        for &(col, row) in &pivots {
            let coef = rows[row] & 0xFFFF & !(1 << col);
            let v = (rows[row] >> 16 & 1) ^ ((coef & x as u32).count_ones() & 1);
            if v != 0 {
                x |= 1 << col;
            }
        }
        if best.is_none_or(|b| x.count_ones() < b.count_ones()) {
            best = Some(x);
        }
    }
    best
}

impl Lights {
    /// The puzzle for `seed` (the same everywhere): five to eight presses on a dark square,
    /// needing four at least to undo.
    pub fn generate(seed: u32) -> Lights {
        let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0x116_475);
        loop {
            let presses = rng.random_range(5..=8);
            let mut on: u16 = 0;
            let mut pressed: u16 = 0;
            while pressed.count_ones() < presses {
                let i = rng.random_range(0..N * N);
                if pressed & 1 << i == 0 {
                    pressed |= 1 << i;
                    on ^= press_mask(i);
                }
            }
            if solve(on).is_some_and(|s| s.count_ones() >= 4) {
                return Lights {
                    start: (0..N * N).map(|i| (on >> i & 1) as u8).collect(),
                };
            }
        }
    }

    pub fn start(&self) -> Vec<u8> {
        self.start.clone()
    }

    /// Light `i` clicked: it and the ones next to it switch over.
    pub fn poke(&self, at: &mut [u8], i: usize) {
        if i >= N * N || at.len() < N * N {
            return;
        }
        let m = press_mask(i);
        for (j, v) in at.iter_mut().enumerate().take(N * N) {
            if m & 1 << j != 0 {
                *v ^= 1;
            }
        }
    }

    pub fn solved(&self, at: &[u8]) -> bool {
        at.len() == N * N && bits(at) == 0
    }

    /// A light to press next (for the bots): one of the fewest presses left.
    pub fn hint(&self, at: &[u8]) -> Option<usize> {
        if at.len() != N * N {
            return None;
        }
        let s = solve(bits(at))?;
        (s != 0).then(|| s.trailing_zeros() as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lights_need_some_presses_and_hints_solve_them() {
        for seed in 0..500u32 {
            let l = Lights::generate(seed);
            let mut at = l.start();
            assert!(!l.solved(&at), "seed {seed}");
            let mut presses = 0;
            while !l.solved(&at) {
                let i = l.hint(&at).expect("a hint");
                l.poke(&mut at, i);
                presses += 1;
                assert!(presses <= 16, "seed {seed}");
            }
            assert!(presses >= 4, "seed {seed}: {presses} presses");
        }
    }

    #[test]
    fn solving_finds_the_fewest_presses() {
        // A single press: one press back.
        for i in 0..N * N {
            assert_eq!(solve(press_mask(i)).map(|s| s.count_ones()), Some(1));
        }
        assert_eq!(solve(0), Some(0));
    }
}
