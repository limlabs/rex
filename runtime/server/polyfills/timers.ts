/* eslint-disable @typescript-eslint/no-explicit-any */
// Timer polyfills for bare V8
//
// setTimeout/clearTimeout implement a proper timer queue so that libraries
// like pg-pool (which rely on idle timeouts) work correctly. The IO loop
// calls `globalThis.__rex_drain_timers()` on each iteration to fire timers
// whose delay has elapsed.

(function() {
    const g = globalThis as any;

    interface TimerEntry {
        id: number;
        fn: (...args: any[]) => void;
        args: any[];
        runAt: number;
    }

    let nextId = 1;
    const queue: TimerEntry[] = [];

    if (typeof g.setTimeout === 'undefined') {
        g.setTimeout = function(fn: (...args: any[]) => void, delay?: number, ...args: any[]): number {
            const id = nextId++;
            const ms = typeof delay === 'number' && delay > 0 ? delay : 0;

            if (ms === 0) {
                queueMicrotask(() => fn(...args));
                return id;
            }

            queue.push({ id, fn, args, runAt: Date.now() + ms });
            return id;
        };

        g.clearTimeout = function(id: number) {
            const idx = queue.findIndex(t => t.id === id);
            if (idx !== -1) {
                queue.splice(idx, 1);
            }
        };
    }

    if (typeof g.setInterval === 'undefined') {
        g.setInterval = function() { return nextId++; };
        g.clearInterval = function() {};
    }

    if (typeof g.setImmediate === 'undefined') {
        g.setImmediate = function(fn: (...args: any[]) => void, ...args: any[]): number {
            const id = nextId++;
            queueMicrotask(() => fn(...args));
            return id;
        };
        g.clearImmediate = function() {};
    }

    if (typeof g.queueMicrotask === 'undefined') {
        g.queueMicrotask = function(fn: () => void) { fn(); };
    }

    // Called by Rust IO loop — fires expired setTimeout callbacks.
    g.__rex_drain_timers = function(): boolean {
        const now = Date.now();
        let fired = false;
        for (let i = queue.length - 1; i >= 0; i--) {
            if (queue[i].runAt <= now) {
                const timer = queue.splice(i, 1)[0];
                timer.fn(...timer.args);
                fired = true;
            }
        }
        return fired;
    };
})();
