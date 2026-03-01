"""Tests for dashboard_helpers — run with: cd benchmarks && uv run pytest"""

import polars as pl
import json

from dashboard_helpers import (
    has_suite,
    sort_by_fw,
    load_results,
    make_dx_summary,
    make_server_summary,
    FW_MAP,
    EP_LABELS,
)

# ── Fixtures ───────────────────────────────────────────────────


def _make_df(rows: list[dict]) -> pl.DataFrame:
    """Create a labeled DataFrame from raw benchmark rows."""
    df = pl.DataFrame(rows)
    if not df.is_empty() and "framework" in df.columns:
        df = df.with_columns(pl.col("framework").replace(FW_MAP).alias("label"))
    return df


SAMPLE_DX = [
    {"suite": "dx", "framework": "rex", "metric": "install_time_ms", "value": 800},
    {"suite": "dx", "framework": "rex", "metric": "dependency_count", "value": 5},
    {"suite": "dx", "framework": "rex", "metric": "node_modules_mb", "value": 12},
    {"suite": "dx", "framework": "rex", "metric": "cold_start_ms", "value": 200},
    {"suite": "dx", "framework": "rex", "metric": "rebuild_ms", "value": 50},
    {"suite": "dx", "framework": "rex", "metric": "lint_time_ms", "value": 30},
    {"suite": "dx", "framework": "nextjs", "metric": "install_time_ms", "value": 4000},
    {"suite": "dx", "framework": "nextjs", "metric": "dependency_count", "value": 300},
    {"suite": "dx", "framework": "nextjs", "metric": "node_modules_mb", "value": 180},
    {"suite": "dx", "framework": "nextjs", "metric": "cold_start_ms", "value": 3000},
    {"suite": "dx", "framework": "nextjs", "metric": "rebuild_ms", "value": 400},
    {"suite": "dx", "framework": "nextjs", "metric": "lint_time_ms", "value": 2500},
]

SAMPLE_SERVER = [
    {"suite": "server", "framework": "rex", "metric": "rps", "value": 40000, "endpoint": "/"},
    {"suite": "server", "framework": "rex", "metric": "rps", "value": 50000, "endpoint": "/about"},
    {"suite": "server", "framework": "nextjs", "metric": "rps", "value": 2000, "endpoint": "/"},
    {
        "suite": "server",
        "framework": "nextjs",
        "metric": "rps",
        "value": 3000,
        "endpoint": "/about",
    },
    {
        "suite": "server",
        "framework": "rex",
        "metric": "build_time_ms",
        "value": 100,
        "endpoint": None,
    },
    {
        "suite": "server",
        "framework": "nextjs",
        "metric": "build_time_ms",
        "value": 5000,
        "endpoint": None,
    },
]


# ── has_suite ──────────────────────────────────────────────────


class TestHasSuite:
    def test_empty_dataframe(self):
        assert has_suite(pl.DataFrame(), "dx") is False

    def test_no_suite_column(self):
        df = pl.DataFrame({"metric": ["rps"], "value": [100]})
        assert has_suite(df, "dx") is False

    def test_matching_suite(self):
        df = _make_df(SAMPLE_DX)
        assert has_suite(df, "dx") is True

    def test_non_matching_suite(self):
        df = _make_df(SAMPLE_DX)
        assert has_suite(df, "server") is False

    def test_multiple_suites(self):
        df = _make_df(SAMPLE_DX + SAMPLE_SERVER)
        assert has_suite(df, "dx") is True
        assert has_suite(df, "server") is True
        assert has_suite(df, "client") is False


# ── sort_by_fw ─────────────────────────────────────────────────


class TestSortByFw:
    def test_sorts_by_fw_order(self):
        df = pl.DataFrame({"label": ["TanStack Start", "Rex", "Next.js (Pages)"]})
        sorted_df = sort_by_fw(df)
        assert sorted_df["label"].to_list() == ["Rex", "Next.js (Pages)", "TanStack Start"]

    def test_unknown_frameworks_sorted_last(self):
        df = pl.DataFrame({"label": ["Unknown", "Rex"]})
        sorted_df = sort_by_fw(df)
        assert sorted_df["label"].to_list() == ["Rex", "Unknown"]

    def test_custom_column(self):
        df = pl.DataFrame({"fw": ["Next.js (App)", "Rex"]})
        sorted_df = sort_by_fw(df, col="fw")
        assert sorted_df["fw"].to_list() == ["Rex", "Next.js (App)"]

    def test_single_row(self):
        df = pl.DataFrame({"label": ["Rex"]})
        sorted_df = sort_by_fw(df)
        assert sorted_df["label"].to_list() == ["Rex"]


