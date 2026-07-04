/// DedrooM Performance Benchmarks
///
/// Measures latency and throughput for loop detection, all compressors,
/// content routing, and the full pipeline.
///
/// Run: cargo run --example bench
///
/// Results are printed as a markdown table for easy reading.

use std::time::Instant;

use dedroom_core::config::{
    ContentRouterConfig, DedrooMConfig, LoopDetectionConfig,
};
use dedroom_core::compression::{
    ContentRouter, compress_json_array, compress_code, compress_logs, compress_text, estimate_tokens,
};
use dedroom_core::loop_detection::LoopDetector;
use dedroom_core::pipeline::{Pipeline, ToolCall};

// ── Helpers ────────────────────────────────────────────────────────────────

fn elapsed_ns(start: Instant) -> f64 {
    start.elapsed().as_nanos() as f64
}

fn elapsed_us(start: Instant) -> f64 {
    start.elapsed().as_nanos() as f64 / 1_000.0
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_nanos() as f64 / 1_000_000.0
}

fn warmup_cpu() {
    let mut x = 0u64;
    for i in 0..1_000_000 {
        x ^= (i as u64).wrapping_mul(13);
    }
    std::hint::black_box(x);
}

fn bench_mean_ns<F>(iterations: usize, label: &str, mut f: F) -> f64
where
    F: FnMut(),
{
    // First, run a single warmup iteration
    f();
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let total_ns = elapsed_ns(start);
    let avg_ns = total_ns / iterations as f64;
    let total_ms = total_ns / 1_000_000.0;
    println!("  {:<30} {:>10} iters   {:>8.0} ns/op  ({:.2} µs)  [{:.0}ms total]", label, iterations, avg_ns, avg_ns / 1000.0, total_ms);
    avg_ns
}

fn bench_mean_by<F>(iterations: usize, label: &str, mut f: F) -> f64
where
    F: FnMut() -> u8,
{
    let start = Instant::now();
    let mut sum = 0u64;
    for _ in 0..iterations {
        sum += f() as u64;
    }
    let total_ns = elapsed_ns(start);
    let avg_ns = total_ns / iterations as f64;
    println!("  {:<30} {:>10} iters   {:>8.0} ns/op  ({:.2} µs)", label, iterations, avg_ns, avg_ns / 1000.0);
    std::hint::black_box(sum);
    avg_ns
}

fn print_header(title: &str) {
    println!("\n{}", "─".repeat(70));
    println!("  {}", title);
    println!("{}", "─".repeat(70));
}

// ── Benchmark data generators ─────────────────────────────────────────────

