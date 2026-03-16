"""
Condition definitions: each condition provides a set of tools for the agent,
build/serve commands, and a tool executor that maps tool calls to side effects.
"""

from __future__ import annotations

import os
import subprocess
import textwrap
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

# ---------------------------------------------------------------------------
# Tool schemas (Anthropic API format)
# ---------------------------------------------------------------------------

# -- File tools (read/write/list for raw conditions) --

FILE_TOOLS = [
    {
        "name": "read_file",
        "description": "Read the contents of a file. Returns the file content as text.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path from the project root",
                },
            },
            "required": ["path"],
        },
    },
    {
        "name": "write_file",
        "description": "Write content to a file. Creates parent directories if needed.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path from the project root",
                },
                "content": {
                    "type": "string",
                    "description": "The full file content to write",
                },
            },
            "required": ["path", "content"],
        },
    },
    {
        "name": "list_files",
        "description": "List files in the project matching a glob pattern.",
        "input_schema": {
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern, e.g. 'pages/**/*.tsx' or '**/*'",
                    "default": "**/*",
                },
            },
        },
    },
    {
        "name": "run_command",
        "description": "Run a shell command in the project directory. Use for npm install, checking build output, etc.",
        "input_schema": {
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute",
                },
            },
            "required": ["command"],
        },
    },
]

# -- Harness tools (high-level web-dev primitives) --

HARNESS_TOOLS = [
    {
        "name": "create_page",
        "description": (
            "Create a new page at the given route path. "
            "The page will be a React component that is server-rendered by Rex. "
            "You provide just the JSX body and optional getServerSideProps logic."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "route": {
                    "type": "string",
                    "description": (
                        "URL path for the page. Examples: '/about', '/blog/[slug]', '/users'. "
                        "Dynamic segments use [param] syntax."
                    ),
                },
                "body_tsx": {
                    "type": "string",
                    "description": (
                        "The JSX body of the page component. This will be wrapped in a "
                        "function component automatically. Use 'props' to access any data "
                        "from getServerSideProps. Example: '<div><h1>{props.title}</h1></div>'"
                    ),
                },
                "gssp": {
                    "type": "string",
                    "description": (
                        "Optional: the body of getServerSideProps. Has access to 'context' "
                        "(with context.params, context.query, context.req, context.res). "
                        "Must return an object with a 'props' key. "
                        "Example: 'return { props: { title: context.params.slug } }'"
                    ),
                },
                "imports": {
                    "type": "string",
                    "description": (
                        "Optional: additional import statements to add at the top of the file. "
                        "Example: 'import MyComponent from \"../components/MyComponent\"'"
                    ),
                },
            },
            "required": ["route", "body_tsx"],
        },
    },
    {
        "name": "create_api_route",
        "description": (
            "Create an API route handler. The handler receives req and res objects "
            "and should call res.status().json() or res.end() to respond."
        ),
        "input_schema": {
            "type": "object",
            "properties": {
                "route": {
                    "type": "string",
                    "description": "API path, e.g. '/api/users' or '/api/todos'",
                },
                "handler_ts": {
                    "type": "string",
                    "description": (
                        "TypeScript handler function body. Has access to 'req' (with req.method, "
                        "req.body, req.query) and 'res' (with res.status(), res.json()). "
                        "Example: 'res.status(200).json({ message: \"hello\" })'"
                    ),
                },
            },
            "required": ["route", "handler_ts"],
        },
    },
    {
        "name": "create_component",
        "description": "Create a reusable React component in the components/ directory.",
        "input_schema": {
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "PascalCase component name, e.g. 'UserCard'",
                },
                "body_tsx": {
                    "type": "string",
                    "description": "Full component code including the function and return statement.",
                },
            },
            "required": ["name", "body_tsx"],
        },
    },
    {
        "name": "read_file",
        "description": "Read the contents of a file. Returns the file content as text.",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path from the project root",
                },
            },
            "required": ["path"],
        },
    },
    {
        "name": "list_files",
        "description": "List files in the project matching a glob pattern.",
        "input_schema": {
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern, e.g. 'pages/**/*.tsx' or '**/*'",
                    "default": "**/*",
                },
            },
        },
    },
]


# ---------------------------------------------------------------------------
# Condition dataclass
# ---------------------------------------------------------------------------


@dataclass
class Condition:
    name: str
    tools: list[dict]
    build_cmd: list[str]
    serve_cmd: list[str]
    starter: str
    setup_hook: Callable[[Path], None] | None = None


def rex_harness(rex_bin: str) -> Condition:
    """High-level web-dev tools. Agent uses create_page/create_api_route instead of raw files."""
    return Condition(
        name="rex_harness",
        tools=HARNESS_TOOLS,
        build_cmd=[rex_bin, "build"],
        serve_cmd=[rex_bin, "start"],
        starter="rex",
    )


def rex_raw(rex_bin: str) -> Condition:
    """Raw file editing. Same framework, but agent writes files directly."""
    return Condition(
        name="rex_raw",
        tools=FILE_TOOLS,
        build_cmd=[rex_bin, "build"],
        serve_cmd=[rex_bin, "start"],
        starter="rex",
    )


# ---------------------------------------------------------------------------
# Tool executors
# ---------------------------------------------------------------------------


def _route_to_filepath(route: str) -> str:
    """Convert a URL route to a pages/ file path.

    /about          -> pages/about.tsx
    /blog/[slug]    -> pages/blog/[slug].tsx
    /api/users      -> pages/api/users.ts
    /               -> pages/index.tsx
    """
    rel = route.strip("/")
    if not rel:
        rel = "index"

    is_api = rel.startswith("api/")
    ext = ".ts" if is_api else ".tsx"
    return f"pages/{rel}{ext}"


