//! CodeCompressor — AST-aware code compression via tree-sitter.
//!
//! Preserves imports, signatures, type annotations, doc comments, and
//! top-level structure while compressing function/method/class bodies.
//! Uses tree-sitter for accurate AST-based body detection and stripping.
//!
//! Supported languages:
//! - Rust (via tree-sitter-rust)
//! - Python (via tree-sitter-python)
//! - JavaScript / TypeScript (via tree-sitter-javascript)
//! - Fallback to line-heuristic mode for all other languages.
//!
//! ## Performance note
//! Tree-sitter parsers are cached in thread-local storage so they are
//! reused across calls. Creating a parser is expensive (~ms) but reusing
//! one is nearly free. The cache is per-thread and per-language.

use std::cell::RefCell;
use std::collections::HashMap;
use tree_sitter::{Parser, Language};

// ── Thread-local parser cache ─────────────────────────────────────────────
//
// Reusing Parsers avoids re-loading grammar WASM/bytecode on every call.
// Each entry is keyed by language name ("rust", "python", "javascript").

thread_local! {
    static PARSER_CACHE: RefCell<HashMap<String, Parser>> = RefCell::new(HashMap::new());
}

/// Borrow a parser from the thread-local cache, or create a new one.
fn take_or_create_parser(language: &str) -> Option<Parser> {
    // Fast path: check cache first
    let lang_key = language.to_string();
    let cached = PARSER_CACHE.with(|cache| cache.borrow_mut().remove(&lang_key));
    if let Some(p) = cached {
        return Some(p);
    }

    // Cache miss: create a new parser and configure it
    let lang = resolve_language(language)?;
    let mut parser = Parser::new();
    parser.set_language(&lang).ok()?;
    Some(parser)
}

/// Return a parser to the thread-local cache for future reuse.
fn return_parser(language: &str, parser: Parser) {
    let lang_key = language.to_string();
    PARSER_CACHE.with(|cache| {
        cache.borrow_mut().insert(lang_key, parser);
    });
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Compress source code by stripping function bodies while preserving
/// structure. Uses tree-sitter for AST-accurate parsing; falls back to
/// a line-heuristic mode when tree-sitter grammar isn't available for
/// the requested language.
pub fn compress_code(input: &str, language: &str) -> String {
    if input.len() < 30 || input.lines().count() < 6 {
        // Too short to compress meaningfully
        return input.to_string();
    }

    // Try tree-sitter first
    if let Some(parsed) = compress_via_treesitter(input, language) {
        return parsed;
    }

    // Fallback: line heuristic
    compress_via_heuristic(input, language)
}

// ── Tree-sitter implementation ─────────────────────────────────────────────

/// Attempt to compress code using tree-sitter AST.
/// Returns `None` if the language grammar is not available.
fn compress_via_treesitter(input: &str, language: &str) -> Option<String> {
    let mut parser = take_or_create_parser(language)?;

    let tree = match parser.parse(input, None) {
        Some(t) => t,
        None => {
            // Parse failed — return parser to cache so it's not wasted
            return_parser(language, parser);
            return None;
        }
    };
    let root = tree.root_node();

    // Collect the byte ranges of function/method bodies we want to strip.
    // We do this before any modification to avoid borrowing issues.
    let mut body_ranges: Vec<std::ops::Range<usize>> = Vec::new();

    match language {
        "rust" => collect_rust_bodies(root, input, &mut body_ranges),
        "python" => collect_python_bodies(root, input, &mut body_ranges),
        "javascript" | "typescript" | "js" | "ts" => {
            collect_js_bodies(root, input, &mut body_ranges)
        }
        _ => {
            return_parser(language, parser);
            return None;
        }
    }

    if body_ranges.is_empty() {
        return_parser(language, parser);
        return None;
    }

    // Build compressed output by replacing each body range with "..."
    let mut result = String::with_capacity(input.len());
    let mut last_end = 0usize;

    for range in &body_ranges {
        // Append content before this body
        result.push_str(&input[last_end..range.start]);
        result.push_str("/* ... */");
        last_end = range.end;
    }
    result.push_str(&input[last_end..]);

    // Return parser to cache before returning result
    return_parser(language, parser);

    Some(result)
}

/// Resolve a language name to a tree-sitter Language.
fn resolve_language(language: &str) -> Option<Language> {
    match language {
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "javascript" | "typescript" | "js" | "ts" => {
            Some(tree_sitter_javascript::LANGUAGE.into())
        }
        _ => None,
    }
}

/// Collect body byte ranges for Rust: `function_item`, `method_implementation`,
/// `closure_expression`, and inline `block`s that are function bodies.
fn collect_rust_bodies(
    node: tree_sitter::Node,
    source: &str,
    ranges: &mut Vec<std::ops::Range<usize>>,
) {
    let _ = source; // unused but kept for API consistency
    let kind = node.kind();
    let is_function = kind == "function_item"
        || kind == "function_signature"
        || kind == "method_implementation";

    if is_function {
        // Traverse children to find the `block` (body)
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" {
                let sr = child.start_byte();
                let er = child.end_byte();
                // Only strip non-empty blocks that are more than just "{}"
                if er - sr > 4 {
                    ranges.push(sr + 1..er - 1);
                }
            }
        }
        return; // Don't recurse into function children
    }

    // Don't enter strings or comments
    if kind == "string_literal" || kind == "line_comment" || kind == "block_comment" {
        return;
    }

    // Recurse
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_bodies(child, source, ranges);
    }
}