fn gen_large_json_array(size: usize) -> String {
    let rows: Vec<String> = (0..size)
        .map(|i| {
            format!(
                r#"{{"id":{},"name":"User_{}","email":"user{}@example.com","role":"{}","score":{},"active":{}}}"#,
                i,
                i,
                i,
                ["admin", "user", "moderator", "editor"][i % 4],
                (i * 7) % 100,
                i % 2 == 0,
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
}

fn gen_code(size: usize) -> String {
    let mut lines = Vec::new();
    lines.push("use std::collections::HashMap;".to_string());
    lines.push("use std::sync::Arc;".to_string());
    lines.push(String::new());
    lines.push("#[derive(Debug, Clone)]".to_string());
    lines.push("pub struct Data {".to_string());
    lines.push("    pub id: u64,".to_string());
    lines.push("    pub name: String,".to_string());
    lines.push("    pub values: Vec<f64>,".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    for i in 0..size {
        lines.push(format!(
            r#"pub fn process_{}(input: &[Data]) -> Result<Vec<f64>, String> {{"#,
            i
        ));
        lines.push("    if input.is_empty() {".to_string());
        lines.push(r#"        return Err("empty input".to_string());"#.to_string());
        lines.push("    }".to_string());
        lines.push("    let mut results = Vec::with_capacity(input.len());".to_string());
        lines.push("    for item in input {".to_string());
        lines.push(format!(
            "        let avg = item.values.iter().sum::<f64>() / item.values.len() as f64;"
        ));
        lines.push("        results.push(avg);".to_string());
        lines.push("    }".to_string());
        lines.push("    Ok(results)".to_string());
        lines.push("}".to_string());
        lines.push(String::new());
    }
    lines.join("\n")
}

fn gen_logs_with_duplicates(size: usize, unique_ratio: f64) -> String {
    let unique_count = (size as f64 * unique_ratio) as usize;
    let unique_lines: Vec<String> = (0..unique_count)
        .map(|i| {
            format!(
                "[INFO] 2026-01-{:02}T{:02}:{:02}:{:02}Z service={} event=processing_{} status=ok",
                (i % 28) + 1,
                (i % 24),
                (i % 60),
                (i % 60),
                ["api", "worker", "scheduler", "db"][i % 4],
                i,
            )
        })
        .collect();

    let mut lines = Vec::new();
    for i in 0..size {
        if i % 10 == 0 {
            lines.push(format!(
                "[ERROR] 2026-01-{:02}T{:02}:{:02}:{:02}Z service={} event=critical_failure error_code=ERR_{}",
                (i % 28) + 1,
                (i % 24),
                (i % 60),
                (i % 60),
                ["api", "worker", "scheduler", "db"][i % 4],
                i,
            ));
        } else {
            lines.push(unique_lines[i % unique_count].clone());
        }
    }
    lines.join("\n")
}

fn gen_text_paragraphs(count: usize) -> String {
    let paragraphs: Vec<String> = (0..count)
        .map(|i| {
            format!(
                "This is paragraph {} of the sample text. It contains several sentences that describe \
                 various aspects of a software system. The system processes data efficiently and handles \
                 errors gracefully. Multiple components work together to provide a seamless experience. \
                 \n\n---\n\n",
                i
            )
        })
        .collect();
    paragraphs.join("\n")
}

// ── 1. LOOP DETECTION BENCHMARKS ─────────────────────────────────────────

fn bench_loop_detection() {
    print_header("1. Loop Detection Benchmarks");

    let config = LoopDetectionConfig::default();

    // 1a. Cold start latency
    {
        let mut detector = LoopDetector::new(&config);
        let tool = "write_file";
        let args = r#"{"path":"/tmp/test.txt"}"#;

        bench_mean_ns(100_000, "Cold start (empty history)", || {
            let _ = std::hint::black_box(detector.verify(tool, args));
        });
    }

    // 1b. Warm latency (with populated history)
    {
        let mut detector = LoopDetector::new(&config);
        for i in 0..10 {
            detector.record_result(
                "write_file",
                &format!(r#"{{"path":"/tmp/test_{}.txt"}}"#, i),
                false,
            );
        }
        let args = r#"{"path":"/tmp/test_10.txt"}"#;

        bench_mean_ns(100_000, "Warm (10 calls in history)", || {
            let _ = std::hint::black_box(detector.verify("write_file", args));
        });

        bench_mean_ns(100_000, "Warm — different args", || {
            let _ = std::hint::black_box(detector.verify("read_file", r#"{"path":"/tmp/other.txt"}"#));
        });
    }

    // 1c. Throughput with varied payloads
    {
        let mut detector = LoopDetector::new(&config);
        bench_mean_ns(100_000, "Simple args (few fields)", || {
            let v = detector.verify("write_file", r#"{"path":"/tmp/x.txt"}"#);
            detector.record_result("write_file", r#"{"path":"/tmp/x.txt"}"#, false);
            std::hint::black_box(v);
        });
    }

    {
        let mut detector = LoopDetector::new(&config);
        bench_mean_ns(50_000, "Complex args (many fields)", || {
            let args = r#"{"path":"/tmp/x.txt","content":"hello world","mode":"write","permissions":644,"owner":"root","group":"staff"}"#;
            let v = detector.verify("write_file", args);
            detector.record_result("write_file", args, false);
            std::hint::black_box(v);
        });
    }

    // 1d. Loop detection blocking speed (adversarial)
    {
        let mut detector = LoopDetector::new(&config);
        let tool = "write_file";
        let args = r#"{"path":"/tmp/test.txt"}"#;
        for _ in 0..3 {
            detector.verify(tool, args);
            detector.record_result(tool, args, true);
        }

        // The 4th call should block (fast path)
        bench_mean_ns(100_000, "Block detection (adversarial)", || {
            let v = std::hint::black_box(detector.verify(tool, args));
            std::hint::black_box(v.is_blocked());
        });
    }

    // 1e. Volatile field stripping speed
    {
        let mut detector = LoopDetector::new(&LoopDetectionConfig {
            volatile_fields: dedroom_core::config::VolatileFieldConfig {
                auto_inference: true,
                min_occurrences: 2,
                configured: vec![
                    dedroom_core::config::ConfiguredVolatileField {
                        tool: "search".into(),
                        fields: vec!["request_id".into(), "timestamp".into()],
                    },
                ],
            },
            ..Default::default()
        });

        bench_mean_ns(50_000, "With volatile field stripping", || {
            let args = r#"{"query":"hello world","request_id":"abc123","timestamp":"2026-01-01T00:00:00Z"}"#;
            let v = detector.verify("search", args);
            detector.record_result("search", args, false);
            std::hint::black_box(v);
        });
    }

    // 1f. Rule engine speed
    {
        let config = LoopDetectionConfig {
            rules: vec![
                dedroom_core::config::RuleConfig {
                    tool: "*".into(),
                    kind: dedroom_core::config::RuleKind::Exact {
                        value: r#"{"command":"rm -rf /"}"#.into(),
                    },
                    on_match: dedroom_core::config::RuleAction::Block,
                },
            ],
            ..Default::default()
        };
        let mut detector = LoopDetector::new(&config);

        bench_mean_ns(100_000, "Rule engine (safe call, no match)", || {
            let v = detector.verify("execute_command", r#"{"command":"ls -la"}"#);
            std::hint::black_box(v);
        });

        bench_mean_ns(50_000, "Rule engine (blocking call, match)", || {
            let v = detector.verify("execute_command", r#"{"command":"rm -rf /"}"#);
            std::hint::black_box(v);
        });
    }

    // 1g. Memory usage: count how many calls fit in a history window
    {
        let mut detector = LoopDetector::new(&config);
        let start = Instant::now();
        for i in 0..1_000 {
            detector.verify("write_file", &format!(r#"{{"path":"/tmp/test_{}.txt"}}"#, i));
            detector.record_result("write_file", &format!(r#"{{"path":"/tmp/test_{}.txt"}}"#, i), false);
        }
        let dur = elapsed_us(start);
        println!("  {:<30} {:>10} calls  {:>8.0} µs total  ({:.1} ns/call)", "History fill (1k entries)", 1000, dur, dur * 1000.0 / 1000.0);
    }
}

// ── 2. COMPRESSION BENCHMARKS ────────────────────────────────────────────

fn bench_compression() {
    print_header("2. Compression Benchmarks");

    // 2a. SmartCrusher
    {
        for size in [10usize, 100, 1000] {
            let json = gen_large_json_array(size);
            let tokens = estimate_tokens(&json);

            let iters = match size {
                10 => 500,
                100 => 100,
                _ => 30,
            };
            bench_mean_by(iters, &format!("SmartCrusher {} rows", size), || {
                let result = compress_json_array(&json, 0.3).unwrap();
                std::hint::black_box(result.compressed_count as u8)
            });

            println!("    └─ Input: {} rows, ~{} tokens", size, tokens);
        }
    }

    // 2b. SmartCrusher with different retention rates (1000 rows)
    {
        let json = gen_large_json_array(1000);
        for retention in [0.05, 0.1, 0.2, 0.3, 0.5] {
            bench_mean_by(30, &format!("SmartCrusher retention={:.0}%", retention * 100.0), || {
                let result = compress_json_array(&json, retention).unwrap();
                std::hint::black_box(result.compressed_count as u8)
            });
        }
    }

    // 2c. CodeCompressor small (< 10 lines = no compression)
    {
        let code = "fn main() {\n    println!(\"hello world\");\n}\n";
        bench_mean_ns(2_000, "CodeCompressor (small, <10 lines)", || {
            let _ = std::hint::black_box(compress_code(code, "rust"));
        });
    }

    // 2d. CodeCompressor medium (10 functions)
    {
        let code = gen_code(10);
        let lines = code.lines().count();
        bench_mean_ns(200, &format!("CodeCompressor ({} lines, 10 fns)", lines), || {
            let _ = std::hint::black_box(compress_code(&code, "rust"));
        });
    }

    // 2e. CodeCompressor large (50 functions)
    {
        let code = gen_code(50);
        let lines = code.lines().count();
        bench_mean_ns(100, &format!("CodeCompressor ({} lines, 50 fns)", lines), || {
            let _ = std::hint::black_box(compress_code(&code, "rust"));
        });
    }

    // 2f. LogCompressor
    {
        for (size, unique_ratio) in &[(100usize, 0.1), (1000, 0.05)] {
            let logs = gen_logs_with_duplicates(*size, *unique_ratio);
            bench_mean_ns(
                (5_000usize / size).max(50),
                &format!("LogCompressor {} lines ({}% unique)", size, (unique_ratio * 100.0) as u8),
                || {
                    let _ = std::hint::black_box(compress_logs(&logs));
                },
            );
        }
    }

    // 2g. TextCompressor
    {
        for count in &[10usize, 100, 1000] {
            let text = gen_text_paragraphs(*count);
            bench_mean_ns(
                (5_000usize / count).max(50),
                &format!("TextCompressor {} paragraphs", count),
                || {
                    let _ = std::hint::black_box(compress_text(&text));
                },
            );
        }
    }
}

// ── 3. CONTENT ROUTER BENCHMARKS ─────────────────────────────────────────

fn bench_content_router() {
    print_header("3. Content Router Benchmarks");

    let router = ContentRouter::new(&ContentRouterConfig::default());

    let code_sample = gen_code(2);
    let samples: Vec<(&str, &str)> = vec![
        ("JSON array (10 items)", r#"[{"id":1},{"id":2},{"id":3},{"id":4},{"id":5},{"id":6},{"id":7},{"id":8},{"id":9},{"id":10}]"#),
        ("JSON object (deep)", r#"{"user":{"name":"Alice","address":{"city":"NYC","zip":"10001"},"preferences":{"theme":"dark","notifications":true}}}"#),
        ("Rust code (20 lines)", &code_sample),
        ("Log output", "[INFO] 2026-01-01T12:00:00Z Starting\n[ERROR] something failed\n[INFO] 2026-01-01T12:00:01Z Retrying\n[ERROR] failed again\n"),
        ("Plain text", "Just a regular sentence with no special formatting."),
        ("Diff output", "--- a/file\n+++ b/file\n@@ -1,3 +1,4 @@\n old text\n+new text\n"),
    ];

    for (label, content) in &samples {
        bench_mean_ns(100_000, &format!("Router: {}", label), || {
            let _ = std::hint::black_box(router.detect_type(content));
        });
    }
}

// ── 4. PIPELINE BENCHMARKS ───────────────────────────────────────────────

fn bench_pipeline() {
    print_header("4. Pipeline Benchmarks");

    let rt = tokio::runtime::Runtime::new().unwrap();

    // 4a. First call (allow + compress)
    {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);

        let json = gen_large_json_array(100);
        let tool = ToolCall {
            name: "search".into(),
            args: r#"{"query":"test"}"#.into(),
            result: Some(json),
            is_error: false,
        };

        bench_mean_by(50, "Pipeline: first call (allow + compress)", || {
            let result = rt.block_on(pipeline.process_tool_call(&tool));
            std::hint::black_box(result.loop_verdict.to_code())
        });
    }

    // 4b. Loop detection through pipeline (block)
    {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);

        let tool = ToolCall {
            name: "write_file".into(),
            args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("done".into()),
            is_error: true,
        };

        // Prime the loop detector
        for _ in 0..3 {
            let _ = rt.block_on(pipeline.process_tool_call(&tool));
        }

        bench_mean_by(50, "Pipeline: block on 4th repeat", || {
            let result = rt.block_on(pipeline.process_tool_call(&tool));
            std::hint::black_box(if result.loop_verdict.is_blocked() { 1u8 } else { 0u8 })
        });
    }

    // 4c. Full pipeline with large result
    {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);

        let json = gen_large_json_array(1000);
        let tool = ToolCall {
            name: "search".into(),
            args: r#"{"query":"test"}"#.into(),
            result: Some(json),
            is_error: false,
        };
        bench_mean_by(20, "Pipeline: 1000-row result (allow + compress)", || {
            let result = rt.block_on(pipeline.process_tool_call(&tool));
            std::hint::black_box(result.compression_results.len() as u8)
        });
    }

    // 4d. Pipeline with error loop (hint injection)
    {
        let config = DedrooMConfig::default();
        let mut pipeline = Pipeline::new(config);

        let tool = ToolCall {
            name: "write_file".into(),
            args: r#"{"path":"/tmp/x.txt"}"#.into(),
            result: Some("error: no space left on device".into()),
            is_error: true,
        };

        for _ in 0..4 {
            let _ = rt.block_on(pipeline.process_tool_call(&tool));
        }

        bench_mean_by(50, "Pipeline: error loop (block + hint)", || {
            let result = rt.block_on(pipeline.process_tool_call(&tool));
            std::hint::black_box(result.injection_hint.is_some() as u8)
        });
    }

    // 4e. Savings ledger recording speed
    {
        let mut pipeline = Pipeline::new(DedrooMConfig::default());

        let tool = ToolCall {
            name: "read_file".into(),
            args: r#"{"path":"/tmp/data.txt"}"#.into(),
            result: Some(gen_large_json_array(500)),
            is_error: false,
        };

        bench_mean_by(200, "Pipeline: savings recording", || {
            let result = rt.block_on(pipeline.process_tool_call(&tool));
            std::hint::black_box(result.loop_verdict.to_code())
        });
    }
}

// ── 5. COMPRESSION QUALITY STATS ─────────────────────────────────────────

fn bench_compression_quality() {
    print_header("5. Compression Quality Metrics");

    // SmartCrusher
    for size in [10usize, 100, 1000] {
        let json = gen_large_json_array(size);
        if let Ok(result) = compress_json_array(&json, 0.3) {
            let original_tokens = estimate_tokens(&json);
            let compressed_tokens = estimate_tokens(&result.content);
            let reduction = if original_tokens > 0 {
                (1.0 - compressed_tokens as f64 / original_tokens as f64) * 100.0
            } else {
                0.0
            };
            println!("  {:<30} rows={:>5}  orig={:>6} tok  comp={:>6} tok  {:+>5.1}% reduction  rows kept: {}/{}",
                "SmartCrusher",
                size,
                original_tokens,
                compressed_tokens,
                reduction,
                result.compressed_count,
                result.original_count,
            );
        }
    }

    // CodeCompressor
    for funcs in [5usize, 10, 20] {
        let code = gen_code(funcs);
        let original_tokens = estimate_tokens(&code);
        let compressed = compress_code(&code, "rust");
        let compressed_tokens = estimate_tokens(&compressed);
        let reduction = if original_tokens > 0 {
            (1.0 - compressed_tokens as f64 / original_tokens as f64) * 100.0
        } else {
            0.0
        };
        println!("  {:<30} funcs={:>3}  orig={:>6} tok  comp={:>6} tok  {:+>5.1}% reduction",
            "CodeCompressor",
            funcs,
            original_tokens,
            compressed_tokens,
            reduction,
        );
    }

    // LogCompressor
    for size in [100usize, 1000] {
        let logs = gen_logs_with_duplicates(size, 0.1);
        let original_tokens = estimate_tokens(&logs);
        let compressed = compress_logs(&logs);
        let compressed_tokens = estimate_tokens(&compressed);
        let reduction = if original_tokens > 0 {
            (1.0 - compressed_tokens as f64 / original_tokens as f64) * 100.0
        } else {
            0.0
        };
        println!("  {:<30} lines={:>4}  orig={:>6} tok  comp={:>6} tok  {:+>5.1}% reduction",
            "LogCompressor",
            size,
            original_tokens,
            compressed_tokens,
            reduction,
        );
    }
}

// ── MAIN ──────────────────────────────────────────────────────────────────

fn main() {
    println!("{}", "╔".to_string() + &"═".repeat(68) + "╗");
    println!("║  DedrooM Performance Benchmarks                                    ║");
    println!("║  Rust edition 2024 — real measurements, no simulations              ║");
    println!("╚{}╝", "═".repeat(68));

    println!("\nWarming up CPU cache...");
    warmup_cpu();
    println!("Ready.\n");

    bench_loop_detection();
    bench_compression();
    bench_content_router();
    bench_pipeline();
    bench_compression_quality();

    println!("\n{}", "═".repeat(70));
    println!("  Benchmarks complete.");
    println!("{}", "═".repeat(70));
}
