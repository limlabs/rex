/* eslint-disable @typescript-eslint/no-explicit-any */
// performance.now() stub for bare V8
if (typeof globalThis.performance === 'undefined') {
    (globalThis as any).performance = { now: function() { return Date.now(); } };
}
