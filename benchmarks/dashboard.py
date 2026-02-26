import marimo

__generated_with = "0.20.2"
app = marimo.App(width="medium")


@app.cell
def _():
    import marimo as mo
    import json
    import subprocess
    import sys
    import time
    import pandas as pd
    import plotly.express as px
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots
    from pathlib import Path

    COLORS = {"Rex": "#7c3aed", "Next.js": "#171717", "TanStack Start": "#e8590c"}
    FW_ORDER = ["Rex", "Next.js", "TanStack Start"]
    FW_MAP = {"rex": "Rex", "nextjs": "Next.js", "tanstack": "TanStack Start"}
    BENCH_DIR = Path(__file__).parent
    RESULTS_PATH = BENCH_DIR / "results.json"
    return BENCH_DIR, COLORS, FW_MAP, FW_ORDER, Path, RESULTS_PATH, go, json, make_subplots, mo, pd, px, subprocess, sys, time


@app.cell
def _(mo):
    mo.md("""
    # Rex Benchmark Dashboard

    Comparing **Rex** (Rust-native Pages Router) against **Next.js 15** and
    **TanStack Start v1** on identical page fixtures.
    """)
    return


# ════════════════════════════════════════════════════════════════
# CONTROLS + RUNNER
# ════════════════════════════════════════════════════════════════


@app.cell
def _(mo):
    get_bench_ts, set_bench_ts = mo.state(0)
    get_bench_log, set_bench_log = mo.state("")
    return get_bench_log, get_bench_ts, set_bench_log, set_bench_ts


@app.cell
def _(BENCH_DIR, RESULTS_PATH, mo, set_bench_log, set_bench_ts, subprocess, sys, time):
    suite_select = mo.ui.multiselect(
        options={"DX": "dx", "Server": "server", "Client": "client"},
        value=["dx", "server", "client"],
        label="Suites",
    )
    fw_select = mo.ui.multiselect(
        options={"Rex": "rex", "Next.js": "nextjs", "TanStack Start": "tanstack"},
        value=["rex"],
        label="Frameworks",
    )
    requests_input = mo.ui.number(value=5000, start=100, stop=50000, step=100, label="Requests (server)")
    concurrency_input = mo.ui.number(value=50, start=1, stop=500, step=10, label="Concurrency")

    def _run_bench(_):
        _suites = ",".join(suite_select.value) if suite_select.value else "dx,server,client"
        _fws = ",".join(fw_select.value) if fw_select.value else "rex"
        _cmd = [
            sys.executable, str(BENCH_DIR / "bench.py"),
            "--suite", _suites,
            "--framework", _fws,
            "--requests", str(int(requests_input.value)),
            "--concurrency", str(int(concurrency_input.value)),
            "--json", str(RESULTS_PATH),
        ]
        _proc = subprocess.run(_cmd, capture_output=True, text=True, cwd=str(BENCH_DIR), timeout=600)
        # Strip ANSI escape codes for display
        import re as _re
        _clean = _re.sub(r'\x1b\[[0-9;]*m', '', _proc.stdout + _proc.stderr)
        set_bench_log(_clean)
        set_bench_ts(time.time())

    run_btn = mo.ui.button(label="Run Benchmarks", on_click=_run_bench, kind="success")

    mo.vstack([
        mo.hstack([suite_select, fw_select], justify="start", gap=1),
        mo.hstack([requests_input, concurrency_input], justify="start", gap=1),
        run_btn,
    ])
    return concurrency_input, fw_select, requests_input, run_btn, suite_select


@app.cell
def _(get_bench_log, mo):
    _log = get_bench_log()
    if _log:
        mo.accordion({"Benchmark output": mo.md(f"```\n{_log}\n```")})
    return


# ════════════════════════════════════════════════════════════════
# DATA LOADING
# ════════════════════════════════════════════════════════════════


@app.cell
def _(FW_MAP, RESULTS_PATH, get_bench_ts, json, mo, pd):
    _ = get_bench_ts()
    if RESULTS_PATH.exists():
        _raw = json.loads(RESULTS_PATH.read_text())
        df = pd.DataFrame(_raw)
        if not df.empty:
            df["label"] = df["framework"].map(FW_MAP)
    else:
        mo.md("> No results yet. Select options above and click **Run Benchmarks**.")
        df = pd.DataFrame()
    return (df,)


# ════════════════════════════════════════════════════════════════
# DX SUITE
# ════════════════════════════════════════════════════════════════


