"""
Rex MCP tools for the harness benchmark.

These run in-process via the Claude Agent SDK and provide structured
feedback to the agent about the Rex project state.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

from claude_agent_sdk import create_sdk_mcp_server, tool


def _find_rex_bin() -> str:
    """Find the rex binary (same logic as runner.py)."""
    if "REX_BIN" in os.environ:
        return os.environ["REX_BIN"]
    # Try common locations
    for candidate in [
        Path(__file__).parent.parent.parent / "target" / "debug" / "rex",
        Path(__file__).parent.parent.parent / "target" / "release" / "rex",
    ]:
        if candidate.exists():
            return str(candidate)
    # Git worktree fallback
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            repo_root = Path(result.stdout.strip()).parent
            for profile in ["debug", "release"]:
                candidate = repo_root / "target" / profile / "rex"
                if candidate.exists():
                    return str(candidate)
    except Exception:
        pass
    return "rex"


REX_BIN = _find_rex_bin()


@tool(
    "rex_check",
    "Build the Rex project and return structured results. "
    "Returns whether the build succeeded and any error messages. "
    "Call this after creating or modifying page files to verify they compile correctly.",
    {"type": "object", "properties": {}, "required": []},
)
async def rex_check(args: dict) -> dict:
    """Build the project, return success/failure with errors."""
    # The CWD is set by the agent session
    cwd = os.getcwd()

    proc = subprocess.run(
        [REX_BIN, "build", "--root", cwd],
        capture_output=True,
        text=True,
        timeout=60,
        cwd=cwd,
    )

    if proc.returncode == 0:
        return {
            "content": [
                {
                    "type": "text",
                    "text": "Build succeeded. All pages compile correctly.",
                }
            ]
        }

    # Parse errors from stderr
    stderr = proc.stderr.strip()
    stdout = proc.stdout.strip()
    error_output = stderr or stdout

    return {
        "content": [
            {
                "type": "text",
                "text": f"Build FAILED (exit {proc.returncode}).\n\nErrors:\n{error_output[:2000]}",
            }
        ],
        "isError": True,
    }


@tool(
    "rex_status",
    "Get the current status of the Rex project: what page files exist, "
    "what routes they map to, and whether the project builds. "
    "Call this to orient yourself before making changes.",
    {"type": "object", "properties": {}, "required": []},
)
async def rex_status(args: dict) -> dict:
    """Return project status: pages, routes, build state."""
    cwd = Path(os.getcwd())
    pages_dir = cwd / "pages"

    # Find all page files
    pages = []
    if pages_dir.exists():
        for f in sorted(pages_dir.rglob("*.tsx")):
            rel = f.relative_to(pages_dir)
            # Convert file path to route
            route = "/" + str(rel.with_suffix("")).replace("\\", "/")
            if route.endswith("/index"):
                route = route[: -len("/index")] or "/"
            # Skip special files
            name = f.stem
            if name.startswith("_"):
                pages.append(f"  {rel} (special: {name})")
            else:
                pages.append(f"  {rel} -> {route}")

        for f in sorted(pages_dir.rglob("*.ts")):
            if f.suffix == ".ts":
                rel = f.relative_to(pages_dir)
                route = "/" + str(rel.with_suffix("")).replace("\\", "/")
                pages.append(f"  {rel} -> {route}")

    # Quick build check
    proc = subprocess.run(
        [REX_BIN, "build", "--root", str(cwd)],
        capture_output=True,
        text=True,
        timeout=60,
        cwd=str(cwd),
    )
    build_ok = proc.returncode == 0

    lines = []
    lines.append(f"Pages directory: {'exists' if pages_dir.exists() else 'MISSING'}")
    if pages:
        lines.append(f"Page files ({len(pages)}):")
        lines.extend(pages)
    else:
        lines.append("No page files found.")
    lines.append(f"Build: {'OK' if build_ok else 'FAILED'}")
    if not build_ok:
        lines.append(f"Build errors:\n{(proc.stderr or proc.stdout)[:500]}")

    return {"content": [{"type": "text", "text": "\n".join(lines)}]}


def create_rex_mcp_server():
    """Create an MCP server config with rex_check and rex_status tools."""
    return create_sdk_mcp_server("rex", tools=[rex_check, rex_status])
