import marimo

__generated_with = "0.20.2"
app = marimo.App(width="medium")


@app.cell
def _():
    import importlib.util
    import re
    import subprocess
    import time

    import marimo as mo
    import polars as pl
    import plotly.express as px
    import plotly.graph_objects as go
    from pathlib import Path
    from plotly.subplots import make_subplots

    _spec = importlib.util.spec_from_file_location(
        "dashboard_helpers", str(Path(__file__).parent / "dashboard_helpers.py")
    )
    _mod = importlib.util.module_from_spec(_spec)
    _spec.loader.exec_module(_mod)

    COLORS = _mod.COLORS
    FW_ORDER = _mod.FW_ORDER
    EP_LABELS = _mod.EP_LABELS
    has_suite = _mod.has_suite
    sort_by_fw = _mod.sort_by_fw
    load_results = _mod.load_results
    make_dx_summary = _mod.make_dx_summary
    make_server_summary = _mod.make_server_summary

    BENCH_DIR = Path(__file__).parent
    # Default to checked-in results; fresh runs write to results.json
    RESULTS_README = BENCH_DIR / "results-readme.json"
    RESULTS_FRESH = BENCH_DIR / "results.json"
    RESULTS_PATH = RESULTS_FRESH if RESULTS_FRESH.exists() else RESULTS_README

    return (
        BENCH_DIR,
        COLORS,
        EP_LABELS,
        FW_ORDER,
        RESULTS_FRESH,
        RESULTS_PATH,
        RESULTS_README,
        go,
        has_suite,
        load_results,
        make_dx_summary,
        make_server_summary,
        make_subplots,
        mo,
        Path,
        pl,
        px,
        re,
        sort_by_fw,
        subprocess,
        time,
    )


@app.cell
def _(mo):
    mo.md("""
    # Rex Benchmark Dashboard

    Comparing **Rex** (Rust-native Pages Router) against **Next.js 15** and
    **TanStack Start v1** on identical page fixtures.
    """)
    return


@app.cell
def _(mo):
    get_bench_ts, set_bench_ts = mo.state(0)
    return get_bench_ts, set_bench_ts


@app.cell
def _(mo):
    suite_select = mo.ui.multiselect(
        options={"DX": "dx", "Server": "server", "Client": "client"},
        value=["DX", "Server", "Client"],
        label="Suites",
    )
    fw_select = mo.ui.multiselect(
        options={
            "Rex": "rex",
            "Next.js (Pages)": "nextjs",
            "Next.js (App)": "nextjs_app",
            "TanStack Start": "tanstack",
        },
        value=["Rex"],
        label="Frameworks",
    )
    requests_input = mo.ui.number(
        value=5000,
        start=100,
        stop=50000,
        step=100,
        label="Requests (server)",
    )
    concurrency_input = mo.ui.number(
        value=50,
        start=1,
        stop=500,
        step=10,
        label="Concurrency",
    )
    run_btn = mo.ui.run_button(label="Run Benchmarks")

    mo.vstack(
        [
            mo.hstack([suite_select, fw_select], justify="start", gap=1),
            mo.hstack([requests_input, concurrency_input], justify="start", gap=1),
            run_btn,
        ]
    )
    return concurrency_input, fw_select, requests_input, run_btn, suite_select


