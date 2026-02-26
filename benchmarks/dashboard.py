import marimo

__generated_with = "0.20.2"
app = marimo.App(width="medium")


@app.cell
def _():
    import marimo as mo

    mo.md(
        """
        # Rex vs Next.js Benchmark Results

        Comparing **Rex** (Rust-native Pages Router) against **Next.js 15** on identical page
        fixtures: SSR (`getServerSideProps`), SSG (`getStaticProps`), dynamic routes, and API endpoints.

        Both frameworks serve the same pages with the same React components.
        Benchmarked with `ab` (Apache Bench) on the same machine, same concurrency.
        """
    )
    return (mo,)


@app.cell
def _(mo):
    import json
    from pathlib import Path

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
        raw = []
    else:
        raw = json.loads(results_path.read_text())

    raw[:2]
    return raw, results_path


@app.cell
def _(raw):
    import pandas as pd

    df = pd.DataFrame(raw)
    if not df.empty:
        df["label"] = df["framework"].str.replace("nextjs", "Next.js").str.replace("rex", "Rex")
        df["endpoint_label"] = df["endpoint"].map(
            {
                "/": "SSR index",
                "/about": "SSG about",
                "/blog/hello-world": "Dynamic route",
                "/api/hello": "API route",
            }
        )
    df
    return (df,)


@app.cell
def _(df, mo):
    import plotly.express as px

    if df.empty:
        mo.md("> No data to plot.")
    else:
        # ── Throughput (RPS) ──
        fig_rps = px.bar(
            df,
            x="endpoint_label",
            y="rps",
            color="label",
            facet_col="mode",
            barmode="group",
            title="Throughput (Requests/sec) — higher is better",
            labels={"rps": "Requests/sec", "endpoint_label": "", "label": "Framework"},
            color_discrete_map={"Rex": "#7c3aed", "Next.js": "#171717"},
            category_orders={"mode": ["dev", "prod"]},
        )
        fig_rps.update_layout(
            height=450,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.04, xanchor="center", x=0.5),
        )
        fig_rps.for_each_annotation(lambda a: a.update(text=a.text.replace("mode=", "").title()))
        mo.ui.plotly(fig_rps)
    return (fig_rps,)


@app.cell
def _(df, mo):
    import plotly.express as px

    if df.empty:
        mo.md("> No data to plot.")
    else:
        # ── Latency (ms) ──
        fig_lat = px.bar(
            df,
            x="endpoint_label",
            y="latency_ms",
            color="label",
            facet_col="mode",
            barmode="group",
            title="Mean Latency (ms) — lower is better",
            labels={"latency_ms": "Latency (ms)", "endpoint_label": "", "label": "Framework"},
            color_discrete_map={"Rex": "#7c3aed", "Next.js": "#171717"},
            category_orders={"mode": ["dev", "prod"]},
        )
        fig_lat.update_layout(
            height=450,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.04, xanchor="center", x=0.5),
        )
        fig_lat.for_each_annotation(lambda a: a.update(text=a.text.replace("mode=", "").title()))
        mo.ui.plotly(fig_lat)
    return (fig_lat,)


@app.cell
def _(df, mo):
    import plotly.express as px

    if df.empty:
        mo.md("> No data to plot.")
    else:
        # ── Cold start comparison ──
        cold = df.drop_duplicates(subset=["framework", "mode"])[["label", "mode", "cold_start_ms", "memory_mb"]]
        fig_cold = px.bar(
            cold,
            x="mode",
            y="cold_start_ms",
            color="label",
            barmode="group",
            title="Cold Start (ms) — lower is better",
            labels={"cold_start_ms": "Cold Start (ms)", "mode": "", "label": "Framework"},
            color_discrete_map={"Rex": "#7c3aed", "Next.js": "#171717"},
            category_orders={"mode": ["dev", "prod"]},
        )
        fig_cold.update_layout(
            height=400,
            font=dict(family="Inter, system-ui, sans-serif"),
            legend=dict(orientation="h", yanchor="bottom", y=1.04, xanchor="center", x=0.5),
            xaxis=dict(tickvals=["dev", "prod"], ticktext=["Dev", "Prod"]),
        )
        mo.ui.plotly(fig_cold)
    return (cold, fig_cold)


@app.cell
def _(df, mo):
    if df.empty:
        mo.md("> No data.")
    else:
        # ── Summary table: Rex advantage multiplier ──
        rex_dev = df[(df["framework"] == "rex") & (df["mode"] == "dev")].set_index("endpoint")
        next_dev = df[(df["framework"] == "nextjs") & (df["mode"] == "dev")].set_index("endpoint")
        rex_prod = df[(df["framework"] == "rex") & (df["mode"] == "prod")].set_index("endpoint")
        next_prod = df[(df["framework"] == "nextjs") & (df["mode"] == "prod")].set_index("endpoint")

        rows = []
        for ep in ["/", "/about", "/blog/hello-world", "/api/hello"]:
            label = {"/": "SSR index", "/about": "SSG about", "/blog/hello-world": "Dynamic route", "/api/hello": "API route"}[ep]
            dev_x = round(rex_dev.loc[ep, "rps"] / max(next_dev.loc[ep, "rps"], 0.01), 1)
            prod_x = round(rex_prod.loc[ep, "rps"] / max(next_prod.loc[ep, "rps"], 0.01), 1)
            rows.append(
                {
                    "Endpoint": label,
                    "Rex Dev RPS": f'{rex_dev.loc[ep, "rps"]:,.0f}',
                    "Next Dev RPS": f'{next_dev.loc[ep, "rps"]:,.0f}',
                    "Dev Speedup": f"**{dev_x}x**",
                    "Rex Prod RPS": f'{rex_prod.loc[ep, "rps"]:,.0f}',
                    "Next Prod RPS": f'{next_prod.loc[ep, "rps"]:,.0f}',
                    "Prod Speedup": f"**{prod_x}x**",
                }
            )

        import pandas as pd

        summary = pd.DataFrame(rows)
        mo.md("## Speedup Summary")
        mo.ui.table(summary, selection=None)
    return


@app.cell
def _(df, mo):
    if df.empty:
        mo.md("")
    else:
        cold_rex_dev = df[(df["framework"] == "rex") & (df["mode"] == "dev")].iloc[0]["cold_start_ms"]
        cold_next_dev = df[(df["framework"] == "nextjs") & (df["mode"] == "dev")].iloc[0]["cold_start_ms"]
        cold_rex_prod = df[(df["framework"] == "rex") & (df["mode"] == "prod")].iloc[0]["cold_start_ms"]
        cold_next_prod = df[(df["framework"] == "nextjs") & (df["mode"] == "prod")].iloc[0]["cold_start_ms"]

        mo.md(
            f"""
            ## Cold Start

            | | Rex | Next.js | Speedup |
            |---|---:|---:|---:|
            | **Dev** | {cold_rex_dev:,.0f}ms | {cold_next_dev:,.0f}ms | **{cold_next_dev/cold_rex_dev:.0f}x** |
            | **Prod** | {cold_rex_prod:,.0f}ms | {cold_next_prod:,.0f}ms | **{cold_next_prod/cold_rex_prod:.0f}x** |

            ---
            *Measured on the same machine. Rex uses V8 isolates for SSR; Next.js uses Node.js.
            Both serve identical React pages with `getServerSideProps` / `getStaticProps`.*
            """
        )
    return


if __name__ == "__main__":
    app.run()
