# Benchmark Methodology

How the numbers in the README performance table are produced, and how to reproduce them.

## Quick Start

```sh
cd benchmarks
uv run python bench.py --suite dx,server --framework rex,nextjs --iterations 1 --json results-readme.json
```

Raw measurements are saved to `results-readme.json` (checked into the repo). The README table is derived from those measurements as described below.

## Environment

- **Machine**: Apple M3 Max, 36 GB
- **OS**: macOS
- **Tool**: Apache Bench (`ab`) for throughput/latency; wall-clock timing for everything else
- **Network speed**: Auto-detected before each run by downloading the TypeScript tarball (~10 MB) from the npm registry. Printed in the banner and recorded in results as `network_speed`.

## What's Measured

### DX Suite (`--suite dx`)

| README metric | Raw metric | How it's measured |
|---|---|---|
| **Install size** | `install_size_mb` | `node_modules` directory size + Rex binary size |
| **Install time** | `install_time_ms` | `npm install --no-audit --no-fund` after `npm cache clean --force`. No local cache. |
| **Dev cold start** | `cold_start_ms` | Time from process spawn to first successful HTTP response on `/` |
| **Lint** | `lint_time_ms` | Wall-clock time of `rex lint` or `npx eslint .` |

### Server Suite (`--suite server`)

| README metric | Raw metric | How it's measured |
|---|---|---|
| **Production build** | `build_time_ms` | Wall-clock time of `rex build` or `npx next build` (clean output dir, no cache) |
| **SSR throughput** | `rps` (endpoint `/`) | Apache Bench: 10k requests, 100 concurrent, after 200 warmup requests |
| **SSR latency** | `latency_mean_ms` (endpoint `/`) | Mean time-per-request reported by Apache Bench (same run as throughput) |

## Processing

- **Single iteration** (`--iterations 1`): The raw value is used directly.
- **Multiple iterations** (`--iterations N`): The **median** is reported. Standard deviation is recorded alongside.
- All timing values are in milliseconds, rounded to the nearest integer.
- Throughput (req/s) is reported as-is from Apache Bench's "Requests per second" output.
- Latency uses Apache Bench's "Time per request (mean)" — the per-request average, not the per-concurrent-group average.

## Raw Results Format

`results-readme.json` is a flat JSON array. Each entry has:

```json
{
  "suite": "dx" | "server" | "meta",
  "framework": "rex" | "nextjs",
  "metric": "install_time_ms" | "rps" | ...,
  "value": 842.0,
  "iterations": 1,
  "stddev": 0.0,
  "endpoint": "/"
}
```

- `iterations` and `stddev` appear on sampled metrics (timing, throughput).
- `endpoint` appears on per-endpoint server metrics.
- The `meta` suite contains `network_speed` (e.g., `"126.8 Mbps"`).

## README Table Mapping

| README row | JSON filter |
|---|---|
| SSR throughput | `suite=server, framework=rex, metric=rps, endpoint=/` |
| SSR latency | `suite=server, framework=rex, metric=latency_mean_ms, endpoint=/` |
| Production build | `suite=server, metric=build_time_ms` |
| Dev cold start | `suite=dx, metric=cold_start_ms` |
| Install size | `suite=dx, metric=install_size_mb` |
| Install time | `suite=dx, metric=install_time_ms` |
| Lint | `suite=dx, metric=lint_time_ms` |

## Fairness Controls

- **No build cache**: Build output directories are cleaned before each build measurement.
- **No npm cache**: `npm cache clean --force` runs before each install measurement.
- **Same pages**: Both frameworks render the same index/about/blog pages with the same `getServerSideProps` data fetching.
- **Same ab parameters**: Identical request count, concurrency, and warmup for both frameworks.
- **Network speed recorded**: So readers can contextualize install-time numbers against their own connection.

## Reproducing

1. **Prerequisites**: Rust toolchain, Node.js 20+, [uv](https://docs.astral.sh/uv/), Apache Bench (`ab` — ships with macOS).

2. **Build Rex**:
   ```sh
   cargo build --release
   ```

3. **Install fixture deps**:
   ```sh
   cd fixtures/basic && npm install && cd ../..
   cd benchmarks/next-basic && npm install && cd ../..
   ```

4. **Run**:
   ```sh
   cd benchmarks
   uv run python bench.py --suite dx,server --framework rex,nextjs --iterations 1 --json results-readme.json
   ```

5. **Compare**: Diff your `results-readme.json` against the checked-in version to see how your machine/network differs.
