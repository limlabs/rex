import marimo

__generated_with = "0.20.2"
app = marimo.App(width="medium")


@app.cell
def _():
    import marimo as mo
    import json
    import pandas as pd
    import plotly.express as px
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots
    from pathlib import Path

    COLORS = {"Rex": "#7c3aed", "Next.js": "#171717", "TanStack Start": "#e8590c"}
    FW_ORDER = ["Rex", "Next.js", "TanStack Start"]
    FW_MAP = {"rex": "Rex", "nextjs": "Next.js", "tanstack": "TanStack Start"}
    return COLORS, FW_MAP, FW_ORDER, Path, go, json, make_subplots, mo, pd, px


@app.cell
def _(mo):
    mo.md("""
    # Rex Benchmark Dashboard

    Comparing **Rex** (Rust-native Pages Router) against **Next.js 15** and
    **TanStack Start v1** on identical page fixtures.

    Three benchmark suites:
    - **DX** — Developer experience: install time, dependencies, startup, rebuild speed
    - **Server** — Production throughput (RPS), latency (p50/p99), build time, memory
    - **Client** — JS bundle size, Lighthouse Web Vitals
    """)
    return


@app.cell
def _(FW_MAP, Path, json, mo, pd):
    results_path = Path(__file__).parent / "results.json"
    if not results_path.exists():
        mo.md(
            """
            > **No results found.** Run the benchmark first:
            > ```
            > ./benchmarks/run.sh --json benchmarks/results.json
            > ```
            """
        )
        df = pd.DataFrame()
    else:
        raw = json.loads(results_path.read_text())
        df = pd.DataFrame(raw)
        if not df.empty:
            df["label"] = df["framework"].map(FW_MAP)
    return (df,)


@app.cell
def _(df, mo):
    _has_dx = not df.empty and "suite" in df.columns and (df["suite"] == "dx").any()
    if not _has_dx:
        mo.md("> No DX data. Run: `./benchmarks/run.sh --suite dx --json benchmarks/results.json`")
    else:
        mo.md("## Developer Experience")
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    if df.empty or "suite" not in df.columns or not (df["suite"] == "dx").any():
        mo.md("")
    else:
        dx = df[df["suite"] == "dx"].copy()

        metrics = [
            ("install_time_ms", "npm install (ms)", True),
            ("dependency_count", "Dependencies", True),
            ("node_modules_mb", "node_modules (MB)", True),
        ]

        fig = make_subplots(
            rows=1, cols=3,
            subplot_titles=[m[1] for m in metrics],
            horizontal_spacing=0.08,
        )

        for col_idx, (metric, title, lower_better) in enumerate(metrics, 1):
            subset = dx[dx["metric"] == metric].sort_values(
                "label", key=lambda s: s.map({v: i for i, v in enumerate(FW_ORDER)})
            )
            for _, row in subset.iterrows():
                fig.add_trace(
                    go.Bar(
                        x=[row["label"]],
                        y=[row["value"]],
                        name=row["label"],
                        marker_color=COLORS.get(row["label"], "#888"),
                        showlegend=(col_idx == 1),
                        legendgroup=row["label"],
                    ),
                    row=1, col=col_idx,
                )

        fig.update_layout(
            height=350,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
            title_text="Dependency Footprint — lower is better",
            barmode="group",
        )
        mo.ui.plotly(fig)
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    if df.empty or "suite" not in df.columns or not (df["suite"] == "dx").any():
        mo.md("")
    else:
        dx = df[df["suite"] == "dx"].copy()

        metrics = [
            ("cold_start_ms", "Cold Start (ms)"),
            ("rebuild_ms", "HMR Rebuild (ms)"),
            ("dev_memory_mb", "Dev Memory (MB)"),
        ]

        fig = make_subplots(
            rows=1, cols=3,
            subplot_titles=[m[1] for m in metrics],
            horizontal_spacing=0.08,
        )

        for col_idx, (metric, title) in enumerate(metrics, 1):
            subset = dx[dx["metric"] == metric].sort_values(
                "label", key=lambda s: s.map({v: i for i, v in enumerate(FW_ORDER)})
            )
            for _, row in subset.iterrows():
                fig.add_trace(
                    go.Bar(
                        x=[row["label"]],
                        y=[row["value"]],
                        name=row["label"],
                        marker_color=COLORS.get(row["label"], "#888"),
                        showlegend=(col_idx == 1),
                        legendgroup=row["label"],
                    ),
                    row=1, col=col_idx,
                )

        fig.update_layout(
            height=350,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
            title_text="Dev Server Performance — lower is better",
            barmode="group",
        )
        mo.ui.plotly(fig)
    return


