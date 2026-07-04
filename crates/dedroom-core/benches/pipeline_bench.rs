//! Benchmark comparing full `Pipeline::process_tool_call()` throughput with
//! in-memory vs SQLite backends (CCR + loop history).
//!
//! Run with:
//!   cargo bench -p dedroom-core --features sqlite --bench pipeline_bench
//!   cargo bench -p dedroom-core --bench pipeline_bench        (in-memory only)

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId, black_box};
use dedroom_core::config::DedrooMConfig;
use dedroom_core::pipeline::{Pipeline, ToolCall};

// ── Helpers ────────────────────────────────────────────────────────────────

/// Build an in-memory Pipeline.
fn make_pipeline_in_memory() -> Pipeline {
    Pipeline::new(DedrooMConfig::default())
}

/// Build a SQLite-backed Pipeline (uses `:memory:` databases).
#[cfg(feature = "sqlite")]
fn make_pipeline_sqlite() -> Pipeline {
    let yaml = r#"
        loop_detection:
          max_repeats: 10
          history_backend: sqlite
          history_path: ":memory:"
        compression:
          ccr:
            backend: sqlite
            path: ":memory:"
            ttl_seconds: 3600
    "#;
    let config = DedrooMConfig::from_yaml_str(yaml).unwrap();
    Pipeline::new(config)
}

/// Build a Pipeline backed by on-disk SQLite databases in a temp directory.
#[cfg(feature = "sqlite")]
fn make_pipeline_ondisk(db_dir: &std::path::Path) -> Pipeline {
    let hist_path = db_dir.join("loop_history.db");
    let ccr_path = db_dir.join("ccr.db");
    let yaml = format!(
        r#"
            loop_detection:
              max_repeats: 10
              history_backend: sqlite
              history_path: "{hist}"
            compression:
              ccr:
                backend: sqlite
                path: "{ccr}"
                ttl_seconds: 3600
        "#,
        hist = hist_path.display(),
        ccr = ccr_path.display(),
    );
    let config = DedrooMConfig::from_yaml_str(&yaml).unwrap();
    Pipeline::new(config)
}

/// A tool call with a small result that triggers compression and CCR writes.
fn tool_call_with_result() -> ToolCall {
    ToolCall {
        name: "read_file".into(),
        args: r#"{"path":"/tmp/bench.txt"}"#.into(),
        result: Some(
            "line 1: hello world\nline 2: foo bar\nline 3: baz qux\n\
             line 4: hello again\nline 5: final line"
                .into(),
        ),
        is_error: false,
    }
}

/// A tool call with no result — only exercises loop detection path.
fn tool_call_no_result() -> ToolCall {
    ToolCall {
        name: "read_file".into(),
        args: r#"{"path":"/tmp/bench.txt"}"#.into(),
        result: None,
        is_error: false,
    }
}

// ── Pipeline: allow (no result) ────────────────────────────────────────────

fn bench_pipeline_allow_in_memory(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("process_tool_call (allow, no result) / in-memory");
    group.sample_size(100);

    group.bench_function(BenchmarkId::from_parameter("pipeline"), |b| {
        b.iter(|| {
            let mut pipeline = make_pipeline_in_memory();
            let result = rt.block_on(pipeline.process_tool_call(&tool_call_no_result()));
            black_box(result);
        });
    });

    group.finish();
}

fn bench_pipeline_allow_sqlite(c: &mut Criterion) {
    #[cfg(feature = "sqlite")]
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut group = c.benchmark_group("process_tool_call (allow, no result) / sqlite");
        group.sample_size(100);

        group.bench_function(BenchmarkId::from_parameter("pipeline"), |b| {
            b.iter(|| {
                let mut pipeline = make_pipeline_sqlite();
                let result = rt.block_on(pipeline.process_tool_call(&tool_call_no_result()));
                black_box(result);
            });
        });

        group.finish();
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = c;
        eprintln!("Skipping SQLite benchmarks — enable with --features sqlite");
    }
}

// ── Pipeline: allow + compress (with result → CCR write) ───────────────────

fn bench_pipeline_compress_in_memory(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("process_tool_call (compress + CCR) / in-memory");
    group.sample_size(100);

    group.bench_function(BenchmarkId::from_parameter("pipeline"), |b| {
        b.iter(|| {
            let mut pipeline = make_pipeline_in_memory();
            let result = rt.block_on(pipeline.process_tool_call(&tool_call_with_result()));
            black_box(result);
        });
    });

    group.finish();
}

fn bench_pipeline_compress_sqlite(c: &mut Criterion) {
    #[cfg(feature = "sqlite")]
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut group = c.benchmark_group("process_tool_call (compress + CCR) / sqlite");
        group.sample_size(100);

        group.bench_function(BenchmarkId::from_parameter("pipeline"), |b| {
            b.iter(|| {
                let mut pipeline = make_pipeline_sqlite();
                let result = rt.block_on(pipeline.process_tool_call(&tool_call_with_result()));
                black_box(result);
            });
        });

        group.finish();
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = c;
        eprintln!("Skipping SQLite benchmarks — enable with --features sqlite");
    }
}

