#!/usr/bin/env python3
"""
Web-Bench adapter: runs the first N tasks from ByteDance's Web-Bench Next.js
project against Rex and Next.js, using Playwright tests for evaluation.

The Web-Bench Next.js project builds a shopping mart app through 20 sequential
tasks. Each task builds on the previous. Playwright tests verify correctness.

Usage:
    uv run python -m harness.web_bench.adapter                     # tasks 1-3
    uv run python -m harness.web_bench.adapter --tasks 5           # tasks 1-5
    uv run python -m harness.web_bench.adapter --condition rex_guided
    uv run python -m harness.web_bench.adapter --model claude-sonnet-4-6
"""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from claude_agent_sdk import ClaudeAgentOptions, ResultMessage, query

from ..guides import NEXTJS_GUIDED, REX_GUIDED
from ..runner import REX_BIN, bold, dim, green, red

WEB_BENCH_DIR = Path("/tmp/web-bench/projects/nextjs")
LIBRARIES_DIR = Path("/tmp/web-bench/libraries")


# ---------------------------------------------------------------------------
# Task loading
# ---------------------------------------------------------------------------


def load_web_bench_tasks(max_tasks: int = 3) -> list[dict]:
    """Load tasks from Web-Bench's tasks.jsonl."""
    tasks_file = WEB_BENCH_DIR / "tasks.jsonl"
    if not tasks_file.exists():
        print(f"{red('ERROR')}: Web-Bench not found at {WEB_BENCH_DIR}")
        print("Clone it: git clone --depth 1 https://github.com/bytedance/web-bench /tmp/web-bench")
        sys.exit(1)

    tasks = []
    with open(tasks_file) as f:
        for line in f:
            task = json.loads(line)
            tasks.append(task)
            if len(tasks) >= max_tasks:
                break
    return tasks


# ---------------------------------------------------------------------------
# Rex-adapted task prompts
# ---------------------------------------------------------------------------

# Tasks 1-3 reference App Router paths. We rewrite them for Pages Router.
REX_PROMPT_REWRITES = {
    "task-1": (
        "1) create home page (pages/index.tsx) at route '/' showing "
        "'🛍️🛍️🛍️ Welcome to Shopping Mart !' in h1 tag "
        "2) create login page (pages/login.tsx) at route '/login' showing "
        "'💡 Please Login First' in h1 tag"
    ),
    "task-2": (
        "1) Every page in this app will have an appealing header showing "
        "'🛍️ WebBench Shopping Mart', and a beautiful footer showing 'Copyright: Web Bench'. "
        "2) Add className 'site-header' to header and 'site-footer' to footer. "
        "3) Create a _app.tsx wrapper in pages/ to implement this — wrap all pages with "
        "the header and footer. "
        "4) header is fixed at the page top; footer is fixed at the page bottom; "
        "main(children) occupies the remaining space. "
        "5) create a CSS file and import it to beautify CSS."
    ),
    "task-3": (
        "1) Create a custom 404 page (pages/404.tsx) that shows "
        "'Oops! Looks like you have wandered off the beaten path.' in h1 tag. "
        "Add a button with class 'not-found-go-to-home' that navigates to '/'. "
        "2) When clicking '🛍️ WebBench Shopping Mart' in the header, navigate to '/'. "
        "3) Style the 404 page with CSS."
    ),
}


def get_prompt(task: dict, condition: str) -> str:
    """Get the task prompt, optionally rewritten for Rex Pages Router."""
    task_id = task["id"]
    # Rex App Router and Next.js use the original prompts unchanged
    if condition == "rex_guided" and task_id in REX_PROMPT_REWRITES:
        return REX_PROMPT_REWRITES[task_id]
    return task["description"]


# ---------------------------------------------------------------------------
# Workspace setup
# ---------------------------------------------------------------------------


