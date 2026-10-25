//! Seeded random number generation for deterministic replay.
//!
//! The algorithms are written out here, not taken from a crate, so the stream for a given
//! seed can never change under us: saved worlds and replays depend on it.

use std::num::NonZeroU64;

/// xoshiro256++ seeded through SplitMix64.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    state: [u64; 4],
}

/// One SplitMix64 step: advances `state` and returns the next output.
///
/// Also serves as a stable integer hash for code that needs a value per lattice point
/// instead of a stream (terrain noise).
pub(crate) fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn from_seed(seed: u64) -> Self {
        let mut sm = seed;
        Self {
            state: [
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
            ],
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = self.state;
        let result = s0.wrapping_add(s3).rotate_left(23).wrapping_add(s0);
        let t = s1 << 17;
        let s2 = s2 ^ s0;
        let s3 = s3 ^ s1;
        let s1 = s1 ^ s2;
        let s0 = s0 ^ s3;
        let s2 = s2 ^ t;
        self.state = [s0, s1, s2, s3.rotate_left(45)];
        result
    }

    /// A uniform value in `0..bound`, without modulo bias.
    pub fn below(&mut self, bound: NonZeroU64) -> u64 {
        let bound = bound.get();
        // Values below this would make some remainders more likely than others.
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next_u64();
            if value >= threshold {
                return value % bound;
            }
        }
    }

    /// An independent stream derived from this generator's current state and `key`.
    ///
    /// Does not advance `self`. Fork from a generator that is never drawn from (for example
    /// one per world seed), keyed by something stable like a chunk position, so a result
    /// never depends on the order things were generated in.
    pub fn fork(&self, key: u64) -> Self {
        let mut sm = self.state[0] ^ self.state[2].rotate_left(32);
        let mixed = splitmix64(&mut sm) ^ key;
        let mut sm = mixed;
        Self::from_seed(splitmix64(&mut sm) ^ self.state[1] ^ self.state[3])
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    // The proptest prelude also exports a trait named `Rng`; name ours explicitly.
    use super::Rng;

    fn bound(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).unwrap()
    }

    #[test]
    fn splitmix64_matches_the_reference_output() {
        let mut state = 0;
        assert_eq!(splitmix64(&mut state), 0xe220_a839_7b1d_cdaf);
        assert_eq!(splitmix64(&mut state), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(splitmix64(&mut state), 0x06c4_5d18_8009_454f);
    }

    // Vectors from an independent reference implementation. If this fails, the stream
    // changed and every saved world and replay is invalidated.
    #[test]
    fn stream_is_pinned_for_known_seeds() {
        let mut rng = Rng::from_seed(0);
        assert_eq!(rng.next_u64(), 0x5317_5d61_490b_23df);
        assert_eq!(rng.next_u64(), 0x61da_6f3d_c380_d507);
        assert_eq!(rng.next_u64(), 0x5c0f_df91_ec9a_7bfc);
        assert_eq!(rng.next_u64(), 0x02ee_bf8c_3bbe_5e1a);

        let mut rng = Rng::from_seed(42);
        assert_eq!(rng.next_u64(), 0xd076_4d4f_4476_689f);
        assert_eq!(rng.next_u64(), 0x519e_4174_576f_3791);
        assert_eq!(rng.next_u64(), 0xfbe0_7cfb_0c24_ed8c);
        assert_eq!(rng.next_u64(), 0xb37d_9f60_0cd8_35b8);
    }

    #[test]
    fn below_one_is_always_zero() {
        let mut rng = Rng::from_seed(7);
        assert!((0..100).all(|_| rng.below(bound(1)) == 0));
    }

    #[test]
    fn below_reaches_every_value_in_a_small_range() {
        let mut rng = Rng::from_seed(7);
        let mut seen = [false; 6];
        for _ in 0..600 {
            seen[rng.below(bound(6)) as usize] = true;
        }
        assert!(seen.iter().all(|hit| *hit));
    }

    proptest! {
        #[test]
        fn same_seed_gives_the_same_sequence(seed in any::<u64>()) {
            let mut a = Rng::from_seed(seed);
            let mut b = Rng::from_seed(seed);
            prop_assert!((0..50).all(|_| a.next_u64() == b.next_u64()));
        }

        #[test]
        fn different_seeds_give_different_sequences(a in any::<u64>(), b in any::<u64>()) {
            prop_assume!(a != b);
            let mut ra = Rng::from_seed(a);
            let mut rb = Rng::from_seed(b);
            prop_assert!((0..4).any(|_| ra.next_u64() != rb.next_u64()));
        }

        #[test]
        fn below_stays_under_the_bound(seed in any::<u64>(), n in 1..=u64::MAX) {
            let mut rng = Rng::from_seed(seed);
            prop_assert!((0..20).all(|_| rng.below(bound(n)) < n));
        }

        #[test]
        fn fork_is_repeatable_and_leaves_the_parent_untouched(
            seed in any::<u64>(), key in any::<u64>(),
        ) {
            let parent = Rng::from_seed(seed);
            let before = parent.clone();
            let mut first = parent.fork(key);
            let mut second = parent.fork(key);
            prop_assert_eq!(&parent, &before);
            prop_assert!((0..20).all(|_| first.next_u64() == second.next_u64()));
        }

        #[test]
        fn forks_with_different_keys_differ(seed in any::<u64>(), a in any::<u64>(), b in any::<u64>()) {
            prop_assume!(a != b);
            let parent = Rng::from_seed(seed);
            let mut fa = parent.fork(a);
            let mut fb = parent.fork(b);
            prop_assert!((0..4).any(|_| fa.next_u64() != fb.next_u64()));
        }
    }
}