@app.cell
def _(
    BENCH_DIR,
    RESULTS_FRESH,
    concurrency_input,
    fw_select,
    mo,
    re,
    requests_input,
    run_btn,
    set_bench_ts,
    subprocess,
    suite_select,
    time,
):
    mo.stop(not run_btn.value)

    def run_benchmarks():
        import sys as _sys

        suites = ",".join(suite_select.value) if suite_select.value else "dx,server,client"
        fws = ",".join(fw_select.value) if fw_select.value else "rex"
        cmd = [
            _sys.executable,
            str(BENCH_DIR / "bench.py"),
            "--suite",
            suites,
            "--framework",
            fws,
            "--requests",
            str(int(requests_input.value)),
            "--concurrency",
            str(int(concurrency_input.value)),
            "--json",
            str(RESULTS_FRESH),
        ]

        t0 = time.time()
        output_lines = []
        progress_pat = re.compile(r"\[PROGRESS (\d+/\d+)\] (.+)")

        with mo.status.spinner(title="Starting benchmarks...", remove_on_exit=True) as spinner:
            proc = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                cwd=str(BENCH_DIR),
            )
            for line in proc.stdout:
                output_lines.append(line)
                match = progress_pat.search(line)
                if match:
                    spinner.update(
                        title=f"Running benchmarks ({match.group(1)}) — {match.group(2)}",
                    )
            proc.wait(timeout=600)

        elapsed = time.time() - t0
        raw = "".join(output_lines)
        clean = re.sub(r"\x1b\[[0-9;]*m", "", raw)
        set_bench_ts(time.time())

        return mo.vstack(
            [
                mo.md(f"Benchmarks completed in **{elapsed:.1f}s**"),
                mo.accordion({"Benchmark output": mo.md(f"```\n{clean}\n```")}),
            ]
        )

    run_benchmarks()
    return


@app.cell
def _(RESULTS_FRESH, RESULTS_README, get_bench_ts, load_results):
    get_bench_ts()
    # Prefer fresh run results; fall back to checked-in data
    path = RESULTS_FRESH if RESULTS_FRESH.exists() else RESULTS_README
    df = load_results(path)
    return (df,)


# --- DX Section ---


@app.cell
def _(df, has_suite, mo):
    mo.md("## Developer Experience") if has_suite(df, "dx") else None
    return


@app.cell
def _(COLORS, df, go, has_suite, make_subplots, mo, pl, sort_by_fw):
    def plot_dx_footprint():
        if not has_suite(df, "dx"):
            return None
        dx = df.filter(pl.col("suite") == "dx")
        metrics = [
            ("install_time_ms", "npm install (ms)"),
            ("dependency_count", "Dependencies"),
            ("node_modules_mb", "node_modules (MB)"),
        ]
        fig = make_subplots(
            rows=1,
            cols=3,
            subplot_titles=[t for _, t in metrics],
            horizontal_spacing=0.08,
        )
        for ci, (metric, _) in enumerate(metrics, 1):
            data = sort_by_fw(dx.filter(pl.col("metric") == metric))
            for row in data.iter_rows(named=True):
                fig.add_trace(
                    go.Bar(
                        x=[row["label"]],
                        y=[row["value"]],
                        name=row["label"],
                        marker_color=COLORS.get(row["label"], "#888"),
                        showlegend=(ci == 1),
                        legendgroup=row["label"],
                    ),
                    row=1,
                    col=ci,
                )
        fig.update_layout(
            height=350,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
            title_text="Dependency Footprint — lower is better",
            barmode="group",
        )
        return mo.ui.plotly(fig)

    plot_dx_footprint()
    return


@app.cell
def _(COLORS, df, go, has_suite, make_subplots, mo, pl, sort_by_fw):
    def plot_dx_perf():
        if not has_suite(df, "dx"):
            return None
        dx = df.filter(pl.col("suite") == "dx")
        metrics = [
            ("cold_start_ms", "Cold Start (ms)"),
            ("rebuild_ms", "HMR Rebuild (ms)"),
            ("dev_memory_mb", "Dev Memory (MB)"),
        ]
        fig = make_subplots(
            rows=1,
            cols=3,
            subplot_titles=[t for _, t in metrics],
            horizontal_spacing=0.08,
        )
        for ci, (metric, _) in enumerate(metrics, 1):
            data = sort_by_fw(dx.filter(pl.col("metric") == metric))
            for row in data.iter_rows(named=True):
                fig.add_trace(
                    go.Bar(
                        x=[row["label"]],
                        y=[row["value"]],
                        name=row["label"],
                        marker_color=COLORS.get(row["label"], "#888"),
                        showlegend=(ci == 1),
                        legendgroup=row["label"],
                    ),
                    row=1,
                    col=ci,
                )
        fig.update_layout(
            height=350,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
            title_text="Dev Server Performance — lower is better",
            barmode="group",
        )
        return mo.ui.plotly(fig)

    plot_dx_perf()
    return


