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
#   ./benchmarks/run.sh                    # dev + prod, both frameworks
#   ./benchmarks/run.sh --rex-only         # skip Next.js
#   ./benchmarks/run.sh --next-only        # skip Rex
#   ./benchmarks/run.sh --dev-only         # skip production mode
#   ./benchmarks/run.sh --prod-only        # skip dev mode
#   ./benchmarks/run.sh --json results.json  # write JSON results
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
JSON_FILE=""

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

kill_tree() {
    local pid=$1
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
}

# Extract RPS from ab output
ab_rps() {
    local url=$1
    # Warmup
    ab -n "$WARMUP" -c 10 "$url" >/dev/null 2>&1 || true
    # Bench
    ab -n "$REQUESTS" -c "$CONCURRENCY" "$url" 2>&1 | \
        grep "Requests per second" | awk '{print $4}'
}

# Extract mean time per request from ab output
ab_latency() {
    local url=$1
    ab -n "$REQUESTS" -c "$CONCURRENCY" "$url" 2>&1 | \
        grep "Time per request" | head -1 | awk '{print $4}'
}

# Run ab and capture both RPS and latency, print nicely
run_ab_capture() {
    local url=$1
    local label=$2
    # Warmup
    ab -n "$WARMUP" -c 10 "$url" >/dev/null 2>&1 || true
    # Bench and capture full output
    local output
    output=$(ab -n "$REQUESTS" -c "$CONCURRENCY" "$url" 2>&1)
    local rps=$(echo "$output" | grep "Requests per second" | awk '{print $4}')
    local latency=$(echo "$output" | grep "Time per request" | head -1 | awk '{print $4}')
    local failed=$(echo "$output" | grep "Failed requests" | awk '{print $3}')

    echo ""
    echo "  $(bold "$label")"
    echo "$output" | grep -E '(Requests per second|Time per request|Transfer rate|Failed requests)' | sed 's/^/    /'

    # Return values via global vars
    _BENCH_RPS="$rps"
    _BENCH_LATENCY="$latency"
    _BENCH_FAILED="${failed:-0}"
}

get_memory_mb() {
    local pid=$1
    local rss_kb
    rss_kb=$(ps -o rss= -p "$pid" 2>/dev/null || echo "0")
    echo $(( rss_kb / 1024 ))
}

now_ms() {
    python3 -c 'import time; print(int(time.time()*1e9))'
}

# ── JSON result accumulator ──────────────────────────────

declare -a JSON_RESULTS=()

add_result() {
    local framework=$1 mode=$2 endpoint=$3 rps=$4 latency_ms=$5 cold_start_ms=$6 memory_mb=$7
    JSON_RESULTS+=("{\"framework\":\"$framework\",\"mode\":\"$mode\",\"endpoint\":\"$endpoint\",\"rps\":$rps,\"latency_ms\":$latency_ms,\"cold_start_ms\":$cold_start_ms,\"memory_mb\":$memory_mb}")
}

write_json() {
    local file=$1
    echo "[" > "$file"
    local first=true
    for entry in "${JSON_RESULTS[@]}"; do
        if [ "$first" = true ]; then
            first=false
        else
            echo "," >> "$file"
        fi
        printf "  %s" "$entry" >> "$file"
    done
    echo "" >> "$file"
    echo "]" >> "$file"
}

# ── Benchmark a single framework + mode ──────────────────

