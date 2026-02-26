#!/usr/bin/env bash
#
# Rex vs Next.js benchmark suite
#
# Prerequisites:
#   - cargo build --release
#   - cd benchmarks/next-basic && npm install
#   - oha (cargo install oha) or ab (comes with macOS)
#
# Usage:
#   ./benchmarks/run.sh              # full suite
#   ./benchmarks/run.sh --rex-only   # skip Next.js
#   ./benchmarks/run.sh --next-only  # skip Rex
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
REX_BIN="${REX_BIN:-$PROJECT_ROOT/target/release/rex}"
REX_FIXTURE="${REX_FIXTURE:-$PROJECT_ROOT/fixtures/basic}"
NEXT_DIR="$SCRIPT_DIR/next-basic"
REQUESTS="${BENCH_REQUESTS:-5000}"
CONCURRENCY="${BENCH_CONCURRENCY:-50}"
WARMUP="${BENCH_WARMUP:-100}"
REX_PORT=4100
NEXT_PORT=4200

# Colors
bold() { printf "\033[1m%s\033[0m" "$1"; }
dim() { printf "\033[2m%s\033[0m" "$1"; }
green() { printf "\033[32m%s\033[0m" "$1"; }
cyan() { printf "\033[36m%s\033[0m" "$1"; }

# ── Helpers ──────────────────────────────────────────────

find_load_tester() {
    if command -v oha &>/dev/null; then
        echo "oha"
    elif command -v ab &>/dev/null; then
        echo "ab"
    else
        echo ""
    fi
}

wait_for_port() {
    local port=$1
    local timeout=${2:-30}
    local start=$(date +%s)
    while ! nc -z 127.0.0.1 "$port" 2>/dev/null; do
        if (( $(date +%s) - start > timeout )); then
            echo "ERROR: Port $port not ready after ${timeout}s"
            return 1
        fi
        sleep 0.1
    done
}

measure_startup() {
    local label=$1
    shift
    local start_ns=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
    "$@" &
    local pid=$!
    local port=${!#}  # last arg should be port
    # Actually, we need the port from context
    echo "$pid"
}

kill_tree() {
    local pid=$1
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
}

run_oha() {
    local url=$1
    local label=$2
    # Warmup
    oha -n "$WARMUP" -c 10 -q 0 --no-tui "$url" >/dev/null 2>&1 || true
    # Actual benchmark
    echo ""
    echo "  $(bold "$label")"
    oha -n "$REQUESTS" -c "$CONCURRENCY" -q 0 --no-tui "$url" 2>&1 | \
        grep -E '(Requests/sec|Slowest|Fastest|Average|Total|Success rate|Status code)' | \
        sed 's/^/    /'
}

run_ab() {
    local url=$1
    local label=$2
    # Warmup
    ab -n "$WARMUP" -c 10 "$url" >/dev/null 2>&1 || true
    # Actual benchmark
    echo ""
    echo "  $(bold "$label")"
    ab -n "$REQUESTS" -c "$CONCURRENCY" "$url" 2>&1 | \
        grep -E '(Requests per second|Time per request|Transfer rate|Failed requests)' | \
        sed 's/^/    /'
}

run_bench() {
    local url=$1
    local label=$2
    if [ "$LOAD_TESTER" = "oha" ]; then
        run_oha "$url" "$label"
    elif [ "$LOAD_TESTER" = "ab" ]; then
        run_ab "$url" "$label"
    fi
}

# ── Benchmark setup ──────────────────────────────────────

LOAD_TESTER=$(find_load_tester)
if [ -z "$LOAD_TESTER" ]; then
    echo "ERROR: No load testing tool found. Install oha (cargo install oha) or ensure ab is available."
    exit 1
fi

RUN_REX=true
RUN_NEXT=true
for arg in "$@"; do
    case "$arg" in
        --rex-only) RUN_NEXT=false ;;
        --next-only) RUN_REX=false ;;
    esac
done

echo ""
echo "  $(bold "Rex Benchmark Suite")"
echo ""
echo "  $(dim "Tool:")        $LOAD_TESTER"
echo "  $(dim "Requests:")    $REQUESTS"
echo "  $(dim "Concurrency:") $CONCURRENCY"
echo "  $(dim "Warmup:")      $WARMUP requests"
echo ""

