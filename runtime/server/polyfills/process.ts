/* eslint-disable @typescript-eslint/no-explicit-any */
// process polyfill for bare V8
// Note: process.env is overwritten by Rust's build_process_env_script() at isolate
// creation time, so the env object here is just a fallback.
if (typeof (globalThis as any).process === 'undefined') {
    (globalThis as any).process = { env: { NODE_ENV: 'production' } };
}
(function() {
    const p = (globalThis as any).process;

    // nextTick — map to queueMicrotask (semantically equivalent for userland code)
    if (typeof p.nextTick !== 'function') {
        p.nextTick = function(fn: (...args: any[]) => void, ...args: any[]) {
            if (typeof queueMicrotask === 'function') {
                queueMicrotask(() => fn(...args));
            } else {
                Promise.resolve().then(() => fn(...args));
            }
        };
    }

    // versions — intentionally omit `node` key so libraries that check
    // `process.versions.node` know they're NOT running on Node.js.
    // pg uses this to decide between legacy crypto and WebCrypto paths.
    if (!p.versions) {
        p.versions = {};
    }

    // platform — used by pg for connection parameter defaults
    if (!p.platform) {
        p.platform = 'linux';
    }

    // cwd — return project root if available
    if (typeof p.cwd !== 'function') {
        p.cwd = function() {
            return (globalThis as any).__rex_project_root || '/';
        };
    }

    // stdout/stderr — used by some packages for logging
    if (!p.stdout) {
        p.stdout = {
            write(data: any) {
                if (typeof console !== 'undefined') console.log(String(data).replace(/\n$/, ''));
                return true;
            },
            isTTY: false,
        };
    }
    if (!p.stderr) {
        p.stderr = {
            write(data: any) {
                if (typeof console !== 'undefined') console.error(String(data).replace(/\n$/, ''));
                return true;
            },
            isTTY: false,
        };
    }

    // argv — empty, but some packages check for it
    if (!p.argv) {
        p.argv = [];
    }

    // pid
    if (!p.pid) {
        p.pid = 1;
    }

    // hrtime — high-resolution time (used by some profiling/timing code)
    if (typeof p.hrtime !== 'function') {
        p.hrtime = function(prev?: [number, number]): [number, number] {
            const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
            const s = Math.floor(now / 1000);
            const ns = Math.round((now % 1000) * 1e6);
            if (prev) {
                let ds = s - prev[0];
                let dns = ns - prev[1];
                if (dns < 0) { ds--; dns += 1e9; }
                return [ds, dns];
            }
            return [s, ns];
        };
        p.hrtime.bigint = function(): bigint {
            const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
            return BigInt(Math.round(now * 1e6));
        };
    }

    // exit — stub that logs but doesn't actually exit (V8 isolate)
    if (typeof p.exit !== 'function') {
        p.exit = function(code?: number) {
            if (typeof console !== 'undefined') {
                console.warn('process.exit(' + (code || 0) + ') called in V8 isolate (no-op)');
            }
        };
    }

    // on/once/removeListener — no-op event emitter stubs
    if (typeof p.on !== 'function') {
        p.on = function() { return p; };
        p.once = function() { return p; };
        p.off = function() { return p; };
        p.removeListener = function() { return p; };
        p.addListener = function() { return p; };
        p.emit = function() { return false; };
        p.listeners = function() { return []; };
        p.removeAllListeners = function() { return p; };
    }
})();
