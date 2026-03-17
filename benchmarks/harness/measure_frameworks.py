#!/usr/bin/env python3
"""
Measure framework-level factors that affect agent performance:
1. Build speed (empty project, 1-page project, 3-page project)
2. Error message quality (inject same bug, compare error output)

Usage:
    uv run python -m harness.measure_frameworks
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
from pathlib import Path

from .runner import REX_BIN, bold, green, red, setup_workdir

# ---------------------------------------------------------------------------
# Pages to inject (same content, adapted syntax per framework)
# ---------------------------------------------------------------------------

# A simple page — works in all frameworks with minor syntax differences
PAGES = {
    "rex": {
        "pages/index.tsx": 'import React from "react";\nexport default function Home() { return <h1>Home</h1>; }\nexport async function getServerSideProps() { return { props: {} }; }\n',
        "pages/about.tsx": 'import React from "react";\nexport default function About() { return <h1>About</h1>; }\n',
        "pages/contact.tsx": 'import React from "react";\nexport default function Contact() { return <h1>Contact</h1>; }\n',
    },
    "nextjs": {
        "pages/index.tsx": "export default function Home() { return <h1>Home</h1>; }\nexport async function getServerSideProps() { return { props: {} }; }\n",
        "pages/about.tsx": "export default function About() { return <h1>About</h1>; }\n",
        "pages/contact.tsx": "export default function Contact() { return <h1>Contact</h1>; }\n",
    },
    "tanstack": {
        "src/routes/index.tsx": "import { createFileRoute } from '@tanstack/react-router'\nexport const Route = createFileRoute('/')({ component: () => <h1>Home</h1> })\n",
        "src/routes/about.tsx": "import { createFileRoute } from '@tanstack/react-router'\nexport const Route = createFileRoute('/about')({ component: () => <h1>About</h1> })\n",
        "src/routes/contact.tsx": "import { createFileRoute } from '@tanstack/react-router'\nexport const Route = createFileRoute('/contact')({ component: () => <h1>Contact</h1> })\n",
    },
    "remix": {
        "app/routes/_index.tsx": "export default function Home() { return <h1>Home</h1>; }\n",
        "app/routes/about.tsx": "export default function About() { return <h1>About</h1>; }\n",
        "app/routes/contact.tsx": "export default function Contact() { return <h1>Contact</h1>; }\n",
    },
}

# Broken page — same logical bug: unclosed JSX, missing closing paren in map
BROKEN_PAGES = {
    "rex": {
        "pages/broken.tsx": 'import React from "react";\nexport default function Broken({ items }: any) {\n  return (\n    <div>\n      <h1>Items</h1>\n      <ul>\n        {items.map((item: any) =>\n          <li key={item.id}>{item.name}</li>\n        }\n      </ul>\n    </div>\n  );\n}\nexport async function getServerSideProps() {\n  return { props: { items: [{ id: 1, name: "A" }] } };\n}\n',
    },
    "nextjs": {
        "pages/broken.tsx": 'export default function Broken({ items }: any) {\n  return (\n    <div>\n      <h1>Items</h1>\n      <ul>\n        {items.map((item: any) =>\n          <li key={item.id}>{item.name}</li>\n        }\n      </ul>\n    </div>\n  );\n}\nexport async function getServerSideProps() {\n  return { props: { items: [{ id: 1, name: "A" }] } };\n}\n',
    },
    "tanstack": {
        "src/routes/broken.tsx": "import { createFileRoute } from '@tanstack/react-router'\nexport const Route = createFileRoute('/broken')({\n  component: () => {\n    const items = [{ id: 1, name: 'A' }]\n    return (\n      <div>\n        <h1>Items</h1>\n        <ul>\n          {items.map((item) =>\n            <li key={item.id}>{item.name}</li>\n          }\n        </ul>\n      </div>\n    )\n  }\n})\n",
    },
    "remix": {
        "app/routes/broken.tsx": 'export default function Broken() {\n  const items = [{ id: 1, name: "A" }];\n  return (\n    <div>\n      <h1>Items</h1>\n      <ul>\n        {items.map((item: any) =>\n          <li key={item.id}>{item.name}</li>\n        }\n      </ul>\n    </div>\n  );\n}\n',
    },
}

BUILD_CMDS = {
    "rex": [REX_BIN, "build"],
    "nextjs": ["npx", "next", "build"],
    "tanstack": ["npx", "vite", "build"],
    "remix": ["npx", "react-router", "build"],
}


def _build_cmd_with_root(framework: str, workdir: Path) -> list[str]:
    cmd = BUILD_CMDS[framework][:]
    if framework == "rex":
        cmd += ["--root", str(workdir)]
    return cmd


def _write_pages(workdir: Path, pages: dict[str, str]) -> None:
    for rel, content in pages.items():
        fp = workdir / rel
        fp.parent.mkdir(parents=True, exist_ok=True)
        fp.write_text(content)


def _tsr_generate(workdir: Path) -> None:
    """Run TanStack route generation."""
    subprocess.run(
        ["npx", "tsr", "generate"],
        cwd=workdir,
        capture_output=True,
        timeout=30,
    )


# ---------------------------------------------------------------------------
# Build speed measurement
# ---------------------------------------------------------------------------


def measure_build_speed(framework: str, num_pages: int, runs: int = 3) -> dict:
    """Measure build time for a framework with N pages."""
    workdir = setup_workdir(framework)

    try:
        # Write the requested number of pages
        pages = PAGES[framework]
        page_list = list(pages.items())[:num_pages]
        for rel, content in page_list:
            fp = workdir / rel
            fp.parent.mkdir(parents=True, exist_ok=True)
            fp.write_text(content)

        # TanStack needs route tree generation
        if framework == "tanstack" and num_pages > 0:
            _tsr_generate(workdir)

        cmd = _build_cmd_with_root(framework, workdir)
        times = []

        for _ in range(runs):
            # Clean build artifacts between runs
            for d in [".next", ".rex", "dist", "build"]:
                p = workdir / d
                if p.exists():
                    shutil.rmtree(p)

            t0 = time.monotonic()
            proc = subprocess.run(
                cmd,
                cwd=workdir,
                capture_output=True,
                text=True,
                timeout=120,
                env={**os.environ, "NODE_ENV": "production"},
            )
            elapsed = (time.monotonic() - t0) * 1000
            times.append(elapsed)

            if proc.returncode != 0:
                return {
                    "framework": framework,
                    "pages": num_pages,
                    "build_ok": False,
                    "error": (proc.stderr or proc.stdout)[:300],
                }

        return {
            "framework": framework,
            "pages": num_pages,
            "build_ok": True,
            "times_ms": times,
            "median_ms": sorted(times)[len(times) // 2],
            "min_ms": min(times),
            "max_ms": max(times),
        }

    finally:
        shutil.rmtree(workdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Error message quality
# ---------------------------------------------------------------------------


def measure_error_quality(framework: str) -> dict:
    """Inject a broken page, capture the build error message."""
    workdir = setup_workdir(framework)

    try:
        # Write working pages first
        pages = PAGES.get(framework, {})
        _write_pages(workdir, pages)

        # Write broken page
        broken = BROKEN_PAGES.get(framework, {})
        _write_pages(workdir, broken)

        # TanStack needs route generation
        if framework == "tanstack":
            _tsr_generate(workdir)

        cmd = _build_cmd_with_root(framework, workdir)
        proc = subprocess.run(
            cmd,
            cwd=workdir,
            capture_output=True,
            text=True,
            timeout=120,
            env={**os.environ, "NODE_ENV": "production"},
        )

        stderr = proc.stderr.strip()
        stdout = proc.stdout.strip()
        error_output = stderr or stdout

        # Analyze error quality
        mentions_file = any(broken_file in error_output for broken_file in broken.keys())
        mentions_line = any(
            f":{i}" in error_output or f"line {i}" in error_output.lower() for i in range(1, 20)
        )
        has_suggestion = any(
            word in error_output.lower()
            for word in ["did you mean", "expected", "try", "missing", "instead"]
        )

        return {
            "framework": framework,
            "exit_code": proc.returncode,
            "error_length": len(error_output),
            "error_text": error_output[:2000],
            "mentions_filename": mentions_file,
            "mentions_line_number": mentions_line,
            "has_fix_suggestion": has_suggestion,
        }

    finally:
        shutil.rmtree(workdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> None:
    frameworks = ["rex", "nextjs", "tanstack", "remix"]

    # --- Build Speed ---
    print(bold("Build Speed (3 runs each, median)"))
    print()
    print(f"{'Framework':<12} {'0 pages':>10} {'1 page':>10} {'3 pages':>10}")
    print("-" * 44)

    speed_results = []
    for fw in frameworks:
        row = f"{fw:<12}"
        for n in [0, 1, 3]:
            r = measure_build_speed(fw, n)
            speed_results.append(r)
            if r["build_ok"]:
                ms = r["median_ms"]
                row += f"{ms:>9.0f}ms"
            else:
                row += f"{'FAIL':>10}"
        print(row)

    # --- Error Messages ---
    print()
    print(bold("Error Message Quality (same JSX bug across frameworks)"))
    print()
    print(f"{'Framework':<12} {'Exit':>5} {'Len':>6} {'File?':>6} {'Line?':>6} {'Suggest?':>9}")
    print("-" * 50)

    error_results = []
    for fw in frameworks:
        r = measure_error_quality(fw)
        error_results.append(r)
        file_ok = green("yes") if r["mentions_filename"] else red("no")
        line_ok = green("yes") if r["mentions_line_number"] else red("no")
        suggest = green("yes") if r["has_fix_suggestion"] else red("no")
        print(
            f"{fw:<12} {r['exit_code']:>5} {r['error_length']:>6} "
            f"{file_ok:>15} {line_ok:>15} {suggest:>18}"
        )

    # Print actual error messages
    print()
    print(bold("Error Messages"))
    for r in error_results:
        print(f"\n--- {r['framework']} (exit {r['exit_code']}, {r['error_length']} chars) ---")
        print(r["error_text"][:800])

    # Save results
    out = Path("results/framework_factors.json")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps({"build_speed": speed_results, "error_quality": error_results}, indent=2)
    )
    print(f"\nResults written to {out}")


if __name__ == "__main__":
    main()
