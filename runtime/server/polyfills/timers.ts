/* eslint-disable @typescript-eslint/no-explicit-any */
// setTimeout, clearTimeout, queueMicrotask stubs for bare V8
if (typeof globalThis.setTimeout === 'undefined') {
    (globalThis as any).setTimeout = function(fn: () => void) { fn(); return 0; };
    (globalThis as any).clearTimeout = function() {};
}
if (typeof globalThis.queueMicrotask === 'undefined') {
    (globalThis as any).queueMicrotask = function(fn: () => void) { fn(); };
}