@app.cell
def _(FW_ORDER, df, mo, pd):
    if df.empty or "suite" not in df.columns or not (df["suite"] == "dx").any():
        mo.md("")
    else:
        dx = df[df["suite"] == "dx"].copy()
        pivot = dx.pivot_table(index="label", columns="metric", values="value", aggfunc="first")
        pivot = pivot.reindex([fw for fw in FW_ORDER if fw in pivot.index])

        rows = []
        for fw in pivot.index:
            row = {"Framework": fw}
            if "install_time_ms" in pivot.columns:
                row["Install"] = f'{pivot.loc[fw, "install_time_ms"]:,.0f}ms'
            if "dependency_count" in pivot.columns:
                row["Deps"] = f'{pivot.loc[fw, "dependency_count"]:,.0f}'
            if "node_modules_mb" in pivot.columns:
                row["node_modules"] = f'{pivot.loc[fw, "node_modules_mb"]:,.0f}MB'
            if "cold_start_ms" in pivot.columns:
                row["Cold Start"] = f'{pivot.loc[fw, "cold_start_ms"]:,.0f}ms'
            if "rebuild_ms" in pivot.columns:
                row["Rebuild"] = f'{pivot.loc[fw, "rebuild_ms"]:,.0f}ms'
            if "dev_memory_mb" in pivot.columns:
                row["Memory"] = f'{pivot.loc[fw, "dev_memory_mb"]:,.0f}MB'
            rows.append(row)

        mo.md("### DX Summary")
        mo.ui.table(pd.DataFrame(rows), selection=None)
    return


@app.cell
def _(df, mo):
    _has_server = not df.empty and "suite" in df.columns and (df["suite"] == "server").any()
    if not _has_server:
        mo.md("> No server data. Run: `./benchmarks/run.sh --suite server --json benchmarks/results.json`")
    else:
        mo.md("## Production Server Performance")
    return


