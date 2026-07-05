/// Quick compression quality check for CodeCompressor with tree-sitter.
/// Run: cargo run --example compression_quality
use dedroom_core::compression::{compress_code, estimate_tokens};

fn check(label: &str, language: &str, code: &str) {
    let original_tokens = estimate_tokens(code);
    let compressed = compress_code(code, language);
    let compressed_tokens = estimate_tokens(&compressed);
    let reduction_pct = if original_tokens > 0 {
        ((original_tokens - compressed_tokens) as f64 / original_tokens as f64) * 100.0
    } else {
        0.0
    };
    println!(
        "  {:<35} orig={:>5} tok  comp={:>5} tok  {:+.1}%  has_body_strip={}",
        label,
        original_tokens,
        compressed_tokens,
        reduction_pct,
        compressed.contains("/* ... */")
    );
}

fn main() {
    println!("═══ CodeCompressor Quality (tree-sitter AST) ═══\n");

    // ── Rust ──
    println!("── Rust ──");

    let rust_single = r#"fn process(items: Vec<i32>) -> i32 {
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
    check("single function", "rust", rust_single);

    let rust_full = r#"use std::collections::HashMap;

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
}

fn utility(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    for chunk in data.chunks(32) {
        let processed: Vec<u8> = chunk.iter().map(|b| b ^ 0xFF).collect();
        result.extend_from_slice(&processed);
    }
    Ok(result)
}
"#;
    check("full file (4 fns, 1 struct)", "rust", rust_full);

    let rust_nested = r#"fn outer() {
    let x = 1;
    let closure = || {
        let y = 2;
        y + 1
    };
    let z = closure();
    println!("{}", z);
}
"#;
    check("nested closure", "rust", rust_nested);

    // ── Python ──
    println!("\n── Python ──");

    let py_simple = r#"import os
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
    check("two functions", "python", py_simple);

    let py_class = r#"class DataProcessor:
    """Process data with various transformations."""

    def __init__(self, data: list):
        self.data = data
        self.cache = {}

    def transform(self, factor: float) -> list:
        result = []
        for item in self.data:
            transformed = item * factor
            result.append(transformed)
        return result

    def analyze(self) -> dict:
        stats = {
            "mean": sum(self.data) / len(self.data),
            "max": max(self.data),
            "min": min(self.data),
            "count": len(self.data),
        }
        return stats
"#;
    check("class with methods", "python", py_class);

    let py_decorated = r#"import time
import functools

def timer(func):
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        start = time.time()
        result = func(*args, **kwargs)
        elapsed = time.time() - start
        print(f"{func.__name__} took {elapsed:.2f}s")
        return result
    return wrapper

@timer
def slow_function(n: int) -> int:
    total = 0
    for i in range(n):
        total += i * i
    return total
"#;
    check("decorated function", "python", py_decorated);

    // ── JavaScript ──
    println!("\n── JavaScript ──");

    let js_simple = r#"const fs = require('fs');

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
    check("function + class", "javascript", js_simple);

    // ── Fallback (no tree-sitter grammar) ──
    println!("\n── Fallback (heuristic) ──");

    let go_code = r#"package main

import "fmt"

func main() {
    items := []int{1, 2, 3, 4, 5}
    for i, v := range items {
        fmt.Printf("item[%d] = %d\n", i, v)
    }
    fmt.Println("done")
}

func helper(x int) int {
    result := x * 2
    if result > 100 {
        return result
    }
    return 0
}
"#;
    check("Go (heuristic fallback)", "go", go_code);

    println!("\n═══ Done ═══");
}
