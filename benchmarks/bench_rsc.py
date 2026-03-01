#!/usr/bin/env python3
"""
Rex RSC Benchmark — Compares Pages Router vs App Router (RSC) performance.

Measures:
  - Build time: pages-only vs pages + RSC three-pass build
  - Server render latency: renderToString vs RSC two-pass render
  - Client bundle size: pages vs RSC app routes
  - Flight data payload size vs JSON props payload
  - V8 isolate startup time with RSC polyfills

Usage:
  uv run python bench_rsc.py                    # all benchmarks
  uv run python bench_rsc.py --suite build      # build time only
  uv run python bench_rsc.py --suite render      # render latency only
  uv run python bench_rsc.py --suite bundle      # bundle size only
  uv run python bench_rsc.py --iterations 5      # median of 5 runs
  uv run python bench_rsc.py --json results.json # write JSON output
"""

# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///

from __future__ import annotations

import argparse
import json
import os
import re
import socket
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

SCRIPT_DIR = Path(__file__).parent.resolve()
PROJECT_ROOT = SCRIPT_DIR.parent
REX_BIN = Path(os.environ.get("REX_BIN", PROJECT_ROOT / "target/release/rex"))
PAGES_FIXTURE = PROJECT_ROOT / "fixtures/basic"
APP_FIXTURE = PROJECT_ROOT / "fixtures/app-router"


@dataclass
class BenchResult:
    name: str
    value: float
    unit: str
    comparison: str = ""  # e.g., "1.5x slower than pages"