@app.cell
def _(COLORS, FW_ORDER, df, mo, px):
    if df.empty or "suite" not in df.columns or not (df["suite"] == "server").any():
        mo.md("")
    else:
        server = df[df["suite"] == "server"].copy()
        rps = server[(server["metric"] == "rps") & (server["endpoint"].notna())].copy()

        if rps.empty:
            mo.md("> No RPS data.")
        else:
            rps["endpoint_label"] = rps["endpoint"].map({
                "/": "SSR index",
                "/about": "SSG about",
                "/blog/hello-world": "Dynamic route",
                "/api/hello": "API route",
            })

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
            mo.ui.plotly(fig)
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    if df.empty or "suite" not in df.columns or not (df["suite"] == "server").any():
        mo.md("")
    else:
        server = df[df["suite"] == "server"].copy()
        ep_metrics = server[server["endpoint"].notna()].copy()
        ep_metrics["endpoint_label"] = ep_metrics["endpoint"].map({
            "/": "SSR index",
            "/about": "SSG about",
            "/blog/hello-world": "Dynamic route",
            "/api/hello": "API route",
        })

        lat_mean = ep_metrics[ep_metrics["metric"] == "latency_mean_ms"]
        lat_p99 = ep_metrics[ep_metrics["metric"] == "latency_p99_ms"]

        if lat_mean.empty and lat_p99.empty:
            mo.md("> No latency data.")
        else:
            fig = make_subplots(
                rows=1, cols=2,
                subplot_titles=["Mean Latency (ms)", "p99 Latency (ms)"],
                horizontal_spacing=0.1,
            )

            for col_idx, (subset, title) in enumerate([(lat_mean, "Mean"), (lat_p99, "p99")], 1):
                if subset.empty:
                    continue
                for fw in FW_ORDER:
                    fw_data = subset[subset["label"] == fw]
                    if fw_data.empty:
                        continue
                    fig.add_trace(
                        go.Bar(
                            x=fw_data["endpoint_label"],
                            y=fw_data["value"],
                            name=fw,
                            marker_color=COLORS.get(fw, "#888"),
                            showlegend=(col_idx == 1),
                            legendgroup=fw,
                        ),
                        row=1, col=col_idx,
                    )

            fig.update_layout(
                height=400,
                font=dict(family="Inter, system-ui, sans-serif"),
                legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
                title_text="Latency — lower is better",
                barmode="group",
            )
            mo.ui.plotly(fig)
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    if df.empty or "suite" not in df.columns or not (df["suite"] == "server").any():
        mo.md("")
    else:
        server = df[df["suite"] == "server"].copy()
        fw_metrics = server[~server["endpoint"].notna() | (server["endpoint"] == "")].copy()

        metrics = [
            ("build_time_ms", "Build Time (ms)"),
            ("cold_start_ms", "Cold Start (ms)"),
            ("memory_mb", "Memory (MB)"),
        ]

        fig = make_subplots(
            rows=1, cols=3,
            subplot_titles=[m[1] for m in metrics],
            horizontal_spacing=0.08,
        )

        for col_idx, (metric, title) in enumerate(metrics, 1):
            subset = fw_metrics[fw_metrics["metric"] == metric].sort_values(
                "label", key=lambda s: s.map({v: i for i, v in enumerate(FW_ORDER)})
            )
            for _, row in subset.iterrows():
                fig.add_trace(
                    go.Bar(
                        x=[row["label"]],
                        y=[row["value"]],
                        name=row["label"],
                        marker_color=COLORS.get(row["label"], "#888"),
                        showlegend=(col_idx == 1),
                        legendgroup=row["label"],
                    ),
                    row=1, col=col_idx,
                )

        fig.update_layout(
            height=350,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
            title_text="Build & Startup — lower is better",
            barmode="group",
        )
        mo.ui.plotly(fig)
    return


@app.cell
def _(FW_ORDER, df, mo, pd):
    if df.empty or "suite" not in df.columns or not (df["suite"] == "server").any():
        mo.md("")
    else:
        server = df[df["suite"] == "server"].copy()
        rps = server[(server["metric"] == "rps") & (server["endpoint"].notna())]

        if rps.empty:
            mo.md("> No server data to summarize.")
        else:
            pivot = rps.pivot_table(index="endpoint", columns="label", values="value", aggfunc="first")
            frameworks = [fw for fw in FW_ORDER if fw in pivot.columns]

            ep_labels = {
                "/": "SSR index",
                "/about": "SSG about",
                "/blog/hello-world": "Dynamic route",
                "/api/hello": "API route",
            }

            rows = []
            for ep in ["/", "/about", "/blog/hello-world", "/api/hello"]:
                if ep not in pivot.index:
                    continue
                row = {"Endpoint": ep_labels.get(ep, ep)}
                for fw in frameworks:
                    rps_val = pivot.loc[ep, fw] if fw in pivot.columns else 0
                    row[f"{fw} RPS"] = f"{rps_val:,.0f}"
                # Speedup vs each non-Rex framework
                if "Rex" in frameworks:
                    rex_rps = pivot.loc[ep, "Rex"] if "Rex" in pivot.columns else 0
                    for fw in frameworks:
                        if fw == "Rex":
                            continue
                        other_rps = pivot.loc[ep, fw] if fw in pivot.columns else 0.01
                        speedup = round(rex_rps / max(other_rps, 0.01), 1)
                        row[f"vs {fw}"] = f"**{speedup}x**"
                rows.append(row)

            mo.md("### Throughput Summary")
            mo.ui.table(pd.DataFrame(rows), selection=None)
    return