/// Collect body ranges for Python: `function_definition`, `class_definition`,
/// `decorated_definition`.
fn collect_python_bodies(
    node: tree_sitter::Node,
    source: &str,
    ranges: &mut Vec<std::ops::Range<usize>>,
) {
    let _ = source; // unused but kept for API consistency
    let kind = node.kind();

    // Handle decorated definitions: recurse into children to find the
    // inner function_definition or class_definition
    if kind == "decorated_definition" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_python_bodies(child, source, ranges);
        }
        return;
    }

    let is_function = kind == "function_definition" || kind == "class_definition";

    if is_function {
        // Find the `block` or `body` child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "block" || child.kind() == "body" {
                let sr = child.start_byte();
                let er = child.end_byte();
                if er - sr > 4 {
                    ranges.push(sr..er);
                }
            }
        }
        return;
    }

    if kind == "string" || kind == "comment" {
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_python_bodies(child, source, ranges);
    }
}

/// Collect body ranges for JS/TS: `function_declaration`, `method_definition`,
/// `arrow_function`, `class_declaration`.
fn collect_js_bodies(
    node: tree_sitter::Node,
    source: &str,
    ranges: &mut Vec<std::ops::Range<usize>>,
) {
    let _ = source; // unused but kept for API consistency
    let kind = node.kind();
    let is_function = kind == "function_declaration"
        || kind == "method_definition"
        || kind == "arrow_function"
        || kind == "class_declaration"
        || kind == "generator_function_declaration";

    if is_function {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "statement_block" || child.kind() == "class_body" {
                let sr = child.start_byte();
                let er = child.end_byte();
                if er - sr > 4 {
                    ranges.push(sr + 1..er - 1);
                }
            }
        }
        return;
    }

    if kind == "string" || kind == "comment" || kind == "template_string" {
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_js_bodies(child, source, ranges);
    }
}

// ── Heuristic fallback ─────────────────────────────────────────────────────

