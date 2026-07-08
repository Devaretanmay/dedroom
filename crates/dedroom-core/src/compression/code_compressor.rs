//! CodeCompressor — AST-aware code compression via tree-sitter.
//!
//! Preserves imports, signatures, type annotations, doc comments, and
//! top-level structure while compressing function/method/class bodies.

use std::cell::RefCell;
use std::collections::HashMap;
use tree_sitter::{Parser, Language};

thread_local! {
    static PARSER_CACHE: RefCell<HashMap<String, Parser>> = RefCell::new(HashMap::new());
}

fn take_or_create_parser(language: &str) -> Option<Parser> {
    let lang_key = language.to_string();
    let cached = PARSER_CACHE.with(|cache| cache.borrow_mut().remove(&lang_key));
    if let Some(p) = cached {
        return Some(p);
    }
    let lang = resolve_language(language)?;
    let mut parser = Parser::new();
    parser.set_language(&lang).ok()?;
    Some(parser)
}

fn return_parser(language: &str, parser: Parser) {
    let lang_key = language.to_string();
    PARSER_CACHE.with(|cache| {
        cache.borrow_mut().insert(lang_key, parser);
    });
}

/// Compress source code by stripping function bodies. Uses tree-sitter.
/// Returns input unchanged if language grammar unavailable or too short.
pub fn compress_code(input: &str, language: &str) -> String {
    if input.len() < 30 || input.lines().count() < 6 {
        return input.to_string();
    }
    compress_via_treesitter(input, language).unwrap_or_else(|| input.to_string())
}

fn compress_via_treesitter(input: &str, language: &str) -> Option<String> {
    let mut parser = take_or_create_parser(language)?;
    let tree = parser.parse(input, None)?;
    let root = tree.root_node();

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

    let mut result = String::with_capacity(input.len());
    let mut last_end = 0usize;
    for range in &body_ranges {
        result.push_str(&input[last_end..range.start]);
        result.push_str("/* ... */");
        last_end = range.end;
    }
    result.push_str(&input[last_end..]);
    return_parser(language, parser);
    Some(result)
}

/// Detect programming language from content heuristics.
/// Returns one of "rust", "python", "javascript", or "unknown".
pub fn detect_language(content: &str) -> &'static str {
    let lines: Vec<&str> = content.lines().take(10).collect();

    // Check shebang first
    if let Some(first) = lines.first() {
        let trimmed = first.trim();
        if trimmed.starts_with("#!/") {
            if trimmed.contains("python") {
                return "python";
            }
            if trimmed.contains("node") {
                return "javascript";
            }
            if trimmed.contains("bash") || trimmed.contains("sh") || trimmed.contains("zsh") {
                return "unknown"; // no tree-sitter grammar for shell
            }
            if trimmed.contains("perl") || trimmed.contains("ruby") {
                return "unknown";
            }
        }
    }

    // Keyword-based detection from first 10 non-empty lines
    for line in &lines {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Rust: function items, use statements, impl blocks, pub items, struct/enum/trait/type definitions
        if t.starts_with("fn ")
            || t.starts_with("pub ")
            || t.starts_with("impl")
            || t.starts_with("use ")
            || t.starts_with("struct ")
            || t.starts_with("enum ")
            || t.starts_with("trait ")
            || t.starts_with("type ")
            || t.starts_with("let ")
            || t.starts_with("const ")
            || t.starts_with("async fn")
            || t.starts_with("unsafe ")
            || t.starts_with("#[")
            || t.starts_with("mod ")
        {
            return "rust";
        }
        // Python: def, class, import, from, async def, @decorator, if __name__, with
        if t.starts_with("def ")
            || t.starts_with("class ")
            || t.starts_with("import ")
            || t.starts_with("from ")
            || t.starts_with("async def")
            || t.starts_with("@")
            || t == "if __name__ == '__main__':"
        {
            return "python";
        }
        // JavaScript/TypeScript: function, const/let + arrow, import from, export, require, module.exports
        if t.starts_with("function ")
            || t.starts_with("export ")
            || t.starts_with("import ")
            || t.starts_with("require(")
            || t.starts_with("module.")
            || t.contains("=>")
            || t.starts_with("interface ")
            || t.starts_with("type ")
            || t.starts_with("class ")
        {
            return "javascript";
        }
        // Generic shell/config — skip
        if t.starts_with("#") || t.starts_with("//") || t.starts_with("echo ") {
            continue;
        }
    }

    // Fallback: look for multi-line Rust-style sigils in the whole content
    if content.contains("::") || content.contains("fn ") {
        return "rust";
    }

    "unknown"
}

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

