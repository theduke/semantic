//! Seeded randomness for the randomised backend tests.
//!
//! Seeds come from `SEMANTIC_TEST_SEED` (a comma-separated list of `u64`s)
//! or from each test's defaults; [`SeedGuard`] prints the seed of a failing
//! run so it can be reproduced.

/// Deterministic SplitMix64 generator.
#[derive(Debug, Clone)]
pub struct TestRng(u64);

impl TestRng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..bound` (`bound > 0`).
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    /// Uniform index into a collection of `len > 0` items.
    pub fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    /// `true` with probability `1 / n`.
    pub fn one_in(&mut self, n: u64) -> bool {
        self.below(n) == 0
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.index(items.len())]
    }

    /// A generator for a sub-task, independent of later draws of `self`.
    pub fn fork(&mut self) -> Self {
        Self(self.next_u64())
    }
}

/// Seeds to run: `SEMANTIC_TEST_SEED` (comma-separated) when set, else
/// `defaults`.
pub fn seeds_from_env(defaults: &[u64]) -> Vec<u64> {
    match std::env::var("SEMANTIC_TEST_SEED") {
        Ok(value) => value
            .split(',')
            .map(|seed| {
                seed.trim()
                    .parse()
                    .unwrap_or_else(|_| panic!("SEMANTIC_TEST_SEED: invalid seed {seed:?}"))
            })
            .collect(),
        Err(_) => defaults.to_vec(),
    }
}

/// A `usize` from the environment variable `name`, else `default`.
pub fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .map(|value| {
            value
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("{name}: invalid number {value:?}"))
        })
        .unwrap_or(default)
}

/// Prints the seed of the enclosing test run when it panics.
pub struct SeedGuard {
    test: &'static str,
    seed: u64,
}

impl SeedGuard {
    pub fn new(test: &'static str, seed: u64) -> Self {
        Self { test, seed }
    }
}

impl Drop for SeedGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "{} failed with seed {}; reproduce with SEMANTIC_TEST_SEED={}",
                self.test, self.seed, self.seed
            );
        }
    }
}