# ── load_results ───────────────────────────────────────────────


class TestLoadResults:
    def test_missing_file(self, tmp_path):
        result = load_results(tmp_path / "nonexistent.json")
        assert result.is_empty()

    def test_empty_json(self, tmp_path):
        path = tmp_path / "results.json"
        path.write_text("[]")
        result = load_results(path)
        assert result.is_empty()

    def test_valid_data(self, tmp_path):
        path = tmp_path / "results.json"
        path.write_text(json.dumps(SAMPLE_DX))
        result = load_results(path)
        assert not result.is_empty()
        assert "label" in result.columns
        rex_rows = result.filter(pl.col("label") == "Rex")
        assert len(rex_rows) == 6

    def test_labels_mapped(self, tmp_path):
        path = tmp_path / "results.json"
        path.write_text(
            json.dumps(
                [
                    {
                        "suite": "dx",
                        "framework": "nextjs_app",
                        "metric": "install_time_ms",
                        "value": 1000,
                    },
                ]
            )
        )
        result = load_results(path)
        assert result["label"][0] == "Next.js (App)"


# ── make_dx_summary ───────────────────────────────────────────


class TestMakeDxSummary:
    def test_empty_df(self):
        assert make_dx_summary(pl.DataFrame()) == []

    def test_no_dx_data(self):
        df = _make_df(SAMPLE_SERVER)
        assert make_dx_summary(df) == []

    def test_produces_rows(self):
        df = _make_df(SAMPLE_DX)
        rows = make_dx_summary(df)
        assert len(rows) == 2
        # Rex should be first (FW_ORDER)
        assert rows[0]["Framework"] == "Rex"
        assert rows[1]["Framework"] == "Next.js (Pages)"

    def test_formatted_values(self):
        df = _make_df(SAMPLE_DX)
        rows = make_dx_summary(df)
        rex = rows[0]
        assert rex["Install"] == "800ms"
        assert rex["Deps"] == "5"
        assert rex["node_modules"] == "12MB"
        assert rex["Cold Start"] == "200ms"
        assert rex["Rebuild"] == "50ms"
        assert rex["Lint"] == "30ms"

    def test_missing_metric_excluded(self):
        # Only install_time_ms, no other metrics
        df = _make_df(
            [
                {"suite": "dx", "framework": "rex", "metric": "install_time_ms", "value": 500},
            ]
        )
        rows = make_dx_summary(df)
        assert len(rows) == 1
        assert "Install" in rows[0]
        assert "Rebuild" not in rows[0]

    def test_binary_mb_included(self):
        df = _make_df(
            [
                {"suite": "dx", "framework": "rex", "metric": "binary_mb", "value": 61},
                {"suite": "dx", "framework": "rex", "metric": "install_time_ms", "value": 800},
            ]
        )
        rows = make_dx_summary(df)
        assert len(rows) == 1
        assert rows[0]["Binary"] == "61MB"


# ── make_server_summary ───────────────────────────────────────


class TestMakeServerSummary:
    def test_empty_df(self):
        assert make_server_summary(pl.DataFrame()) == []

    def test_no_server_data(self):
        df = _make_df(SAMPLE_DX)
        assert make_server_summary(df) == []

    def test_produces_rows(self):
        df = _make_df(SAMPLE_SERVER)
        rows = make_server_summary(df)
        # Only "/" and "/about" have rps data
        assert len(rows) == 2
        assert rows[0]["Endpoint"] == "SSR index"
        assert rows[1]["Endpoint"] == "SSG about"

    def test_rps_columns(self):
        df = _make_df(SAMPLE_SERVER)
        rows = make_server_summary(df)
        index_row = rows[0]
        assert "Rex RPS" in index_row
        assert "Next.js (Pages) RPS" in index_row
        assert index_row["Rex RPS"] == "40,000"
        assert index_row["Next.js (Pages) RPS"] == "2,000"

    def test_comparison_column(self):
        df = _make_df(SAMPLE_SERVER)
        rows = make_server_summary(df)
        index_row = rows[0]
        # Rex 40000 / Next.js 2000 = 20.0x
        assert index_row["vs Next.js (Pages)"] == "20.0x"

    def test_skips_non_rps_metrics(self):
        # build_time_ms rows should not appear in summary (endpoint is None)
        df = _make_df(SAMPLE_SERVER)
        rows = make_server_summary(df)
        endpoints = [r["Endpoint"] for r in rows]
        assert all(ep in EP_LABELS.values() for ep in endpoints)
