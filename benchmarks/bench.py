#!/usr/bin/env python3
"""
Rex Benchmark Suite — Rex vs Next.js 16 (Pages & App Router) vs TanStack Start vs Vinext

Compares frameworks on identical page fixtures across three suites:
  dx      Developer experience: install time, deps, startup, rebuild
  server  Production: build time, throughput (RPS), latency, memory
  client  Client-side: JS bundle size, Lighthouse Web Vitals

Usage:
  uv run python bench.py                              # all suites, all frameworks
  uv run python bench.py --suite server               # production benchmarks only
  uv run python bench.py --suite dx,server            # multiple suites
  uv run python bench.py --framework rex              # one framework only
  uv run python bench.py --json results.json          # write JSON output
  uv run python bench.py --requests 10000 --concurrency 100
  uv run python bench.py --iterations 5                     # median of 5 runs for noisy metrics
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import socket
import statistics
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

# ── Paths ───────────────────────────────────────────────────────

SCRIPT_DIR = Path(__file__).parent.resolve()
PROJECT_ROOT = SCRIPT_DIR.parent
REX_BIN = Path(os.environ.get("REX_BIN", PROJECT_ROOT / "target/release/rex"))
REX_FIXTURE = Path(os.environ.get("REX_FIXTURE", PROJECT_ROOT / "fixtures/basic"))
REX_APP_FIXTURE = PROJECT_ROOT / "fixtures/app-router"
NEXT_DIR = SCRIPT_DIR / "next-basic"
NEXT_APP_DIR = SCRIPT_DIR / "next-app-basic"
TANSTACK_DIR = SCRIPT_DIR / "tanstack-basic"
VINEXT_DIR = SCRIPT_DIR / "vinext-basic"
NEXT_TW_DIR = SCRIPT_DIR / "next-tailwind"
TANSTACK_TW_DIR = SCRIPT_DIR / "tanstack-tailwind"
REX_TW_FIXTURE = PROJECT_ROOT / "fixtures/tailwind"

ENDPOINTS = ["/", "/about", "/static", "/blog/hello-world", "/api/hello", "/gallery"]
ENDPOINT_LABELS = {
    "/": "SSR index",
    "/about": "SSR about",
    "/static": "Static (no data)",
    "/blog/hello-world": "Dynamic route",
    "/api/hello": "API route",
    "/gallery": "Gallery (images)",
}

# Image optimization endpoints per framework
IMAGE_ENDPOINTS = {
    "rex": "/_rex/image?url=%2Fimages%2Fhero.jpg&w=640&q=75",
    "nextjs": "/_next/image?url=%2Fimages%2Fhero.jpg&w=640&q=75",
}

# ── Colors ──────────────────────────────────────────────────────


def bold(s: str) -> str:
    return f"\033[1m{s}\033[0m"


def dim(s: str) -> str:
    return f"\033[2m{s}\033[0m"


def green(s: str) -> str:
    return f"\033[32m{s}\033[0m"


def magenta(s: str) -> str:
    return f"\033[35m{s}\033[0m"


def cyan(s: str) -> str:
    return f"\033[36m{s}\033[0m"


def yellow(s: str) -> str:
    return f"\033[33m{s}\033[0m"


def red(s: str) -> str:
    return f"\033[31m{s}\033[0m"


FW_COLOR = {
    "rex": magenta,
    "rex_app": magenta,
    "nextjs": cyan,
    "nextjs_app": cyan,
    "tanstack": yellow,
    "vinext": red,
    "rex_tw": magenta,
    "nextjs_tw": cyan,
    "tanstack_tw": yellow,
}
FW_LABEL = {
    "rex": "Rex (Pages)",
    "rex_app": "Rex (App)",
    "nextjs": "Next.js (Pages)",
    "nextjs_app": "Next.js (App)",
    "tanstack": "TanStack Start",
    "vinext": "Vinext",
    "rex_tw": "Rex + TW",
    "nextjs_tw": "Next.js + TW",
    "tanstack_tw": "TanStack + TW",
}


def section(fw: str, suite: str):
    color = FW_COLOR.get(fw, dim)
    label = FW_LABEL.get(fw, fw)
    print(f"\n{color(f'━━━ {label} ({suite}) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━')}", flush=True)


# ── Helpers ─────────────────────────────────────────────────────


def measure_download_speed() -> str:
    """Measure network download speed by fetching a known file from npm registry.

    Uses typescript tarball (~10MB) for a representative throughput measurement.
    Returns a human-readable string like '85.3 Mbps'.
    """
    url = "https://registry.npmjs.org/typescript/-/typescript-5.8.3.tgz"
    try:
        import urllib.request

        # Warm connection (DNS + TLS), discard result
        urllib.request.urlopen(url, timeout=15).read()
        # Timed download
        t0 = time.monotonic()
        with urllib.request.urlopen(url, timeout=30) as resp:
            data = resp.read()
        elapsed = time.monotonic() - t0
        size_bits = len(data) * 8
        mbps = round(size_bits / elapsed / 1_000_000, 1)
        return f"{mbps} Mbps"
    except Exception:
        return "unknown"


def find_free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_port(port: int, timeout: float = 30) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def get_rss_mb(pid: int) -> float:
    """Get resident set size in MB for a process."""
    try:
        out = subprocess.check_output(["ps", "-o", "rss=", "-p", str(pid)], text=True)
        rss_kb = int(out.strip())
        return round(rss_kb / 1024, 1)
    except (subprocess.CalledProcessError, ValueError):
        return 0.0


def dir_size_mb(path: Path) -> float:
    if not path.exists():
        return 0.0
    total = sum(f.stat().st_size for f in path.rglob("*") if f.is_file())
    return round(total / (1024 * 1024), 1)


def count_deps(project_dir: Path) -> int:
    try:
        out = subprocess.check_output(
            ["npm", "ls", "--all", "--parseable"],
            cwd=project_dir,
            text=True,
            stderr=subprocess.DEVNULL,
        )
        return len(out.strip().splitlines())
    except subprocess.CalledProcessError:
        return 0


def js_total_kb(directory: Path) -> float:
    if not directory.exists():
        return 0.0
    total = sum(f.stat().st_size for f in directory.rglob("*") if f.suffix in (".js", ".mjs"))
    return round(total / 1024, 1)


def css_total_kb(directory: Path) -> float:
    if not directory.exists():
        return 0.0
    total = sum(f.stat().st_size for f in directory.rglob("*.css") if f.is_file())
    return round(total / 1024, 1)


def _gzip_size(path: Path) -> int:
    import gzip

    data = path.read_bytes()
    return len(gzip.compress(data, compresslevel=9))


def js_total_gz_kb(directory: Path) -> float:
    if not directory.exists():
        return 0.0
    total = sum(_gzip_size(f) for f in directory.rglob("*") if f.suffix in (".js", ".mjs"))
    return round(total / 1024, 1)


def css_total_gz_kb(directory: Path) -> float:
    if not directory.exists():
        return 0.0
    total = sum(_gzip_size(f) for f in directory.rglob("*.css") if f.is_file())
    return round(total / 1024, 1)


def curl_body(port: int, path: str) -> str:
    try:
        out = subprocess.check_output(
            ["curl", "-s", f"http://127.0.0.1:{port}{path}"],
            timeout=10,
            text=True,
        )
        return out
    except Exception:
        return ""


# ── Process management ──────────────────────────────────────────


@dataclass
class ServerProcess:
    proc: subprocess.Popen
    port: int

    def rss_mb(self) -> float:
        return get_rss_mb(self.proc.pid)

    def kill(self):
        try:
            self.proc.terminate()
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()
            self.proc.wait(timeout=5)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.kill()


def start_rex(mode: str, port: int) -> Optional[ServerProcess]:
    if not REX_BIN.exists():
        print(f"  {yellow('SKIP')}: Rex binary not found at {REX_BIN}")
        return None
    cmd = [str(REX_BIN), mode, "--root", str(REX_FIXTURE), "--port", str(port)]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if not wait_for_port(port, timeout=30):
        proc.kill()
        print(f"  {yellow('SKIP')}: Rex failed to start on port {port}")
        return None
    return ServerProcess(proc, port)


def start_rex_app(mode: str, port: int) -> Optional[ServerProcess]:
    if not REX_BIN.exists():
        print(f"  {yellow('SKIP')}: Rex binary not found at {REX_BIN}")
        return None
    cmd = [str(REX_BIN), mode, "--root", str(REX_APP_FIXTURE), "--port", str(port)]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if not wait_for_port(port, timeout=30):
        proc.kill()
        print(f"  {yellow('SKIP')}: Rex (App) failed to start on port {port}")
        return None
    return ServerProcess(proc, port)


def start_next(mode: str, port: int) -> Optional[ServerProcess]:
    if not (NEXT_DIR / "node_modules").exists():
        print(
            f"  {yellow('SKIP')}: Next.js not installed. Run: cd benchmarks/next-basic && npm install"
        )
        return None
    cmd = ["npx", "next", mode, "--port", str(port)]
    proc = subprocess.Popen(cmd, cwd=NEXT_DIR, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    timeout = 60 if mode == "dev" else 30
    if not wait_for_port(port, timeout=timeout):
        proc.kill()
        print(f"  {yellow('SKIP')}: Next.js failed to start on port {port}")
        return None
    return ServerProcess(proc, port)


def start_next_app(mode: str, port: int) -> Optional[ServerProcess]:
    if not (NEXT_APP_DIR / "node_modules").exists():
        print(
            f"  {yellow('SKIP')}: Next.js (App) not installed. Run: cd benchmarks/next-app-basic && npm install"
        )
        return None
    cmd = ["npx", "next", mode, "--port", str(port)]
    proc = subprocess.Popen(
        cmd, cwd=NEXT_APP_DIR, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    timeout = 60 if mode == "dev" else 30
    if not wait_for_port(port, timeout=timeout):
        proc.kill()
        print(f"  {yellow('SKIP')}: Next.js (App) failed to start on port {port}")
        return None
    return ServerProcess(proc, port)


def start_tanstack(mode: str, port: int) -> Optional[ServerProcess]:
    if not (TANSTACK_DIR / "node_modules").exists():
        print(
            f"  {yellow('SKIP')}: TanStack Start not installed. Run: cd benchmarks/tanstack-basic && npm install"
        )
        return None
    if mode == "dev":
        cmd = ["npx", "vite", "dev", "--port", str(port), "--host", "127.0.0.1"]
    else:
        # Production: vite preview serves the built dist/
        cmd = ["npx", "vite", "preview", "--port", str(port), "--host", "127.0.0.1"]
    proc = subprocess.Popen(
        cmd, cwd=TANSTACK_DIR, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    timeout = 60 if mode == "dev" else 30
    if not wait_for_port(port, timeout=timeout):
        proc.kill()
        print(f"  {yellow('SKIP')}: TanStack Start failed to start on port {port}")
        return None
    return ServerProcess(proc, port)


def start_vinext(mode: str, port: int) -> Optional[ServerProcess]:
    if not (VINEXT_DIR / "node_modules").exists():
        print(
            f"  {yellow('SKIP')}: Vinext not installed. Run: cd benchmarks/vinext-basic && npm install"
        )
        return None
    cmd = ["npx", "vinext", mode, "--hostname", "127.0.0.1", "--port", str(port)]
    proc = subprocess.Popen(
        cmd, cwd=VINEXT_DIR, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    timeout = 60 if mode == "dev" else 30
    if not wait_for_port(port, timeout=timeout):
        proc.kill()
        print(f"  {yellow('SKIP')}: Vinext failed to start on port {port}")
        return None
    return ServerProcess(proc, port)


def start_rex_tw(mode: str, port: int) -> Optional[ServerProcess]:
    if not REX_BIN.exists():
        print(f"  {yellow('SKIP')}: Rex binary not found at {REX_BIN}")
        return None
    cmd = [str(REX_BIN), mode, "--root", str(REX_TW_FIXTURE), "--port", str(port)]
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if not wait_for_port(port, timeout=30):
        proc.kill()
        print(f"  {yellow('SKIP')}: Rex (Tailwind) failed to start on port {port}")
        return None
    return ServerProcess(proc, port)


def start_next_tw(mode: str, port: int) -> Optional[ServerProcess]:
    if not (NEXT_TW_DIR / "node_modules").exists():
        print(
            f"  {yellow('SKIP')}: Next.js + TW not installed. Run: cd benchmarks/next-tailwind && npm install"
        )
        return None
    cmd = ["npx", "next", mode, "--port", str(port)]
    proc = subprocess.Popen(
        cmd, cwd=NEXT_TW_DIR, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    timeout = 60 if mode == "dev" else 30
    if not wait_for_port(port, timeout=timeout):
        proc.kill()
        print(f"  {yellow('SKIP')}: Next.js + TW failed to start on port {port}")
        return None
    return ServerProcess(proc, port)


def start_tanstack_tw(mode: str, port: int) -> Optional[ServerProcess]:
    if not (TANSTACK_TW_DIR / "node_modules").exists():
        print(
            f"  {yellow('SKIP')}: TanStack + TW not installed. Run: cd benchmarks/tanstack-tailwind && npm install"
        )
        return None
    if mode == "dev":
        cmd = ["npx", "vite", "dev", "--port", str(port), "--host", "127.0.0.1"]
    else:
        cmd = ["npx", "vite", "preview", "--port", str(port), "--host", "127.0.0.1"]
    proc = subprocess.Popen(
        cmd, cwd=TANSTACK_TW_DIR, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    timeout = 60 if mode == "dev" else 30
    if not wait_for_port(port, timeout=timeout):
        proc.kill()
        print(f"  {yellow('SKIP')}: TanStack + TW failed to start on port {port}")
        return None
    return ServerProcess(proc, port)


# ── Apache Bench wrapper ───────────────────────────────────────


@dataclass
class AbResult:
    rps: float = 0.0
    latency_mean_ms: float = 0.0
    latency_p50_ms: float = 0.0
    latency_p99_ms: float = 0.0
    failed: int = 0


def run_ab(url: str, requests: int, concurrency: int, warmup: int) -> AbResult:
    """Run Apache Bench and parse results."""
    if not shutil.which("ab"):
        print(f"  {yellow('SKIP')}: ab (Apache Bench) not found")
        return AbResult()

    # Warmup
    subprocess.run(
        ["ab", "-n", str(warmup), "-c", "10", url],
        capture_output=True,
        timeout=30,
    )

    # Benchmark
    try:
        out = subprocess.check_output(
            ["ab", "-l", "-n", str(requests), "-c", str(concurrency), url],
            stderr=subprocess.STDOUT,
            text=True,
            timeout=120,
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
        print(f"  {yellow('WARN')}: ab failed for {url}")
        return AbResult()

    result = AbResult()

    # Parse RPS
    m = re.search(r"Requests per second:\s+([\d.]+)", out)
    if m:
        result.rps = float(m.group(1))

    # Parse mean latency
    m = re.search(r"Time per request:\s+([\d.]+) \[ms\] \(mean\)", out)
    if m:
        result.latency_mean_ms = float(m.group(1))

    # Parse percentiles
    m = re.search(r"\s+50%\s+(\d+)", out)
    if m:
        result.latency_p50_ms = float(m.group(1))
    m = re.search(r"\s+99%\s+(\d+)", out)
    if m:
        result.latency_p99_ms = float(m.group(1))

    # Parse failures
    m = re.search(r"Failed requests:\s+(\d+)", out)
    if m:
        result.failed = int(m.group(1))

    # Print summary
    for line in out.splitlines():
        if any(k in line for k in ("Requests per second", "Time per request", "Failed requests")):
            print(f"    {line.strip()}", flush=True)

    return result


# ── Results accumulator ─────────────────────────────────────────

results: list[dict] = []
_step = 0
_total_steps = 0


def progress(fw: str, suite: str):
    """Emit a machine-parseable progress line for the dashboard."""
    global _step
    _step += 1
    label = FW_LABEL.get(fw, fw)
    suite_label = {"dx": "DX", "server": "Server", "client": "Client"}.get(suite, suite)
    print(f"[PROGRESS {_step}/{_total_steps}] {label} — {suite_label}", flush=True)


def add(suite: str, framework: str, metric: str, value: float, **extra):
    entry = {"suite": suite, "framework": framework, "metric": metric, "value": value}
    entry.update(extra)
    results.append(entry)


def summarize(samples: list[float]) -> tuple[float, float]:
    """Return (median, stddev) for a list of samples."""
    med = statistics.median(samples)
    sd = statistics.stdev(samples) if len(samples) > 1 else 0.0
    return round(med, 2), round(sd, 2)


def add_sampled(suite: str, framework: str, metric: str, samples: list[float], **extra):
    """Record a metric measured over multiple iterations."""
    med, sd = summarize(samples)
    entry = {
        "suite": suite,
        "framework": framework,
        "metric": metric,
        "value": med,
        "iterations": len(samples),
        "stddev": sd,
    }
    entry.update(extra)
    results.append(entry)
    return med, sd


def fmt_sampled(label: str, med: float, sd: float, unit: str, iterations: int) -> str:
    """Format a sampled metric for display."""
    if iterations > 1:
        return f"  {bold(label)}   {green(f'{med}{unit}')} {dim(f'(median of {iterations}, stddev={sd}{unit})')}"
    return f"  {bold(label)}   {green(f'{med}{unit}')}"


# ════════════════════════════════════════════════════════════════
# DX SUITE
# ════════════════════════════════════════════════════════════════


def file_size_mb(path: Path) -> float:
    if not path.exists():
        return 0.0
    return round(path.stat().st_size / (1024 * 1024), 1)


def dx_framework(
    fw: str,
    project_dir: Path,
    start_fn,
    about_page: Path,
    iterations: int = 1,
    lint_cmd: list[str] | None = None,
):
    progress(fw, "dx")
    section(fw, "DX")

    # ── Rex binary size (users must download this too) ──
    bin_mb = 0.0
    if fw in ("rex", "rex_app") and REX_BIN.exists():
        bin_mb = file_size_mb(REX_BIN)
        print(f"  {bold('Binary size:')}    {green(f'{bin_mb}MB')}")
        add("dx", fw, "binary_mb", bin_mb)

    # ── Dependencies & node_modules size ──
    nm = project_dir / "node_modules"
    if nm.exists():
        deps = count_deps(project_dir)
        nm_mb = dir_size_mb(nm)
        total_mb = round(nm_mb + bin_mb, 1)
        print(f"  {bold('Dependencies:')}   {green(str(deps))}")
        print(f"  {bold('node_modules:')}   {green(f'{nm_mb}MB')}")
        print(
            f"  {bold('Install size:')}   {green(f'{total_mb}MB')}  {dim('(node_modules + binary)') if bin_mb else ''}"
        )
        add("dx", fw, "dependency_count", deps)
        add("dx", fw, "node_modules_mb", nm_mb)
        add("dx", fw, "install_size_mb", total_mb)

    # ── npm install time (clean) ──
    install_samples: list[float] = []
    for i in range(iterations):
        if iterations > 1:
            print(f"  {dim(f'npm install iteration {i + 1}/{iterations}...')}")
        else:
            print(f"  {dim('Measuring npm install (clean)...')}")
        backup = None
        if nm.exists():
            backup = Path(tempfile.mkdtemp())
            shutil.move(str(nm), str(backup / "node_modules"))

        t0 = time.monotonic()
        # Clear npm cache so we measure a true cold-network install
        subprocess.run(
            ["npm", "cache", "clean", "--force"],
            capture_output=True,
            timeout=60,
        )
        subprocess.run(
            ["npm", "install", "--no-audit", "--no-fund"],
            cwd=project_dir,
            capture_output=True,
            timeout=120,
        )
        install_samples.append(round((time.monotonic() - t0) * 1000))

        if backup:
            shutil.rmtree(backup, ignore_errors=True)

    med, sd = add_sampled("dx", fw, "install_time_ms", install_samples)
    print(fmt_sampled("Install time:", med, sd, "ms", iterations))

    # ── Cold start (dev) ──
    cold_samples: list[float] = []
    mem = 0.0
    for i in range(iterations):
        if iterations > 1:
            print(f"  {dim(f'Cold start iteration {i + 1}/{iterations}...')}")
        port = find_free_port()
        t0 = time.monotonic()
        server = start_fn("dev", port)
        if server is None:
            print(f"  {yellow('Could not start dev server')}")
            if not cold_samples:
                return
            break

        with server:
            curl_body(port, "/")
            cold_samples.append(round((time.monotonic() - t0) * 1000))
            # Capture memory on last iteration
            if i == iterations - 1:
                mem = server.rss_mb()

    med, sd = add_sampled("dx", fw, "cold_start_ms", cold_samples)
    print(fmt_sampled("Cold start:  ", med, sd, "ms", iterations))

    # ── Dev memory (from last cold start iteration) ──
    print(f"  {bold('Dev memory:')}     {green(f'{mem}MB')}")
    add("dx", fw, "dev_memory_mb", mem)

    # ── HMR rebuild time ──
    if about_page.exists():
        # Start a fresh dev server for HMR measurement
        port = find_free_port()
        server = start_fn("dev", port)
        if server is None:
            print(f"  {yellow('Could not start dev server for HMR')}")
            print()
            return

        with server:
            # Warm up
            curl_body(port, "/about")
            time.sleep(0.5)

            original = about_page.read_text()
            rebuild_samples: list[float] = []

            for i in range(iterations):
                if iterations > 1:
                    print(f"  {dim(f'HMR iteration {i + 1}/{iterations}...')}")
                marker = f"__BENCH_MARKER_{int(time.time())}_{i}__"
                modified = original.replace("<h1>About</h1>", f"<h1>{marker}</h1>")
                about_page.write_text(modified)

                t0 = time.monotonic()
                deadline = t0 + 20
                found = False
                while time.monotonic() < deadline:
                    body = curl_body(port, "/about")
                    if marker in body:
                        found = True
                        break
                    time.sleep(0.1)

                if found:
                    rebuild_samples.append(round((time.monotonic() - t0) * 1000))
                else:
                    print(f"  {yellow(f'HMR iteration {i + 1}: timed out (20s)')}")

                # Restore for next iteration
                about_page.write_text(original)
                if i < iterations - 1:
                    time.sleep(0.5)

            if rebuild_samples:
                med, sd = add_sampled("dx", fw, "rebuild_ms", rebuild_samples)
                print(fmt_sampled("Rebuild time:", med, sd, "ms", len(rebuild_samples)))
            else:
                print(f"  {yellow('Rebuild: all iterations timed out')}")

    # ── Lint time ──
    if lint_cmd is not None:
        lint_samples: list[float] = []
        for i in range(iterations):
            if iterations > 1:
                print(f"  {dim(f'Lint iteration {i + 1}/{iterations}...')}")
            t0 = time.monotonic()
            subprocess.run(lint_cmd, cwd=project_dir, capture_output=True, timeout=60)
            lint_samples.append(round((time.monotonic() - t0) * 1000))

        med, sd = add_sampled("dx", fw, "lint_time_ms", lint_samples)
        print(fmt_sampled("Lint time:   ", med, sd, "ms", iterations))

    print()


def run_dx(frameworks: list[str], iterations: int = 1):
    print(f"\n  {bold('▸ DX Suite')} — developer experience metrics")
    if iterations > 1:
        print(f"  {dim(f'Iterations: {iterations} (reporting median)')}")
    print()

    if "rex" in frameworks:
        dx_framework(
            "rex",
            REX_FIXTURE,
            start_rex,
            REX_FIXTURE / "pages/about.tsx",
            iterations,
            lint_cmd=[str(REX_BIN), "lint", "--root", str(REX_FIXTURE)],
        )
    if "rex_app" in frameworks:
        dx_framework(
            "rex_app",
            REX_APP_FIXTURE,
            start_rex_app,
            REX_APP_FIXTURE / "app/about/page.tsx",
            iterations,
        )
    if "nextjs" in frameworks:
        dx_framework(
            "nextjs",
            NEXT_DIR,
            start_next,
            NEXT_DIR / "pages/about.tsx",
            iterations,
            lint_cmd=["npx", "next", "lint", "--dir", "pages", "--no-cache"],
        )
    if "nextjs_app" in frameworks:
        dx_framework(
            "nextjs_app",
            NEXT_APP_DIR,
            start_next_app,
            NEXT_APP_DIR / "app/about/page.tsx",
            iterations,
            lint_cmd=["npx", "next", "lint", "--dir", "app", "--no-cache"],
        )
    if "tanstack" in frameworks:
        dx_framework(
            "tanstack",
            TANSTACK_DIR,
            start_tanstack,
            TANSTACK_DIR / "src/routes/about.tsx",
            iterations,
        )
    if "vinext" in frameworks:
        dx_framework(
            "vinext",
            VINEXT_DIR,
            start_vinext,
            VINEXT_DIR / "app/about/page.tsx",
            iterations,
        )
    if "rex_tw" in frameworks:
        dx_framework(
            "rex_tw",
            REX_TW_FIXTURE,
            start_rex_tw,
            REX_TW_FIXTURE / "pages/about.tsx",
            iterations,
            lint_cmd=[str(REX_BIN), "lint", "--root", str(REX_TW_FIXTURE)],
        )
    if "nextjs_tw" in frameworks:
        dx_framework(
            "nextjs_tw",
            NEXT_TW_DIR,
            start_next_tw,
            NEXT_TW_DIR / "pages/about.tsx",
            iterations,
            lint_cmd=["npx", "next", "lint", "--dir", "pages", "--no-cache"],
        )
    if "tanstack_tw" in frameworks:
        dx_framework(
            "tanstack_tw",
            TANSTACK_TW_DIR,
            start_tanstack_tw,
            TANSTACK_TW_DIR / "src/routes/about.tsx",
            iterations,
        )


# ════════════════════════════════════════════════════════════════
# SERVER SUITE
# ════════════════════════════════════════════════════════════════


def server_framework(
    fw: str,
    build_fn,
    start_fn,
    build_output_dir: Optional[Path],
    requests: int,
    concurrency: int,
    warmup: int,
    iterations: int = 1,
):
    progress(fw, "server")
    section(fw, "Server")

    # ── Build (iterated) ──
    build_samples: list[float] = []
    for i in range(iterations):
        if iterations > 1:
            print(f"  {dim(f'Build iteration {i + 1}/{iterations}...')}")
        t0 = time.monotonic()
        if not build_fn():
            if not build_samples:
                return
            break
        build_samples.append(round((time.monotonic() - t0) * 1000))

    med, sd = add_sampled("server", fw, "build_time_ms", build_samples)
    print(fmt_sampled("Build time: ", med, sd, "ms", len(build_samples)))

    if build_output_dir and build_output_dir.exists():
        build_mb = dir_size_mb(build_output_dir)
        print(f"  {bold('Build output:')}  {green(f'{build_mb}MB')}")
        add("server", fw, "build_output_mb", build_mb)

    # ── Cold start (iterated) ──
    cold_samples: list[float] = []
    last_server = None
    last_port = 0
    for i in range(iterations):
        if iterations > 1:
            print(f"  {dim(f'Cold start iteration {i + 1}/{iterations}...')}")
        port = find_free_port()
        t0 = time.monotonic()
        server = start_fn("start", port)
        if server is None:
            if not cold_samples:
                return
            break
        curl_body(port, "/")
        cold_samples.append(round((time.monotonic() - t0) * 1000))
        # Keep last server alive for ab + memory measurement
        if last_server:
            last_server.kill()
        if i == iterations - 1:
            last_server = server
            last_port = port
        else:
            server.kill()

    med, sd = add_sampled("server", fw, "cold_start_ms", cold_samples)
    print(fmt_sampled("Cold start: ", med, sd, "ms", len(cold_samples)))

    # ── Benchmark endpoints (on last server) ──
    if last_server is None:
        return

    with last_server:
        for ep in ENDPOINTS:
            label = ENDPOINT_LABELS.get(ep, ep)
            print(f"\n  {bold(label)}")
            ab = run_ab(f"http://127.0.0.1:{last_port}{ep}", requests, concurrency, warmup)
            add("server", fw, "rps", ab.rps, endpoint=ep)
            add("server", fw, "latency_mean_ms", ab.latency_mean_ms, endpoint=ep)
            add("server", fw, "latency_p50_ms", ab.latency_p50_ms, endpoint=ep)
            add("server", fw, "latency_p99_ms", ab.latency_p99_ms, endpoint=ep)

        # ── Image optimization endpoint ──
        img_ep = IMAGE_ENDPOINTS.get(fw)
        if img_ep:
            # Cold resize: clear cache and time first request
            if fw == "rex":
                cache_dir = REX_FIXTURE / ".rex" / "cache" / "images"
                if cache_dir.exists():
                    shutil.rmtree(cache_dir)
            t0 = time.monotonic()
            curl_body(last_port, img_ep)
            cold_ms = round((time.monotonic() - t0) * 1000, 1)
            print(f"\n  {bold('Image cold resize:')}  {green(f'{cold_ms}ms')}")
            add("server", fw, "image_cold_resize_ms", cold_ms)

            # Cached image serve throughput
            print(f"\n  {bold('Image endpoint (cached)')}")
            ab = run_ab(f"http://127.0.0.1:{last_port}{img_ep}", requests, concurrency, warmup)
            add("server", fw, "rps", ab.rps, endpoint=img_ep)
            add("server", fw, "latency_mean_ms", ab.latency_mean_ms, endpoint=img_ep)
            add("server", fw, "latency_p50_ms", ab.latency_p50_ms, endpoint=img_ep)
            add("server", fw, "latency_p99_ms", ab.latency_p99_ms, endpoint=img_ep)

        # ── Memory ──
        mem = last_server.rss_mb()
        print(f"\n  {bold('Memory (RSS):')}  {green(f'{mem}MB')}")
        add("server", fw, "memory_mb", mem)

    print()


def run_server(
    frameworks: list[str], requests: int, concurrency: int, warmup: int, iterations: int = 1
):
    if not shutil.which("ab"):
        print(f"  {yellow('SKIP server suite: ab (Apache Bench) not found')}")
        return

    print(f"\n  {bold('▸ Server Suite')} — production throughput & latency")
    parts = f"  {dim('Requests:')} {requests}  {dim('Concurrency:')} {concurrency}  {dim('Warmup:')} {warmup}"
    if iterations > 1:
        parts += f"  {dim(f'Iterations: {iterations}')}"
    print(parts + "\n")

    if "rex" in frameworks:

        def build_rex():
            if not REX_BIN.exists():
                print(f"  {yellow('SKIP')}: Rex binary not found")
                return False
            subprocess.run(
                [str(REX_BIN), "build", "--root", str(REX_FIXTURE)],
                capture_output=True,
                timeout=60,
            )
            return True

        server_framework(
            "rex",
            build_rex,
            start_rex,
            REX_FIXTURE / ".rex/build",
            requests,
            concurrency,
            warmup,
            iterations,
        )

    if "rex_app" in frameworks:

        def build_rex_app():
            if not REX_BIN.exists():
                print(f"  {yellow('SKIP')}: Rex binary not found")
                return False
            subprocess.run(
                [str(REX_BIN), "build", "--root", str(REX_APP_FIXTURE)],
                capture_output=True,
                timeout=60,
            )
            return True

        server_framework(
            "rex_app",
            build_rex_app,
            start_rex_app,
            REX_APP_FIXTURE / ".rex/build",
            requests,
            concurrency,
            warmup,
            iterations,
        )

    if "nextjs" in frameworks:

        def build_next():
            if not (NEXT_DIR / "node_modules").exists():
                print(f"  {yellow('SKIP')}: Next.js not installed")
                return False
            subprocess.run(
                ["npx", "next", "build"],
                cwd=NEXT_DIR,
                capture_output=True,
                timeout=120,
            )
            return True

        server_framework(
            "nextjs",
            build_next,
            start_next,
            NEXT_DIR / ".next",
            requests,
            concurrency,
            warmup,
            iterations,
        )

    if "nextjs_app" in frameworks:

        def build_next_app():
            if not (NEXT_APP_DIR / "node_modules").exists():
                print(f"  {yellow('SKIP')}: Next.js (App) not installed")
                return False
            subprocess.run(
                ["npx", "next", "build"],
                cwd=NEXT_APP_DIR,
                capture_output=True,
                timeout=120,
            )
            return True

        server_framework(
            "nextjs_app",
            build_next_app,
            start_next_app,
            NEXT_APP_DIR / ".next",
            requests,
            concurrency,
            warmup,
            iterations,
        )

    if "tanstack" in frameworks:

        def build_tanstack():
            if not (TANSTACK_DIR / "node_modules").exists():
                print(f"  {yellow('SKIP')}: TanStack Start not installed")
                return False
            subprocess.run(
                ["npx", "vite", "build"],
                cwd=TANSTACK_DIR,
                capture_output=True,
                timeout=120,
            )
            return True

        server_framework(
            "tanstack",
            build_tanstack,
            start_tanstack,
            TANSTACK_DIR / "dist",
            requests,
            concurrency,
            warmup,
            iterations,
        )

    if "vinext" in frameworks:

        def build_vinext():
            if not (VINEXT_DIR / "node_modules").exists():
                print(f"  {yellow('SKIP')}: Vinext not installed")
                return False
            subprocess.run(
                ["npx", "vinext", "build"],
                cwd=VINEXT_DIR,
                capture_output=True,
                timeout=120,
            )
            return True

        server_framework(
            "vinext",
            build_vinext,
            start_vinext,
            VINEXT_DIR / ".vinext",
            requests,
            concurrency,
            warmup,
            iterations,
        )

    if "rex_tw" in frameworks:

        def build_rex_tw():
            if not REX_BIN.exists():
                print(f"  {yellow('SKIP')}: Rex binary not found")
                return False
            subprocess.run(
                [str(REX_BIN), "build", "--root", str(REX_TW_FIXTURE)],
                capture_output=True,
                timeout=60,
            )
            return True

        server_framework(
            "rex_tw",
            build_rex_tw,
            start_rex_tw,
            REX_TW_FIXTURE / ".rex/build",
            requests,
            concurrency,
            warmup,
            iterations,
        )

    if "nextjs_tw" in frameworks:

        def build_next_tw():
            if not (NEXT_TW_DIR / "node_modules").exists():
                print(f"  {yellow('SKIP')}: Next.js + TW not installed")
                return False
            subprocess.run(
                ["npx", "next", "build"],
                cwd=NEXT_TW_DIR,
                capture_output=True,
                timeout=120,
            )
            return True

        server_framework(
            "nextjs_tw",
            build_next_tw,
            start_next_tw,
            NEXT_TW_DIR / ".next",
            requests,
            concurrency,
            warmup,
            iterations,
        )

    if "tanstack_tw" in frameworks:

        def build_tanstack_tw():
            if not (TANSTACK_TW_DIR / "node_modules").exists():
                print(f"  {yellow('SKIP')}: TanStack + TW not installed")
                return False
            subprocess.run(
                ["npx", "vite", "build"],
                cwd=TANSTACK_TW_DIR,
                capture_output=True,
                timeout=120,
            )
            return True

        server_framework(
            "tanstack_tw",
            build_tanstack_tw,
            start_tanstack_tw,
            TANSTACK_TW_DIR / "dist",
            requests,
            concurrency,
            warmup,
            iterations,
        )


# ════════════════════════════════════════════════════════════════
# CLIENT SUITE
# ════════════════════════════════════════════════════════════════


def client_bundle_sizes(fw: str, build_dir: Path):
    progress(fw, "client")
    section(fw, "Client")

    if not build_dir.exists():
        print(f"  {yellow('No build output')} — run server suite first or build manually")
        return

    js_kb = js_total_kb(build_dir)
    css_kb = css_total_kb(build_dir)
    js_gz_kb = js_total_gz_kb(build_dir)
    css_gz_kb = css_total_gz_kb(build_dir)
    print(f"  {bold('Total JS:')}    {green(f'{js_kb}KB')}  {dim(f'({js_gz_kb}KB gzip)')}")
    print(f"  {bold('Total CSS:')}   {green(f'{css_kb}KB')}  {dim(f'({css_gz_kb}KB gzip)')}")
    add("client", fw, "total_js_kb", js_kb)
    add("client", fw, "total_css_kb", css_kb)
    add("client", fw, "total_js_gz_kb", js_gz_kb)
    add("client", fw, "total_css_gz_kb", css_gz_kb)
    print()


LIGHTHOUSE_PAGES = {
    "rex": [
        ("/", "index"),
        ("/about", "about"),
        ("/blog/hello-world", "blog"),
        ("/gallery", "gallery"),
    ],
    "rex_app": [
        ("/", "index"),
        ("/about", "about"),
        ("/blog/hello-world", "blog"),
        ("/dashboard", "dashboard"),
    ],
    "nextjs": [
        ("/", "index"),
        ("/about", "about"),
        ("/blog/hello-world", "blog"),
        ("/gallery", "gallery"),
    ],
    "nextjs_app": [("/", "index"), ("/about", "about"), ("/blog/hello-world", "blog")],
    "vinext": [("/", "index"), ("/about", "about"), ("/blog/hello-world", "blog")],
    "tanstack": [
        ("/", "index"),
        ("/about", "about"),
        ("/blog/hello-world", "blog"),
        ("/gallery", "gallery"),
    ],
}


def lighthouse_audit(fw: str, port: int):
    """Run Lighthouse on key pages if available. Uses no throttling for local
    benchmarks so raw server/client differences are visible."""
    if not shutil.which("lighthouse") and not shutil.which("npx"):
        return

    fw_label = FW_LABEL.get(fw, fw)
    color = FW_COLOR.get(fw, dim)
    print(f"\n  {color(f'Lighthouse — {fw_label}')}", flush=True)

    pages = LIGHTHOUSE_PAGES.get(fw, [("/", "index"), ("/about", "about")])

    for ep, label in pages:
        url = f"http://127.0.0.1:{port}{ep}"
        print(f"  {dim(f'Lighthouse: {label}...')}")
        try:
            out = subprocess.check_output(
                [
                    "npx",
                    "lighthouse",
                    url,
                    "--output=json",
                    "--chrome-flags=--headless --no-sandbox",
                    "--only-categories=performance",
                    "--throttling-method=provided",
                    "--quiet",
                ],
                stderr=subprocess.DEVNULL,
                timeout=120,
                text=True,
            )
            data = json.loads(out)
            audits = data.get("audits", {})
            perf = data.get("categories", {}).get("performance", {}).get("score", 0) * 100

            metrics = {
                "lcp_ms": audits.get("largest-contentful-paint", {}).get("numericValue"),
                "fcp_ms": audits.get("first-contentful-paint", {}).get("numericValue"),
                "ttfb_ms": audits.get("server-response-time", {}).get("numericValue"),
                "tbt_ms": audits.get("total-blocking-time", {}).get("numericValue"),
                "cls": audits.get("cumulative-layout-shift", {}).get("numericValue"),
            }

            print(f"  {bold(f'{label}:')} score={green(str(int(perf)))}", end="")
            for name, val in metrics.items():
                if val is not None:
                    add("client", fw, name, round(val, 2), endpoint=ep)
                    if name != "cls":
                        print(f"  {name}={green(f'{val:.0f}ms')}", end="")
                    else:
                        print(f"  {name}={green(f'{val:.3f}')}", end="")
            add("client", fw, "performance_score", perf, endpoint=ep)
            print()

        except Exception:
            print(f"  {yellow(f'Lighthouse failed for {label}')}")


def run_client(frameworks: list[str]):
    print(f"\n  {bold('▸ Client Suite')} — bundle size & Web Vitals\n")

    # Check Lighthouse availability
    has_lighthouse = False
    try:
        subprocess.run(
            ["npx", "lighthouse", "--version"],
            capture_output=True,
            timeout=15,
        )
        has_lighthouse = True
        print(f"  {dim('Lighthouse detected — will run Web Vitals audits')}\n")
    except Exception:
        print(f"  {dim('Lighthouse not found — bundle sizes only')}")
        print(f"  {dim('Install: npm install -g lighthouse')}\n")

    # Bundle sizes
    if "rex" in frameworks:
        # Ensure build exists
        if not (REX_FIXTURE / ".rex/build").exists() and REX_BIN.exists():
            subprocess.run([str(REX_BIN), "build", "--root", str(REX_FIXTURE)], capture_output=True)
        client_bundle_sizes("rex", REX_FIXTURE / ".rex/build/client")

    if "rex_app" in frameworks:
        if not (REX_APP_FIXTURE / ".rex/build").exists() and REX_BIN.exists():
            subprocess.run(
                [str(REX_BIN), "build", "--root", str(REX_APP_FIXTURE)], capture_output=True
            )
        client_bundle_sizes("rex_app", REX_APP_FIXTURE / ".rex/build/client")

    if "nextjs" in frameworks:
        if not (NEXT_DIR / ".next").exists():
            subprocess.run(["npx", "next", "build"], cwd=NEXT_DIR, capture_output=True)
        client_bundle_sizes("nextjs", NEXT_DIR / ".next/static")

    if "nextjs_app" in frameworks:
        if not (NEXT_APP_DIR / ".next").exists():
            subprocess.run(["npx", "next", "build"], cwd=NEXT_APP_DIR, capture_output=True)
        client_bundle_sizes("nextjs_app", NEXT_APP_DIR / ".next/static")

    if "tanstack" in frameworks:
        if not (TANSTACK_DIR / "dist").exists():
            subprocess.run(["npx", "vite", "build"], cwd=TANSTACK_DIR, capture_output=True)
        client_bundle_sizes("tanstack", TANSTACK_DIR / "dist/client")

    if "vinext" in frameworks:
        if not (VINEXT_DIR / ".vinext").exists():
            subprocess.run(["npx", "vinext", "build"], cwd=VINEXT_DIR, capture_output=True)
        client_bundle_sizes("vinext", VINEXT_DIR / ".vinext/static")

    if "rex_tw" in frameworks:
        if not (REX_TW_FIXTURE / ".rex/build").exists() and REX_BIN.exists():
            subprocess.run(
                [str(REX_BIN), "build", "--root", str(REX_TW_FIXTURE)], capture_output=True
            )
        client_bundle_sizes("rex_tw", REX_TW_FIXTURE / ".rex/build/client")

    if "nextjs_tw" in frameworks:
        if not (NEXT_TW_DIR / ".next").exists():
            subprocess.run(["npx", "next", "build"], cwd=NEXT_TW_DIR, capture_output=True)
        client_bundle_sizes("nextjs_tw", NEXT_TW_DIR / ".next/static")

    if "tanstack_tw" in frameworks:
        if not (TANSTACK_TW_DIR / "dist").exists():
            subprocess.run(["npx", "vite", "build"], cwd=TANSTACK_TW_DIR, capture_output=True)
        client_bundle_sizes("tanstack_tw", TANSTACK_TW_DIR / "dist/client")

    # Lighthouse
    if has_lighthouse:
        print(f"  {dim('Running Lighthouse audits (this takes a while)...')}\n")

        if "rex" in frameworks and REX_BIN.exists():
            port = find_free_port()
            server = start_rex("start", port)
            if server:
                with server:
                    lighthouse_audit("rex", port)

        if "rex_app" in frameworks and REX_BIN.exists():
            port = find_free_port()
            server = start_rex_app("start", port)
            if server:
                with server:
                    lighthouse_audit("rex_app", port)

        if "nextjs" in frameworks and (NEXT_DIR / "node_modules").exists():
            port = find_free_port()
            server = start_next("start", port)
            if server:
                with server:
                    lighthouse_audit("nextjs", port)

        if "nextjs_app" in frameworks and (NEXT_APP_DIR / "node_modules").exists():
            port = find_free_port()
            server = start_next_app("start", port)
            if server:
                with server:
                    lighthouse_audit("nextjs_app", port)

        if "tanstack" in frameworks and (TANSTACK_DIR / "dist").exists():
            port = find_free_port()
            server = start_tanstack("start", port)
            if server:
                with server:
                    lighthouse_audit("tanstack", port)

        if "vinext" in frameworks and (VINEXT_DIR / "node_modules").exists():
            port = find_free_port()
            server = start_vinext("start", port)
            if server:
                with server:
                    lighthouse_audit("vinext", port)

        if "rex_tw" in frameworks and REX_BIN.exists():
            port = find_free_port()
            server = start_rex_tw("start", port)
            if server:
                with server:
                    lighthouse_audit("rex_tw", port)

        if "nextjs_tw" in frameworks and (NEXT_TW_DIR / "node_modules").exists():
            port = find_free_port()
            server = start_next_tw("start", port)
            if server:
                with server:
                    lighthouse_audit("nextjs_tw", port)

        if "tanstack_tw" in frameworks and (TANSTACK_TW_DIR / "dist").exists():
            port = find_free_port()
            server = start_tanstack_tw("start", port)
            if server:
                with server:
                    lighthouse_audit("tanstack_tw", port)


# ── Main ────────────────────────────────────────────────────────


def main():
    parser = argparse.ArgumentParser(description="Rex Benchmark Suite")
    parser.add_argument(
        "--suite",
        default="dx,server,client",
        help="Comma-separated suites: dx, server, client (default: all)",
    )
    parser.add_argument(
        "--framework",
        default="rex,rex_app,nextjs,nextjs_app,tanstack,vinext,rex_tw,nextjs_tw,tanstack_tw",
        help="Comma-separated: rex, rex_app, nextjs, nextjs_app, tanstack, vinext, rex_tw, nextjs_tw, tanstack_tw (default: all)",
    )
    parser.add_argument("--json", dest="json_file", help="Write results to JSON file")
    parser.add_argument(
        "--requests", type=int, default=10000, help="Requests per benchmark (default: 10000)"
    )
    parser.add_argument(
        "--concurrency", type=int, default=100, help="Concurrent connections (default: 100)"
    )
    parser.add_argument("--warmup", type=int, default=200, help="Warmup requests (default: 200)")
    parser.add_argument(
        "--iterations",
        type=int,
        default=5,
        help="Iterations for noisy metrics like cold start, HMR, build time (default: 1). Reports median and stddev when >1.",
    )
    args = parser.parse_args()

    suites = [s.strip() for s in args.suite.split(",")]
    frameworks = [f.strip() for f in args.framework.split(",")]

    # Compute total steps for progress reporting
    global _total_steps
    _total_steps = len(suites) * len(frameworks)

    # Measure network speed for install-time context
    net_speed = measure_download_speed()

    # Banner
    print(f"\n  {bold('Rex Benchmark Suite')}\n")
    print(f"  {dim('Suites:')}      {', '.join(suites)}")
    fw_strs = [FW_COLOR.get(fw, dim)(FW_LABEL.get(fw, fw)) for fw in frameworks]
    print(f"  {dim('Frameworks:')}  {' '.join(fw_strs)}")
    if args.iterations > 1:
        print(f"  {dim('Iterations:')}  {args.iterations} (noisy metrics report median ± stddev)")
    print(f"  {dim('Network:')}     {net_speed} (npm registry download)")
    print()

    # Record network speed in results for reproducibility
    results.append({"suite": "meta", "metric": "network_speed", "value": net_speed})

    # Run
    if "dx" in suites:
        run_dx(frameworks, args.iterations)
    if "server" in suites:
        run_server(frameworks, args.requests, args.concurrency, args.warmup, args.iterations)
    if "client" in suites:
        run_client(frameworks)

    # Write JSON
    if args.json_file:
        with open(args.json_file, "w") as f:
            json.dump(results, f, indent=2)
        print(f"\n  {dim(f'Results written to {args.json_file} ({len(results)} entries)')}")

    print(f"\n{dim('Done.')}")


if __name__ == "__main__":
    main()