@app.cell
def _(df, mo):
    _has_client = not df.empty and "suite" in df.columns and (df["suite"] == "client").any()
    if not _has_client:
        mo.md("> No client data. Run: `./benchmarks/run.sh --suite client --json benchmarks/results.json`")
    else:
        mo.md("## Client-Side Performance")
    return


@app.cell
def _(COLORS, FW_ORDER, df, go, make_subplots, mo):
    if df.empty or "suite" not in df.columns or not (df["suite"] == "client").any():
        mo.md("")
    else:
        client = df[df["suite"] == "client"].copy()
        js = client[client["metric"] == "total_js_kb"]
        css = client[client["metric"] == "total_css_kb"]

        if js.empty and css.empty:
            mo.md("> No bundle size data.")
        else:
            fig = make_subplots(
                rows=1, cols=2,
                subplot_titles=["JavaScript (KB)", "CSS (KB)"],
                horizontal_spacing=0.12,
            )

            for col_idx, subset in enumerate([js, css], 1):
                if subset.empty:
                    continue
                subset = subset.sort_values(
                    "label", key=lambda s: s.map({v: i for i, v in enumerate(FW_ORDER)})
                )
                for _, row in subset.iterrows():
                    fig.add_trace(
                        go.Bar(
                            x=[row["label"]],
                            y=[row["value"]],
                            name=row["label"],
                            marker_color=COLORS.get(row["label"], "#888"),
                            showlegend=(col_idx == 1),
                            legendgroup=row["label"],
                        ),
                        row=1, col=col_idx,
                    )

            fig.update_layout(
                height=350,
                font=dict(family="Inter, system-ui, sans-serif"),
                legend=dict(orientation="h", yanchor="bottom", y=1.08, xanchor="center", x=0.5),
                title_text="Client Bundle Size — lower is better",
                barmode="group",
            )
            mo.ui.plotly(fig)
    return


@app.cell
def _(COLORS, FW_ORDER, df, mo, px):
    if df.empty or "suite" not in df.columns or not (df["suite"] == "client").any():
        mo.md("")
    else:
        client = df[df["suite"] == "client"].copy()
        lh = client[client["metric"].isin(["lcp_ms", "fcp_ms", "ttfb_ms", "tbt_ms"])]

        if lh.empty:
            mo.md(
                """
                > **No Lighthouse data.** Install Lighthouse for Web Vitals:
                > ```
                > npm install -g lighthouse
                > ./benchmarks/run.sh --suite client --json benchmarks/results.json
                > ```
                """
            )
        else:
            lh["metric_label"] = lh["metric"].map({
                "lcp_ms": "LCP",
                "fcp_ms": "FCP",
                "ttfb_ms": "TTFB",
                "tbt_ms": "TBT",
            })
            ep_label = lh["endpoint"].map({
                "/": "index",
                "/about": "about",
                "/blog/hello-world": "blog",
            }).fillna("")
            lh = lh.copy()
            lh["page"] = ep_label

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
            mo.ui.plotly(fig)
    return


@app.cell
def _(mo):
    mo.md("""
    ---
    *All benchmarks run on the same machine with identical page fixtures.
    Rex uses V8 isolates for SSR; Next.js and TanStack Start use Node.js.
    Generated by `./benchmarks/run.sh`.*
    """)
    return


if __name__ == "__main__":
    app.run()
