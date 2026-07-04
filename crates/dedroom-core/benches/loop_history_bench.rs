//! Benchmark comparing in-memory vs SQLite loop detection history backends.
//!
//! Measures `push` and `count_repeats` throughput at various window sizes.
//! Run with:
//!   cargo bench -p dedroom-core --features sqlite
//!   cargo bench -p dedroom-core                     (in-memory only)

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId, black_box};
use dedroom_core::config::CountMode;
use dedroom_core::loop_detection::HistoryBackend;

// ── Helpers ────────────────────────────────────────────────────────────────

/// Pre-populate an in-memory backend with `count` entries.
fn populate_in_memory(window: usize, count: usize) -> dedroom_core::loop_detection::HistoryTracker {
    let mut h = dedroom_core::loop_detection::HistoryTracker::new(window);
    for i in 0..count {
        let args = format!(r#"{{"path":"/tmp/bench_{i}.txt"}}"#);
        h.push("write_file".into(), args, i % 3 == 0);
    }
    h
}

/// Pre-populate a SQLite backend with `count` entries.
#[cfg(feature = "sqlite")]
fn populate_sqlite(
    window: usize,
    count: usize,
) -> dedroom_core::loop_detection::SqliteHistoryTracker {
    let mut h =
        dedroom_core::loop_detection::SqliteHistoryTracker::new_in_memory(window).unwrap();
    for i in 0..count {
        let args = format!(r#"{{"path":"/tmp/bench_{i}.txt"}}"#);
        h.push("write_file".into(), args, i % 3 == 0);
    }
    h
}

// ── Push benchmarks ────────────────────────────────────────────────────────

fn bench_push_in_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("push / in-memory");
    for window in [10usize, 50, 100, 500] {
        group.bench_with_input(
            BenchmarkId::from_parameter(window),
            &window,
            |b, &w| {
                let mut h = dedroom_core::loop_detection::HistoryTracker::new(w);
                b.iter(|| {
                    h.push(
                        black_box("write_file".into()),
                        black_box(r#"{"path":"/tmp/x.txt"}"#.into()),
                        black_box(false),
                    );
                });
            },
        );
    }
    group.finish();
}

fn bench_push_sqlite(c: &mut Criterion) {
    #[cfg(feature = "sqlite")]
    {
        let mut group = c.benchmark_group("push / sqlite (memory)");
        for window in [10usize, 50, 100, 500] {
            group.bench_with_input(
                BenchmarkId::from_parameter(window),
                &window,
                |b, &w| {
                    let mut h =
                        dedroom_core::loop_detection::SqliteHistoryTracker::new_in_memory(w)
                            .unwrap();
                    b.iter(|| {
                        h.push(
                            black_box("write_file".into()),
                            black_box(r#"{"path":"/tmp/x.txt"}"#.into()),
                            black_box(false),
                        );
                    });
                },
            );
        }
        group.finish();
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = c; // Silence unused warning
        eprintln!("Skipping SQLite benchmarks — enable with --features sqlite");
    }
}

// ── Count-repeats benchmarks ───────────────────────────────────────────────

fn bench_count_repeats_in_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("count_repeats / in-memory");
    for window in [10usize, 50, 100, 500] {
        group.bench_with_input(
            BenchmarkId::from_parameter(window),
            &window,
            |b, &w| {
                let h = populate_in_memory(w, w); // fill to capacity
                b.iter(|| {
                    black_box(h.count_repeats(
                        black_box("write_file"),
                        black_box(r#"{"path":"/tmp/bench_0.txt"}"#),
                        black_box(CountMode::All),
                    ));
                });
            },
        );
    }
    group.finish();
}

fn bench_count_repeats_sqlite(c: &mut Criterion) {
    #[cfg(feature = "sqlite")]
    {
        let mut group = c.benchmark_group("count_repeats / sqlite (memory)");
        for window in [10usize, 50, 100, 500] {
            group.bench_with_input(
                BenchmarkId::from_parameter(window),
                &window,
                |b, &w| {
                    let h = populate_sqlite(w, w); // fill to capacity
                    b.iter(|| {
                        black_box(h.count_repeats(
                            black_box("write_file"),
                            black_box(r#"{"path":"/tmp/bench_0.txt"}"#),
                            black_box(CountMode::All),
                        ));
                    });
                },
            );
        }
        group.finish();
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = c;
        eprintln!("Skipping SQLite benchmarks — enable with --features sqlite");
    }
}

// ── Mixed workload benchmarks ──────────────────────────────────────────────

fn bench_mixed_in_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed (50% push, 50% count) / in-memory");
    for window in [10usize, 50, 100, 500] {
        group.bench_with_input(
            BenchmarkId::from_parameter(window),
            &window,
            |b, &w| {
                let mut h = populate_in_memory(w, w);
                let mut step = 0u64;
                b.iter(|| {
                    if step % 2 == 0 {
                        h.push(
                            black_box("write_file".into()),
                            black_box(r#"{"path":"/tmp/x.txt"}"#.into()),
                            black_box(false),
                        );
                    } else {
                        black_box(h.count_repeats(
                            black_box("write_file"),
                            black_box(r#"{"path":"/tmp/x.txt"}"#),
                            black_box(CountMode::All),
                        ));
                    }
                    step += 1;
                });
            },
        );
    }
    group.finish();
}

fn bench_mixed_sqlite(c: &mut Criterion) {
    #[cfg(feature = "sqlite")]
    {
        let mut group = c.benchmark_group("mixed (50% push, 50% count) / sqlite (memory)");
        for window in [10usize, 50, 100, 500] {
            group.bench_with_input(
                BenchmarkId::from_parameter(window),
                &window,
                |b, &w| {
                    let mut h = populate_sqlite(w, w);
                    let mut step = 0u64;
                    b.iter(|| {
                        if step % 2 == 0 {
                            h.push(
                                black_box("write_file".into()),
                                black_box(r#"{"path":"/tmp/x.txt"}"#.into()),
                                black_box(false),
                            );
                        } else {
                            black_box(h.count_repeats(
                                black_box("write_file"),
                                black_box(r#"{"path":"/tmp/x.txt"}"#),
                                black_box(CountMode::All),
                            ));
                        }
                        step += 1;
                    });
                },
            );
        }
        group.finish();
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = c;
        eprintln!("Skipping SQLite benchmarks — enable with --features sqlite");
    }
}

criterion_group!(
    benches,
    bench_push_in_memory,
    bench_push_sqlite,
    bench_count_repeats_in_memory,
    bench_count_repeats_sqlite,
    bench_mixed_in_memory,
    bench_mixed_sqlite,
);
criterion_main!(benches);
