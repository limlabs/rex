"""Shared constants and data helpers for the benchmark dashboard.

Extracted from dashboard.py so they can be unit-tested with pytest
and imported into the marimo notebook without being stripped by the formatter.
"""

from __future__ import annotations

import json
from pathlib import Path

import polars as pl

# ── Constants ──────────────────────────────────────────────────

COLORS = {
    "Rex": "#7c3aed",
    "Next.js (Pages)": "#171717",
    "Next.js (App)": "#3b82f6",
    "TanStack Start": "#e8590c",
    "Rex + TW": "#a855f7",
    "Next.js + TW": "#06b6d4",
    "TanStack + TW": "#f59e0b",
}

FW_ORDER = [
    "Rex",
    "Next.js (Pages)",
    "Next.js (App)",
    "TanStack Start",
    "Rex + TW",
    "Next.js + TW",
    "TanStack + TW",
]

FW_MAP = {
    "rex": "Rex",
    "nextjs": "Next.js (Pages)",
    "nextjs_app": "Next.js (App)",
    "tanstack": "TanStack Start",
    "rex_tw": "Rex + TW",
    "nextjs_tw": "Next.js + TW",
    "tanstack_tw": "TanStack + TW",
}

EP_LABELS = {
    "/": "SSR index",
    "/about": "SSG about",
    "/blog/hello-world": "Dynamic route",
    "/api/hello": "API route",
}

EP_ORDER = ["/", "/about", "/blog/hello-world", "/api/hello"]


# ── Helpers ────────────────────────────────────────────────────


def has_suite(df: pl.DataFrame, suite: str) -> bool:
    return not df.is_empty() and "suite" in df.columns and (df["suite"] == suite).any()


def sort_by_fw(frame: pl.DataFrame, col: str = "label") -> pl.DataFrame:
    keys = list(FW_ORDER)
    vals = list(range(len(FW_ORDER)))
    return frame.sort(pl.col(col).replace_strict(keys, vals, default=len(FW_ORDER)))


def load_results(path: Path) -> pl.DataFrame:
    if not path.exists():
        return pl.DataFrame()
    raw = json.loads(path.read_text())
    if not raw:
        return pl.DataFrame()
    frame = pl.DataFrame(raw)
    if frame.is_empty():
        return frame
    return frame.with_columns(pl.col("framework").replace(FW_MAP).alias("label"))


# ── Summary table builders ────────────────────────────────────


DX_METRICS = [
    ("binary_mb", "Binary", "MB"),
    ("install_time_ms", "Install", "ms"),
    ("dependency_count", "Deps", ""),
    ("node_modules_mb", "node_modules", "MB"),
    ("cold_start_ms", "Cold Start", "ms"),
    ("rebuild_ms", "Rebuild", "ms"),
    ("dev_memory_mb", "Memory", "MB"),
]


def make_dx_summary(df: pl.DataFrame) -> list[dict]:
    """Build rows for the DX summary table from benchmark results."""
    if not has_suite(df, "dx"):
        return []
    dx = df.filter(pl.col("suite") == "dx")
    pivot = dx.pivot(on="metric", index="label", values="value", aggregate_function="first")
    pivot = sort_by_fw(pivot.filter(pl.col("label").is_in(FW_ORDER)))
    rows = []
    for row in pivot.iter_rows(named=True):
        entry = {"Framework": row["label"]}
        for metric, col_name, unit in DX_METRICS:
            if metric in pivot.columns and row[metric] is not None:
                entry[col_name] = f"{row[metric]:,.0f}{unit}"
        rows.append(entry)
    return rows


def make_server_summary(df: pl.DataFrame) -> list[dict]:
    """Build rows for the server throughput summary table."""
    if not has_suite(df, "server"):
        return []
    server = df.filter(pl.col("suite") == "server")
    rps = server.filter(pl.col("metric") == "rps", pl.col("endpoint").is_not_null())
    if rps.is_empty():
        return []
    pivot = rps.pivot(on="label", index="endpoint", values="value", aggregate_function="first")
    fws = [fw for fw in FW_ORDER if fw in pivot.columns]
    rows = []
    for ep in EP_ORDER:
        ep_row = pivot.filter(pl.col("endpoint") == ep)
        if ep_row.is_empty():
            continue
        entry = {"Endpoint": EP_LABELS.get(ep, ep)}
        for fw in fws:
            val = ep_row[fw][0] if fw in ep_row.columns else 0
            entry[f"{fw} RPS"] = f"{val:,.0f}"
        if "Rex" in fws:
            rex_rps = ep_row["Rex"][0] if "Rex" in ep_row.columns else 0
            for fw in fws:
                if fw == "Rex":
                    continue
                other = ep_row[fw][0] if fw in ep_row.columns else 0.01
                entry[f"vs {fw}"] = f"{round(rex_rps / max(other, 0.01), 1)}x"
        rows.append(entry)
    return rows