bench_rex() {
    local mode=$1  # "dev" or "prod"
    local port=$REX_PORT

    if [ ! -f "$REX_BIN" ]; then
        echo "ERROR: Rex binary not found at $REX_BIN"
        echo "Run: cargo build --release"
        exit 1
    fi

    echo "$(cyan "━━━ Rex ($mode) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")"

    if [ "$mode" = "prod" ]; then
        # Build first
        echo ""
        echo "  $(dim "Building...")"
        local build_start
        build_start=$(now_ms)
        "$REX_BIN" build --root "$REX_FIXTURE" &>/dev/null
        local build_end
        build_end=$(now_ms)
        local build_ms=$(( (build_end - build_start) / 1000000 ))
        echo "  $(bold "Build time:") $(green "${build_ms}ms")"
    fi

    # Start server
    local start_ns
    start_ns=$(now_ms)
    if [ "$mode" = "dev" ]; then
        "$REX_BIN" dev --root "$REX_FIXTURE" --port $port &>/dev/null &
    else
        "$REX_BIN" start --root "$REX_FIXTURE" --port $port &>/dev/null &
    fi
    local pid=$!
    wait_for_port $port
    # Hit once to ensure fully warmed
    curl -s "http://127.0.0.1:$port/" >/dev/null 2>&1 || true
    local end_ns
    end_ns=$(now_ms)
    local cold_ms=$(( (end_ns - start_ns) / 1000000 ))
    echo ""
    echo "  $(bold "Cold start:") $(green "${cold_ms}ms")"

    # Benchmarks
    local endpoints=("/" "/about" "/blog/hello-world" "/api/hello")
    local labels=("SSR index (GSSP)" "SSG about (GSP)" "Dynamic /blog/:slug" "API /api/hello")

    for i in "${!endpoints[@]}"; do
        run_ab_capture "http://127.0.0.1:$port${endpoints[$i]}" "${labels[$i]}"
        add_result "rex" "$mode" "${endpoints[$i]}" "${_BENCH_RPS:-0}" "${_BENCH_LATENCY:-0}" "$cold_ms" "0"
    done

    # Memory
    local mem_mb
    mem_mb=$(get_memory_mb "$pid")
    echo ""
    echo "  $(bold "Memory (RSS):") $(green "${mem_mb}MB")"

    # Update memory in last 4 results
    local len=${#JSON_RESULTS[@]}
    for i in $(seq $((len-4)) $((len-1))); do
        JSON_RESULTS[$i]=$(echo "${JSON_RESULTS[$i]}" | sed "s/\"memory_mb\":0/\"memory_mb\":$mem_mb/")
    done

    kill_tree $pid
    echo ""
}

bench_next() {
    local mode=$1  # "dev" or "prod"
    local port=$NEXT_PORT

    if [ ! -d "$NEXT_DIR/node_modules" ]; then
        echo "ERROR: Next.js not installed. Run:"
        echo "  cd benchmarks/next-basic && npm install"
        exit 1
    fi

    echo "$(cyan "━━━ Next.js ($mode) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")"

    if [ "$mode" = "prod" ]; then
        # Build first
        echo ""
        echo "  $(dim "Building...")"
        local build_start
        build_start=$(now_ms)
        cd "$NEXT_DIR"
        npx next build &>/dev/null
        cd "$PROJECT_ROOT"
        local build_end
        build_end=$(now_ms)
        local build_ms=$(( (build_end - build_start) / 1000000 ))
        echo "  $(bold "Build time:") $(green "${build_ms}ms")"
    fi

    # Start server
    local start_ns
    start_ns=$(now_ms)
    cd "$NEXT_DIR"
    if [ "$mode" = "dev" ]; then
        npx next dev --port $port &>/dev/null &
    else
        npx next start --port $port &>/dev/null &
    fi
    local pid=$!
    cd "$PROJECT_ROOT"
    wait_for_port $port 60
    # Next.js compiles on first request in dev mode
    curl -s "http://127.0.0.1:$port/" >/dev/null 2>&1 || true
    if [ "$mode" = "dev" ]; then
        sleep 2  # wait for dev compilation
    fi
    local end_ns
    end_ns=$(now_ms)
    local cold_ms=$(( (end_ns - start_ns) / 1000000 ))
    echo ""
    echo "  $(bold "Cold start:") $(green "${cold_ms}ms")"

    # Benchmarks
    local endpoints=("/" "/about" "/blog/hello-world" "/api/hello")
    local labels=("SSR index (GSSP)" "SSG about (GSP)" "Dynamic /blog/:slug" "API /api/hello")

    for i in "${!endpoints[@]}"; do
        run_ab_capture "http://127.0.0.1:$port${endpoints[$i]}" "${labels[$i]}"
        add_result "nextjs" "$mode" "${endpoints[$i]}" "${_BENCH_RPS:-0}" "${_BENCH_LATENCY:-0}" "$cold_ms" "0"
    done

    # Memory
    local mem_mb
    mem_mb=$(get_memory_mb "$pid")
    echo ""
    echo "  $(bold "Memory (RSS):") $(green "${mem_mb}MB")"

    # Update memory in last 4 results
    local len=${#JSON_RESULTS[@]}
    for i in $(seq $((len-4)) $((len-1))); do
        JSON_RESULTS[$i]=$(echo "${JSON_RESULTS[$i]}" | sed "s/\"memory_mb\":0/\"memory_mb\":$mem_mb/")
    done

    kill_tree $pid
    echo ""
}

# ── Parse args ───────────────────────────────────────────

RUN_REX=true
RUN_NEXT=true
RUN_DEV=true
RUN_PROD=true

while [[ $# -gt 0 ]]; do
    case "$1" in
        --rex-only)  RUN_NEXT=false ;;
        --next-only) RUN_REX=false ;;
        --dev-only)  RUN_PROD=false ;;
        --prod-only) RUN_DEV=false ;;
        --json)      JSON_FILE="$2"; shift ;;
    esac
    shift
done

LOAD_TESTER=$(find_load_tester)
if [ -z "$LOAD_TESTER" ]; then
    echo "ERROR: No load testing tool found. Install oha (cargo install oha) or ensure ab is available."
    exit 1
fi

echo ""
echo "  $(bold "Rex Benchmark Suite")"
echo ""
echo "  $(dim "Tool:")        $LOAD_TESTER"
echo "  $(dim "Requests:")    $REQUESTS"
echo "  $(dim "Concurrency:") $CONCURRENCY"
echo "  $(dim "Warmup:")      $WARMUP requests"
echo ""

# ── Run benchmarks ───────────────────────────────────────

if [ "$RUN_DEV" = true ]; then
    [ "$RUN_REX" = true ]  && bench_rex "dev"
    [ "$RUN_NEXT" = true ] && bench_next "dev"
fi

if [ "$RUN_PROD" = true ]; then
    [ "$RUN_REX" = true ]  && bench_rex "prod"
    [ "$RUN_NEXT" = true ] && bench_next "prod"
fi

# ── Write JSON ───────────────────────────────────────────

if [ -n "$JSON_FILE" ]; then
    write_json "$JSON_FILE"
    echo "  $(dim "Results written to $JSON_FILE")"
fi

echo "$(dim "Done.")"
