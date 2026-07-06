//! Mutation strategies — known ways to break a loop by varying the approach.
//!
//! Each strategy takes the current tool call context and generates one or
//! more alternative approaches the agent could try instead.

use serde_json::Value;

// ── Mutation Types ─────────────────────────────────────────────────────────

/// A generated mutation — an alternative approach to the current tool call.
#[derive(Debug, Clone)]
pub struct Mutation {
    /// Human-readable label describing this strategy.
    pub strategy: &'static str,
    /// The suggested alternative (injected as a hint to the agent).
    pub suggestion: String,
    /// How likely this is to help (0.0 – 1.0).
    pub confidence: f64,
    /// Estimated cost/risk (higher = riskier).
    pub risk: f64,
}

/// Context snapshot for mutation generation.
#[derive(Debug, Clone)]
pub struct MutationContext<'a> {
    pub tool_name: &'a str,
    pub tool_args: &'a str,
    pub is_error: bool,
    pub repeat_count: u32,
    pub tilt_index: f64,
    pub session_tool_count: usize,
}

// ── Strategy registry ──────────────────────────────────────────────────────

/// Known tool-pair substitutions (tool → alternative approach).
const TOOL_SUBSTITUTIONS: &[(&str, &str, &str)] = &[
    ("list_files", "find", "Use `find` with specific path+pattern instead of `list_files`"),
    ("web_search", "browse_page", "Try `browse_page` on the most relevant result URL instead of searching again"),
    ("read_file", "grep + head", "Use `grep` with `head` to read only matching lines instead of the whole file"),
    ("write_file", "edit_file", "Use `edit_file` to make targeted edits instead of overwriting the entire file"),
    ("search", "grep", "Use `grep` locally instead of `search` to avoid hitting external APIs"),
    ("execute_command", "script", "Put the command in a script file and run that instead"),
];

/// Parameter keys that commonly cause loops when too large.
const VOLATILE_PARAMS: &[&str] = &[
    "limit", "count", "max_results", "batch_size", "size", "depth",
    "offset", "page_size", "n", "num", "max_items",
];

// ── Strategy implementations ───────────────────────────────────────────────

/// Strategy A: Tweak numeric parameters (e.g. reduce batch size, limit).
pub fn parameter_tweak(ctx: &MutationContext) -> Vec<Mutation> {
    if ctx.tilt_index < 0.3 {
        return Vec::new(); // only suggest tweaks when tilted
    }

    let mut mutations = Vec::new();
    if let Ok(args) = serde_json::from_str::<Value>(ctx.tool_args) {
        if let Value::Object(map) = &args {
            for param in VOLATILE_PARAMS {
                if let Some(Value::Number(n)) = map.get(*param) {
                    if let Some(val) = n.as_u64() {
                        if val > 1 {
                            let reduced = if val > 100 { val / 10 } else { val / 2 }.max(1);
                            mutations.push(Mutation {
                                strategy: "parameter_tweak",
                                suggestion: format!(
                                    "The `{param}` parameter is set to {val}. Try reducing it to {reduced} \
                                     to narrow the scope and avoid repeated failures."
                                ),
                                confidence: if val > 100 { 0.7 } else { 0.5 },
                                risk: 0.2,
                            });
                            // Only the most impactful tweak
                            break;
                        }
                    }
                }
            }
        }
    }
    mutations
}

/// Strategy B: Substitute the tool with a known alternative.
pub fn tool_substitution(ctx: &MutationContext) -> Vec<Mutation> {
    let mut mutations = Vec::new();
    for (tool, _substitute, description) in TOOL_SUBSTITUTIONS {
        if ctx.tool_name == *tool {
            mutations.push(Mutation {
                strategy: "tool_substitution",
                suggestion: format!("{description}. This may break the loop pattern."),
                confidence: 0.6,
                risk: 0.3,
            });
        }
    }
    mutations
}

/// Strategy C: Decompose into smaller steps.
pub fn decomposition(ctx: &MutationContext) -> Vec<Mutation> {
    let mut mutations = Vec::new();
    // Check if the tool name suggests batch/bulk operations
    let decomposable = [
        "batch", "bulk", "bulk_", "batch_", "process_all", "process_many",
        "foreach", "map", "update_all", "delete_all",
    ];
    let tool_lower = ctx.tool_name.to_lowercase();
    let is_batch = decomposable.iter().any(|kw| tool_lower.contains(kw));

    if is_batch && ctx.repeat_count > 1 {
        mutations.push(Mutation {
            strategy: "decomposition",
            suggestion: format!(
                "Instead of `{}` in one batch, try processing one item at a time. \
                 Pick the first item, handle it, then proceed step by step.",
                ctx.tool_name,
            ),
            confidence: 0.55,
            risk: 0.25,
        });
    }

    // Also suggest decomposition for search/read tools with large result sets
    if ctx.tilt_index > 0.6 {
        if let Ok(args) = serde_json::from_str::<Value>(ctx.tool_args) {
            let has_limit = args.get("limit").or_else(|| args.get("max_results"));
            if has_limit.is_some_and(|v| v.as_u64().unwrap_or(0) > 10) {
                mutations.push(Mutation {
                    strategy: "decomposition",
                    suggestion: format!(
                        "The result set may be too large. Try a more specific query or \
                         use filtering to narrow results before processing."
                    ),
                    confidence: 0.5,
                    risk: 0.2,
                });
            }
        }
    }

    mutations
}