/// Compress source code by stripping function bodies while preserving
/// structure. This is a simplified version that uses line heuristics
/// rather than full tree-sitter (reducing dependency weight).
fn compress_via_heuristic(input: &str, language: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    if lines.len() < 6 {
        return input.to_string();
    }

    let mut output: Vec<&str> = Vec::new();
    let mut body_line_count = 0usize;
    let mut in_body = false;

    for line in &lines {
        let trimmed = line.trim();

        // Always keep imports and module declarations
        if is_declaration(trimmed, language) {
            output.push(line);
            continue;
        }

        // Track brace depth for body detection
        let opens = trimmed.matches('{').count() as i32;
        let closes = trimmed.matches('}').count() as i32;

        // Detect function/method start
        if is_function_start(trimmed, language) {
            in_body = true;
            body_line_count = 0;
            output.push(line);
            continue;
        }

        if in_body {
            body_line_count += 1;

            // Keep annotations/doc-comments
            let is_annotation = trimmed.starts_with("///")
                || trimmed.starts_with("//")
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
                || trimmed.starts_with("#[")
                || trimmed.starts_with("@");

            // Keep the first line of body, then compress the rest
            // Track when we're back to base depth (end of block)
            // For python-like indentation, track dedent
            let is_block_end = if language == "python" {
                // Python functions end when we see a line at the same or lesser
                // indentation than the `def` start and it's not blank/comment/annotation
                body_line_count > 1 && !trimmed.starts_with(' ')
                    && !trimmed.is_empty() && !is_annotation
                    && !trimmed.starts_with("return")
                    && !trimmed.starts_with("pass")
                    && !trimmed.starts_with("raise")
                    && !trimmed.starts_with("yield")
            } else {
                opens == 0 && closes > 0 && body_line_count > 1
            };

            if body_line_count == 1 || is_annotation || is_block_end {
                output.push(line);
            } else {
                output.push("");
            }

            if is_block_end {
                in_body = false;
            }
        } else {
            output.push(line);
        }
    }

    // Remove consecutive blank lines (keep max 2)
    let mut result: Vec<&str> = Vec::new();
    let mut blank_run = 0;
    for line in &output {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 2 {
                result.push("");
            }
        } else {
            blank_run = 0;
            result.push(line);
        }
    }

    result.join("\n")
}

