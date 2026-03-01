#!/usr/bin/env python3
"""
Rex RSC Benchmark — Compares Rex Pages Router, Rex App Router, and Next.js App Router.

Measures:
  - Build time: pages-only vs RSC three-pass build vs Next.js build
  - Server render latency: renderToString vs RSC two-pass render vs Next.js RSC
  - Client bundle size: server vs client split for each framework
  - Flight data payload size vs JSON props payload

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
import shutil
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
NEXTJS_FIXTURE = PROJECT_ROOT / "fixtures/nextjs-app-router"


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


def start_nextjs_server(fixture: Path, port: int) -> subprocess.Popen:
    proc = subprocess.Popen(
        ["npx", "next", "start", "-p", str(port)],
        cwd=str(fixture),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if not wait_for_port(port, timeout=30.0):
        proc.kill()
        raise RuntimeError(f"Next.js server failed to start on port {port}")
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


def measure_nextjs_build_time(fixture: Path, iterations: int) -> list[float]:
    """Measure `next build` time for a fixture."""
    times = []
    for _ in range(iterations):
        # Clean .next to ensure cold build
        next_dir = fixture / ".next"
        if next_dir.exists():
            shutil.rmtree(next_dir)

        start = time.monotonic()
        result = subprocess.run(
            ["npx", "next", "build"],
            cwd=str(fixture),
            capture_output=True,
            text=True,
        )
        elapsed = time.monotonic() - start
        if result.returncode == 0:
            times.append(elapsed * 1000)  # ms
        else:
            print(f"  Next.js build failed: {result.stderr[:200]}", file=sys.stderr)
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


def measure_bundle_size(fixture: Path) -> dict[str, dict[str, int]]:
    """Build and measure output bundle sizes, split by server/client."""
    subprocess.run(
        [str(REX_BIN), "build", "--root", str(fixture)],
        capture_output=True,
    )

    server_sizes = {}
    client_sizes = {}
    build_dir = fixture / ".rex" / "build"
    if build_dir.exists():
        for f in build_dir.rglob("*.js"):
            rel = str(f.relative_to(build_dir))
            size = f.stat().st_size
            # Files under server/ are server-only (never sent to browser)
            if rel.startswith("server"):
                server_sizes[rel] = size
            else:
                client_sizes[rel] = size

    return {"server": server_sizes, "client": client_sizes}


def measure_nextjs_bundle_size(fixture: Path) -> dict[str, dict[str, int]]:
    """Build Next.js and measure output bundle sizes, split by server/client."""
    # Ensure built
    next_dir = fixture / ".next"
    if not next_dir.exists():
        result = subprocess.run(
            ["npx", "next", "build"],
            cwd=str(fixture),
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            print(f"  Next.js build failed: {result.stderr[:200]}", file=sys.stderr)
            return {"server": {}, "client": {}}

    server_sizes = {}
    client_sizes = {}

    # Next.js outputs:
    #   .next/server/ — server bundles (RSC, SSR, not sent to browser)
    #   .next/static/ — client bundles (shipped to browser)
    server_dir = next_dir / "server"
    static_dir = next_dir / "static"

    if server_dir.exists():
        for f in server_dir.rglob("*.js"):
            rel = str(f.relative_to(next_dir))
            server_sizes[rel] = f.stat().st_size

    if static_dir.exists():
        for f in static_dir.rglob("*.js"):
            rel = str(f.relative_to(next_dir))
            client_sizes[rel] = f.stat().st_size

    return {"server": server_sizes, "client": client_sizes}


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


def fmt_kb(size_bytes: int) -> str:
    """Format bytes as KB with 1 decimal."""
    return f"{size_bytes / 1024:.1f} KB"


def run_build_suite(iterations: int) -> BenchSuite:
    suite = BenchSuite(name="build")
    print("\n=== Build Time ===")

    pages_median = None
    rex_app_median = None

    if REX_BIN.exists():
        # Pages router build
        print(f"  Rex pages router ({PAGES_FIXTURE.name})...")
        pages_times = measure_build_time(PAGES_FIXTURE, iterations)
        if pages_times:
            pages_median = statistics.median(pages_times)
            suite.results.append(BenchResult("rex_pages_build_time", pages_median, "ms"))
            print(f"    {pages_median:.0f}ms (median of {len(pages_times)})")

        # App router build
        print(f"  Rex app router ({APP_FIXTURE.name})...")
        app_times = measure_build_time(APP_FIXTURE, iterations)
        if app_times:
            rex_app_median = statistics.median(app_times)
            ratio = rex_app_median / pages_median if pages_median else 0
            suite.results.append(
                BenchResult(
                    "rex_app_build_time", rex_app_median, "ms", f"{ratio:.1f}x vs rex pages"
                )
            )
            print(
                f"    {rex_app_median:.0f}ms (median of {len(app_times)}, {ratio:.1f}x vs rex pages)"
            )
    else:
        print(f"  Rex binary not found at {REX_BIN}, skipping Rex builds")

    # Next.js build
    if NEXTJS_FIXTURE.exists() and (NEXTJS_FIXTURE / "node_modules").exists():
        print(f"  Next.js app router ({NEXTJS_FIXTURE.name})...")
        next_times = measure_nextjs_build_time(NEXTJS_FIXTURE, iterations)
        if next_times:
            next_median = statistics.median(next_times)
            comparison = ""
            if rex_app_median:
                ratio = next_median / rex_app_median
                comparison = f"{ratio:.1f}x vs rex app"
            suite.results.append(
                BenchResult("nextjs_app_build_time", next_median, "ms", comparison)
            )
            print(
                f"    {next_median:.0f}ms (median of {len(next_times)}{', ' + comparison if comparison else ''})"
            )
    else:
        print("  Next.js fixture not found or not installed, skipping")

    return suite


def run_render_suite(iterations: int) -> BenchSuite:
    suite = BenchSuite(name="render")
    print("\n=== Server Render Latency ===")

    pages_median = None
    rex_rsc_median = None

    if not REX_BIN.exists():
        print(f"  Rex binary not found at {REX_BIN}, skipping Rex servers")
    else:
        # Pages router server
        pages_port = find_free_port()
        print(f"  Starting Rex pages server on port {pages_port}...")
        pages_proc = start_server(PAGES_FIXTURE, pages_port)

        try:
            print("  Measuring pages render...")
            pages_times = measure_render_latency(pages_port, "/", iterations)
            if pages_times:
                pages_median = statistics.median(pages_times)
                suite.results.append(BenchResult("rex_pages_render_p50", pages_median, "ms"))
                print(f"    p50: {pages_median:.1f}ms")
                if len(pages_times) >= 10:
                    p99 = sorted(pages_times)[int(len(pages_times) * 0.99)]
                    suite.results.append(BenchResult("rex_pages_render_p99", p99, "ms"))
                    print(f"    p99: {p99:.1f}ms")
        finally:
            pages_proc.kill()
            pages_proc.wait()

        # App router server
        app_port = find_free_port()
        print(f"  Starting Rex app server on port {app_port}...")
        try:
            app_proc = start_server(APP_FIXTURE, app_port)
        except RuntimeError as e:
            print(f"  Rex app server failed: {e}")
            return suite

        try:
            print("  Measuring RSC render...")
            app_times = measure_render_latency(app_port, "/", iterations)
            if app_times:
                rex_rsc_median = statistics.median(app_times)
                ratio = rex_rsc_median / pages_median if pages_median else 0
                suite.results.append(
                    BenchResult(
                        "rex_rsc_render_p50", rex_rsc_median, "ms", f"{ratio:.1f}x vs rex pages"
                    )
                )
                print(f"    p50: {rex_rsc_median:.1f}ms ({ratio:.1f}x vs rex pages)")
                if len(app_times) >= 10:
                    p99 = sorted(app_times)[int(len(app_times) * 0.99)]
                    suite.results.append(BenchResult("rex_rsc_render_p99", p99, "ms"))
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

    # Next.js server
    if NEXTJS_FIXTURE.exists() and (NEXTJS_FIXTURE / ".next").exists():
        next_port = find_free_port()
        print(f"  Starting Next.js server on port {next_port}...")
        try:
            next_proc = start_nextjs_server(NEXTJS_FIXTURE, next_port)
        except RuntimeError as e:
            print(f"  Next.js server failed: {e}")
            return suite

        try:
            print("  Measuring Next.js RSC render...")
            next_times = measure_render_latency(next_port, "/", iterations)
            if next_times:
                next_median = statistics.median(next_times)
                comparison = ""
                if rex_rsc_median:
                    ratio = next_median / rex_rsc_median
                    comparison = f"{ratio:.1f}x vs rex rsc"
                suite.results.append(
                    BenchResult("nextjs_rsc_render_p50", next_median, "ms", comparison)
                )
                print(
                    f"    p50: {next_median:.1f}ms{' (' + comparison + ')' if comparison else ''}"
                )
        finally:
            next_proc.kill()
            next_proc.wait()
    else:
        print("  Next.js fixture not built, skipping (run `next build` first)")

    return suite


def run_bundle_suite() -> BenchSuite:
    suite = BenchSuite(name="bundle")
    print("\n=== Bundle Size ===")

    rex_pages_client = 0
    rex_app_client = 0

    if REX_BIN.exists():
        # Rex Pages Router
        print(f"  Rex pages router ({PAGES_FIXTURE.name})...")
        pages_sizes = measure_bundle_size(PAGES_FIXTURE)
        pages_server = sum(pages_sizes["server"].values())
        pages_client = sum(pages_sizes["client"].values())
        rex_pages_client = pages_client
        suite.results.append(BenchResult("rex_pages_server_js", pages_server, "bytes"))
        suite.results.append(BenchResult("rex_pages_client_js", pages_client, "bytes"))
        print(
            f"    Server: {fmt_kb(pages_server)} ({len(pages_sizes['server'])} files) — not sent to browser"
        )
        print(
            f"    Client: {fmt_kb(pages_client)} ({len(pages_sizes['client'])} files) — shipped to browser"
        )

        # Rex App Router
        print(f"  Rex app router ({APP_FIXTURE.name})...")
        app_sizes = measure_bundle_size(APP_FIXTURE)
        app_server = sum(app_sizes["server"].values())
        app_client = sum(app_sizes["client"].values())
        rex_app_client = app_client
        suite.results.append(BenchResult("rex_app_server_js", app_server, "bytes"))
        suite.results.append(BenchResult("rex_app_client_js", app_client, "bytes"))
        client_ratio = f"{app_client / pages_client:.2f}x vs rex pages" if pages_client else ""
        print(
            f"    Server: {fmt_kb(app_server)} ({len(app_sizes['server'])} files) — not sent to browser"
        )
        print(
            f"    Client: {fmt_kb(app_client)} ({len(app_sizes['client'])} files) — shipped to browser ({client_ratio})"
        )

        # List individual client files for app router
        if app_sizes["client"]:
            for name, size in sorted(app_sizes["client"].items()):
                print(f"      {name}: {fmt_kb(size)}")
    else:
        print(f"  Rex binary not found at {REX_BIN}, skipping Rex builds")

    # Next.js App Router
    if NEXTJS_FIXTURE.exists() and (NEXTJS_FIXTURE / "node_modules").exists():
        print(f"  Next.js app router ({NEXTJS_FIXTURE.name})...")
        next_sizes = measure_nextjs_bundle_size(NEXTJS_FIXTURE)
        next_server = sum(next_sizes["server"].values())
        next_client = sum(next_sizes["client"].values())
        suite.results.append(BenchResult("nextjs_app_server_js", next_server, "bytes"))
        suite.results.append(BenchResult("nextjs_app_client_js", next_client, "bytes"))
        comparison = ""
        if rex_app_client:
            ratio = next_client / rex_app_client if rex_app_client else 0
            comparison = f"{ratio:.1f}x vs rex app" if ratio else ""
        print(
            f"    Server: {fmt_kb(next_server)} ({len(next_sizes['server'])} files) — not sent to browser"
        )
        print(
            f"    Client: {fmt_kb(next_client)} ({len(next_sizes['client'])} files) — shipped to browser{' (' + comparison + ')' if comparison else ''}"
        )
    else:
        print("  Next.js fixture not found or not installed, skipping")

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
    print(f"  Next.js fixture: {NEXTJS_FIXTURE}")

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