fn collect_rust_bodies(node: tree_sitter::Node, source: &str, ranges: &mut Vec<std::ops::Range<usize>>) {
    let _ = source;
    match node.kind() {
        "function_item" | "function_signature" | "method_implementation" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "block" && child.end_byte() - child.start_byte() > 4 {
                    ranges.push(child.start_byte() + 1..child.end_byte() - 1);
                }
            }
        }
        "string_literal" | "line_comment" | "block_comment" => {}
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_rust_bodies(child, source, ranges);
            }
        }
    }
}

fn collect_python_bodies(node: tree_sitter::Node, source: &str, ranges: &mut Vec<std::ops::Range<usize>>) {
    let _ = source;
    match node.kind() {
        "decorated_definition" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_python_bodies(child, source, ranges);
            }
        }
        "function_definition" | "class_definition" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if (child.kind() == "block" || child.kind() == "body") && child.end_byte() - child.start_byte() > 4 {
                    ranges.push(child.start_byte()..child.end_byte());
                }
            }
        }
        "string" | "comment" => {}
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_python_bodies(child, source, ranges);
            }
        }
    }
}

fn collect_js_bodies(node: tree_sitter::Node, source: &str, ranges: &mut Vec<std::ops::Range<usize>>) {
    let _ = source;
    let is_function = matches!(node.kind(),
        "function_declaration" | "method_definition" | "arrow_function"
        | "class_declaration" | "generator_function_declaration"
    );
    if is_function {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if (child.kind() == "statement_block" || child.kind() == "class_body")
                && child.end_byte() - child.start_byte() > 4
            {
                ranges.push(child.start_byte() + 1..child.end_byte() - 1);
            }
        }
        return;
    }
    if matches!(node.kind(), "string" | "comment" | "template_string") {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_js_bodies(child, source, ranges);
    }
}

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
        let input = "fn process(items: Vec<i32>) -> i32 {\n    let mut sum = 0;\n    for item in items {\n        sum += item;\n    }\n    if sum > 100 {\n        println!(\"big\");\n    } else {\n        println!(\"small\");\n    }\n    sum\n}";
        let result = compress_code(input, "rust");
        assert!(result.contains("fn process"));
        assert!(result.contains("/* ... */"));
        assert!(result.len() < input.len());
    }

    #[test]
    fn test_rust_long_file_compressed() {
        let input = "use std::collections::HashMap;\n\n/// Process items and return results.\npub fn process(items: Vec<i32>) -> i32 {\n    let mut sum = 0;\n    for item in items {\n        sum += item;\n    }\n    if sum > 100 {\n        println!(\"big\");\n    } else {\n        println!(\"small\");\n    }\n    sum\n}\n\nfn helper() -> String {\n    let x = 42;\n    let y = x.to_string();\n    y\n}\n\npub struct MyStruct {\n    pub field: i32,\n}\n\nimpl MyStruct {\n    pub fn new(val: i32) -> Self {\n        Self { field: val }\n    }\n\n    pub fn get_field(&self) -> i32 {\n        self.field\n    }\n}";
        let result = compress_code(input, "rust");
        assert!(result.contains("use std::collections::HashMap;"));
        assert!(result.contains("/// Process items and return results."));
        assert!(result.contains("pub fn process(items: Vec<i32>) -> i32 {"));
        assert!(result.contains("/* ... */"));
        assert!(result.contains("pub struct MyStruct"));
        assert!(result.len() < input.len());
    }

    #[test]
    fn test_unknown_language_unchanged() {
        let input = "fn main() {\n    let x = 1;\n    let y = 2;\n    let z = x + y;\n    println!(\"{}\", z);\n}";
        // Unknown language -> no tree-sitter grammar -> returns input unchanged
        assert_eq!(compress_code(input, "unknown"), input);
    }
}