// ── Pipeline: warm (history pre-populated) — shows cost of larger history ───

fn bench_pipeline_warm_in_memory(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("process_tool_call (warm, default config) / in-memory");
    group.sample_size(100);

    // Pre-populate once outside the measured loop
    let mut pipeline = make_pipeline_in_memory();
    for i in 0..50 {
        let t = ToolCall {
            name: "search".into(),
            args: format!(r#"{{"query":"warmup_{i}"}}"#),
            result: Some("result".into()),
            is_error: false,
        };
        rt.block_on(pipeline.process_tool_call(&t));
    }

    group.bench_function("warm pipeline", |b| {
        b.iter(|| {
            let result = rt.block_on(pipeline.process_tool_call(&tool_call_no_result()));
            black_box(result);
        });
    });

    group.finish();
}

fn bench_pipeline_warm_sqlite(c: &mut Criterion) {
    #[cfg(feature = "sqlite")]
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut group = c.benchmark_group("process_tool_call (warm, default config) / sqlite");
        group.sample_size(100);

        // Pre-populate once outside the measured loop
        let mut pipeline = make_pipeline_sqlite();
        for i in 0..50 {
            let t = ToolCall {
                name: "search".into(),
                args: format!(r#"{{"query":"warmup_{i}"}}"#),
                result: Some("result".into()),
                is_error: false,
            };
            rt.block_on(pipeline.process_tool_call(&t));
        }

        group.bench_function("warm pipeline", |b| {
            b.iter(|| {
                let result = rt.block_on(pipeline.process_tool_call(&tool_call_no_result()));
                black_box(result);
            });
        });

        group.finish();
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = c;
        eprintln!("Skipping SQLite benchmarks — enable with --features sqlite");
    }
}

// ── Pipeline: allow (no result) — on-disk SQLite ─────────────────────────────

fn bench_pipeline_allow_ondisk(c: &mut Criterion) {
    #[cfg(feature = "sqlite")]
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut group = c.benchmark_group("process_tool_call (allow, no result) / on-disk sqlite");
        group.sample_size(100);

        group.bench_function("pipeline", |b| {
            b.iter(|| {
                let mut pipeline = make_pipeline_ondisk(dir.path());
                let result = rt.block_on(pipeline.process_tool_call(&tool_call_no_result()));
                black_box(result);
            });
        });

        group.finish();
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = c;
        eprintln!("Skipping on-disk SQLite benchmarks — enable with --features sqlite");
    }
}

// ── Pipeline: compress + CCR — on-disk SQLite ────────────────────────────────

fn bench_pipeline_compress_ondisk(c: &mut Criterion) {
    #[cfg(feature = "sqlite")]
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut group = c.benchmark_group("process_tool_call (compress + CCR) / on-disk sqlite");
        group.sample_size(100);

        group.bench_function("pipeline", |b| {
            b.iter(|| {
                let mut pipeline = make_pipeline_ondisk(dir.path());
                let result = rt.block_on(pipeline.process_tool_call(&tool_call_with_result()));
                black_box(result);
            });
        });

        group.finish();
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = c;
        eprintln!("Skipping on-disk SQLite benchmarks — enable with --features sqlite");
    }
}

// ── Pipeline: warm — on-disk SQLite ──────────────────────────────────────────

fn bench_pipeline_warm_ondisk(c: &mut Criterion) {
    #[cfg(feature = "sqlite")]
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut group = c.benchmark_group("process_tool_call (warm, default config) / on-disk sqlite");
        group.sample_size(100);

        // Pre-populate once outside the measured loop
        let mut pipeline = make_pipeline_ondisk(dir.path());
        for i in 0..50 {
            let t = ToolCall {
                name: "search".into(),
                args: format!(r#"{{"query":"warmup_{i}"}}"#),
                result: Some("result".into()),
                is_error: false,
            };
            rt.block_on(pipeline.process_tool_call(&t));
        }

        group.bench_function("warm pipeline", |b| {
            b.iter(|| {
                let result = rt.block_on(pipeline.process_tool_call(&tool_call_no_result()));
                black_box(result);
            });
        });

        group.finish();
    }

    #[cfg(not(feature = "sqlite"))]
    {
        let _ = c;
        eprintln!("Skipping on-disk SQLite benchmarks — enable with --features sqlite");
    }
}

criterion_group!(
    benches,
    bench_pipeline_allow_in_memory,
    bench_pipeline_allow_sqlite,
    bench_pipeline_allow_ondisk,
    bench_pipeline_compress_in_memory,
    bench_pipeline_compress_sqlite,
    bench_pipeline_compress_ondisk,
    bench_pipeline_warm_in_memory,
    bench_pipeline_warm_sqlite,
    bench_pipeline_warm_ondisk,
);
criterion_main!(benches);