@app.cell
def _(df, make_dx_summary, mo, pl):
    def dx_summary():
        rows = make_dx_summary(df)
        if not rows:
            return None
        return mo.vstack(
            [
                mo.md("### DX Summary"),
                mo.ui.table(pl.DataFrame(rows), selection=None),
            ]
        )

    dx_summary()
    return


# --- Server Section ---


@app.cell
def _(df, has_suite, mo):
    mo.md("## Production Server Performance") if has_suite(df, "server") else None
    return


@app.cell
def _(COLORS, EP_LABELS, FW_ORDER, df, has_suite, mo, pl, px):
    def plot_rps():
        if not has_suite(df, "server"):
            return None
        server = df.filter(pl.col("suite") == "server")
        rps = server.filter(pl.col("metric") == "rps", pl.col("endpoint").is_not_null())
        if rps.is_empty():
            return None
        ep_keys, ep_vals = list(EP_LABELS.keys()), list(EP_LABELS.values())
        rps = rps.with_columns(
            pl.col("endpoint").replace(ep_keys, ep_vals).alias("endpoint_label"),
        )
        fig = px.bar(
            rps,
            x="endpoint_label",
            y="value",
            color="label",
            barmode="group",
            title="Throughput (Requests/sec) — higher is better",
            labels={"value": "Requests/sec", "endpoint_label": "", "label": "Framework"},
            color_discrete_map=COLORS,
            category_orders={"label": FW_ORDER},
        )
        fig.update_layout(
            height=450,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.04, xanchor="center", x=0.5),
        )
        return mo.ui.plotly(fig)

    plot_rps()
    return


@app.cell
def _(COLORS, EP_LABELS, FW_ORDER, df, go, has_suite, make_subplots, mo, pl):
    def plot_latency():
        if not has_suite(df, "server"):
            return None
        server = df.filter(pl.col("suite") == "server")
        ep = server.filter(pl.col("endpoint").is_not_null())
        ep_keys, ep_vals = list(EP_LABELS.keys()), list(EP_LABELS.values())
        ep = ep.with_columns(
            pl.col("endpoint").replace(ep_keys, ep_vals).alias("endpoint_label"),
        )
        lat_mean = ep.filter(pl.col("metric") == "latency_mean_ms")
        lat_p99 = ep.filter(pl.col("metric") == "latency_p99_ms")
        if lat_mean.is_empty() and lat_p99.is_empty():
            return None
        fig = make_subplots(
            rows=1,
            cols=2,
            subplot_titles=["Mean Latency (ms)", "p99 Latency (ms)"],
            horizontal_spacing=0.1,
        )
        for ci, subset in enumerate([lat_mean, lat_p99], 1):
            if subset.is_empty():
                continue
            for fw in FW_ORDER:
                fw_data = subset.filter(pl.col("label") == fw)
                if fw_data.is_empty():
                    continue
                fig.add_trace(
                    go.Bar(
                        x=fw_data["endpoint_label"].to_list(),
                        y=fw_data["value"].to_list(),
                        name=fw,
                        marker_color=COLORS.get(fw, "#888"),
                        showlegend=(ci == 1),
                        legendgroup=fw,
                    ),
                    row=1,
                    col=ci,
                )
        fig.update_layout(
            height=400,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
            title_text="Latency — lower is better",
            barmode="group",
        )
        return mo.ui.plotly(fig)

    plot_latency()
    return


@app.cell
def _(COLORS, df, go, has_suite, make_subplots, mo, pl, sort_by_fw):
    def plot_build_startup():
        if not has_suite(df, "server"):
            return None
        server = df.filter(pl.col("suite") == "server")
        fw_metrics = server.filter(pl.col("endpoint").is_null() | (pl.col("endpoint") == ""))
        metrics = [
            ("build_time_ms", "Build Time (ms)"),
            ("cold_start_ms", "Cold Start (ms)"),
            ("memory_mb", "Memory (MB)"),
        ]
        fig = make_subplots(
            rows=1,
            cols=3,
            subplot_titles=[t for _, t in metrics],
            horizontal_spacing=0.08,
        )
        for ci, (metric, _) in enumerate(metrics, 1):
            data = sort_by_fw(fw_metrics.filter(pl.col("metric") == metric))
            for row in data.iter_rows(named=True):
                fig.add_trace(
                    go.Bar(
                        x=[row["label"]],
                        y=[row["value"]],
                        name=row["label"],
                        marker_color=COLORS.get(row["label"], "#888"),
                        showlegend=(ci == 1),
                        legendgroup=row["label"],
                    ),
                    row=1,
                    col=ci,
                )
        fig.update_layout(
            height=350,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
            title_text="Build & Startup — lower is better",
            barmode="group",
        )
        return mo.ui.plotly(fig)

    plot_build_startup()
    return


