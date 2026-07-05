fn main() {
    let input = "use std::collections::HashMap;\n\npub fn test_0() {\n    let x = 0;\n    println!(\"Hello {}\", x);\n}\n";
    let compressed = dedroom_core::compression::code_compressor::compress_code(input, "auto");
    println!("ORIGINAL:\n{}", input);
    println!("COMPRESSED:\n{}", compressed);
}
