/* eslint-disable @typescript-eslint/no-explicit-any */
// ReadableStream polyfill for bare V8 (React 19 streaming SSR)
if (typeof globalThis.ReadableStream === 'undefined') {
    (globalThis as any).ReadableStream = function ReadableStream(this: any, underlyingSource: any) {
        this._queue = [] as any[];
        this._closed = false;
        this._errored = false;
        this._error = undefined as any;
        this._reader = null as any;
        this._readerResolve = null as any;
        this._pulling = false;
        this._pullAgain = false;
        const controller = {
            enqueue: (chunk: any) => {
                if (this._closed || this._errored) return;
                if (this._readerResolve) {
                    const resolve = this._readerResolve;
                    this._readerResolve = null;
                    resolve({ value: chunk, done: false });
                } else {
                    this._queue.push(chunk);
                }
            },
            close: () => {
                if (this._closed || this._errored) return;
                this._closed = true;
                if (this._readerResolve) {
                    const resolve = this._readerResolve;
                    this._readerResolve = null;
                    resolve({ value: undefined, done: true });
                }
            },
            error: (e: any) => {
                if (this._closed || this._errored) return;
                this._errored = true;
                this._error = e;
                if (this._readerResolve) {
                    const resolve = this._readerResolve;
                    this._readerResolve = null;
                    resolve(Promise.reject(e));
                }
            },
            desiredSize: 1
        };
        this._controller = controller;
        this._underlyingSource = underlyingSource || {};
        if (typeof this._underlyingSource.start === 'function') {
            this._underlyingSource.start(controller);
        }
    };
    (globalThis as any).ReadableStream.prototype._callPull = function(this: any) {
        if (this._pulling || this._closed || this._errored) return;
        if (typeof this._underlyingSource.pull !== 'function') return;
        this._pulling = true;
        try {
            const result = this._underlyingSource.pull(this._controller);
            if (result && typeof result.then === 'function') {
                result.then(() => {
                    this._pulling = false;
                    if (this._pullAgain) {
                        this._pullAgain = false;
                        this._callPull();
                    }
                }, (err: any) => {
                    this._pulling = false;
                    this._controller.error(err);
                });
            } else {
                this._pulling = false;
            }
        } catch(e) {
            this._pulling = false;
            this._controller.error(e);
        }
    };
    (globalThis as any).ReadableStream.prototype.getReader = function(this: any) {
        this._reader = true;
        return {
            read: () => {
                if (this._errored) return Promise.reject(this._error);
                if (this._queue.length > 0) {
                    const value = this._queue.shift();
                    this._callPull();
                    return Promise.resolve({ value: value, done: false });
                }
                if (this._closed) {
                    return Promise.resolve({ value: undefined, done: true });
                }
                this._callPull();
                if (this._queue.length > 0) {
                    const value = this._queue.shift();
                    return Promise.resolve({ value: value, done: false });
                }
                if (this._closed) {
                    return Promise.resolve({ value: undefined, done: true });
                }
                return new Promise((resolve) => {
                    this._readerResolve = resolve;
                });
            },
            cancel: () => {
                this._closed = true;
                this._queue = [];
                return Promise.resolve();
            },
            releaseLock: () => {
                this._reader = null;
            }
        };
    };
}
