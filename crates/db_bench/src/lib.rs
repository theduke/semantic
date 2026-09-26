//! Benchmarks of the embedded Semantic database.
//!
//! The library holds the schema, the deterministic data generator, the
//! seeded [`Fixture`]s and the workloads; `benches/db.rs` drives them with
//! criterion. See `docs/benchmarks.md` for how to run and compare them.

pub mod data;
mod fixture;
pub mod schema;

pub use fixture::{
    CONCURRENT_WRITE_BATCH, Engine, Fixture, PAGE, ReadWorkload, TX_STATEMENTS, WritePath,
};

/// Collection sizes benchmarked by default.
pub const DEFAULT_SIZES: [u64; 2] = [10_000, 100_000];
/// Size added by `SEMANTIC_BENCH_LARGE=1`.
pub const LARGE_SIZE: u64 = 1_000_000;

/// Benchmark selection from environment variables:
///
/// * `SEMANTIC_BENCH_SIZES`: comma-separated collection sizes
///   (default `10000,100000`).
/// * `SEMANTIC_BENCH_LARGE=1`: also run 1M rows.
/// * `SEMANTIC_BENCH_ENGINES`: comma-separated engines, `memory` and/or
///   `redb` (default both).
///
/// Filtering here (rather than with criterion's name filter) avoids seeding
/// databases whose benchmarks would all be skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchSelection {
    pub sizes: Vec<u64>,
    pub engines: Vec<Engine>,
}

impl BenchSelection {
    pub fn from_env() -> Result<Self, anyhow::Error> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let large = var("SEMANTIC_BENCH_LARGE").is_some_and(|v| v.trim() != "0");
        Self::parse(
            var("SEMANTIC_BENCH_SIZES").as_deref(),
            large,
            var("SEMANTIC_BENCH_ENGINES").as_deref(),
        )
    }

    fn parse(
        sizes: Option<&str>,
        large: bool,
        engines: Option<&str>,
    ) -> Result<Self, anyhow::Error> {
        let mut sizes = match sizes {
            Some(list) => list
                .split(',')
                .map(|size| {
                    let size = size.trim().replace('_', "");
                    match size.parse::<u64>() {
                        Ok(size) if size >= 100 => Ok(size),
                        _ => Err(anyhow::anyhow!("invalid benchmark size {size:?} (min 100)")),
                    }
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => DEFAULT_SIZES.to_vec(),
        };
        if large && !sizes.contains(&LARGE_SIZE) {
            sizes.push(LARGE_SIZE);
        }
        let engines = match engines {
            Some(list) => list
                .split(',')
                .map(|name| {
                    Engine::from_name(name)
                        .ok_or_else(|| anyhow::anyhow!("unknown benchmark engine {name:?}"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Engine::ALL.to_vec(),
        };
        Ok(Self { sizes, engines })
    }
}

#[cfg(test)]
mod tests;
