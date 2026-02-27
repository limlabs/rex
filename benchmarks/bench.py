#!/usr/bin/env python3
"""
Rex Benchmark Suite — Rex vs Next.js 15 (Pages & App Router) vs TanStack Start

Compares three frameworks on identical page fixtures across three suites:
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
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import socket
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
NEXT_DIR = SCRIPT_DIR / "next-basic"
NEXT_APP_DIR = SCRIPT_DIR / "next-app-basic"
TANSTACK_DIR = SCRIPT_DIR / "tanstack-basic"

ENDPOINTS = ["/", "/about", "/blog/hello-world", "/api/hello"]
ENDPOINT_LABELS = {
    "/": "SSR index",
    "/about": "SSG about",
    "/blog/hello-world": "Dynamic route",
    "/api/hello": "API route",
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


FW_COLOR = {"rex": magenta, "nextjs": cyan, "nextjs_app": cyan, "tanstack": yellow}
FW_LABEL = {
    "rex": "Rex",
    "nextjs": "Next.js (Pages)",
    "nextjs_app": "Next.js (App)",
    "tanstack": "TanStack Start",
}


def section(fw: str, suite: str):
    color = FW_COLOR.get(fw, dim)
    label = FW_LABEL.get(fw, fw)
    print(f"\n{color(f'━━━ {label} ({suite}) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━')}", flush=True)


# ── Helpers ─────────────────────────────────────────────────────


def find_free_port() -> int:
    with socket.socket() as s:
        s.bind(("", 0))
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
            ["ab", "-n", str(requests), "-c", str(concurrency), url],
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


# ════════════════════════════════════════════════════════════════
# DX SUITE
# ════════════════════════════════════════════════════════════════


def dx_framework(
    fw: str,
    project_dir: Path,
    start_fn,
    about_page: Path,
):
    progress(fw, "dx")
    section(fw, "DX")

    # ── Dependencies & node_modules size ──
    nm = project_dir / "node_modules"
    if nm.exists():
        deps = count_deps(project_dir)
        nm_mb = dir_size_mb(nm)
        print(f"  {bold('Dependencies:')}   {green(str(deps))}")
        print(f"  {bold('node_modules:')}   {green(f'{nm_mb}MB')}")
        add("dx", fw, "dependency_count", deps)
        add("dx", fw, "node_modules_mb", nm_mb)

    # ── npm install time (clean) ──
    print(f"  {dim('Measuring npm install (clean)...')}")
    backup = None
    if nm.exists():
        backup = Path(tempfile.mkdtemp())
        nm.rename(backup / "node_modules")

    t0 = time.monotonic()
    subprocess.run(
        ["npm", "install", "--prefer-offline", "--no-audit", "--no-fund"],
        cwd=project_dir,
        capture_output=True,
        timeout=120,
    )
    install_ms = round((time.monotonic() - t0) * 1000)
    print(f"  {bold('Install time:')}   {green(f'{install_ms}ms')}")
    add("dx", fw, "install_time_ms", install_ms)

    if backup:
        shutil.rmtree(backup, ignore_errors=True)

    # ── Cold start (dev) ──
    port = find_free_port()
    t0 = time.monotonic()
    server = start_fn("dev", port)
    if server is None:
        print(f"  {yellow('Could not start dev server')}")
        return

    with server:
        # Hit once to fully warm up
        curl_body(port, "/")
        # JS frameworks compile on first request
        if fw != "rex":
            time.sleep(2)
        cold_ms = round((time.monotonic() - t0) * 1000)
        print(f"  {bold('Cold start:')}     {green(f'{cold_ms}ms')}")
        add("dx", fw, "cold_start_ms", cold_ms)

        # ── Dev memory ──
        mem = server.rss_mb()
        print(f"  {bold('Dev memory:')}     {green(f'{mem}MB')}")
        add("dx", fw, "dev_memory_mb", mem)

        # ── HMR rebuild time ──
        if about_page.exists():
            original = about_page.read_text()
            marker = f"__BENCH_MARKER_{int(time.time())}__"
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

            rebuild_ms = round((time.monotonic() - t0) * 1000)
            about_page.write_text(original)

            if found:
                print(f"  {bold('Rebuild time:')}   {green(f'{rebuild_ms}ms')}")
                add("dx", fw, "rebuild_ms", rebuild_ms)
            else:
                print(f"  {yellow('Rebuild: timed out (20s)')}")

    print()


def run_dx(frameworks: list[str]):
    print(f"\n  {bold('▸ DX Suite')} — developer experience metrics\n")

    if "rex" in frameworks:
        dx_framework("rex", REX_FIXTURE, start_rex, REX_FIXTURE / "pages/about.tsx")
    if "nextjs" in frameworks:
        dx_framework("nextjs", NEXT_DIR, start_next, NEXT_DIR / "pages/about.tsx")
    if "nextjs_app" in frameworks:
        dx_framework(
            "nextjs_app", NEXT_APP_DIR, start_next_app, NEXT_APP_DIR / "app/about/page.tsx"
        )
    if "tanstack" in frameworks:
        dx_framework(
            "tanstack", TANSTACK_DIR, start_tanstack, TANSTACK_DIR / "src/routes/about.tsx"
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
):
    progress(fw, "server")
    section(fw, "Server")

    # ── Build ──
    t0 = time.monotonic()
    if not build_fn():
        return
    build_ms = round((time.monotonic() - t0) * 1000)
    print(f"  {bold('Build time:')}    {green(f'{build_ms}ms')}")
    add("server", fw, "build_time_ms", build_ms)

    if build_output_dir and build_output_dir.exists():
        build_mb = dir_size_mb(build_output_dir)
        print(f"  {bold('Build output:')}  {green(f'{build_mb}MB')}")
        add("server", fw, "build_output_mb", build_mb)

    # ── Start production server ──
    port = find_free_port()
    t0 = time.monotonic()
    server = start_fn("start", port)
    if server is None:
        return

    with server:
        curl_body(port, "/")
        cold_ms = round((time.monotonic() - t0) * 1000)
        print(f"  {bold('Cold start:')}    {green(f'{cold_ms}ms')}")
        add("server", fw, "cold_start_ms", cold_ms)

        # ── Benchmark endpoints ──
        for ep in ENDPOINTS:
            label = ENDPOINT_LABELS.get(ep, ep)
            print(f"\n  {bold(label)}")
            ab = run_ab(f"http://127.0.0.1:{port}{ep}", requests, concurrency, warmup)
            add("server", fw, "rps", ab.rps, endpoint=ep)
            add("server", fw, "latency_mean_ms", ab.latency_mean_ms, endpoint=ep)
            add("server", fw, "latency_p50_ms", ab.latency_p50_ms, endpoint=ep)
            add("server", fw, "latency_p99_ms", ab.latency_p99_ms, endpoint=ep)

        # ── Memory ──
        mem = server.rss_mb()
        print(f"\n  {bold('Memory (RSS):')}  {green(f'{mem}MB')}")
        add("server", fw, "memory_mb", mem)

    print()


def run_server(frameworks: list[str], requests: int, concurrency: int, warmup: int):
    if not shutil.which("ab"):
        print(f"  {yellow('SKIP server suite: ab (Apache Bench) not found')}")
        return

    print(f"\n  {bold('▸ Server Suite')} — production throughput & latency")
    print(
        f"  {dim('Requests:')} {requests}  {dim('Concurrency:')} {concurrency}  {dim('Warmup:')} {warmup}\n"
    )

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
    print(f"  {bold('Total JS:')}    {green(f'{js_kb}KB')}")
    print(f"  {bold('Total CSS:')}   {green(f'{css_kb}KB')}")
    add("client", fw, "total_js_kb", js_kb)
    add("client", fw, "total_css_kb", css_kb)
    print()


def lighthouse_audit(fw: str, port: int):
    """Run Lighthouse on key pages if available."""
    if not shutil.which("lighthouse") and not shutil.which("npx"):
        return

    fw_label = FW_LABEL.get(fw, fw)
    color = FW_COLOR.get(fw, dim)
    print(f"\n  {color(f'Lighthouse — {fw_label}')}", flush=True)

    pages = [("/", "index"), ("/about", "about"), ("/blog/hello-world", "blog")]

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
                    "--throttling-method=devtools",
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

    # Lighthouse
    if has_lighthouse:
        print(f"  {dim('Running Lighthouse audits (this takes a while)...')}\n")

        if "rex" in frameworks and REX_BIN.exists():
            port = find_free_port()
            server = start_rex("start", port)
            if server:
                with server:
                    lighthouse_audit("rex", port)

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
        default="rex,nextjs,nextjs_app,tanstack",
        help="Comma-separated: rex, nextjs, nextjs_app, tanstack (default: all)",
    )
    parser.add_argument("--json", dest="json_file", help="Write results to JSON file")
    parser.add_argument(
        "--requests", type=int, default=5000, help="Requests per benchmark (default: 5000)"
    )
    parser.add_argument(
        "--concurrency", type=int, default=50, help="Concurrent connections (default: 50)"
    )
    parser.add_argument("--warmup", type=int, default=100, help="Warmup requests (default: 100)")
    args = parser.parse_args()

    suites = [s.strip() for s in args.suite.split(",")]
    frameworks = [f.strip() for f in args.framework.split(",")]

    # Compute total steps for progress reporting
    global _total_steps
    _total_steps = len(suites) * len(frameworks)

    # Banner
    print(f"\n  {bold('Rex Benchmark Suite')}\n")
    print(f"  {dim('Suites:')}      {', '.join(suites)}")
    fw_strs = [FW_COLOR.get(fw, dim)(FW_LABEL.get(fw, fw)) for fw in frameworks]
    print(f"  {dim('Frameworks:')}  {' '.join(fw_strs)}")
    print()

    # Run
    if "dx" in suites:
        run_dx(frameworks)
    if "server" in suites:
        run_server(frameworks, args.requests, args.concurrency, args.warmup)
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