@dataclass
class BenchSuite:
    name: str
    results: list[BenchResult] = field(default_factory=list)


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_port(port: int, timeout: float = 30.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except (ConnectionRefusedError, OSError):
            time.sleep(0.1)
    return False


def start_server(fixture: Path, port: int) -> subprocess.Popen:
    proc = subprocess.Popen(
        [str(REX_BIN), "dev", "--root", str(fixture), "--port", str(port)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if not wait_for_port(port):
        proc.kill()
        raise RuntimeError(f"Server failed to start on port {port}")
    return proc


def measure_build_time(fixture: Path, iterations: int) -> list[float]:
    """Measure `rex build` time for a fixture."""
    times = []
    for _ in range(iterations):
        start = time.monotonic()
        result = subprocess.run(
            [str(REX_BIN), "build", "--root", str(fixture)],
            capture_output=True,
            text=True,
        )
        elapsed = time.monotonic() - start
        if result.returncode == 0:
            times.append(elapsed * 1000)  # ms
        else:
            print(f"  Build failed: {result.stderr[:200]}", file=sys.stderr)
    return times


def measure_render_latency(port: int, path: str, iterations: int) -> list[float]:
    """Measure HTTP request latency to a running server."""
    import urllib.request

    url = f"http://127.0.0.1:{port}{path}"
    times = []

    # Warm up
    for _ in range(3):
        try:
            urllib.request.urlopen(url, timeout=5)
        except Exception:
            pass

    for _ in range(iterations):
        start = time.monotonic()
        try:
            urllib.request.urlopen(url, timeout=10)
            elapsed = time.monotonic() - start
            times.append(elapsed * 1000)  # ms
        except Exception as e:
            print(f"  Request failed: {e}", file=sys.stderr)
    return times


def measure_bundle_size(fixture: Path) -> dict[str, int]:
    """Build and measure output bundle sizes."""
    subprocess.run(
        [str(REX_BIN), "build", "--root", str(fixture)],
        capture_output=True,
    )

    sizes = {}
    build_dir = fixture / ".rex" / "build"
    if build_dir.exists():
        for f in build_dir.rglob("*.js"):
            rel = str(f.relative_to(build_dir))
            sizes[rel] = f.stat().st_size

    return sizes


def measure_flight_payload(port: int, path: str) -> Optional[int]:
    """Measure flight data payload size from RSC endpoint."""
    import urllib.request

    # Get build_id from a page
    try:
        resp = urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=5)
        body = resp.read().decode()
        match = re.search(r'"build_id":"([^"]+)"', body)
        if not match:
            return None
        build_id = match.group(1)

        rsc_url = f"http://127.0.0.1:{port}/_rex/rsc/{build_id}{path}"
        resp = urllib.request.urlopen(rsc_url, timeout=5)
        data = resp.read()
        return len(data)
    except Exception as e:
        print(f"  Flight payload error: {e}", file=sys.stderr)
        return None


def run_build_suite(iterations: int) -> BenchSuite:
    suite = BenchSuite(name="build")
    print("\n=== Build Time ===")

    if not REX_BIN.exists():
        print(f"  Rex binary not found at {REX_BIN}, skipping")
        return suite

    # Pages router build
    print(f"  Pages router ({PAGES_FIXTURE.name})...")
    pages_times = measure_build_time(PAGES_FIXTURE, iterations)
    if pages_times:
        median = statistics.median(pages_times)
        suite.results.append(BenchResult("pages_build_time", median, "ms"))
        print(f"    {median:.0f}ms (median of {len(pages_times)})")

    # App router build
    print(f"  App router ({APP_FIXTURE.name})...")
    app_times = measure_build_time(APP_FIXTURE, iterations)
    if app_times:
        median = statistics.median(app_times)
        ratio = median / statistics.median(pages_times) if pages_times else 0
        suite.results.append(BenchResult("app_build_time", median, "ms", f"{ratio:.1f}x vs pages"))
        print(f"    {median:.0f}ms (median of {len(app_times)}, {ratio:.1f}x vs pages)")

    return suite


def run_render_suite(iterations: int) -> BenchSuite:
    suite = BenchSuite(name="render")
    print("\n=== Server Render Latency ===")

    if not REX_BIN.exists():
        print(f"  Rex binary not found at {REX_BIN}, skipping")
        return suite

    # Pages router server
    pages_port = find_free_port()
    print(f"  Starting pages server on port {pages_port}...")
    pages_proc = start_server(PAGES_FIXTURE, pages_port)

    try:
        print("  Measuring pages render...")
        pages_times = measure_render_latency(pages_port, "/", iterations)
        if pages_times:
            median = statistics.median(pages_times)
            suite.results.append(BenchResult("pages_render_p50", median, "ms"))
            print(f"    p50: {median:.1f}ms")
            if len(pages_times) >= 10:
                p99 = sorted(pages_times)[int(len(pages_times) * 0.99)]
                suite.results.append(BenchResult("pages_render_p99", p99, "ms"))
                print(f"    p99: {p99:.1f}ms")
    finally:
        pages_proc.kill()
        pages_proc.wait()

    # App router server
    app_port = find_free_port()
    print(f"  Starting app server on port {app_port}...")
    try:
        app_proc = start_server(APP_FIXTURE, app_port)
    except RuntimeError as e:
        print(f"  App server failed: {e}")
        return suite

    try:
        print("  Measuring RSC render...")
        app_times = measure_render_latency(app_port, "/", iterations)
        if app_times:
            median = statistics.median(app_times)
            ratio = median / statistics.median(pages_times) if pages_times else 0
            suite.results.append(
                BenchResult("rsc_render_p50", median, "ms", f"{ratio:.1f}x vs pages")
            )
            print(f"    p50: {median:.1f}ms ({ratio:.1f}x vs pages)")
            if len(app_times) >= 10:
                p99 = sorted(app_times)[int(len(app_times) * 0.99)]
                suite.results.append(BenchResult("rsc_render_p99", p99, "ms"))
                print(f"    p99: {p99:.1f}ms")

        # Flight data payload size
        print("  Measuring flight data payload...")
        flight_size = measure_flight_payload(app_port, "/about")
        if flight_size is not None:
            suite.results.append(BenchResult("flight_payload_about", flight_size, "bytes"))
            print(f"    /about flight: {flight_size} bytes")
    finally:
        app_proc.kill()
        app_proc.wait()

    return suite


def run_bundle_suite() -> BenchSuite:
    suite = BenchSuite(name="bundle")
    print("\n=== Bundle Size ===")

    if not REX_BIN.exists():
        print(f"  Rex binary not found at {REX_BIN}, skipping")
        return suite

    print(f"  Building pages router ({PAGES_FIXTURE.name})...")
    pages_sizes = measure_bundle_size(PAGES_FIXTURE)
    total_pages = sum(pages_sizes.values())
    suite.results.append(BenchResult("pages_total_js", total_pages, "bytes"))
    print(f"    Total JS: {total_pages / 1024:.1f} KB ({len(pages_sizes)} files)")

    print(f"  Building app router ({APP_FIXTURE.name})...")
    app_sizes = measure_bundle_size(APP_FIXTURE)
    total_app = sum(app_sizes.values())
    suite.results.append(BenchResult("app_total_js", total_app, "bytes"))
    print(f"    Total JS: {total_app / 1024:.1f} KB ({len(app_sizes)} files)")

    # Server vs client split
    server_js = sum(v for k, v in app_sizes.items() if "server" in k or "rsc" in k)
    client_js = total_app - server_js
    suite.results.append(BenchResult("app_server_js", server_js, "bytes"))
    suite.results.append(BenchResult("app_client_js", client_js, "bytes"))
    print(f"    Server: {server_js / 1024:.1f} KB, Client: {client_js / 1024:.1f} KB")

    return suite


def main():
    parser = argparse.ArgumentParser(description="Rex RSC Benchmark")
    parser.add_argument(
        "--suite",
        default="all",
        help="Comma-separated: build, render, bundle, all",
    )
    parser.add_argument("--iterations", type=int, default=10, help="Iterations per metric")
    parser.add_argument("--json", type=str, help="Write JSON results to file")
    args = parser.parse_args()

    suites_to_run = args.suite.split(",") if args.suite != "all" else ["build", "render", "bundle"]

    print(f"Rex RSC Benchmark (iterations={args.iterations})")
    print(f"  Rex binary: {REX_BIN}")
    print(f"  Pages fixture: {PAGES_FIXTURE}")
    print(f"  App fixture: {APP_FIXTURE}")

    all_suites = []

    if "build" in suites_to_run:
        all_suites.append(run_build_suite(args.iterations))
    if "render" in suites_to_run:
        all_suites.append(run_render_suite(args.iterations))
    if "bundle" in suites_to_run:
        all_suites.append(run_bundle_suite())

    if args.json:
        output = {}
        for s in all_suites:
            output[s.name] = {
                r.name: {"value": r.value, "unit": r.unit, "comparison": r.comparison}
                for r in s.results
            }
        with open(args.json, "w") as f:
            json.dump(output, f, indent=2)
        print(f"\nResults written to {args.json}")


if __name__ == "__main__":
    main()
