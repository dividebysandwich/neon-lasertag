//! Megatarget mode: the secondary targets that are shooting puzzles, with their pieces in the
//! pockets (`Mega::relay_route`):
//!
//! - relays (`SecondaryKind::Relay`): a beam from an emitter on the pocket's end wall to a
//!   receiver over the target, by way of `RELAYS` prisms hanging in the pocket. A shot turns a
//!   prism a quarter turn (anyone's: enemies can put it out again); with every one turned
//!   right (state 0) the beam reaches the receiver and charges the shooter.
//!
//! - lamps (`SecondaryKind::Sequence`): `LAMPS` lamps on the pocket's walls flash a sequence
//!   of `STEPS` (`sequence`, over and over while nobody's into it: `show`); shooting them in
//!   that order charges the shooter; a lamp out of turn (anyone's) starts it over.
//!
//! Everything follows from a seed, as the consoles' puzzles: the sim sends the seed and the
//! pieces' states.

use glam::Vec3;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::level::Mega;
use crate::rules::SecondaryKind;

/// Relays on a beam's way, and how big a piece is to bolts (a ball's radius, m).
pub const RELAYS: usize = 3;
pub const PIECE_R: f32 = 0.38;
/// Seconds a piece takes no more shots after one (turning), and the beam holds once it
/// reaches the receiver (before the relays are scrambled again).
pub const LOCK: f32 = 0.3;
pub const HOLD: f32 = 1.5;
/// Lamps in a pocket, and steps in their sequence.
pub const LAMPS: usize = 4;
pub const STEPS: usize = 5;
/// The show: each step this long (lit for `SHOW_LIT` of it), then a pause.
pub const SHOW_STEP: f32 = 0.7;
pub const SHOW_LIT: f32 = 0.5;
pub const SHOW_PAUSE: f32 = 1.6;

/// The pieces bolts hit in secondary target `side`'s pocket, by its kind (none: not a shooting
/// puzzle).
pub fn pieces(m: &Mega, kind: SecondaryKind, side: usize) -> Vec<Vec3> {
    match kind {
        SecondaryKind::Relay => m.relay_route[side][1..=RELAYS].to_vec(),
        SecondaryKind::Sequence => m.lamps[side].iter().map(|l| l.0).collect(),
        _ => Vec::new(),
    }
}

/// The first piece a segment `a`..`b` meets (from outside it, ahead): its side, its index and
/// where.
pub fn hit(m: &Mega, kinds: [SecondaryKind; 2], a: Vec3, b: Vec3) -> Option<(usize, usize, Vec3)> {
    let d = b - a;
    let dd = d.length_squared();
    if dd < 1e-12 {
        return None;
    }
    let mut best: Option<(f32, usize, usize)> = None;
    for side in 0..2 {
        for (i, c) in pieces(m, kinds[side], side).into_iter().enumerate() {
            let f = a - c;
            let bh = f.dot(d);
            let cc = f.length_squared() - PIECE_R * PIECE_R;
            let disc = bh * bh - dd * cc;
            if disc < 0.0 || cc <= 0.0 {
                continue;
            }
            let t = (-bh - disc.sqrt()) / dd;
            if (0.0..=1.0).contains(&t) && best.is_none_or(|x| t < x.0) {
                best = Some((t, side, i));
            }
        }
    }
    best.map(|(t, side, i)| (side, i, a + d * t))
}

/// Relay `i`'s way in and out on the beam's route (unit vectors).
pub fn relay_turn(route: &[Vec3; RELAYS + 2], i: usize) -> (Vec3, Vec3) {
    let p = route[i + 1];
    (
        (p - route[i]).normalize_or(Vec3::X),
        (route[i + 2] - p).normalize_or(Vec3::X),
    )
}

/// How the relays start for `seed`: each some quarter turns off (0: right), two or more off.
pub fn relay_start(seed: u32) -> Vec<u8> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0x4E1A_75);
    loop {
        let s: Vec<u8> = (0..RELAYS).map(|_| rng.random_range(0..4u8)).collect();
        if s.iter().filter(|&&k| k != 0).count() >= 2 {
            return s;
        }
    }
}

/// The lamps' sequence for `seed` (no lamp twice in a row).
pub fn sequence(seed: u32) -> [u8; STEPS] {
    let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0x5E9_0E9CE);
    let mut s = [0u8; STEPS];
    for i in 0..STEPS {
        loop {
            let l = rng.random_range(0..LAMPS as u8);
            if i == 0 || s[i - 1] != l {
                s[i] = l;
                break;
            }
        }
    }
    s
}