def setup_workspace(condition: str) -> Path:
    """Create a workspace for the agent to work in."""
    tmp = Path(tempfile.mkdtemp(prefix=f"webbench_{condition}_"))

    if condition == "rex_app":
        # Rex App Router — use app/ directory, same conventions as Next.js App Router
        from ..guides import REX_APP_GUIDED

        starters_dir = Path(__file__).parent.parent / "starters" / "rex-app"
        shutil.copytree(starters_dir, tmp, dirs_exist_ok=True)
        (tmp / "app").mkdir(exist_ok=True)
        (tmp / "CLAUDE.md").write_text(REX_APP_GUIDED)
    elif condition.startswith("rex"):
        # Rex Pages Router
        starters_dir = Path(__file__).parent.parent / "starters" / "rex"
        shutil.copytree(starters_dir, tmp, dirs_exist_ok=True)
        (tmp / "pages").mkdir(exist_ok=True)
        (tmp / "CLAUDE.md").write_text(REX_GUIDED)

        # Symlink rex binary
        rex_bin = Path(REX_BIN)
        if rex_bin.exists():
            link = tmp / "rex"
            if not link.exists():
                link.symlink_to(rex_bin.resolve())

    elif condition.startswith("nextjs"):
        # Next.js starter from Web-Bench's src-init
        src_init = WEB_BENCH_DIR / "src-init"
        shutil.copytree(src_init, tmp / "src", dirs_exist_ok=True)

        # Copy package.json, strip workspace: refs, add @playwright/test
        pkg = json.loads((WEB_BENCH_DIR / "package.json").read_text())
        # Remove workspace:* refs (npm can't resolve them — they're pnpm-only)
        for section in ("dependencies", "devDependencies"):
            if section in pkg:
                pkg[section] = {
                    k: v for k, v in pkg[section].items() if not str(v).startswith("workspace:")
                }
        pkg.setdefault("devDependencies", {})["@playwright/test"] = "^1.58.0"
        (tmp / "package.json").write_text(json.dumps(pkg, indent=2))
        (tmp / "CLAUDE.md").write_text(NEXTJS_GUIDED)

    # Install deps
    if (tmp / "package.json").exists():
        subprocess.run(
            ["npm", "install", "--no-audit", "--no-fund"],
            cwd=tmp,
            capture_output=True,
            timeout=120,
        )

    # Install Playwright for testing — use --legacy-peer-deps to avoid conflicts
    # with framework packages that may have strict peer deps
    subprocess.run(
        [
            "npm",
            "install",
            "--no-audit",
            "--no-fund",
            "--legacy-peer-deps",
            "@playwright/test",
        ],
        cwd=tmp,
        capture_output=True,
        timeout=60,
    )

    return tmp


# ---------------------------------------------------------------------------
# Playwright test runner
# ---------------------------------------------------------------------------


def _write_playwright_config(workdir: Path, port: int, server_cmd: str | None = None) -> None:
    """Write a Playwright config. If server_cmd is None, expect an already-running server."""
    web_server_block = ""
    if server_cmd:
        web_server_block = f"""
  webServer: {{
    command: `{server_cmd}`,
    url: `http://localhost:{port}`,
    reuseExistingServer: true,
    timeout: 30000,
  }},"""

    config = f"""\
const {{ defineConfig, devices }} = require('@playwright/test');
module.exports = defineConfig({{
  testDir: './test',
  timeout: 60000,
  expect: {{ timeout: 10000 }},
  fullyParallel: true,
  retries: 0,
  reporter: 'line',
  use: {{
    baseURL: 'http://localhost:{port}',
    trace: 'off',
  }},{web_server_block}
  projects: [{{ name: 'chromium', use: {{ ...devices['Desktop Chrome'] }} }}],
}});
"""
    (workdir / "playwright.config.js").write_text(config)


def _setup_test_libs(workdir: Path) -> None:
    """Copy Playwright test specs and Web-Bench library utilities."""
    test_dir = workdir / "test"
    test_dir.mkdir(exist_ok=True)

    # Copy all test specs
    for spec in sorted(WEB_BENCH_DIR.glob("test/task-*.spec.js")):
        shutil.copy(spec, test_dir / spec.name)

    # Copy test utility libraries
    lib_dir = workdir / "node_modules" / "@web-bench"
    lib_dir.mkdir(parents=True, exist_ok=True)
    if (LIBRARIES_DIR / "test-util").exists():
        shutil.copytree(LIBRARIES_DIR / "test-util", lib_dir / "test-util", dirs_exist_ok=True)
    if (LIBRARIES_DIR / "shop-test-util").exists():
        shutil.copytree(
            LIBRARIES_DIR / "shop-test-util", lib_dir / "shop-test-util", dirs_exist_ok=True
        )


