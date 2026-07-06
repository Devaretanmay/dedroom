#!/usr/bin/env python3
"""
Security Audit Agent with DedrooM Guardrails
LangChain + Groq/Ollama/OpenRouter
"""

import os
import sys
import json
import logging
import tempfile
from pathlib import Path
from typing import List, Dict, Any
from datetime import datetime
from langchain_openai import ChatOpenAI
from langgraph.prebuilt import create_react_agent
from langchain.tools import tool

from dedroom import DedrooM

# ----------------------------------------------------------------------
# Configuration
# ----------------------------------------------------------------------
logging.basicConfig(level=logging.INFO, format="%(levelname)s - %(message)s")
logger = logging.getLogger("audit-agent")

# Environment variables for LLM provider
LLM_BASE = os.getenv("OPENAI_API_BASE", "https://api.groq.com/openai/v1")
LLM_KEY = os.getenv("OPENAI_API_KEY", "")
LLM_MODEL = os.getenv("OPENAI_MODEL", "llama-3.3-70b-versatile")

if "localhost" in LLM_BASE or "11434" in LLM_BASE:
    LLM_KEY = "ollama"

# ----------------------------------------------------------------------
# DedrooM Pipeline
# ----------------------------------------------------------------------
DEDROOM_CONFIG = r"""
compression:
  compressors:
    code_compressor: true
    smart_crusher: true
    log_compressor: true
  min_input_length: 100

redaction:
  patterns:
    - 'sk-[A-Za-z0-9]{48}'
    - 'ghp_[A-Za-z0-9]{36}'
    - 'password\s*=\s*[''"][^''"]+[''"]'
    - 'mongodb://[^:]+:[^@]+@'
    - '\b[A-Z]{2}[0-9]{6}\b'

loop_detection:
  max_repeats: 3
  adaptive:
    enabled: true
    error_reduction: 1

persistence:
  backend: "sqlite"
  path: "./dedroom_audit.db"
"""

pipeline = DedrooM(DEDROOM_CONFIG)

# ----------------------------------------------------------------------
# Generate a Realistic Test Repository
# ----------------------------------------------------------------------
def create_test_repo() -> Path:
    """Create a temporary directory with realistic source files."""
    repo = Path(tempfile.mkdtemp(prefix="audit_repo_"))
    logger.info("Test repository created at %s", repo)

    (repo / "app.py").write_text("""
import os
from flask import Flask, request

app = Flask(__name__)

# Hardcoded secret - security risk!
ADMIN_KEY = 'sk-12345-abcde-67890'

# Database connection - password exposed
DB_URI = 'mongodb://admin:SuperSecurePass@prod-db:27017/mydb'

@app.route('/')
def home():
    return 'Hello'

@app.route('/users')
def users():
    # Fetch users from DB
    return '[]'

if __name__ == '__main__':
    app.run()
""")

    (repo / "Dockerfile").write_text("""
FROM python:3.11-slim
WORKDIR /app
COPY . .
# Running as root - security risk!
RUN pip install -r requirements.txt
CMD ["python", "app.py"]
""")

    (repo / "requirements.txt").write_text("""
flask==2.0.1
requests==2.25.0
jinja2==3.0.0
""")

    (repo / ".env").write_text("""
SECRET_KEY=ghp_abcdefghijklmnopqrstuvwxyz1234567890
DB_PASS=SuperSecurePass
""")

    # Large deployment log (to test compression)
    with open(repo / "deploy_log.txt", "w") as f:
        for i in range(300):
            f.write(
                f"[{i}] INFO: Deploying container to cluster-{i%3}\n"
                f"[{i}] INFO: Health check passed for service-{i%5}\n"
                f"[{i}] WARN: Memory usage 85%\n"
                f"[{i}] INFO: Completed step {i}\n"
            )

    return repo

# ----------------------------------------------------------------------
# LangChain Tools with DedrooM
# ----------------------------------------------------------------------
audit_tool_calls = []

@tool
def list_files() -> str:
    """List all files in the root of the repository."""
    audit_tool_calls.append("list_files()")
    files = os.listdir(str(REPO_DIR))
    return "\n".join(files)

