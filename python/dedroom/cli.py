"""DedrooM CLI — wraps AI agents with loop detection and context compression.

When installed via pip, this module provides the `dedroom` CLI command.
It finds the compiled Rust binary (built alongside the Python extension)
and delegates all arguments to it.
"""

import os
import subprocess
import sys
from pathlib import Path


def _is_python_script(path: Path) -> bool:
    """Check if a file is a Python script (not a compiled binary)."""
    try:
        with open(path, "rb") as f:
            header = f.read(32)
        # Python scripts start with shebang containing "python"
        return b"#!" in header and b"python" in header.lower()
    except (OSError, IOError):
        return False


def _find_cargo_bin(name: str) -> Path | None:
    """Check ~/.cargo/bin/ for a binary (cargo install location)."""
    cargo_bin = Path.home() / ".cargo" / "bin"
    for candidate_name in (name, f"{name}.exe"):
        candidate = cargo_bin / candidate_name
        if candidate.exists() and not _is_python_script(candidate):
            return candidate
    return None


def _find_bundled_binary() -> Path | None:
    """Check for bundled CLI binaries in the package's bin/ directory.

    When the wheel is built with bundled binaries (via python/build.py),
    they are placed at dedroom/bin/dedroom and dedroom/bin/dedroom-proxy.
    This is checked first so `pip install dedroom` gives a fully working CLI.
    """
    try:
        import dedroom as _pkg

        pkg_dir = Path(_pkg.__file__).resolve().parent
        for name in ("dedroom", "dedroom-cli", "dedroom-cli.exe"):
            candidate = pkg_dir / "bin" / name
            if candidate.exists() and not _is_python_script(candidate):
                return candidate
    except (ImportError, AttributeError):
        pass
    return None


def _find_binary() -> Path:
    """Find the dedroom Rust binary.

    Search order:
    1. DEDROOM_BINARY env var (explicit override)
    2. Bundled in the pip package (dedroom/bin/)
    3. Next to the compiled _core extension (maturin build)
    4. ~/.cargo/bin/ (cargo install location)
    5. In PATH (skipping any Python wrapper scripts to avoid recursion)
    6. Next to the current Python executable (development install)
    7. Cargo target directory (development)
    """
    # 0. Env var override
    env_override = os.environ.get("DEDROOM_BINARY")
    if env_override:
        candidate = Path(env_override)
        if candidate.exists():
            return candidate

    # 1. Bundled in the pip package (dedroom/bin/)
    bundled = _find_bundled_binary()
    if bundled is not None:
        return bundled

    # 2. Check next to the compiled _core extension
    try:
        import dedroom._core as _core

        core_path = Path(_core.__file__).resolve().parent
        for name in ("dedroom", "dedroom-cli", "dedroom-cli.exe"):
            candidate = core_path / name
            if candidate.exists() and not _is_python_script(candidate):
                return candidate
    except (ImportError, AttributeError):
        pass

    # 3. Check ~/.cargo/bin/ (cargo install puts binaries here)
    result = _find_cargo_bin("dedroom") or _find_cargo_bin("dedroom-cli")
    if result is not None:
        return result

    # 4. Check in PATH (skip Python wrapper scripts to avoid recursion)
    search_names = ["dedroom-cli", "dedroom"]
    for path_str in os.environ.get("PATH", "").split(os.pathsep):
        path = Path(path_str)
        if not path.is_dir():
            continue
        for name in search_names:
            for candidate in (path / name, path / f"{name}.exe"):
                if candidate.exists() and not _is_python_script(candidate):
                    return candidate

    # 5. Check next to the Python interpreter (common in dev setups)
    python_dir = Path(sys.executable).parent
    for name in ("dedroom", "dedroom-cli", "dedroom-cli.exe"):
        candidate = python_dir / name
        if candidate.exists() and not _is_python_script(candidate):
            return candidate

    # 6. Check Cargo target directory (development)
    cwd = Path.cwd().resolve()
    for parent in [cwd] + list(cwd.parents):
        if (parent / "Cargo.toml").exists():
            for profile in ("release", "debug"):
                for name in ("dedroom-cli", "dedroom"):
                    candidate = parent / "target" / profile / name
                    if candidate.exists() and not _is_python_script(candidate):
                        return candidate
                    candidate_exe = parent / "target" / profile / f"{name}.exe"
                    if candidate_exe.exists() and not _is_python_script(candidate_exe):
                        return candidate_exe
            break

    raise FileNotFoundError(
        "Cannot find dedroom binary. "
        "Make sure it's built: cd dedroom && cargo build -p dedroom-cli\n"
        "Or set DEDROOM_BINARY env var to the path of the binary."
    )


def main() -> None:
    """Entry point for the `dedroom` CLI command.

    Finds the compiled Rust binary and delegates all arguments to it.
    """
    try:
        binary = _find_binary()
        
        # Ensure all bundled binaries are executable (pip sometimes strips this)
        bin_dir = binary.parent
        for f in bin_dir.iterdir():
            if f.is_file() and not os.access(f, os.X_OK):
                try:
                    os.chmod(f, 0o755)
                except OSError:
                    pass
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

    # Forward all arguments except the program name
    args = sys.argv[1:]

    try:
        proc = subprocess.run(
            [str(binary), *args],
            stdin=sys.stdin,
            stdout=sys.stdout,
            stderr=sys.stderr,
        )
        sys.exit(proc.returncode)
    except FileNotFoundError:
        print(
            f"Error: Binary not found at {binary}. "
            f"Reinstall or rebuild: cargo build -p dedroom-cli",
            file=sys.stderr,
        )
        sys.exit(1)
    except PermissionError:
        print(f"Error: Binary at {binary} is not executable. Run: chmod +x {binary}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
