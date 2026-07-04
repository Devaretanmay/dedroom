"""DedrooM — unified agent runtime.

Combines loop detection and context compression in a single pipeline
to minimize cost and wasted API calls for autonomous AI agents.

Usage:
    from dedroom import DedrooM
    engine = DedrooM(config_yaml)
    verdict = engine.verify("write_file", '{"path": "/tmp/x.txt"}')
"""

from dedroom._core import DedrooM
from dedroom._core import detect_loop, compress_text

__all__ = ["DedrooM", "detect_loop", "compress_text"]


def load_config(path: str) -> str:
    """Load a YAML config file as a string.

    Args:
        path: Path to YAML configuration file.

    Returns:
        YAML config as a string.
    """
    with open(path) as f:
        return f.read()
