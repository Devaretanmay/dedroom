/// Basic usage example for DedrooM.
///
/// Run with: cargo run --example basic
use dedroom_core::config::DedrooMConfig;
use dedroom_core::loop_detection::LoopDetector;
use dedroom_core::compression::{ContentRouter, compress_json_array, estimate_tokens};
use dedroom_core::config::ContentRouterConfig;

fn main() {
    println!("═══ DedrooM Basic Example ═══\n");

    // ── Loop Detection ──
    println!("▶ Loop Detection");
    let config = DedrooMConfig::from_yaml_str(r#"
loop_detection:
  max_repeats: 3
  strictness: balanced
"#).expect("valid config");

    let mut detector = LoopDetector::new(&config.loop_detection);
    let tool = "write_file";
    let args = r#"{"path":"/tmp/test.txt"}"#;

    for i in 0..5 {
        let verdict = detector.verify(tool, args);
        println!("  Call {}: tool={}, args={}, verdict={}",
            i + 1, tool, args, verdict.to_code());
        detector.record_result(tool, args, false);
    }

    // ── Content Router ──
    println!("\n▶ Content Router");
    let router = ContentRouter::new(&ContentRouterConfig::default());
    let samples = [
        ("JSON array", r#"[{"id":1},{"id":2},{"id":3}]"#),
        ("Rust code", "fn main() {\n    println!(\"hello\");\n}"),
        ("Log output", "[INFO] 2024-01-01T12:00:00 Starting\n[ERROR] failed"),
        ("Plain text", "Just a regular sentence."),
    ];
    for (label, content) in &samples {
        let ctype = router.detect_type(content);
        println!("  {} → {:?}", label, ctype);
    }

    // ── SmartCrusher ──
    println!("\n▶ SmartCrusher");
    let json_input = r#"[
        {"id": 1, "name": "Alice", "role": "engineer", "email": "a@example.com"},
        {"id": 2, "name": "Bob", "role": "designer", "email": "b@example.com"},
        {"id": 3, "name": "Charlie", "role": "manager", "email": "c@example.com"},
        {"id": 4, "name": "Diana", "role": "engineer", "email": "d@example.com"},
        {"id": 5, "name": "Eve", "role": "analyst", "email": "e@example.com"},
        {"id": 6, "name": "Frank", "role": "engineer", "email": "f@example.com"},
        {"id": 7, "name": "Grace", "role": "designer", "email": "g@example.com"},
        {"id": 8, "name": "Hank", "role": "intern", "email": "h@example.com"},
        {"id": 9, "name": "Ivy", "role": "engineer", "email": "i@example.com"},
        {"id": 10, "name": "Jack", "role": "lead", "email": "j@example.com"}
    ]"#;

    match compress_json_array(json_input, 0.3) {
        Ok(result) => {
            println!("  Original: {} rows → Compressed: {} rows ({} dropped)",
                result.original_count, result.compressed_count, result.rows_dropped);
            println!("  Compression ratio: {:.1}%", estimate_tokens(json_input) as f64 / estimate_tokens(&result.content) as f64 * 100.0);
            println!("  Output: {}", result.content);
        }
        Err(e) => println!("  Error: {}", e),
    }

    println!("\n═══ Done ═══");
}