def _route_to_component_name(route: str) -> str:
    """Convert a URL route to a PascalCase component name.

    /about       -> AboutPage
    /blog/[slug] -> BlogSlugPage
    /users       -> UsersPage
    /            -> IndexPage
    """
    rel = route.strip("/") or "index"
    parts = rel.replace("[", "").replace("]", "").split("/")
    return "".join(p.capitalize() for p in parts) + "Page"


def make_harness_executor() -> Callable[[str, dict, Path], tuple[str, bool]]:
    """Tool executor for the rex_harness condition."""

    def executor(name: str, inp: dict, wd: Path) -> tuple[str, bool]:
        try:
            match name:
                case "create_page":
                    return _exec_create_page(wd, inp), False
                case "create_api_route":
                    return _exec_create_api_route(wd, inp), False
                case "create_component":
                    return _exec_create_component(wd, inp), False
                case "read_file":
                    return _exec_read_file(wd, inp), False
                case "list_files":
                    return _exec_list_files(wd, inp), False
                case _:
                    return f"Unknown tool: {name}", True
        except Exception as e:
            return f"Error: {e}", True

    return executor


def make_raw_executor() -> Callable[[str, dict, Path], tuple[str, bool]]:
    """Tool executor for raw file-editing conditions."""

    def executor(name: str, inp: dict, wd: Path) -> tuple[str, bool]:
        try:
            match name:
                case "read_file":
                    return _exec_read_file(wd, inp), False
                case "write_file":
                    return _exec_write_file(wd, inp), False
                case "list_files":
                    return _exec_list_files(wd, inp), False
                case "run_command":
                    return _exec_run_command(wd, inp)
                case _:
                    return f"Unknown tool: {name}", True
        except Exception as e:
            return f"Error: {e}", True

    return executor


# ---------------------------------------------------------------------------
# Shared tool implementations
# ---------------------------------------------------------------------------


def _exec_read_file(wd: Path, inp: dict) -> str:
    path = wd / inp["path"]
    if not path.exists():
        raise FileNotFoundError(f"{inp['path']} does not exist")
    return path.read_text()


def _exec_write_file(wd: Path, inp: dict) -> str:
    path = wd / inp["path"]
    # Prevent path traversal
    try:
        path.resolve().relative_to(wd.resolve())
    except ValueError:
        raise ValueError(f"Path {inp['path']} escapes the project directory")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(inp["content"])
    return f"Wrote {inp['path']} ({len(inp['content'])} bytes)"


def _exec_list_files(wd: Path, inp: dict) -> str:
    pattern = inp.get("pattern", "**/*")
    files = sorted(wd.glob(pattern))
    # Filter to actual files, skip node_modules and .rex
    result = []
    for f in files:
        if f.is_file():
            rel = str(f.relative_to(wd))
            if not rel.startswith(("node_modules/", ".rex/")):
                result.append(rel)
    return "\n".join(result[:200]) if result else "(no files match)"


def _exec_run_command(wd: Path, inp: dict) -> tuple[str, bool]:
    proc = subprocess.run(
        inp["command"],
        shell=True,
        cwd=wd,
        capture_output=True,
        text=True,
        timeout=60,
        env={**os.environ, "PATH": os.environ.get("PATH", "")},
    )
    output = (proc.stdout + proc.stderr).strip()
    return output[:4000] if output else "(no output)", proc.returncode != 0


# ---------------------------------------------------------------------------
# Harness tool implementations
# ---------------------------------------------------------------------------


def _exec_create_page(wd: Path, inp: dict) -> str:
    route = inp["route"]
    filepath = wd / _route_to_filepath(route)
    filepath.parent.mkdir(parents=True, exist_ok=True)

    component_name = _route_to_component_name(route)
    body = inp["body_tsx"]
    gssp = inp.get("gssp")
    imports = inp.get("imports", "")

    lines = ['import React from "react";']
    if imports:
        lines.append(imports)
    lines.append("")
    lines.append(f"export default function {component_name}(props: any) {{")
    lines.append("  return (")
    # Indent the body
    for line in body.strip().split("\n"):
        lines.append(f"    {line}")
    lines.append("  );")
    lines.append("}")

    if gssp:
        lines.append("")
        lines.append("export async function getServerSideProps(context: any) {")
        for line in gssp.strip().split("\n"):
            lines.append(f"  {line}")
        lines.append("}")

    lines.append("")
    filepath.write_text("\n".join(lines))

    rel = filepath.relative_to(wd)
    return f"Created page at {rel} (route: {route})"


def _exec_create_api_route(wd: Path, inp: dict) -> str:
    route = inp["route"]
    filepath = wd / _route_to_filepath(route)
    filepath.parent.mkdir(parents=True, exist_ok=True)

    handler = inp["handler_ts"]

    content = textwrap.dedent(f"""\
        export default function handler(req: any, res: any) {{
          {handler}
        }}
    """)

    filepath.write_text(content)
    rel = filepath.relative_to(wd)
    return f"Created API route at {rel} (route: {route})"


def _exec_create_component(wd: Path, inp: dict) -> str:
    name = inp["name"]
    filepath = wd / "components" / f"{name}.tsx"
    filepath.parent.mkdir(parents=True, exist_ok=True)

    body = inp["body_tsx"]
    content = f'import React from "react";\n\n{body}\n'

    filepath.write_text(content)
    rel = filepath.relative_to(wd)
    return f"Created component at {rel}"
