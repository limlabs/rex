/* eslint-disable @typescript-eslint/no-explicit-any */
// performance polyfill for bare V8
if (typeof globalThis.performance === 'undefined') {
    const _timeOrigin = Date.now();
    (globalThis as any).performance = {
        now: function() { return Date.now() - _timeOrigin; },
        timeOrigin: _timeOrigin,
    };
}
