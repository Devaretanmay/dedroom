/// DedrooM Self-Healing Demo
///
/// Shows the full loop detection -> blocking -> healing hint flow
/// with detailed per-step output.
///
/// Run: cargo run --example demo_self_healing
use dedroom_core::config::DedrooMConfig;
use dedroom_core::pipeline::{Pipeline, ToolCall, PipelineResult};
use dedroom_core::compression::{compress_json_array, compress_code, compress_logs};

fn main() {
    println!();
    println!("  ╔═══════════════════════════════════════════════════╗");
    println!("  ║         DedrooM Self-Healing Demo                ║");
    println!("  ╚═══════════════════════════════════════════════════╝");
    println!();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let config = DedrooMConfig::default();
    let mut pipeline = Pipeline::new(config);

    let tool_name = "write_file";
    let tool_args = r#"{"path":"/tmp/deploy.yaml"}"#;

    println!("  Tool:     {}({})", tool_name, tool_args);
    println!("  Scenario: Agent keeps failing with the same call");
    println!();

    // --- Call 1: First attempt (should be allowed) ---
    println!("  --- Call 1: First attempt -------------------------");
    let result = rt.block_on(pipeline.process_tool_call(&ToolCall {
        name: tool_name.into(),
        args: tool_args.into(),
        result: Some("Error: permission denied".into()),
        is_error: true,
    }, None));
    print_verdict(&result, 1);

    // --- Call 2: Second attempt (still allowed, warming up) ---
    println!("  --- Call 2: Same call, still failing --------------");
    let result = rt.block_on(pipeline.process_tool_call(&ToolCall {
        name: tool_name.into(),
        args: tool_args.into(),
        result: Some("Error: permission denied".into()),
        is_error: true,
    }, None));
    print_verdict(&result, 2);

    // --- Call 3: Third attempt ---
    println!("  --- Call 3: Still looping --------------------------");
    let result = rt.block_on(pipeline.process_tool_call(&ToolCall {
        name: tool_name.into(),
        args: tool_args.into(),
        result: Some("Error: permission denied".into()),
        is_error: true,
    }, None));
    print_verdict(&result, 3);

    // --- Call 4: Blocked + healing hint ---
    println!("  --- Call 4: Blocked with healing hint --------------");
    let result = rt.block_on(pipeline.process_tool_call(&ToolCall {
        name: tool_name.into(),
        args: tool_args.into(),
        result: Some("Error: permission denied".into()),
        is_error: true,
    }, None));
    print_verdict(&result, 4);

    if let Some(ref hint) = result.injection_hint {
        println!("  >> Healing hint injected:");
        for line in hint.lines() {
            println!("  >>   {}", line);
        }
    }

    // --- Call 5: Agent adapts with different args ---
    println!();
    println!("  --- Call 5: Agent adapts (different path) ----------");
    let result = rt.block_on(pipeline.process_tool_call(&ToolCall {
        name: tool_name.into(),
        args: r#"{"path":"/tmp/deploy_backup.yaml"}"#.into(),
        result: Some("written".into()),
        is_error: false,
    }, None));
    print_verdict(&result, 5);

    // --- Healing stats ---
    println!();
    println!("  --- Self-Healing Stats -----------------------------");
    println!("  Mutation attempts:  {}", pipeline.healing_engine.total_attempts());
    println!("  Success rate:       {:.0}%",
        if pipeline.healing_engine.total_attempts() > 0 {
            pipeline.healing_engine.successful_recoveries() as f64 / pipeline.healing_engine.total_attempts() as f64 * 100.0
        } else { 0.0 }
    );

    // --- Compression demo ---
    println!();
    println!("  --- Compression Demo -------------------------------");
    demo_compression();

    // --- Summary ---
    println!();
    println!("  --- Summary ----------------------------------------");
    println!("  Self-healing detects repeated failing calls,");
    println!("  blocks them before they waste tokens, and injects");
    println!("  context-aware hints to help the agent adapt.");
    println!();
    println!("  Compression reduces redundant tool output by");
    println!("  60-95% before it reaches the LLM, saving tokens");
    println!("  on every call.");
    println!();
    println!("  ═══════════════════════════════════════════════════");
}