# ── Rex benchmarks ───────────────────────────────────────

if [ "$RUN_REX" = true ]; then
    if [ ! -f "$REX_BIN" ]; then
        echo "ERROR: Rex binary not found at $REX_BIN"
        echo "Run: cargo build --release"
        exit 1
    fi

    echo "$(cyan "━━━ Rex ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")"

    # Cold start
    START_NS=$(python3 -c 'import time; print(int(time.time()*1e9))')
    "$REX_BIN" dev --root "$REX_FIXTURE" --port $REX_PORT &>/dev/null &
    REX_PID=$!
    wait_for_port $REX_PORT
    END_NS=$(python3 -c 'import time; print(int(time.time()*1e9))')
    COLD_MS=$(( (END_NS - START_NS) / 1000000 ))
    echo ""
    echo "  $(bold "Cold start:") $(green "${COLD_MS}ms")"

    # SSR page (getServerSideProps)
    run_bench "http://127.0.0.1:$REX_PORT/" "SSR index (getServerSideProps)"

    # Static page (getStaticProps)
    run_bench "http://127.0.0.1:$REX_PORT/about" "SSG about (getStaticProps)"

    # Dynamic route
    run_bench "http://127.0.0.1:$REX_PORT/blog/hello-world" "Dynamic /blog/:slug (GSSP)"

    # API route
    run_bench "http://127.0.0.1:$REX_PORT/api/hello" "API /api/hello"

    # Memory
    if command -v ps &>/dev/null; then
        RSS_KB=$(ps -o rss= -p "$REX_PID" 2>/dev/null || echo "0")
        RSS_MB=$(( RSS_KB / 1024 ))
        echo ""
        echo "  $(bold "Memory (RSS):") $(green "${RSS_MB}MB")"
    fi

    kill_tree $REX_PID
    echo ""
fi

# ── Next.js benchmarks ──────────────────────────────────

if [ "$RUN_NEXT" = true ]; then
    if [ ! -d "$NEXT_DIR/node_modules" ]; then
        echo "ERROR: Next.js not installed. Run:"
        echo "  cd benchmarks/next-basic && npm install"
        exit 1
    fi

    echo "$(cyan "━━━ Next.js ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")"

    # Cold start
    START_NS=$(python3 -c 'import time; print(int(time.time()*1e9))')
    cd "$NEXT_DIR"
    npx next dev --port $NEXT_PORT &>/dev/null &
    NEXT_PID=$!
    cd "$PROJECT_ROOT"
    wait_for_port $NEXT_PORT 60  # Next.js is slower to start
    # Next.js compiles on first request, so hit it once and wait
    curl -s "http://127.0.0.1:$NEXT_PORT/" >/dev/null 2>&1 || true
    sleep 2
    END_NS=$(python3 -c 'import time; print(int(time.time()*1e9))')
    COLD_MS=$(( (END_NS - START_NS) / 1000000 ))
    echo ""
    echo "  $(bold "Cold start:") $(green "${COLD_MS}ms")"

    # SSR page
    run_bench "http://127.0.0.1:$NEXT_PORT/" "SSR index (getServerSideProps)"

    # Static page
    run_bench "http://127.0.0.1:$NEXT_PORT/about" "SSG about (getStaticProps)"

    # Dynamic route
    run_bench "http://127.0.0.1:$NEXT_PORT/blog/hello-world" "Dynamic /blog/:slug (GSSP)"

    # API route
    run_bench "http://127.0.0.1:$NEXT_PORT/api/hello" "API /api/hello"

    # Memory
    if command -v ps &>/dev/null; then
        RSS_KB=$(ps -o rss= -p "$NEXT_PID" 2>/dev/null || echo "0")
        RSS_MB=$(( RSS_KB / 1024 ))
        echo ""
        echo "  $(bold "Memory (RSS):") $(green "${RSS_MB}MB")"
    fi

    kill_tree $NEXT_PID
    echo ""
fi

echo "$(dim "Done.")"
