#!/usr/bin/env python3
"""
Rex Harness Benchmark Runner

Compares agent performance across conditions (rex_harness vs rex_raw) on a
suite of web development tasks. Measures completion rate, correctness, token
efficiency, and tool call count.

Usage:
    uv run python -m harness.runner                              # all tasks, all conditions
    uv run python -m harness.runner --task t1-01                 # single task
    uv run python -m harness.runner --tier 1                     # all tier 1 tasks
    uv run python -m harness.runner --condition rex_harness      # single condition
    uv run python -m harness.runner --runs 5                     # 5 runs per combo
    uv run python -m harness.runner --model claude-sonnet-4-6-20250514
    uv run python -m harness.runner --json results/run.json
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

from .agent import run_agent
from .conditions import (
    Condition,
    make_harness_executor,
    make_raw_executor,
    rex_harness,
    rex_raw,
)
from .evaluator import EvalResult, evaluate

HARNESS_DIR = Path(__file__).parent
TASKS_DIR = HARNESS_DIR / "tasks"
STARTERS_DIR = HARNESS_DIR / "starters"
PROJECT_ROOT = HARNESS_DIR.parent.parent


# Rex binary: prefer REX_BIN env var, then check local target/, then
# resolve through git to find the main worktree's target/ (worktrees share no target/).
def _find_rex_bin() -> str:
    if "REX_BIN" in os.environ:
        return os.environ["REX_BIN"]
    for base in [PROJECT_ROOT, PROJECT_ROOT.resolve()]:
        for profile in ["debug", "release"]:
            candidate = base / "target" / profile / "rex"
            if candidate.exists():
                return str(candidate)
    # In a git worktree, the main repo root may differ
    try:
        main_root = subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            capture_output=True,
            text=True,
            cwd=PROJECT_ROOT,
        )
        if main_root.returncode == 0:
            git_common = Path(main_root.stdout.strip())
            repo_root = git_common.parent  # .git -> repo root
            for profile in ["debug", "release"]:
                candidate = repo_root / "target" / profile / "rex"
                if candidate.exists():
                    return str(candidate)
    except Exception:
        pass
    return str(PROJECT_ROOT / "target/debug/rex")


REX_BIN = _find_rex_bin()


# ---------------------------------------------------------------------------
# ANSI colors
# ---------------------------------------------------------------------------


def green(s: str) -> str:
    return f"\033[32m{s}\033[0m"


def red(s: str) -> str:
    return f"\033[31m{s}\033[0m"


def yellow(s: str) -> str:
    return f"\033[33m{s}\033[0m"


def dim(s: str) -> str:
    return f"\033[2m{s}\033[0m"


def bold(s: str) -> str:
    return f"\033[1m{s}\033[0m"


# ---------------------------------------------------------------------------
# Task loading
# ---------------------------------------------------------------------------


def load_tasks(
    task_filter: str | None = None,
    tier_filter: int | None = None,
) -> list[dict]:
    tasks = []
    for f in sorted(TASKS_DIR.glob("*.toml")):
        with open(f, "rb") as fh:
            task = tomllib.load(fh)
        if task_filter and task["task"]["id"] != task_filter:
            continue
        if tier_filter is not None and task["task"]["tier"] != tier_filter:
            continue
        tasks.append(task)
    return tasks


# ---------------------------------------------------------------------------
# Workspace setup
# ---------------------------------------------------------------------------


def setup_workdir(starter: str) -> Path:
    """Copy a starter template to a temp directory and install dependencies."""
    src = STARTERS_DIR / starter
    if not src.exists():
        raise FileNotFoundError(f"Starter template not found: {src}")

    tmp = Path(tempfile.mkdtemp(prefix=f"harness_{starter}_"))
    shutil.copytree(src, tmp, dirs_exist_ok=True)

    # Ensure pages/ directory exists
    (tmp / "pages").mkdir(exist_ok=True)

    # Install npm dependencies
    pkg = tmp / "package.json"
    if pkg.exists():
        proc = subprocess.run(
            ["npm", "install", "--no-audit", "--no-fund"],
            cwd=tmp,
            capture_output=True,
            text=True,
            timeout=120,
        )
        if proc.returncode != 0:
            print(f"  {yellow('WARN')}: npm install failed: {proc.stderr[:200]}")

    return tmp


# ---------------------------------------------------------------------------
# Single run
# ---------------------------------------------------------------------------


def run_one(
    task: dict,
    condition: Condition,
    model: str,
    dry_run: bool = False,
) -> EvalResult:
    """Run a single task under a single condition. Returns evaluation result."""
    starter = task.get("starter", {}).get("template", condition.starter)
    workdir = setup_workdir(starter)

    try:
        # Run condition setup hook if any
        if condition.setup_hook:
            condition.setup_hook(workdir)

        # Inject any extra files
        inject = task.get("starter", {}).get("inject", {})
        for rel_path, content in inject.items():
            fp = workdir / rel_path
            fp.parent.mkdir(parents=True, exist_ok=True)
            fp.write_text(content)

        if dry_run:
            # Simulate a perfect agent: create the expected pages from the
            # task spec checks so we can validate the evaluator pipeline.
            metrics = _dry_run_simulate(task, workdir)
        else:
            # Choose executor based on condition
            if condition.name == "rex_harness":
                executor = make_harness_executor()
            else:
                executor = make_raw_executor()

            # Run the agent
            metrics = run_agent(
                prompt=task["task"]["prompt"],
                tools=condition.tools,
                workdir=workdir,
                tool_executor=executor,
                model=model,
            )

        # Evaluate the result
        return evaluate(task, condition, workdir, metrics)

    finally:
        shutil.rmtree(workdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Dry-run simulation
# ---------------------------------------------------------------------------

# Hardcoded "perfect answers" for each task. This lets us test the full
# pipeline (workdir setup, build, serve, evaluation) without an API call.
_DRY_RUN_PAGES: dict[str, dict[str, str]] = {
    "t1-01": {
        "pages/about.tsx": (
            'import React from "react";\n'
            "export default function AboutPage() {\n"
            "  return <div><h1>About Us</h1>"
            "<p>Learn more about our company.</p></div>;\n"
            "}\n"
        ),
    },
    "t1-02": {
        "pages/blog/[slug].tsx": (
            'import React from "react";\n'
            "export default function BlogSlugPage({ slug }: { slug: string }) {\n"
            "  return <div><h1>Blog: {slug}</h1>"
            "<p>You are reading {slug}</p></div>;\n"
            "}\n"
            "export async function getServerSideProps(context: any) {\n"
            "  return { props: { slug: context.params.slug } };\n"
            "}\n"
        ),
    },
    "t1-03": {
        "pages/users.tsx": (
            'import React from "react";\n'
            "interface User { name: string; role: string; }\n"
            "export default function UsersPage({ users }: { users: User[] }) {\n"
            "  return (\n"
            "    <div>\n"
            "      <h1>Team</h1>\n"
            "      <ul>\n"
            "        {users.map((u: User, i: number) => (\n"
            "          <li key={i}>{u.name} - {u.role}</li>\n"
            "        ))}\n"
            "      </ul>\n"
            "    </div>\n"
            "  );\n"
            "}\n"
            "export async function getServerSideProps() {\n"
            "  return {\n"
            "    props: {\n"
            "      users: [\n"
            '        { name: "Alice", role: "Engineer" },\n'
            '        { name: "Bob", role: "Designer" },\n'
            '        { name: "Carol", role: "Manager" },\n'
            "      ],\n"
            "    },\n"
            "  };\n"
            "}\n"
        ),
    },
}


def _dry_run_simulate(task: dict, workdir: Path) -> AgentMetrics:
    """Create the expected files for a task without calling the API."""
    from .agent import AgentMetrics

    tid = task["task"]["id"]
    pages = _DRY_RUN_PAGES.get(tid, {})
    for rel, content in pages.items():
        fp = workdir / rel
        fp.parent.mkdir(parents=True, exist_ok=True)
        fp.write_text(content)

    return AgentMetrics(
        input_tokens=0,
        output_tokens=0,
        tool_calls=len(pages),
        wall_clock_ms=0,
    )


# ---------------------------------------------------------------------------
# Aggregation helpers
# ---------------------------------------------------------------------------


def median(values: list[float]) -> float:
    if not values:
        return 0.0
    return statistics.median(values)


def aggregate_checks(runs: list[EvalResult]) -> dict[str, float]:
    """Per-check pass rate across runs."""
    check_names: dict[str, dict[str, int]] = {}
    for run in runs:
        for c in run.checks:
            if c.name not in check_names:
                check_names[c.name] = {"passed": 0, "total": 0}
            check_names[c.name]["total"] += 1
            if c.passed:
                check_names[c.name]["passed"] += 1
    return {k: v["passed"] / v["total"] for k, v in check_names.items()}


# ---------------------------------------------------------------------------
# Summary display
# ---------------------------------------------------------------------------


def print_summary(results: list[dict]) -> None:
    """Print a comparison table grouped by tier."""
    for tier in sorted(set(r["tier"] for r in results)):
        tier_results = [r for r in results if r["tier"] == tier]

        print(f"\n{'=' * 78}")
        print(f"  TIER {tier}")
        print(f"{'=' * 78}")

        conditions = sorted(set(r["condition"] for r in tier_results))
        col_w = 22
        header = f"{'Task':<20}" + "".join(f"{c:>{col_w}}" for c in conditions)
        print(header)
        print("-" * len(header))

        for tid in sorted(set(r["task_id"] for r in tier_results)):
            task_results = {r["condition"]: r for r in tier_results if r["task_id"] == tid}
            tname = next(r["task_name"] for r in tier_results if r["task_id"] == tid)

            row = f"{tname:<20}"
            for c in conditions:
                r = task_results.get(c)
                if r:
                    pct = r["pass_rate"]
                    tok = r["median_tokens"]
                    calls = r["median_tool_calls"]
                    tok_str = f"{tok // 1000}k" if tok >= 1000 else str(int(tok))

                    if pct == 1.0:
                        cell = green("100%") + dim(f" ({tok_str}, {int(calls)}c)")
                    elif pct > 0:
                        cell = yellow(f"{pct:.0%}") + dim(f" ({tok_str}, {int(calls)}c)")
                    else:
                        cell = red("0%") + dim(f" ({tok_str}, {int(calls)}c)")

                    # Pad for ANSI codes (they add invisible chars)
                    row += f"  {cell}"
                else:
                    row += f"{'--':>{col_w}}"
            print(row)

    # Print legend
    print(f"\n{dim('(Nk = median tokens, Nc = median tool calls)')}")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Rex Harness Benchmark — compare agent conditions on web dev tasks"
    )
    parser.add_argument("--task", help="Run a specific task ID (e.g. t1-01)")
    parser.add_argument("--tier", type=int, help="Run all tasks in a tier (e.g. 1)")
    parser.add_argument(
        "--condition",
        nargs="+",
        choices=["rex_harness", "rex_raw"],
        help="Conditions to test (default: all)",
    )
    parser.add_argument(
        "--runs",
        type=int,
        default=3,
        help="Runs per task x condition combo (default: 3)",
    )
    parser.add_argument(
        "--model",
        default="claude-sonnet-4-20250514",
        help="Anthropic model to use",
    )
    parser.add_argument("--json", help="Write results to a JSON file")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Test everything except the API call: setup workdir, create pages manually, evaluate",
    )
    args = parser.parse_args()

    # Check API key (unless dry-run)
    if not args.dry_run and not os.environ.get("ANTHROPIC_API_KEY"):
        print(f"{red('ERROR')}: ANTHROPIC_API_KEY environment variable is not set.")
        print("Set it with: export ANTHROPIC_API_KEY=sk-ant-...")
        sys.exit(1)

    # Check rex binary
    if not Path(REX_BIN).exists():
        print(f"{red('ERROR')}: Rex binary not found at {REX_BIN}")
        print("Build with: cargo build")
        print("Or set REX_BIN env var to the path of the rex binary.")
        sys.exit(1)

    # Load tasks
    tasks = load_tasks(args.task, args.tier)
    if not tasks:
        print(f"{red('ERROR')}: No tasks found matching filter")
        sys.exit(1)

    # Build conditions
    all_conditions = {
        "rex_harness": rex_harness(REX_BIN),
        "rex_raw": rex_raw(REX_BIN),
    }
    conditions = {
        k: v for k, v in all_conditions.items() if not args.condition or k in args.condition
    }

    total = len(tasks) * len(conditions) * args.runs
    print(bold("Rex Harness Benchmark"))
    print(f"Tasks: {len(tasks)}, Conditions: {len(conditions)}, Runs: {args.runs}")
    print(f"Total agent invocations: {total}")
    print(f"Model: {args.model}")
    print(f"Rex binary: {REX_BIN}")
    print()

    all_results: list[dict] = []

    for task in tasks:
        tid = task["task"]["id"]
        tname = task["task"]["name"]
        print(f"{bold(f'[{tid}] {tname}')}")

        for cname, condition in conditions.items():
            runs: list[EvalResult] = []

            for run_idx in range(args.runs):
                label = f"  {cname} run {run_idx + 1}/{args.runs}"
                print(f"{label:<40} ", end="", flush=True)

                result = run_one(task, condition, args.model, dry_run=args.dry_run)
                runs.append(result)

                # Print inline result
                if result.passed:
                    status = green("PASS")
                else:
                    failed = [c.name for c in result.checks if not c.passed]
                    status = red("FAIL") + dim(f" [{', '.join(failed)}]")

                m = result.metrics
                stats = dim(
                    f"{m.total_tokens:,} tok, {m.tool_calls} calls, "
                    f"{m.wall_clock_ms / 1000:.1f}s"
                )
                print(f"{status}  {stats}")

            # Aggregate
            all_results.append(
                {
                    "task_id": tid,
                    "task_name": tname,
                    "tier": task["task"]["tier"],
                    "condition": cname,
                    "runs": len(runs),
                    "pass_rate": sum(r.passed for r in runs) / len(runs),
                    "avg_score": sum(r.score for r in runs) / len(runs),
                    "median_tokens": median([r.metrics.total_tokens for r in runs]),
                    "median_tool_calls": median([r.metrics.tool_calls for r in runs]),
                    "median_wall_ms": median([r.metrics.wall_clock_ms for r in runs]),
                    "median_errors": median([r.metrics.errors for r in runs]),
                    "checks": aggregate_checks(runs),
                }
            )

        print()

    # Summary
    print_summary(all_results)

    # Write JSON
    if args.json:
        out = Path(args.json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(all_results, indent=2))
        print(f"\nResults written to {args.json}")


if __name__ == "__main__":
    main()