/// Check if a line is an import or declaration.
fn is_declaration(trimmed: &str, _language: &str) -> bool {
    trimmed.starts_with("import ")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("mod ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("#include")
        || trimmed.starts_with("package ")
        || trimmed.starts_with("extern crate")
        || trimmed.starts_with("pub mod")
        || trimmed.starts_with("pub use")
}

/// Check if a line starts a function/method.
fn is_function_start(trimmed: &str, language: &str) -> bool {
    let fn_keywords: &[&str] = match language {
        "rust" | "go" => &["fn ", "func "],
        "python" => &["def "],
        "typescript" | "javascript" | "js" | "ts" => &["function ", "=>"],
        "java" | "c" | "cpp" | "c++" => &["", ""],
        _ => &["fn ", "def ", "function ", "func "],
    };

    if language == "java" || language == "c" || language == "cpp" || language == "c++" {
        return false;
    }

    for kw in fn_keywords {
        if !kw.is_empty() && trimmed.starts_with(kw) {
            return true;
        }
    }
    false
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_code_not_compressed() {
        let input = "fn main() {\n    println!(\"hello\");\n}";
        assert_eq!(compress_code(input, "rust"), input);
    }

    #[test]
    fn test_rust_function_body_compressed() {
        let input = r#"fn process(items: Vec<i32>) -> i32 {
    let mut sum = 0;
    for item in items {
        sum += item;
    }
    if sum > 100 {
        println!("big");
    } else {
        println!("small");
    }
    sum
}"#;
        let result = compress_code(input, "rust");
        assert!(result.contains("fn process"));
        // The body should be replaced with /* ... */
        assert!(result.contains("/* ... */"));
        // Should be shorter than original
        assert!(result.len() < input.len(), "{} >= {}", result.len(), input.len());
    }

    #[test]
    fn test_rust_long_file_compressed() {
        let input = r#"use std::collections::HashMap;

/// Process items and return results.
pub fn process(items: Vec<i32>) -> i32 {
    let mut sum = 0;
    for item in items {
        sum += item;
    }
    if sum > 100 {
        println!("big");
    } else {
        println!("small");
    }
    sum
}

fn helper() -> String {
    let x = 42;
    let y = x.to_string();
    y
}

pub struct MyStruct {
    pub field: i32,
}

impl MyStruct {
    pub fn new(val: i32) -> Self {
        Self { field: val }
    }

    pub fn get_field(&self) -> i32 {
        self.field
    }
}"#;
        let result = compress_code(input, "rust");
        // Imports preserved
        assert!(result.contains("use std::collections::HashMap;"));
        // Doc comments preserved
        assert!(result.contains("/// Process items and return results."));
        // Function signatures preserved
        assert!(result.contains("pub fn process(items: Vec<i32>) -> i32 {"));
        assert!(result.contains("fn helper() -> String {"));
        // Bodies compressed
        assert!(result.contains("/* ... */"));
        // Struct definitions preserved (not functions)
        assert!(result.contains("pub struct MyStruct"));
        assert!(result.contains("pub field: i32,"));
        // Impl blocks preserved but method bodies compressed
        assert!(result.contains("impl MyStruct {"));
        // Should be at least 30% shorter
        assert!(result.len() < (input.len() * 7 / 10),
            "Expected >30% reduction, got {} vs {} (ratio: {:.1})",
            result.len(), input.len(), input.len() as f64 / result.len().max(1) as f64);
    }

    #[test]
    fn test_python_function_compressed() {
        let input = r#"import os
import sys

def process(path: str) -> str:
    result = os.listdir(path)
    filtered = [x for x in result if x.endswith('.py')]
    return '\n'.join(filtered)

def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "."
    output = process(path)
    print(output)
    sys.exit(0)
"#;
        let result = compress_code(input, "python");
        assert!(result.contains("import os"));
        assert!(result.contains("def process(path: str) -> str:"));
        assert!(result.contains("/* ... */"));
        assert!(result.len() < input.len());
    }

    #[test]
    fn test_javascript_function_compressed() {
        let input = r#"const fs = require('fs');

function process(path) {
    const data = fs.readFileSync(path, 'utf-8');
    const lines = data.split('\n');
    return lines.filter(l => l.length > 0);
}

class Handler {
    constructor(name) {
        this.name = name;
    }

    greet() {
        return `Hello, ${this.name}!`;
    }
}

module.exports = { process, Handler };
"#;
        let result = compress_code(input, "javascript");
        assert!(result.contains("function process(path) {"));
        assert!(result.contains("/* ... */"));
        assert!(result.contains("class Handler {"));
        assert!(result.len() < input.len());
    }

    #[test]
    fn test_imports_preserved() {
        let input = "use std::collections::HashMap;\n\nfn main() {\n    let mut map = HashMap::new();\n    map.insert(1, 2);\n    println!(\"{:?}\", map);\n}";
        let result = compress_code(input, "rust");
        assert!(result.contains("use std::collections::HashMap"));
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(compress_code("", "rust"), "");
    }

    #[test]
    fn test_treesitter_fallback_on_unknown_language() {
        let input = "fn main() {\n    let x = 1;\n    let y = 2;\n    let z = x + y;\n    println!(\"{}\", z);\n}";
        // For a 6-line input, compress_via_heuristic won't compress because
        // lines.len() < 6 check (actually it's >= 6, so it should try)
        let result = compress_code(input, "unknown_lang");
        // Should still produce output (fallback)
        assert!(result.contains("fn main()"));
    }

    #[test]
    fn test_rust_repeated_function_bodies() {
        // Multiple identical functions to verify each body is compressed
        let input = r#"fn a() {
    let x = 1;
    let y = 2;
    let z = x + y;
}

fn b() {
    let a = 10;
    let b = 20;
    let c = a * b;
}

fn c() {
    println!("short");
}
"#;
        let result = compress_code(input, "rust");
        // All three functions should be compressed
        assert!(result.contains("/* ... */"));
        // First function body should be compressed
        assert!(result.contains("fn a() {"));
        assert!(result.len() < input.len());
    }
}
