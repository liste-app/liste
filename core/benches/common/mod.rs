//! Shared bench plumbing: a file-backed store with the large fixture, and
//! percentile reporting. Numbers are reported, not yet enforced; thresholds
//! come once real numbers exist (Section 15).

#![allow(dead_code)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use liste_core::ids::Id;
use liste_core::store::{Store, fixture};

/// The Section 4 large-DB fixture size.
pub const FIXTURE_TASKS: usize = 50_000;

pub fn fixture_tasks() -> usize {
    std::env::var("LISTE_BENCH_TASKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(FIXTURE_TASKS)
}

pub fn temp_db(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("liste-bench-{label}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("liste.db")
}

/// Open a file-backed store and fill it with the fixture, reporting how
/// long that took.
pub fn populated(label: &str) -> (Store, fixture::Fixture) {
    let path = temp_db(label);
    let mut store = Store::open(&path, Id::new()).unwrap();
    let space = Id::new();
    let n = fixture_tasks();
    let start = Instant::now();
    let fx = fixture::populate(&mut store, space, n, 42).unwrap();
    let took = start.elapsed();
    println!(
        "fixture: {n} tasks, {} ops in {:.2}s ({:.1} µs/op), db {}",
        fx.ops,
        took.as_secs_f64(),
        took.as_secs_f64() * 1e6 / fx.ops as f64,
        path.display()
    );
    (store, fx)
}

pub struct Stats {
    pub samples: Vec<Duration>,
}

impl Stats {
    pub fn new() -> Stats {
        Stats {
            samples: Vec::new(),
        }
    }

    pub fn time<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = f();
        self.samples.push(start.elapsed());
        out
    }

    fn pct(&self, p: f64) -> Duration {
        let mut s = self.samples.clone();
        s.sort();
        let idx = ((s.len() as f64 - 1.0) * p).round() as usize;
        s[idx]
    }

    pub fn report(&self, name: &str) {
        let mean = self.samples.iter().sum::<Duration>() / self.samples.len() as u32;
        println!(
            "{name:<40} n={:<6} p50={:>9.1}µs p95={:>9.1}µs p99={:>9.1}µs max={:>9.1}µs mean={:>9.1}µs",
            self.samples.len(),
            self.pct(0.50).as_secs_f64() * 1e6,
            self.pct(0.95).as_secs_f64() * 1e6,
            self.pct(0.99).as_secs_f64() * 1e6,
            self.pct(1.0).as_secs_f64() * 1e6,
            mean.as_secs_f64() * 1e6,
        );
    }
}
