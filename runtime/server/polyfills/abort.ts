/* eslint-disable @typescript-eslint/no-explicit-any */
// AbortController / AbortSignal / DOMException polyfills for bare V8
if (typeof globalThis.AbortController === 'undefined') {
    function AbortSignal(this: any) {
        this.aborted = false;
        this.reason = undefined;
        this._listeners = [] as any[];
    }
    AbortSignal.prototype.addEventListener = function(this: any, type: string, listener: any) {
        if (type === 'abort') this._listeners.push(listener);
    };
    AbortSignal.prototype.removeEventListener = function(this: any, type: string, listener: any) {
        if (type === 'abort') {
            this._listeners = this._listeners.filter(function(l: any) { return l !== listener; });
        }
    };
    AbortSignal.prototype.throwIfAborted = function(this: any) {
        if (this.aborted) throw this.reason;
    };

    (globalThis as any).AbortController = function AbortController(this: any) {
        this.signal = new (AbortSignal as any)();
    };
    (globalThis as any).AbortController.prototype.abort = function(this: any, reason: any) {
        if (this.signal.aborted) return;
        this.signal.aborted = true;
        this.signal.reason = reason || new (globalThis as any).DOMException('The operation was aborted.', 'AbortError');
        const listeners = this.signal._listeners.slice();
        for (let i = 0; i < listeners.length; i++) {
            try { listeners[i]({ type: 'abort', target: this.signal }); } catch { /* intentionally empty */ }
        }
    };

    if (typeof globalThis.DOMException === 'undefined') {
        (globalThis as any).DOMException = function DOMException(this: any, message: string, name: string) {
            this.message = message || '';
            this.name = name || 'Error';
        };
        (globalThis as any).DOMException.prototype = Object.create(Error.prototype);
    }
}