@app.cell
def _(df, mo):
    if not df.empty and "suite" in df.columns and (df["suite"] == "dx").any():
        mo.md("## Developer Experience")
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    def _plot_dx_footprint():
        if df.empty or "suite" not in df.columns or not (df["suite"] == "dx").any():
            return
        _dx = df[df["suite"] == "dx"].copy()
        _metrics = [
            ("install_time_ms", "npm install (ms)"),
            ("dependency_count", "Dependencies"),
            ("node_modules_mb", "node_modules (MB)"),
        ]
        _fig = make_subplots(rows=1, cols=3, subplot_titles=[m[1] for m in _metrics], horizontal_spacing=0.08)
        for _ci, (_m, _t) in enumerate(_metrics, 1):
            _s = _dx[_dx["metric"] == _m].sort_values("label", key=lambda s: s.map({v: i for i, v in enumerate(FW_ORDER)}))
            for _, _r in _s.iterrows():
                _fig.add_trace(go.Bar(x=[_r["label"]], y=[_r["value"]], name=_r["label"], marker_color=COLORS.get(_r["label"], "#888"), showlegend=(_ci == 1), legendgroup=_r["label"]), row=1, col=_ci)
        _fig.update_layout(height=350, font=dict(family="Inter, system-ui, sans-serif"), legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5), title_text="Dependency Footprint — lower is better", barmode="group")
        mo.ui.plotly(_fig)
    _plot_dx_footprint()
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    def _plot_dx_perf():
        if df.empty or "suite" not in df.columns or not (df["suite"] == "dx").any():
            return
        _dx = df[df["suite"] == "dx"].copy()
        _metrics = [("cold_start_ms", "Cold Start (ms)"), ("rebuild_ms", "HMR Rebuild (ms)"), ("dev_memory_mb", "Dev Memory (MB)")]
        _fig = make_subplots(rows=1, cols=3, subplot_titles=[m[1] for m in _metrics], horizontal_spacing=0.08)
        for _ci, (_m, _t) in enumerate(_metrics, 1):
            _s = _dx[_dx["metric"] == _m].sort_values("label", key=lambda s: s.map({v: i for i, v in enumerate(FW_ORDER)}))
            for _, _r in _s.iterrows():
                _fig.add_trace(go.Bar(x=[_r["label"]], y=[_r["value"]], name=_r["label"], marker_color=COLORS.get(_r["label"], "#888"), showlegend=(_ci == 1), legendgroup=_r["label"]), row=1, col=_ci)
        _fig.update_layout(height=350, font=dict(family="Inter, system-ui, sans-serif"), legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5), title_text="Dev Server Performance — lower is better", barmode="group")
        mo.ui.plotly(_fig)
    _plot_dx_perf()
    return


@app.cell
def _(FW_ORDER, df, mo, pd):
    def _dx_summary_table():
        if df.empty or "suite" not in df.columns or not (df["suite"] == "dx").any():
            return
        _dx = df[df["suite"] == "dx"].copy()
        _pivot = _dx.pivot_table(index="label", columns="metric", values="value", aggfunc="first")
        _pivot = _pivot.reindex([fw for fw in FW_ORDER if fw in _pivot.index])
        _rows = []
        for _fw in _pivot.index:
            _r = {"Framework": _fw}
            if "install_time_ms" in _pivot.columns:
                _r["Install"] = f'{_pivot.loc[_fw, "install_time_ms"]:,.0f}ms'
            if "dependency_count" in _pivot.columns:
                _r["Deps"] = f'{_pivot.loc[_fw, "dependency_count"]:,.0f}'
            if "node_modules_mb" in _pivot.columns:
                _r["node_modules"] = f'{_pivot.loc[_fw, "node_modules_mb"]:,.0f}MB'
            if "cold_start_ms" in _pivot.columns:
                _r["Cold Start"] = f'{_pivot.loc[_fw, "cold_start_ms"]:,.0f}ms'
            if "rebuild_ms" in _pivot.columns:
                _r["Rebuild"] = f'{_pivot.loc[_fw, "rebuild_ms"]:,.0f}ms'
            if "dev_memory_mb" in _pivot.columns:
                _r["Memory"] = f'{_pivot.loc[_fw, "dev_memory_mb"]:,.0f}MB'
            _rows.append(_r)
        mo.md("### DX Summary")
        mo.ui.table(pd.DataFrame(_rows), selection=None)
    _dx_summary_table()
    return


# ════════════════════════════════════════════════════════════════
# SERVER SUITE
# ════════════════════════════════════════════════════════════════


@app.cell
def _(df, mo):
    if not df.empty and "suite" in df.columns and (df["suite"] == "server").any():
        mo.md("## Production Server Performance")
    return


