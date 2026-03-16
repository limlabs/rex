"""
Evaluator: runs checks defined in task specs against a built & served Rex project.

Checks are evaluated in order. Gate checks (build, serve) stop evaluation early
on failure — there's no point checking DOM if the build didn't succeed.
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path

import requests
from selectolax.parser import HTMLParser

from .agent import AgentMetrics
from .conditions import Condition


@dataclass
class CheckResult:
    name: str
    passed: bool
    detail: str = ""


@dataclass
class EvalResult:
    task_id: str
    condition: str
    checks: list[CheckResult] = field(default_factory=list)
    metrics: AgentMetrics | None = None

    @property
    def passed(self) -> bool:
        return all(c.passed for c in self.checks)

    @property
    def score(self) -> float:
        if not self.checks:
            return 0.0
        return sum(c.passed for c in self.checks) / len(self.checks)


def evaluate(
    task: dict,
    condition: Condition,
    workdir: Path,
    metrics: AgentMetrics,
) -> EvalResult:
    """Run all checks for a task against a project directory."""
    result = EvalResult(
        task_id=task["task"]["id"],
        condition=condition.name,
        metrics=metrics,
    )
    checks = task["checks"]

    # --- Gate: build ---
    if checks.get("build"):
        ok, detail = _check_build(condition.build_cmd, workdir, condition.name)
        result.checks.append(CheckResult("build", ok, detail))
        if not ok:
            return result

    # --- Gate: serve ---
    server = None
    port = None
    if checks.get("serve"):
        server, port, detail = _start_server(condition.serve_cmd, workdir, condition.name)
        ok = server is not None
        result.checks.append(CheckResult("serve", ok, detail))
        if not ok:
            return result

    try:
        base = f"http://localhost:{port}"

        for check in checks.get("http", []):
            result.checks.append(_check_http(base, check))

        for check in checks.get("ssr", []):
            result.checks.append(_check_ssr(base, check))

        for check in checks.get("dom", []):
            result.checks.append(_check_dom(base, check))

        for check in checks.get("file", []):
            result.checks.append(_check_file(workdir, check))

    finally:
        if server:
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()

    return result


# ---------------------------------------------------------------------------
# Check implementations
# ---------------------------------------------------------------------------


def _check_build(
    cmd: list[str], workdir: Path, condition_name: str = "rex_raw"
) -> tuple[bool, str]:
    """Run the build command, return (success, detail)."""
    full_cmd = cmd[:]
    if condition_name.startswith("rex"):
        full_cmd += ["--root", str(workdir)]
    try:
        proc = subprocess.run(
            full_cmd,
            cwd=workdir,
            capture_output=True,
            text=True,
            timeout=120,
            env={**os.environ, "NODE_ENV": "production"},
        )
        if proc.returncode == 0:
            return True, "build succeeded"
        stderr = proc.stderr.strip()[:500]
        return False, f"exit {proc.returncode}: {stderr}"
    except subprocess.TimeoutExpired:
        return False, "build timed out (120s)"


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("", 0))
        return s.getsockname()[1]


def _build_serve_cmd(cmd: list[str], port: int, workdir: Path, condition_name: str) -> list[str]:
    """Build the full serve command with port and any condition-specific flags."""
    if condition_name.startswith("rex"):
        return cmd + ["--port", str(port), "--root", str(workdir)]
    elif condition_name == "tanstack_raw":
        return cmd + ["--port", str(port), "--host", "127.0.0.1"]
    else:
        # Next.js, Remix: use --port
        return cmd + ["--port", str(port)]


def _start_server(
    cmd: list[str], workdir: Path, condition_name: str = "rex_raw"
) -> tuple[subprocess.Popen | None, int | None, str]:
    """Start the server, wait for it to respond, return (process, port, detail)."""
    port = _find_free_port()
    full_cmd = _build_serve_cmd(cmd, port, workdir, condition_name)

    proc = subprocess.Popen(
        full_cmd,
        cwd=workdir,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={**os.environ, "PORT": str(port)},
    )

    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        try:
            r = requests.get(f"http://localhost:{port}/", timeout=1)
            # Any response means the server is up (even 404 is fine)
            return proc, port, f"listening on :{port}"
        except (requests.ConnectionError, requests.Timeout):
            pass

        # Check if process died
        if proc.poll() is not None:
            stderr = proc.stderr.read().decode() if proc.stderr else ""
            return None, None, f"server exited with {proc.returncode}: {stderr[:300]}"

        time.sleep(0.25)

    proc.terminate()
    return None, None, "server failed to start within 30s"


def _check_http(base: str, check: dict) -> CheckResult:
    """Check HTTP status code (and optional JSON assertions)."""
    method = check.get("method", "GET").upper()
    url = base + check["path"]
    body = check.get("body")
    headers = {"Content-Type": "application/json"} if body else {}
    expected_status = check["status"]

    name = f"http {method} {check['path']} -> {expected_status}"

    try:
        r = requests.request(method, url, data=body, headers=headers, timeout=10)
    except requests.RequestException as e:
        return CheckResult(name, False, f"request failed: {e}")

    if r.status_code != expected_status:
        return CheckResult(name, False, f"got {r.status_code}")

    return CheckResult(name, True)


def _strip_html_comments(html: str) -> str:
    """Remove HTML comments like <!-- --> that React injects between text nodes."""
    import re

    return re.sub(r"<!--.*?-->", "", html)


def _check_ssr(base: str, check: dict) -> CheckResult:
    """Fetch raw HTML (no JS execution) and verify content strings are present."""
    path = check["path"]
    name = f"ssr {path}"

    try:
        r = requests.get(base + path, timeout=10)
    except requests.RequestException as e:
        return CheckResult(name, False, f"request failed: {e}")

    # Strip React's HTML comments (<!-- -->) so text comparisons work
    html = _strip_html_comments(r.text)
    contains = check["contains"]
    if isinstance(contains, str):
        contains = [contains]

    missing = [s for s in contains if s not in html]
    if missing:
        return CheckResult(name, False, f"missing in HTML: {missing}")
    return CheckResult(name, True)


def _check_dom(base: str, check: dict) -> CheckResult:
    """Parse HTML and check DOM selectors using selectolax."""
    path = check["path"]
    selector = check["selector"]
    name = f"dom {path} {selector}"

    try:
        r = requests.get(base + path, timeout=10)
    except requests.RequestException as e:
        return CheckResult(name, False, f"request failed: {e}")

    # Strip React HTML comments before parsing so text extraction is clean
    clean_html = _strip_html_comments(r.text)
    tree = HTMLParser(clean_html)
    nodes = tree.css(selector)

    # Existence check
    if "exists" in check:
        found = len(nodes) > 0
        if found != check["exists"]:
            return CheckResult(name, False, f"exists={found}, expected={check['exists']}")
        return CheckResult(name, True)

    # Selector must be found for text/contains checks
    if not nodes:
        return CheckResult(name, False, "selector not found")

    # For "text" check, use the first matching node (exact match)
    if "text" in check:
        text = nodes[0].text(strip=True)
        if text != check["text"]:
            return CheckResult(name, False, f"text={text!r}, expected={check['text']!r}")

    # For "contains" check, search across ALL matching nodes
    if "contains" in check:
        all_text = " ".join(n.text(strip=True) for n in nodes)
        if check["contains"] not in all_text:
            return CheckResult(
                name,
                False,
                f"text across {len(nodes)} nodes does not contain {check['contains']!r}",
            )

    return CheckResult(name, True)


def _check_file(workdir: Path, check: dict) -> CheckResult:
    """Check a file exists and optionally assert on JSON content."""
    rel = check["path"]
    path = workdir / rel
    name = f"file {rel}"

    if not path.exists():
        return CheckResult(name, False, "file does not exist")

    if "json_path" in check:
        try:
            data = json.loads(path.read_text())
        except json.JSONDecodeError as e:
            return CheckResult(name, False, f"invalid JSON: {e}")

        # Simple json_path support: only $ (root) and $[N].key
        jp = check["json_path"]
        val = _simple_jsonpath(data, jp)

        if "json_equals" in check and val != check["json_equals"]:
            return CheckResult(name, False, f"{jp} = {val!r}, expected {check['json_equals']!r}")

    return CheckResult(name, True)


def _simple_jsonpath(data, path: str):
    """Minimal jsonpath: supports $, $[N], $[N].key, $.key patterns."""
    if path == "$":
        return data

    parts = path.lstrip("$").lstrip(".").split(".")
    current = data
    for part in parts:
        if not part:
            continue
        if "[" in part:
            key = part.split("[")[0]
            idx = int(part.split("[")[1].rstrip("]"))
            if key:
                current = current[key]
            current = current[idx]
        else:
            current = current[part]
    return current
