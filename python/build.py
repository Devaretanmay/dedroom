"""Build script: compiles Rust binaries and bundles them into the wheel.

Run this before `maturin build` to include the CLI binaries:
    python python/build.py && maturin build --release -m crates/dedroom-py/Cargo.toml
"""

import os
import shutil
import subprocess
import sys

PROJECT_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "dedroom", "bin")


def build_binaries():
    """Build the dedroom-cli and dedroom-proxy Rust binaries."""
    print(f"  Building Rust CLI binaries...", flush=True)

    # Build both CLI crates in release mode
    result = subprocess.run(
        ["cargo", "build", "--release", "-p", "dedroom-cli", "-p", "dedroom-proxy"],
        cwd=PROJECT_ROOT,
        capture_output=True,
        text=True,
    )

    if result.returncode != 0:
        print(f"  ERROR: Rust build failed:", flush=True)
        print(result.stderr, flush=True)
        sys.exit(1)

    print(f"  Rust build succeeded.", flush=True)

    # Determine binary names based on platform
    is_windows = sys.platform.startswith("win")
    cli_name = "dedroom.exe" if is_windows else "dedroom"
    proxy_name = "dedroom-proxy.exe" if is_windows else "dedroom-proxy"

    # Source paths (release builds)
    release_dir = os.path.join(PROJECT_ROOT, "target", "release")
    cli_src = os.path.join(release_dir, cli_name)
    proxy_src = os.path.join(release_dir, proxy_name)

    # Ensure bin directory exists
    os.makedirs(BIN_DIR, exist_ok=True)

    # Copy binaries
    for src, name in [(cli_src, cli_name), (proxy_src, proxy_name)]:
        if not os.path.exists(src):
            # Fall back to debug build
            debug_src = os.path.join(PROJECT_ROOT, "target", "debug", name)
            if os.path.exists(debug_src):
                src = debug_src
            else:
                print(f"  ERROR: {name} binary not found. Build failed.", flush=True)
                sys.exit(1)

        dst = os.path.join(BIN_DIR, name)
        shutil.copy2(src, dst)
        os.chmod(dst, 0o755)
        print(f"  ✅ Bundled {name} ({os.path.getsize(dst)} bytes)", flush=True)


if __name__ == "__main__":
    build_binaries()
