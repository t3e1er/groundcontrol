//! Deterministic pseudo-random query sampler for benchmark evaluation.

/// Pure-Rust deterministic sampler implementing XorShift64* and Fisher-Yates shuffle.
///
/// Guarantees identical sample slices run-to-run for a fixed seed, with zero external
/// dependencies and reproducible sampling across all operating systems.
#[derive(Debug, Clone)]
pub struct DeterministicSampler {
    state: u64,
}

impl DeterministicSampler {
    /// Create a new sampler with a specific 64-bit seed.
    ///
    /// If seed is 0, a non-zero default constant is used since XorShift requires non-zero state.
    pub fn new(seed: u64) -> Self {
        let state = if seed == 0 { 0x853c49e6748fea9b } else { seed };
        Self { state }
    }

    /// Generate the next pseudo-random 64-bit unsigned integer using XorShift64*.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Sample `count` unique indices from `total` available items using Fisher-Yates partial shuffle.
    ///
    /// Returned indices are sorted in ascending order to preserve original dataset ordering.
    pub fn sample_indices(&mut self, total: usize, count: usize) -> Vec<usize> {
        if count >= total || total == 0 {
            return (0..total).collect();
        }

        let mut indices: Vec<usize> = (0..total).collect();
        for i in 0..count {
            let remaining = total - i;
            let j = i + ((self.next_u64() as usize) % remaining);
            indices.swap(i, j);
        }

        indices.truncate(count);
        indices.sort_unstable();
        indices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_reproducibility() {
        let mut sampler1 = DeterministicSampler::new(42);
        let mut sampler2 = DeterministicSampler::new(42);

        let slice1 = sampler1.sample_indices(100, 10);
        let slice2 = sampler2.sample_indices(100, 10);

        assert_eq!(slice1, slice2);
        assert_eq!(slice1.len(), 10);
    }

    #[test]
    fn test_different_seeds_produce_different_slices() {
        let mut sampler1 = DeterministicSampler::new(42);
        let mut sampler2 = DeterministicSampler::new(999);

        let slice1 = sampler1.sample_indices(100, 10);
        let slice2 = sampler2.sample_indices(100, 10);

        assert_ne!(slice1, slice2);
    }

    #[test]
    fn test_count_greater_than_total() {
        let mut sampler = DeterministicSampler::new(42);
        let slice = sampler.sample_indices(5, 10);
        assert_eq!(slice, vec![0, 1, 2, 3, 4]);
    }
}