fn print_verdict(result: &PipelineResult, call_num: u32) {
    let code = result.loop_verdict.to_code();
    let label = match code {
        0 => "ALLOW",
        1 => "WARN",
        2 => "BLOCK_RETRY",
        3 => "BLOCK_HALT",
        _ => "UNKNOWN",
    };
    let blocked = if result.loop_verdict.is_blocked() { " [BLOCKED]" } else { "" };
    print!("  Call #{}: verdict={} ({}){}", call_num, code, label, blocked);

    if let Some(cr) = result.compression_results.first() {
        if cr.compressed_tokens < cr.original_tokens {
            let pct = (cr.compressed_tokens as f64 / cr.original_tokens.max(1) as f64) * 100.0;
            println!("  compressed: {} -> {} tokens ({:.0}% of original)",
                cr.original_tokens, cr.compressed_tokens, pct);
        } else {
            println!();
        }
    } else {
        println!();
    }
}

fn demo_compression() {
    // SmartCrusher
    let json = r#"[
        {"id":1,"name":"Alice","role":"engineer","email":"a@example.com"},
        {"id":2,"name":"Bob","role":"designer","email":"b@example.com"},
        {"id":3,"name":"Charlie","role":"manager","email":"c@example.com"},
        {"id":4,"name":"Diana","role":"engineer","email":"d@example.com"},
        {"id":5,"name":"Eve","role":"analyst","email":"e@example.com"},
        {"id":6,"name":"Frank","role":"engineer","email":"f@example.com"},
        {"id":7,"name":"Grace","role":"designer","email":"g@example.com"},
        {"id":8,"name":"Hank","role":"intern","email":"h@example.com"},
        {"id":9,"name":"Ivy","role":"engineer","email":"i@example.com"},
        {"id":10,"name":"Jack","role":"lead","email":"j@example.com"}
    ]"#;
    let orig_tokens = (json.len() as f64 / 4.0).ceil() as u64;
    match compress_json_array(json, 0.3) {
        Ok(result) => {
            let comp_tokens = (result.content.len() as f64 / 4.0).ceil() as u64;
            let pct = if orig_tokens > 0 { (comp_tokens as f64 / orig_tokens as f64) * 100.0 } else { 0.0 };
            println!("  SmartCrusher (JSON, 10 rows):");
            println!("    {} -> {} tokens  ({:.0}% of original, {} rows kept)",
                orig_tokens, comp_tokens, pct, result.compressed_count);
        }
        Err(e) => println!("  SmartCrusher error: {}", e),
    }

    // CodeCompressor
    let code = r#"
fn read_config(path: &str) -> Result<Config, Error> {
    let contents = fs::read_to_string(path)?;
    let config: Config = toml::from_str(&contents)?;
    Ok(config)
}

fn validate_rules(rules: &[Rule]) -> Vec<String> {
    let mut errors = Vec::new();
    for rule in rules {
        if rule.pattern.is_empty() {
            errors.push(format!("Rule '{}' has empty pattern", rule.name));
        }
    }
    errors
}
"#;
    let code_orig = (code.len() as f64 / 4.0).ceil() as u64;
    let compressed = compress_code(code, "rust");
    let code_comp = (compressed.len() as f64 / 4.0).ceil() as u64;
    let pct = if code_orig > 0 { (code_comp as f64 / code_orig as f64) * 100.0 } else { 0.0 };
    println!("  CodeCompressor (Rust, 2 functions):");
    println!("    {} -> {} tokens  ({:.0}% of original)", code_orig, code_comp, pct);

    // LogCompressor
    let logs = (0..20).map(|i| {
        if i % 3 == 0 {
            format!("[ERROR] 2026-07-08T10:00:{}Z Connection timeout", i)
        } else {
            format!("[INFO] 2026-07-08T10:00:{}Z Processing item {}", i, i)
        }
    }).collect::<Vec<_>>().join("\n");
    let log_orig = (logs.len() as f64 / 4.0).ceil() as u64;
    let compressed = compress_logs(&logs);
    let log_comp = (compressed.len() as f64 / 4.0).ceil() as u64;
    let pct = if log_orig > 0 { (log_comp as f64 / log_orig as f64) * 100.0 } else { 0.0 };
    println!("  LogCompressor (20 lines, repetitive):");
    println!("    {} -> {} tokens  ({:.0}% of original)", log_orig, log_comp, pct);
}
