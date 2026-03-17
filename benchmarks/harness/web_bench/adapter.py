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
    """Get the task prompt, optionally rewritten for Rex."""
    task_id = task["id"]
    if condition.startswith("rex") and task_id in REX_PROMPT_REWRITES:
        return REX_PROMPT_REWRITES[task_id]
    return task["description"]


# ---------------------------------------------------------------------------
# Workspace setup
# ---------------------------------------------------------------------------


def setup_workspace(condition: str) -> Path:
    """Create a workspace for the agent to work in."""
    tmp = Path(tempfile.mkdtemp(prefix=f"webbench_{condition}_"))

    if condition.startswith("rex"):
        # Rex starter — copy from our starters
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

        # Copy package.json and config
        shutil.copy(WEB_BENCH_DIR / "package.json", tmp / "package.json")
        (tmp / "CLAUDE.md").write_text(NEXTJS_GUIDED)

    # Install deps
    if (tmp / "package.json").exists():
        subprocess.run(
            ["npm", "install", "--no-audit", "--no-fund"],
            cwd=tmp,
            capture_output=True,
            timeout=120,
        )

    # Install Playwright for testing
    subprocess.run(
        ["npm", "install", "--no-audit", "--no-fund", "@playwright/test"],
        cwd=tmp,
        capture_output=True,
        timeout=60,
    )

    return tmp


# ---------------------------------------------------------------------------
# Playwright test runner
# ---------------------------------------------------------------------------


def _write_rex_playwright_config(workdir: Path, port: int) -> None:
    """Write a Playwright config that starts Rex dev server."""
    config = f"""\
const {{ defineConfig, devices }} = require('@playwright/test');
const PORT = {port};
module.exports = defineConfig({{
  testDir: './test',
  timeout: 60000,
  expect: {{ timeout: 10000 }},
  fullyParallel: true,
  retries: 0,
  reporter: 'line',
  use: {{
    baseURL: `http://localhost:${{PORT}}`,
    trace: 'off',
  }},
  webServer: {{
    command: `{REX_BIN} dev --root . --port ${{PORT}}`,
    url: `http://localhost:${{PORT}}`,
    reuseExistingServer: false,
    timeout: 30000,
  }},
  projects: [{{ name: 'chromium', use: {{ ...devices['Desktop Chrome'] }} }}],
}});
"""
    (workdir / "playwright.config.js").write_text(config)


def run_playwright_tests(
    workdir: Path,
    task_id: str,
    condition: str,
    port: int = 3211,
) -> dict:
    """Run Web-Bench Playwright tests for a specific task."""
    import socket

    # Find a free port
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("", 0))
        port = s.getsockname()[1]

    # Copy test files and libraries
    test_dir = workdir / "test"
    test_dir.mkdir(exist_ok=True)

    # Copy all test specs up to and including this task (sequential dependency)
    task_num = int(task_id.split("-")[1])
    for i in range(1, task_num + 1):
        spec = WEB_BENCH_DIR / f"test/task-{i}.spec.js"
        if spec.exists():
            shutil.copy(spec, test_dir / f"task-{i}.spec.js")

    # Copy test utility libraries
    lib_dir = workdir / "node_modules" / "@web-bench"
    lib_dir.mkdir(parents=True, exist_ok=True)
    if (LIBRARIES_DIR / "test-util").exists():
        shutil.copytree(LIBRARIES_DIR / "test-util", lib_dir / "test-util", dirs_exist_ok=True)
    if (LIBRARIES_DIR / "shop-test-util").exists():
        shutil.copytree(
            LIBRARIES_DIR / "shop-test-util", lib_dir / "shop-test-util", dirs_exist_ok=True
        )

    # Write appropriate Playwright config
    if condition.startswith("rex"):
        _write_rex_playwright_config(workdir, port)
    else:
        shutil.copy(WEB_BENCH_DIR / "playwright.config.js", workdir / "playwright.config.js")

    # Set up environment
    src_dir = "." if condition.startswith("rex") else "src"
    env = {
        **os.environ,
        "EVAL_PROJECT_ROOT": str(workdir / src_dir),
        "EVAL_PROJECT_PORT": str(port),
    }

    # Run only the current task's tests
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
        total = results.get("stats", {}).get("expected", 0)
        passed = total - results.get("stats", {}).get("unexpected", 0)
        return {
            "task_id": task_id,
            "total_tests": total,
            "passed": passed,
            "failed": total - passed,
            "pass_rate": passed / total if total > 0 else 0,
            "raw": results.get("stats", {}),
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

                # Run Playwright tests
                test_result = run_playwright_tests(workdir, tid, condition)
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
