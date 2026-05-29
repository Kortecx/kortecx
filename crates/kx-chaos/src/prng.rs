//! A tiny, dependency-free deterministic PRNG.
//!
//! The harness needs *reproducibility*, not cryptographic quality or perfect
//! uniformity: a recorded seed must replay the exact same plan. `SplitMix64`
//! (Steele/Lea/Flood, the seeding RNG behind `java.util.SplitableRandom`) is a
//! well-known ~10-line generator that gives that with zero hidden global state and
//! no external crate (so nothing new passes through `cargo-deny`). It is consumed
//! entirely while building a [`crate::ChaosPlan`]; nothing else in the harness draws
//! randomness, so the cluster driver is a pure function of the plan.

/// A deterministic `SplitMix64` stream. Construct from a seed; pull values with
/// [`SplitMix64::next_u64`] / [`SplitMix64::below`] / [`SplitMix64::choose`].
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Start a fresh stream from `seed`. Equal seeds yield identical streams.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64-bit value in the stream.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n`. For the small `n` the harness uses (≤ a few dozen) the
    /// modulo bias is negligible *and deterministic* — reproducibility, not
    /// uniformity, is the contract here. Returns `0` if `n == 0`.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        self.next_u64() % n
    }

    /// Pick one element of `xs` deterministically. Returns `None` only for an empty
    /// slice (the caller passes a non-empty fixed table).
    pub fn choose<'a, T>(&mut self, xs: &'a [T]) -> Option<&'a T> {
        let len = u64::try_from(xs.len()).ok()?;
        if len == 0 {
            return None;
        }
        let i = usize::try_from(self.below(len)).ok()?;
        xs.get(i)
    }
}

#[cfg(test)]
mod tests {
    use super::SplitMix64;

    #[test]
    fn same_seed_same_stream() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..1_000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn distinct_seeds_diverge() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(2);
        // Not a statistical claim — just that the streams are not trivially equal.
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn below_is_in_range() {
        let mut r = SplitMix64::new(7);
        for _ in 0..10_000 {
            assert!(r.below(5) < 5);
        }
        assert_eq!(r.below(0), 0);
    }

    #[test]
    fn choose_picks_from_slice() {
        let mut r = SplitMix64::new(9);
        let xs = [10, 20, 30];
        for _ in 0..100 {
            assert!(xs.contains(r.choose(&xs).unwrap()));
        }
        let empty: [u8; 0] = [];
        assert!(r.choose(&empty).is_none());
    }
}
