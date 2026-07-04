"""Tests for the DedrooM Python bindings."""

import json
import pytest
from dedroom import DedrooM, load_config


def test_verify_returns_int():
    """verify() should return an integer verdict code."""
    engine = DedrooM("loop_detection:\n  max_repeats: 3")
    verdict = engine.verify("write_file", '{"path": "/tmp/x.txt"}')
    assert isinstance(verdict, int)
    assert 0 <= verdict <= 3


def test_first_call_allowed():
    """The first call should always be allowed (code 0)."""
    engine = DedrooM("")
    assert engine.verify("read_file", '{"path": "/tmp/x.txt"}') == 0


def test_repeated_calls_eventually_blocked():
    """After enough repeats, the call should be blocked."""
    engine = DedrooM("loop_detection:\n  max_repeats: 3")
    for _ in range(3):
        engine.verify("write_file", '{"path": "/tmp/x.txt"}')
    verdict = engine.verify("write_file", '{"path": "/tmp/x.txt"}')
    assert verdict >= 2  # BlockRetry or BlockHalt


def test_different_args_not_blocked():
    """Different arguments should not trigger loop detection."""
    engine = DedrooM("loop_detection:\n  max_repeats: 3")
    for i in range(5):
        verdict = engine.verify("write_file", json.dumps({"path": f"/tmp/x{i}.txt"}))
        assert verdict == 0  # Allowed


def test_detect_loop_function():
    """The standalone detect_loop function should work."""
    from dedroom import detect_loop
    result = detect_loop(
        "write_file", '{"path": "/tmp/x.txt"}',
        "loop_detection:\n  max_repeats: 3"
    )
    assert isinstance(result, int)


def test_compress_text_function():
    """The standalone compress_text function should work."""
    from dedroom import compress_text
    # This will raise if the compression module isn't loaded,
    # but should at least be callable
    try:
        result = compress_text("hello world", "")
        assert isinstance(result, str)
    except ImportError:
        pytest.skip("compression module not available")


def test_load_config_from_file(tmp_path):
    """load_config should read a YAML file."""
    config_file = tmp_path / "config.yaml"
    config_file.write_text("loop_detection:\n  max_repeats: 5")
    content = load_config(str(config_file))
    assert "max_repeats: 5" in content


def test_empty_config():
    """Empty config should use defaults."""
    engine = DedrooM("")
    assert engine.verify("test", "{}") == 0
