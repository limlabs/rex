/* eslint-disable @typescript-eslint/no-explicit-any */
// MessageChannel polyfill for bare V8 (React 19 scheduler needs this)
if (typeof globalThis.MessageChannel === 'undefined') {
    (globalThis as any).MessageChannel = function(this: any) {
        let cb: any = null;
        this.port1 = {};
        this.port2 = { postMessage: function() { if (cb) cb({ data: undefined }); } };
        Object.defineProperty(this.port1, 'onmessage', {
            set: function(fn: any) { cb = fn; }, get: function() { return cb; }
        });
    };
}
