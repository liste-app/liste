//! IPC round trip to a running host (Section 4: well under 1 ms).
//! Threshold (p95): 1 ms each for capture and for search.

use std::time::{Duration, Instant};

use liste_core::host::{Host, HostConfig};
use liste_ipc::{Client, Endpoint};

struct Stats(Vec<Duration>);

impl Stats {
    fn time<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = f();
        self.0.push(start.elapsed());
        out
    }

    fn pct(&self, p: f64) -> Duration {
        let mut s = self.0.clone();
        s.sort();
        s[((s.len() as f64 - 1.0) * p).round() as usize]
    }

    fn check(&self, name: &str, limit: Duration, failures: &mut Vec<String>) {
        let p95 = self.pct(0.95);
        println!(
            "{name:<40} n={:<6} p50={:>9.1}µs p95={:>9.1}µs p99={:>9.1}µs max={:>9.1}µs",
            self.0.len(),
            self.pct(0.5).as_secs_f64() * 1e6,
            p95.as_secs_f64() * 1e6,
            self.pct(0.99).as_secs_f64() * 1e6,
            self.pct(1.0).as_secs_f64() * 1e6,
        );
        if p95 > limit {
            failures.push(format!(
                "{name}: p95 {:.1}µs exceeds {:.1}µs",
                p95.as_secs_f64() * 1e6,
                limit.as_secs_f64() * 1e6
            ));
        }
    }
}

fn main() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("liste-bench-ipc-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    let endpoint = Endpoint::from_path(&dir.join("host.sock"));
    let host = Host::start(HostConfig::new(&dir, endpoint.clone())).unwrap();
    let mut client = Client::connect(&endpoint).unwrap();
    // Some data so search has something to match.
    for i in 0..500 {
        client
            .capture(
                &format!("call mom about item {i} tomorrow 5pm #family"),
                None,
            )
            .unwrap();
    }

    let mut failures = Vec::new();
    let mut capture = Stats(Vec::new());
    for i in 0..1_000 {
        capture.time(|| {
            client
                .capture(&format!("renew passport {i} jan 5 !high"), None)
                .unwrap()
        });
    }
    capture.check(
        "ipc capture round trip",
        Duration::from_millis(1),
        &mut failures,
    );

    let mut search = Stats(Vec::new());
    for _ in 0..1_000 {
        search.time(|| client.search("call mom", 20).unwrap());
    }
    search.check(
        "ipc search round trip",
        Duration::from_millis(1),
        &mut failures,
    );

    let mut status = Stats(Vec::new());
    for _ in 0..1_000 {
        status.time(|| client.status().unwrap());
    }
    status.check(
        "ipc status round trip",
        Duration::from_millis(1),
        &mut failures,
    );

    host.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
    if failures.is_empty() {
        println!("bench: all thresholds met");
    } else {
        for f in &failures {
            println!("bench: FAIL {f}");
        }
        if cfg!(debug_assertions) {
            println!("bench: debug build, thresholds not enforced");
        } else {
            std::process::exit(1);
        }
    }
}