def _start_rex_server(workdir: Path, port: int) -> subprocess.Popen:
    """Start Rex dev server and wait for it to respond."""
    proc = subprocess.Popen(
        [REX_BIN, "dev", "--root", str(workdir), "--port", str(port)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    # Wait for server to start (poll until any response)
    import requests

    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        try:
            requests.get(f"http://localhost:{port}/", timeout=1)
            return proc  # Any response means server is up (even 404)
        except (requests.ConnectionError, requests.Timeout):
            pass
        if proc.poll() is not None:
            break
        time.sleep(0.25)
    return proc


def run_playwright_tests(
    workdir: Path,
    task_id: str,
    condition: str,
    port: int = 0,
    server_proc: subprocess.Popen | None = None,
) -> dict:
    """Run Web-Bench Playwright tests for a specific task.

    If server_proc is provided, the server is already running.
    Otherwise, Playwright's webServer config starts it.
    """
    # Write Playwright config (no webServer — we manage the server ourselves)
    _write_playwright_config(workdir, port, server_cmd=None)

    src_dir = "." if condition.startswith("rex") else "src"
    env = {
        **os.environ,
        "EVAL_PROJECT_ROOT": str(workdir / src_dir),
        "EVAL_PROJECT_PORT": str(port),
    }

    playwright_cmd = [
        "npx",
        "playwright",
        "test",
        f"test/{task_id}.spec.js",
        "--reporter=json",
    ]

    proc = subprocess.run(
        playwright_cmd,
        cwd=workdir,
        capture_output=True,
        text=True,
        timeout=120,
        env=env,
    )

    # Parse results
    try:
        results = json.loads(proc.stdout)
        stats = results.get("stats", {})
        # Playwright JSON: expected=passed, unexpected=failed, skipped, flaky
        passed = stats.get("expected", 0)
        failed = stats.get("unexpected", 0)
        skipped = stats.get("skipped", 0)
        flaky = stats.get("flaky", 0)
        total = passed + failed + skipped + flaky
        return {
            "task_id": task_id,
            "total_tests": total,
            "passed": passed,
            "failed": failed,
            "pass_rate": passed / total if total > 0 else 0,
            "raw": stats,
        }
    except (json.JSONDecodeError, KeyError):
        return {
            "task_id": task_id,
            "total_tests": 0,
            "passed": 0,
            "failed": 0,
            "pass_rate": 0,
            "error": proc.stderr[:500] if proc.stderr else proc.stdout[:500],
        }


# ---------------------------------------------------------------------------
# Agent runner
# ---------------------------------------------------------------------------


async def run_agent_on_task(
    task: dict,
    condition: str,
    workdir: Path,
    model: str,
) -> dict:
    """Run the agent on a single Web-Bench task."""
    # Clear nesting guard
    env_overrides = {}
    for key in ("CLAUDECODE", "CLAUDE_CODE"):
        if key in os.environ:
            env_overrides[key] = os.environ.pop(key)

    prompt = get_prompt(task, condition)

    options = ClaudeAgentOptions(
        cwd=str(workdir) if condition.startswith("rex") else str(workdir / "src"),
        model=model,
        permission_mode="bypassPermissions",
        allowed_tools=["Read", "Write", "Edit", "Bash", "Glob", "Grep"],
    )

    t0 = time.monotonic()
    tokens = 0
    turns = 0
    cost = 0.0

    try:
        async for message in query(prompt=prompt, options=options):
            if isinstance(message, ResultMessage):
                cost = message.total_cost_usd or 0.0
                turns = getattr(message, "num_turns", 0) or 0
                usage = message.usage or {}
                tokens = (
                    usage.get("input_tokens", 0)
                    + usage.get("cache_read_input_tokens", 0)
                    + usage.get("output_tokens", 0)
                )
    finally:
        for key, val in env_overrides.items():
            os.environ[key] = val

    elapsed_ms = (time.monotonic() - t0) * 1000

    return {
        "task_id": task["id"],
        "tokens": tokens,
        "turns": turns,
        "cost_usd": cost,
        "elapsed_ms": elapsed_ms,
    }


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


async def async_main() -> None:
    import argparse

    parser = argparse.ArgumentParser(description="Web-Bench adapter for Rex harness benchmark")
    parser.add_argument("--tasks", type=int, default=3, help="Number of tasks to run (1-20)")
    parser.add_argument(
        "--condition",
        nargs="+",
        default=["rex_guided", "nextjs_guided"],
        help="Conditions to test",
    )
    parser.add_argument("--model", default="claude-haiku-4-5-20251001")
    parser.add_argument("--json", help="Output JSON file")
    args = parser.parse_args()

    if not os.environ.get("ANTHROPIC_API_KEY"):
        print(f"{red('ERROR')}: ANTHROPIC_API_KEY not set")
        sys.exit(1)

    tasks = load_web_bench_tasks(args.tasks)
    print(bold("Web-Bench Adapter"))
    print(f"Tasks: {len(tasks)}, Conditions: {args.condition}, Model: {args.model}")
    print()

    all_results = []

    for condition in args.condition:
        print(f"{bold(condition)}")
        workdir = setup_workspace(condition)
        _setup_test_libs(workdir)

        # Find a free port and start the server once for all tasks
        import socket

        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.bind(("", 0))
            port = s.getsockname()[1]

        server = None
        try:
            for task in tasks:
                tid = task["id"]
                label = f"  {tid}"
                print(f"{label:<20} ", end="", flush=True)

                # Run agent
                agent_result = await run_agent_on_task(task, condition, workdir, args.model)
                tok_str = f"{agent_result['tokens'] // 1000}k"
                print(
                    dim(f"agent: {tok_str} tok, {agent_result['elapsed_ms']/1000:.0f}s"),
                    end="  ",
                    flush=True,
                )

                # Start/restart server after agent modifies files
                if condition.startswith("rex"):
                    if server:
                        server.terminate()
                        server.wait(timeout=5)
                    server = _start_rex_server(workdir, port)
                elif condition.startswith("nextjs"):
                    # For Next.js, restart dev server (next dev handles its own HMR
                    # but we need a fresh start to pick up new files)
                    if server:
                        server.terminate()
                        server.wait(timeout=5)
                    server = subprocess.Popen(
                        ["npx", "next", "dev", "--port", str(port)],
                        cwd=workdir / "src",
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                    )
                    # Wait for server to start
                    import requests as _req

                    deadline = time.monotonic() + 60
                    while time.monotonic() < deadline:
                        try:
                            _req.get(f"http://localhost:{port}/", timeout=1)
                            break
                        except Exception:
                            pass
                        if server.poll() is not None:
                            break
                        time.sleep(0.5)

                # Run Playwright tests
                test_result = run_playwright_tests(workdir, tid, condition, port=port)
                if test_result["total_tests"] > 0:
                    pct = test_result["pass_rate"]
                    status = green(f"{pct:.0%}") if pct == 1 else red(f"{pct:.0%}")
                    print(f"{status} ({test_result['passed']}/{test_result['total_tests']})")
                else:
                    err = test_result.get("error", "no tests ran")[:100]
                    print(f"{red('ERR')} {dim(err)}")

                all_results.append(
                    {
                        **agent_result,
                        "condition": condition,
                        "test": test_result,
                    }
                )

        finally:
            if server:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
            shutil.rmtree(workdir, ignore_errors=True)

        print()

    # Summary
    print(bold("Summary"))
    conditions = sorted(set(r["condition"] for r in all_results))
    print(f"{'Task':<12}" + "".join(f"{c:>20}" for c in conditions))
    print("-" * (12 + 20 * len(conditions)))

    for tid in sorted(set(r["task_id"] for r in all_results)):
        row = f"{tid:<12}"
        for c in conditions:
            match = next(
                (r for r in all_results if r["task_id"] == tid and r["condition"] == c), None
            )
            if match and match["test"]["total_tests"] > 0:
                pct = match["test"]["pass_rate"]
                tok = match["tokens"] // 1000
                cell = f"{pct:.0%} ({tok}k)"
                row += f"{cell:>20}"
            else:
                row += f"{'ERR':>20}"
        print(row)

    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps(all_results, indent=2))
        print(f"\nResults written to {args.json}")


def main():
    asyncio.run(async_main())


if __name__ == "__main__":
    main()