/// Strategy D: Rephrase the approach.
pub fn rephrase(ctx: &MutationContext) -> Vec<Mutation> {
    let mut mutations = Vec::new();

    if ctx.is_error && ctx.repeat_count >= 2 {
        mutations.push(Mutation {
            strategy: "rephrase",
            suggestion: format!(
                "The current approach to `{}` keeps failing. Pause and review the documentation \
                 or error output. Try a fundamentally different approach — don't repeat the same \
                 failing pattern.",
                ctx.tool_name,
            ),
            confidence: 0.45,
            risk: 0.15,
        });
    }

    if ctx.tilt_index > 0.7 {
        mutations.push(Mutation {
            strategy: "rephrase",
            suggestion: "You seem stuck in an error loop. Take a step back, \
                         review what changed in the environment, and try a \
                         completely different strategy."
                .to_string(),
            confidence: 0.4,
            risk: 0.1,
        });
    }

    mutations
}

/// Generate all candidate mutations for a given context.
pub fn generate_all(ctx: &MutationContext) -> Vec<Mutation> {
    let mut candidates = Vec::new();
    candidates.extend(parameter_tweak(ctx));
    candidates.extend(tool_substitution(ctx));
    candidates.extend(decomposition(ctx));
    candidates.extend(rephrase(ctx));

    // Score and rank: higher confidence + lower risk = better
    candidates.sort_by(|a, b| {
        let score_a = a.confidence - a.risk * 0.5;
        let score_b = b.confidence - b.risk * 0.5;
        score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    candidates
}

/// Pick the single best mutation.
pub fn pick_best(ctx: &MutationContext) -> Option<Mutation> {
    let mut candidates = generate_all(ctx);
    // Deduplicate strategies — keep only the highest-ranked per strategy
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|m| seen.insert(m.strategy));
    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parameter_tweak_detects_limit() {
        let ctx = MutationContext {
            tool_name: "query_database",
            tool_args: r#"{"query":"hello","limit":100}"#,
            is_error: true,
            repeat_count: 3,
            tilt_index: 0.8,
            session_tool_count: 10,
        };
        let mutations = parameter_tweak(&ctx);
        assert!(!mutations.is_empty());
        assert!(mutations[0].suggestion.contains("limit"));
        assert!(mutations[0].suggestion.contains("50")); // 100 / 2
    }

    #[test]
    fn test_tool_substitution_finds_match() {
        let ctx = MutationContext {
            tool_name: "web_search",
            tool_args: "{}",
            is_error: true,
            repeat_count: 3,
            tilt_index: 0.5,
            session_tool_count: 5,
        };
        let mutations = tool_substitution(&ctx);
        assert!(!mutations.is_empty());
        assert!(mutations[0].suggestion.contains("browse_page"));
    }

    #[test]
    fn test_tool_substitution_no_match() {
        let ctx = MutationContext {
            tool_name: "unknown_tool",
            ..Default::default()
        };
        assert!(tool_substitution(&ctx).is_empty());
    }

    #[test]
    fn test_pick_best_returns_highest_confidence() {
        let ctx = MutationContext {
            tool_name: "query_database",
            tool_args: r#"{"query":"hello","limit":50}"#,
            is_error: true,
            repeat_count: 4,
            tilt_index: 0.8,
            session_tool_count: 10,
        };
        let best = pick_best(&ctx);
        assert!(best.is_some());
        // parameter_tweak should rank highest for a non-substitutable tool
        assert_eq!(best.unwrap().strategy, "parameter_tweak");
    }

    #[test]
    fn test_no_low_tilt_no_tweaks() {
        let ctx = MutationContext {
            tool_name: "unknown_tool",
            tool_args: r#"{}""#,
            is_error: false,
            repeat_count: 1,
            tilt_index: 0.1,
            session_tool_count: 3,
        };
        let mutations = generate_all(&ctx);
        // No parameter_tweak (tilt too low), no tool_sub (not in list), no decomposition (not batch)
        // rephrase may fire for low tilt? No, rephrase requires error + repeat >= 2
        assert!(mutations.is_empty());
    }

    impl Default for MutationContext<'_> {
        fn default() -> Self {
            Self {
                tool_name: "",
                tool_args: "{}",
                is_error: false,
                repeat_count: 0,
                tilt_index: 0.0,
                session_tool_count: 0,
            }
        }
    }
}
