/* eslint-disable @typescript-eslint/no-explicit-any */
// setTimeout, clearTimeout, queueMicrotask stubs for bare V8.
//
// We use an iterative queue instead of calling fn() inline to avoid
// infinite recursion when React's internals schedule work from within
// a setTimeout/queueMicrotask callback.
(function() {
    var queue: Array<() => void> = [];
    var draining = false;

    function drain(): void {
        if (draining) return;
        draining = true;
        while (queue.length > 0) {
            var fn = queue.shift()!;
            fn();
        }
        draining = false;
    }

    if (typeof globalThis.setTimeout === 'undefined') {
        (globalThis as any).setTimeout = function(fn: () => void) {
            queue.push(fn);
            drain();
            return 0;
        };
        (globalThis as any).clearTimeout = function() {};
    }
    if (typeof globalThis.queueMicrotask === 'undefined') {
        (globalThis as any).queueMicrotask = function(fn: () => void) {
            queue.push(fn);
            drain();
        };
    }
})();