@tool
def read_file(filename: str) -> str:
    """Read a file from the repository."""
    call_id = f"read_file('{filename}')"
    audit_tool_calls.append(call_id)

    path = REPO_DIR / filename
    if not path.exists():
        return f"Error: {filename} not found"

    raw = path.read_text()

    # 1. Redact secrets
    redacted = pipeline.redact(raw)
    if redacted != raw:
        logger.info("Redacted secrets from %s", filename)

    # 2. Compress large content
    compressed = pipeline.compress(redacted, content_type="code" if filename.endswith(".py") else "logs")
    original_len = len(raw)
    compressed_len = len(compressed)
    if original_len > 200:
        saved = round((1 - compressed_len / original_len) * 100, 1)
        logger.info("Compressed %s: %d → %d chars (saved %s%%)",
                    filename, original_len, compressed_len, saved)

    # 3. Loop detection (passive; blocks only if 3rd identical read)
    verdict = pipeline.verify("read_file", filename)
    if verdict >= 2:
        logger.warning("Blocked duplicate read of %s (call #%d)",
                       filename, len(audit_tool_calls))
        return "[DedrooM] This file has been read twice already. Please use the existing summary."

    return compressed

@tool
def search_vulnerabilities() -> str:
    """Search known vulnerability databases for dependency issues."""
    audit_tool_calls.append("search_vulnerabilities()")
    # Simulated vulnerability DB
    vulns = {
        "flask==2.0.1": "CVE-2021-XXXX: Path Traversal vulnerability (High)",
        "requests==2.25.0": "CVE-2021-YYYY: SSL Certificate Verification Bypass (Medium)"
    }
    return json.dumps(vulns, indent=2)

# ----------------------------------------------------------------------
# Build the LangChain Agent
# ----------------------------------------------------------------------
def build_agent(llm, tools):
    return create_react_agent(llm, tools)

# ----------------------------------------------------------------------
# Run the Audit
# ----------------------------------------------------------------------
def run_audit(repo_path: Path) -> Dict[str, Any]:
    global REPO_DIR
    REPO_DIR = repo_path

    # Override list_files to use the actual repo path
    list_files.func.__defaults__ = (str(REPO_DIR),)

    tools = [list_files, read_file, search_vulnerabilities]

    llm = ChatOpenAI(base_url=LLM_BASE, api_key=LLM_KEY, model=LLM_MODEL, temperature=0)
    logger.info("Using LLM: %s @ %s", LLM_MODEL, LLM_BASE)

    executor = build_agent(llm, tools)

    query = (
        "You are a Security Auditor. You have access to a code repository.\n"
        "Your task is to identify:\n"
        "1. Hardcoded secrets / exposed credentials\n"
        "2. Insecure Docker configurations (e.g., running as root)\n"
        "3. Vulnerable dependencies\n\n"
        "Use the provided tools to gather information. Be thorough but efficient.\n"
        "Once you have all findings, produce a final report with:\n"
        "- Severity (Critical, High, Medium, Low)\n"
        "- Description\n"
        "- Recommended fix\n\n"
        "Question: Audit this repository for security issues. Provide a final report with risks and actionable recommendations."
    )
    logger.info("Running audit...")
    result = executor.invoke({"messages": [("user", query)]})
    return {
        "report": result["messages"][-1].content,
        "tool_calls": audit_tool_calls.copy(),
        "timestamp": datetime.utcnow().isoformat() + "Z"
    }

# ----------------------------------------------------------------------
# Main Entry Point
# ----------------------------------------------------------------------
def main():
    repo = create_test_repo()
    try:
        audit_result = run_audit(repo)

        # Print final report in a structured way
        print("\n" + "=" * 80)
        print("SECURITY AUDIT REPORT")
        print("=" * 80)
        print(audit_result["report"])
        print("\n" + "=" * 80)
        print("DEDROOM INTERVENTION SUMMARY")
        print("=" * 80)
        print(f"Tool calls: {len(audit_result['tool_calls'])}")
        for idx, call in enumerate(audit_result['tool_calls'], 1):
            print(f"  {idx}. {call}")

        # Detect duplicates
        reads = [c for c in audit_result['tool_calls'] if c.startswith("read_file")]
        seen = {}
        duplicates = []
        for c in reads:
            if c in seen:
                seen[c] += 1
                if seen[c] >= 3:
                    duplicates.append(c)
            else:
                seen[c] = 1

        if duplicates:
            print(f"\nDedrooM blocked natural loops on: {', '.join(set(duplicates))}")
        else:
            print("\nNo loops detected. DedrooM provided compression and redaction only.")
        print("Logs saved to ./dedroom_audit.db")

    finally:
        # Optionally keep the repo for inspection
        print(f"\nRepository kept at: {repo} (remove manually if not needed)")

if __name__ == "__main__":
    main()
