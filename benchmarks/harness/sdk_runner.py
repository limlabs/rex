#!/usr/bin/env python3
"""
SDK-based harness runner: uses Claude Agent SDK (Claude Code) instead of
raw Anthropic API calls. The agent gets native tools (Read, Write, Edit,
Bash, Glob, Grep) and each condition is defined by CLAUDE.md + hooks in
the project directory.

Usage:
    uv run python -m harness.sdk_runner                            # all tasks, all conditions
    uv run python -m harness.sdk_runner --task t1-01               # single task
    uv run python -m harness.sdk_runner --tier 2                   # tier 2 tasks
    uv run python -m harness.sdk_runner --condition rex_guided     # single condition
    uv run python -m harness.sdk_runner --model claude-haiku-4-5-20251001
    uv run python -m harness.sdk_runner --json results/sdk_run.json
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
from dataclasses import dataclass, field
from pathlib import Path

from claude_agent_sdk import (  # noqa: E402
    ClaudeAgentOptions,
    ResultMessage,
    query,
)

from .evaluator import EvalResult, evaluate
from .runner import (
    REX_BIN,
    STARTERS_DIR,
    bold,
    dim,
    green,
    load_tasks,
    median,
    print_summary,
    red,
    yellow,
)

# Reuse Condition for eval compatibility
from .conditions import Condition
from .guides import (  # noqa: F401
    NEXTJS_GUIDED,
    REMIX_GUIDED,
    REX_GUIDED,
    REX_MCP,
    REX_RAW,
    TANSTACK_GUIDED,
)

# ---------------------------------------------------------------------------
# Hooks (written as .claude/settings.json in the project)
# ---------------------------------------------------------------------------

_REX_BUILD_HOOK = {
    "hooks": {
        "PostToolUse": [
            {
                "matcher": {"toolName": "Write", "filePath": "pages/**"},
                "hooks": [
                    {
                        "type": "command",
                        "command": f"{REX_BIN} build --root .",
                    }
                ],
            }
        ]
    }
}


# ---------------------------------------------------------------------------
# SDK condition definitions
# ---------------------------------------------------------------------------


@dataclass
class SDKCondition:
    name: str
    starter: str
    claude_md: str
    hooks: dict | None  # Written to .claude/settings.json
    build_cmd: list[str]
    serve_cmd: list[str]
    mcp_server: object | None = None  # McpSdkServerConfig from create_sdk_mcp_server

    def as_eval_condition(self) -> Condition:
        """Convert to Condition for evaluator compatibility."""
        return Condition(
            name=self.name,
            tools=[],
            build_cmd=self.build_cmd,
            serve_cmd=self.serve_cmd,
            starter=self.starter,
        )


def make_conditions() -> dict[str, SDKCondition]:
    from .mcp_tools import create_rex_mcp_server

    conditions = {
        "rex_mcp": SDKCondition(
            name="rex_mcp",
            starter="rex",
            claude_md=REX_MCP,
            hooks=None,
            build_cmd=[REX_BIN, "build"],
            serve_cmd=[REX_BIN, "start"],
            mcp_server=create_rex_mcp_server(),
        ),
        "rex_guided": SDKCondition(
            name="rex_guided",
            starter="rex",
            claude_md=REX_GUIDED,
            hooks=_REX_BUILD_HOOK,
            build_cmd=[REX_BIN, "build"],
            serve_cmd=[REX_BIN, "start"],
        ),
        "rex_raw": SDKCondition(
            name="rex_raw",
            starter="rex",
            claude_md=REX_RAW,
            hooks=None,
            build_cmd=[REX_BIN, "build"],
            serve_cmd=[REX_BIN, "start"],
        ),
        "nextjs_guided": SDKCondition(
            name="nextjs_guided",
            starter="nextjs",
            claude_md=NEXTJS_GUIDED,
            hooks=None,
            build_cmd=["npx", "next", "build"],
            serve_cmd=["npx", "next", "start"],
        ),
        "tanstack_guided": SDKCondition(
            name="tanstack_guided",
            starter="tanstack",
            claude_md=TANSTACK_GUIDED,
            hooks=None,
            build_cmd=["npx", "vite", "build"],
            serve_cmd=["npx", "vite", "preview"],
        ),
        "remix_guided": SDKCondition(
            name="remix_guided",
            starter="remix",
            claude_md=REMIX_GUIDED,
            hooks=None,
            build_cmd=["npx", "react-router", "build"],
            serve_cmd=["npx", "react-router-serve", "./build/server/index.js"],
        ),
    }
    return conditions


# ---------------------------------------------------------------------------
# Agent metrics from SDK
# ---------------------------------------------------------------------------


@dataclass
class SDKMetrics:
    input_tokens: int = 0
    output_tokens: int = 0
    tool_calls: int = 0
    errors: int = 0
    wall_clock_ms: float = 0
    turns: int = 0
    cost_usd: float = 0.0
    trajectory: list[dict] = field(default_factory=list)

    @property
    def total_tokens(self) -> int:
        return self.input_tokens + self.output_tokens

    def trajectory_summary(self) -> list[dict]:
        return self.trajectory


# Adapter so evaluator can use SDKMetrics
class MetricsAdapter:
    """Makes SDKMetrics look like AgentMetrics for the evaluator."""

    def __init__(self, m: SDKMetrics):
        self._m = m

    @property
    def input_tokens(self):
        return self._m.input_tokens

    @property
    def output_tokens(self):
        return self._m.output_tokens

    @property
    def total_tokens(self):
        return self._m.total_tokens

    @property
    def tool_calls(self):
        return self._m.tool_calls

    @property
    def errors(self):
        return self._m.errors

    @property
    def wall_clock_ms(self):
        return self._m.wall_clock_ms

    @property
    def turns(self):
        return self._m.turns

    def trajectory_summary(self):
        return self._m.trajectory_summary()


# ---------------------------------------------------------------------------
# Workspace setup
# ---------------------------------------------------------------------------


def setup_workdir(condition: SDKCondition) -> Path:
    """Copy starter, write CLAUDE.md, write hooks, install deps."""
    src = STARTERS_DIR / condition.starter
    tmp = Path(tempfile.mkdtemp(prefix=f"harness_sdk_{condition.name}_"))
    shutil.copytree(src, tmp, dirs_exist_ok=True)

    # Ensure pages/ or routes/ dir exists
    (tmp / "pages").mkdir(exist_ok=True)

    # Symlink rex binary into the project so `rex` is available in Bash
    rex_bin_path = Path(REX_BIN)
    if rex_bin_path.exists():
        link = tmp / "rex"
        if not link.exists():
            link.symlink_to(rex_bin_path.resolve())

    # Write CLAUDE.md — include rex binary location
    claude_md = condition.claude_md.replace(
        "The rex binary is available",
        f"The rex binary is at ./rex (or {REX_BIN}). It is also available",
    )
    (tmp / "CLAUDE.md").write_text(claude_md)

    # Write hooks config
    if condition.hooks:
        claude_dir = tmp / ".claude"
        claude_dir.mkdir(exist_ok=True)
        (claude_dir / "settings.json").write_text(json.dumps(condition.hooks, indent=2))

    # Install npm deps
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
# Run agent via SDK
# ---------------------------------------------------------------------------


async def run_sdk_agent(
    prompt: str,
    workdir: Path,
    model: str,
    mcp_server: object | None = None,
) -> SDKMetrics:
    """Run a Claude Code session via the Agent SDK."""
    metrics = SDKMetrics()
    t0 = time.monotonic()

    # Unset CLAUDECODE env var to allow nesting (we may be running inside Claude Code)
    env_overrides = {}
    for key in ("CLAUDECODE", "CLAUDE_CODE"):
        if key in os.environ:
            env_overrides[key] = os.environ.pop(key)

    allowed = ["Read", "Write", "Edit", "Bash", "Glob", "Grep"]
    mcp_servers = {}
    if mcp_server is not None:
        mcp_servers["rex"] = mcp_server
        allowed.extend(["mcp__rex__rex_check", "mcp__rex__rex_status"])

    options = ClaudeAgentOptions(
        cwd=str(workdir),
        model=model,
        permission_mode="bypassPermissions",
        allowed_tools=allowed,
        **({"mcp_servers": mcp_servers} if mcp_servers else {}),
    )

    try:
        async for message in query(prompt=prompt, options=options):
            # Count tool calls from assistant messages that contain tool_use blocks
            if type(message).__name__ == "AssistantMessage":
                metrics.tool_calls += 1
            if isinstance(message, ResultMessage):
                metrics.cost_usd = message.total_cost_usd or 0.0
                metrics.turns = getattr(message, "num_turns", 0) or 0
                usage = message.usage or {}
                metrics.input_tokens = usage.get("input_tokens", 0) + usage.get(
                    "cache_read_input_tokens", 0
                )
                metrics.output_tokens = usage.get("output_tokens", 0)
    finally:
        # Restore env vars
        for key, val in env_overrides.items():
            os.environ[key] = val

    metrics.wall_clock_ms = (time.monotonic() - t0) * 1000
    return metrics


# ---------------------------------------------------------------------------
# Single run
# ---------------------------------------------------------------------------


async def run_one(
    task: dict,
    condition: SDKCondition,
    model: str,
) -> EvalResult:
    """Run a single task under a single condition via SDK."""
    workdir = setup_workdir(condition)

    # Inject any starter files from the task spec
    inject = task.get("starter", {}).get("inject", {})
    for rel_path, content in inject.items():
        fp = workdir / rel_path
        fp.parent.mkdir(parents=True, exist_ok=True)
        fp.write_text(content)

    try:
        metrics = await run_sdk_agent(
            prompt=task["task"]["prompt"],
            workdir=workdir,
            model=model,
            mcp_server=condition.mcp_server,
        )

        eval_condition = condition.as_eval_condition()
        adapter = MetricsAdapter(metrics)
        return evaluate(task, eval_condition, workdir, adapter)

    finally:
        shutil.rmtree(workdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Aggregation helpers (reuse from runner.py)
# ---------------------------------------------------------------------------


def aggregate_checks(runs: list[EvalResult]) -> dict[str, float]:
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
# Main
# ---------------------------------------------------------------------------


async def async_main() -> None:
    import argparse

    parser = argparse.ArgumentParser(
        description="Rex Harness Benchmark (SDK runner — uses Claude Code natively)"
    )
    parser.add_argument("--task", help="Run a specific task ID")
    parser.add_argument("--tier", type=int, help="Run all tasks in a tier")
    parser.add_argument(
        "--condition",
        nargs="+",
        help="Conditions to test (default: all)",
    )
    parser.add_argument("--runs", type=int, default=1, help="Runs per combo (default: 1)")
    parser.add_argument("--model", default="claude-haiku-4-5-20251001", help="Model to use")
    parser.add_argument("--json", help="Write results to JSON file")
    args = parser.parse_args()

    # Check API key
    if not os.environ.get("ANTHROPIC_API_KEY"):
        print(f"{red('ERROR')}: ANTHROPIC_API_KEY not set")
        sys.exit(1)

    tasks = load_tasks(args.task, args.tier)
    if not tasks:
        print(f"{red('ERROR')}: No tasks found")
        sys.exit(1)

    all_conditions = make_conditions()
    conditions = {
        k: v for k, v in all_conditions.items() if not args.condition or k in args.condition
    }

    total = len(tasks) * len(conditions) * args.runs
    print(bold("Rex Harness Benchmark (SDK Runner)"))
    print(f"Tasks: {len(tasks)}, Conditions: {len(conditions)}, Runs: {args.runs}")
    print(f"Total agent invocations: {total}")
    print(f"Model: {args.model}")
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

                result = await run_one(task, condition, args.model)
                runs.append(result)

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

            all_results.append(
                {
                    "task_id": tid,
                    "task_name": tname,
                    "tier": task["task"]["tier"],
                    "condition": cname,
                    "model": args.model,
                    "runner": "sdk",
                    "runs": len(runs),
                    "pass_rate": sum(r.passed for r in runs) / len(runs),
                    "avg_score": sum(r.score for r in runs) / len(runs),
                    "median_tokens": median([r.metrics.total_tokens for r in runs]),
                    "median_tool_calls": median([r.metrics.tool_calls for r in runs]),
                    "median_wall_ms": median([r.metrics.wall_clock_ms for r in runs]),
                    "checks": aggregate_checks(runs),
                }
            )

        print()

    print_summary(all_results)

    if args.json:
        out = Path(args.json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(all_results, indent=2))
        print(f"\nResults written to {args.json}")


def main() -> None:
    asyncio.run(async_main())


if __name__ == "__main__":
    main()