@app.cell
def _(COLORS, FW_ORDER, df, mo, px):
    def _plot_rps():
        if df.empty or "suite" not in df.columns or not (df["suite"] == "server").any():
            return
        _server = df[df["suite"] == "server"].copy()
        _rps = _server[(_server["metric"] == "rps") & (_server["endpoint"].notna())].copy()
        if _rps.empty:
            return
        _rps["endpoint_label"] = _rps["endpoint"].map({"/": "SSR index", "/about": "SSG about", "/blog/hello-world": "Dynamic route", "/api/hello": "API route"})
        _fig = px.bar(_rps, x="endpoint_label", y="value", color="label", barmode="group", title="Throughput (Requests/sec) — higher is better", labels={"value": "Requests/sec", "endpoint_label": "", "label": "Framework"}, color_discrete_map=COLORS, category_orders={"label": FW_ORDER})
        _fig.update_layout(height=450, font=dict(family="Inter, system-ui, sans-serif"), legend=dict(orientation="h", yanchor="bottom", y=1.04, xanchor="center", x=0.5))
        mo.ui.plotly(_fig)
    _plot_rps()
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    def _plot_latency():
        if df.empty or "suite" not in df.columns or not (df["suite"] == "server").any():
            return
        _server = df[df["suite"] == "server"].copy()
        _ep = _server[_server["endpoint"].notna()].copy()
        _ep["endpoint_label"] = _ep["endpoint"].map({"/": "SSR index", "/about": "SSG about", "/blog/hello-world": "Dynamic route", "/api/hello": "API route"})
        _lat_mean = _ep[_ep["metric"] == "latency_mean_ms"]
        _lat_p99 = _ep[_ep["metric"] == "latency_p99_ms"]
        if _lat_mean.empty and _lat_p99.empty:
            return
        _fig = make_subplots(rows=1, cols=2, subplot_titles=["Mean Latency (ms)", "p99 Latency (ms)"], horizontal_spacing=0.1)
        for _ci, (_sub, _title) in enumerate([(_lat_mean, "Mean"), (_lat_p99, "p99")], 1):
            if _sub.empty:
                continue
            for _fw in FW_ORDER:
                _fd = _sub[_sub["label"] == _fw]
                if _fd.empty:
                    continue
                _fig.add_trace(go.Bar(x=_fd["endpoint_label"], y=_fd["value"], name=_fw, marker_color=COLORS.get(_fw, "#888"), showlegend=(_ci == 1), legendgroup=_fw), row=1, col=_ci)
        _fig.update_layout(height=400, font=dict(family="Inter, system-ui, sans-serif"), legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5), title_text="Latency — lower is better", barmode="group")
        mo.ui.plotly(_fig)
    _plot_latency()
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    def _plot_build_startup():
        if df.empty or "suite" not in df.columns or not (df["suite"] == "server").any():
            return
        _server = df[df["suite"] == "server"].copy()
        _fw_m = _server[~_server["endpoint"].notna() | (_server["endpoint"] == "")].copy()
        _metrics = [("build_time_ms", "Build Time (ms)"), ("cold_start_ms", "Cold Start (ms)"), ("memory_mb", "Memory (MB)")]
        _fig = make_subplots(rows=1, cols=3, subplot_titles=[m[1] for m in _metrics], horizontal_spacing=0.08)
        for _ci, (_m, _t) in enumerate(_metrics, 1):
            _s = _fw_m[_fw_m["metric"] == _m].sort_values("label", key=lambda s: s.map({v: i for i, v in enumerate(FW_ORDER)}))
            for _, _r in _s.iterrows():
                _fig.add_trace(go.Bar(x=[_r["label"]], y=[_r["value"]], name=_r["label"], marker_color=COLORS.get(_r["label"], "#888"), showlegend=(_ci == 1), legendgroup=_r["label"]), row=1, col=_ci)
        _fig.update_layout(height=350, font=dict(family="Inter, system-ui, sans-serif"), legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5), title_text="Build & Startup — lower is better", barmode="group")
        mo.ui.plotly(_fig)
    _plot_build_startup()
    return


