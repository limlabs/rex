// Node.js `perf_hooks` module polyfill for Rex server bundles.
// Used by postgres.js for query timing.

/* eslint-disable @typescript-eslint/no-explicit-any */

export const performance = typeof globalThis.performance !== 'undefined'
    ? globalThis.performance
    : { now() { return Date.now(); } } as any;

export class PerformanceObserver {
    observe() {}
    disconnect() {}
}

const perf_hooks = { performance, PerformanceObserver };
export default perf_hooks;
