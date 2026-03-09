// Node.js `process` module polyfill for Rex server bundles.
// Re-exports the globalThis.process object set up by the V8 polyfill banner.

/* eslint-disable @typescript-eslint/no-explicit-any */

const _g = globalThis as any;
const proc = _g.process || {};

export const env = proc.env || {};
export const argv = proc.argv || [];
export const platform = proc.platform || 'linux';
export const versions = proc.versions || {};
export const pid = proc.pid || 1;
export const stdout = proc.stdout || { write() { return true; }, isTTY: false };
export const stderr = proc.stderr || { write() { return true; }, isTTY: false };

export function nextTick(fn: (...args: any[]) => void, ...args: any[]): void {
    if (proc.nextTick) proc.nextTick(fn, ...args);
    else if (typeof queueMicrotask === 'function') queueMicrotask(() => fn(...args));
    else Promise.resolve().then(() => fn(...args));
}

export function cwd(): string {
    return proc.cwd ? proc.cwd() : '/';
}

export function hrtime(prev?: [number, number]): [number, number] {
    if (proc.hrtime) return proc.hrtime(prev);
    const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
    const s = Math.floor(now / 1e3);
    const ns = Math.round(now % 1e3 * 1e6);
    if (prev) {
        let ds = s - prev[0];
        let dns = ns - prev[1];
        if (dns < 0) { ds--; dns += 1e9; }
        return [ds, dns];
    }
    return [s, ns];
}

export function exit(_code?: number): never {
    throw new Error('process.exit is not supported in Rex V8 runtime');
}

const process = {
    env, argv, platform, versions, pid, stdout, stderr,
    nextTick, cwd, hrtime, exit,
};
export default process;