@app.cell
def _(FW_ORDER, df, mo, pd):
    def _server_summary_table():
        if df.empty or "suite" not in df.columns or not (df["suite"] == "server").any():
            return
        _server = df[df["suite"] == "server"].copy()
        _rps = _server[(_server["metric"] == "rps") & (_server["endpoint"].notna())]
        if _rps.empty:
            return
        _pivot = _rps.pivot_table(index="endpoint", columns="label", values="value", aggfunc="first")
        _fws = [fw for fw in FW_ORDER if fw in _pivot.columns]
        _ep_labels = {"/": "SSR index", "/about": "SSG about", "/blog/hello-world": "Dynamic route", "/api/hello": "API route"}
        _rows = []
        for _ep in ["/", "/about", "/blog/hello-world", "/api/hello"]:
            if _ep not in _pivot.index:
                continue
            _r = {"Endpoint": _ep_labels.get(_ep, _ep)}
            for _fw in _fws:
                _val = _pivot.loc[_ep, _fw] if _fw in _pivot.columns else 0
                _r[f"{_fw} RPS"] = f"{_val:,.0f}"
            if "Rex" in _fws:
                _rex_rps = _pivot.loc[_ep, "Rex"] if "Rex" in _pivot.columns else 0
                for _fw in _fws:
                    if _fw == "Rex":
                        continue
                    _other = _pivot.loc[_ep, _fw] if _fw in _pivot.columns else 0.01
                    _r[f"vs {_fw}"] = f"**{round(_rex_rps / max(_other, 0.01), 1)}x**"
            _rows.append(_r)
        mo.md("### Throughput Summary")
        mo.ui.table(pd.DataFrame(_rows), selection=None)
    _server_summary_table()
    return


# ════════════════════════════════════════════════════════════════
# CLIENT SUITE
# ════════════════════════════════════════════════════════════════


@app.cell
def _(df, mo):
    if not df.empty and "suite" in df.columns and (df["suite"] == "client").any():
        mo.md("## Client-Side Performance")
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    def _plot_bundle_size():
        if df.empty or "suite" not in df.columns or not (df["suite"] == "client").any():
            return
        _client = df[df["suite"] == "client"].copy()
        _js = _client[_client["metric"] == "total_js_kb"]
        _css = _client[_client["metric"] == "total_css_kb"]
        if _js.empty and _css.empty:
            return
        _fig = make_subplots(rows=1, cols=2, subplot_titles=["JavaScript (KB)", "CSS (KB)"], horizontal_spacing=0.12)
        for _ci, _sub in enumerate([_js, _css], 1):
            if _sub.empty:
                continue
            _sub = _sub.sort_values("label", key=lambda s: s.map({v: i for i, v in enumerate(FW_ORDER)}))
            for _, _r in _sub.iterrows():
                _fig.add_trace(go.Bar(x=[_r["label"]], y=[_r["value"]], name=_r["label"], marker_color=COLORS.get(_r["label"], "#888"), showlegend=(_ci == 1), legendgroup=_r["label"]), row=1, col=_ci)
        _fig.update_layout(height=350, font=dict(family="Inter, system-ui, sans-serif"), legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5), title_text="Client Bundle Size — lower is better", barmode="group")
        mo.ui.plotly(_fig)
    _plot_bundle_size()
    return


@app.cell
def _(COLORS, FW_ORDER, df, mo, px):
    def _plot_lighthouse():
        if df.empty or "suite" not in df.columns or not (df["suite"] == "client").any():
            return
        _client = df[df["suite"] == "client"].copy()
        _lh = _client[_client["metric"].isin(["lcp_ms", "fcp_ms", "ttfb_ms", "tbt_ms"])]
        if _lh.empty:
            return mo.md("> **No Lighthouse data.** Install `lighthouse` globally for Web Vitals.")
        _lh = _lh.copy()
        _lh["metric_label"] = _lh["metric"].map({"lcp_ms": "LCP", "fcp_ms": "FCP", "ttfb_ms": "TTFB", "tbt_ms": "TBT"})
        _lh["page"] = _lh["endpoint"].map({"/": "index", "/about": "about", "/blog/hello-world": "blog"}).fillna("")
        _fig = px.bar(_lh, x="metric_label", y="value", color="label", facet_col="page", barmode="group", title="Lighthouse Web Vitals (ms) — lower is better", labels={"value": "ms", "metric_label": "", "label": "Framework"}, color_discrete_map=COLORS, category_orders={"label": FW_ORDER})
        _fig.update_layout(height=400, font=dict(family="Inter, system-ui, sans-serif"), legend=dict(orientation="h", yanchor="bottom", y=1.04, xanchor="center", x=0.5))
        _fig.for_each_annotation(lambda a: a.update(text=a.text.replace("page=", "").title()))
        mo.ui.plotly(_fig)
    _plot_lighthouse()
    return


@app.cell
def _(mo):
    mo.md("""
    ---
    *All benchmarks run on the same machine with identical page fixtures.
    Rex uses V8 isolates for SSR; Next.js and TanStack Start use Node.js.*
    """)
    return


if __name__ == "__main__":
    app.run()
