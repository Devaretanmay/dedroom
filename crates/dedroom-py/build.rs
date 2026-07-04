//! DedrooM Python bindings build script.
//!
//! On macOS with Homebrew Python, the Python library is installed as a
//! framework. PyO3's build script handles standard library paths, but on
//! macOS we need to tell cargo where to find the framework.
//! The `abi3-py312` feature handles most of this — this is just a fallback
//! for systems where the framework path isn't automatically discovered.

fn main() {
    #[cfg(target_os = "macos")]
    {
        // Try to discover Python framework via python3-config first.
        // Note: `-framework CoreFoundation` is common in the output but not
        // what we need — we specifically check for `-framework Python`.
        let has_python_framework = std::process::Command::new("python3-config")
            .args(["--ldflags", "--embed"])
            .output()
            .ok()
            .map(|o| {
                let out = String::from_utf8_lossy(&o.stdout);
                out.contains("-framework Python")
            })
            .unwrap_or(false);

        if !has_python_framework {
            // Fallback: common Homebrew framework paths
            let paths = [
                "/opt/homebrew/opt/python@3.14/Frameworks",
                "/opt/homebrew/opt/python@3.13/Frameworks",
                "/opt/homebrew/opt/python@3.12/Frameworks",
                "/opt/homebrew/opt/python@3/Frameworks",
                "/opt/homebrew/opt/python/Frameworks",
            ];

            for path in &paths {
                if std::path::Path::new(path).join("Python.framework").exists() {
                    println!("cargo:rustc-link-search=framework={}", path);
                    println!("cargo:rustc-link-lib=framework=Python");
                    return;
                }
            }

            println!("cargo:warning=Python framework not found. Try: PYO3_PYTHON=/path/to/python3");
        }
    }
}