@app.cell
def _(df, make_server_summary, mo, pl):
    def server_summary():
        rows = make_server_summary(df)
        if not rows:
            return None
        return mo.vstack(
            [
                mo.md("### Throughput Summary"),
                mo.ui.table(pl.DataFrame(rows), selection=None),
            ]
        )

    server_summary()
    return


# --- Client Section ---


@app.cell
def _(df, has_suite, mo):
    mo.md("## Client-Side Performance") if has_suite(df, "client") else None
    return


@app.cell
def _(COLORS, df, go, has_suite, make_subplots, mo, pl, sort_by_fw):
    def plot_bundle_size():
        if not has_suite(df, "client"):
            return None
        client = df.filter(pl.col("suite") == "client")
        js_data = client.filter(pl.col("metric") == "total_js_kb")
        css_data = client.filter(pl.col("metric") == "total_css_kb")
        if js_data.is_empty() and css_data.is_empty():
            return None
        fig = make_subplots(
            rows=1,
            cols=2,
            subplot_titles=["JavaScript (KB)", "CSS (KB)"],
            horizontal_spacing=0.12,
        )
        for ci, subset in enumerate([js_data, css_data], 1):
            if subset.is_empty():
                continue
            sorted_data = sort_by_fw(subset)
            for row in sorted_data.iter_rows(named=True):
                fig.add_trace(
                    go.Bar(
                        x=[row["label"]],
                        y=[row["value"]],
                        name=row["label"],
                        marker_color=COLORS.get(row["label"], "#888"),
                        showlegend=(ci == 1),
                        legendgroup=row["label"],
                    ),
                    row=1,
                    col=ci,
                )
        fig.update_layout(
            height=350,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
            title_text="Client Bundle Size — lower is better",
            barmode="group",
        )
        return mo.ui.plotly(fig)

    plot_bundle_size()
    return


@app.cell
def _(COLORS, FW_ORDER, df, has_suite, mo, pl, px):
    def plot_lighthouse():
        if not has_suite(df, "client"):
            return None
        client = df.filter(pl.col("suite") == "client")
        lh = client.filter(pl.col("metric").is_in(["lcp_ms", "fcp_ms", "ttfb_ms", "tbt_ms"]))
        if lh.is_empty():
            return mo.md("> **No Lighthouse data.** Install `lighthouse` globally for Web Vitals.")
        metric_map = {"lcp_ms": "LCP", "fcp_ms": "FCP", "ttfb_ms": "TTFB", "tbt_ms": "TBT"}
        page_map = {"/": "index", "/about": "about", "/blog/hello-world": "blog"}
        lh = lh.with_columns(
            pl.col("metric")
            .replace(
                list(metric_map.keys()),
                list(metric_map.values()),
            )
            .alias("metric_label"),
            pl.col("endpoint")
            .replace(
                list(page_map.keys()),
                list(page_map.values()),
                default="",
            )
            .fill_null("")
            .alias("page"),
        )
        fig = px.bar(
            lh,
            x="metric_label",
            y="value",
            color="label",
            facet_col="page",
            barmode="group",
            title="Lighthouse Web Vitals (ms) — lower is better",
            labels={"value": "ms", "metric_label": "", "label": "Framework"},
            color_discrete_map=COLORS,
            category_orders={"label": FW_ORDER},
        )
        fig.update_layout(
            height=400,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.04, xanchor="center", x=0.5),
        )
        fig.for_each_annotation(lambda a: a.update(text=a.text.replace("page=", "").title()))
        return mo.ui.plotly(fig)

    plot_lighthouse()
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