/// The show at `secs` into the match: the lamp flashing then, if any.
pub fn show(seq: &[u8; STEPS], secs: f32) -> Option<u8> {
    let cycle = STEPS as f32 * SHOW_STEP + SHOW_PAUSE;
    let t = secs.rem_euclid(cycle);
    let step = (t / SHOW_STEP) as usize;
    (step < STEPS && t - step as f32 * SHOW_STEP < SHOW_LIT).then(|| seq[step])
}

/// How far the beam gets: the relays turned right from the emitter on (all: it reaches the
/// receiver).
pub fn beam_reach(state: &[u8]) -> usize {
    state.iter().take_while(|&&k| k == 0).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{ArenaSize, GameMode, Level};

    #[test]
    fn relays_hang_in_their_pockets_in_sight_of_each_other() {
        for seed in 0..12 {
            let level = Level::generate_for(seed, ArenaSize::Medium, GameMode::Megatarget);
            let m = level.mega.as_ref().unwrap();
            let world = level.world();
            for side in 0..2 {
                let route = m.relay_route[side];
                let (a, b) = m.pockets[side];
                for p in &route[1..=RELAYS] {
                    assert!(
                        p.x > a.x && p.x < b.x && p.z > a.z && p.z < b.z,
                        "seed {seed}"
                    );
                    // Clear of the walls and above heads.
                    assert!(p.y > 2.2);
                }
                for w in route.windows(2) {
                    let d = (w[1] - w[0]).normalize();
                    assert!(
                        world.ray(w[0] + d * 0.3, w[1] - d * 0.3).is_none(),
                        "seed {seed} side {side}: the beam's way blocked"
                    );
                }
                // From the pocket's floor, each relay can be shot.
                let eye = (a + b) / 2.0 + Vec3::Y * 1.6;
                for p in &route[1..=RELAYS] {
                    let d = (*p - eye).normalize();
                    assert!(world.ray(eye, *p - d * PIECE_R).is_none(), "seed {seed}");
                }
            }
        }
    }

    #[test]
    fn lamps_hang_on_the_pocket_walls_in_sight() {
        for seed in 0..12 {
            let level = Level::generate_for(seed, ArenaSize::Medium, GameMode::Megatarget);
            let m = level.mega.as_ref().unwrap();
            let world = level.world();
            for side in 0..2 {
                let (a, b) = m.pockets[side];
                let eye = (a + b) / 2.0 + Vec3::Y * 1.6;
                for &(p, n) in &m.lamps[side] {
                    // Against a wall right behind it, in sight from the pocket.
                    assert!(world.ray(p, p - n * 0.3).is_some(), "seed {seed}: no wall");
                    let d = (p - eye).normalize();
                    assert!(world.ray(eye, p - d * PIECE_R).is_none(), "seed {seed}");
                }
            }
        }
    }

    #[test]
    fn sequences_never_repeat_a_lamp_in_a_row_and_show_each_step() {
        for seed in 0..100 {
            let s = sequence(seed);
            assert!(s.windows(2).all(|w| w[0] != w[1]));
            let shown: Vec<u8> = (0..500)
                .filter_map(|k| show(&s, k as f32 * 0.01))
                .collect::<Vec<_>>()
                .chunk_by(|a, b| a == b)
                .map(|c| c[0])
                .collect();
            assert_eq!(shown, s.to_vec(), "seed {seed}");
        }
    }

    #[test]
    fn relays_start_off_and_bolts_hit_them() {
        for seed in 0..100 {
            let s = relay_start(seed);
            assert!(beam_reach(&s) < RELAYS);
        }
        let level = Level::generate_for(1, ArenaSize::Medium, GameMode::Megatarget);
        let m = level.mega.as_ref().unwrap();
        let kinds = [SecondaryKind::Relay, SecondaryKind::Shoot];
        let r = m.relay_route[0][2];
        let from = r + Vec3::new(0.0, -1.0, 2.0);
        let hit = hit(m, kinds, from, r + (r - from)).unwrap();
        assert_eq!((hit.0, hit.1), (0, 1));
        assert!(super::hit(m, [SecondaryKind::Shoot; 2], from, r + (r - from)).is_none());
    }
}
