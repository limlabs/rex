/* eslint-disable @typescript-eslint/no-explicit-any */
// ReadableStream polyfill for bare V8 (React 19 streaming SSR)
//
// Supports both default streams and byte streams (type: "bytes") with
// highWaterMark strategy — required by React's renderToReadableStream which
// creates: new ReadableStream({ type: "bytes", pull, cancel }, { highWaterMark: 0 })
if (typeof globalThis.ReadableStream === 'undefined') {
    (globalThis as any).ReadableStream = function ReadableStream(
        this: any,
        underlyingSource?: any,
        strategy?: any,
    ) {
        const source = underlyingSource || {};
        const isByteStream = source.type === 'bytes';
        const hwm = (strategy && typeof strategy.highWaterMark === 'number')
            ? strategy.highWaterMark
            : (isByteStream ? 0 : 1);

        this._queue = [] as any[];
        this._queueSize = 0;
        this._highWaterMark = hwm;
        this._isByteStream = isByteStream;
        this._closed = false;
        this._errored = false;
        this._error = undefined as any;
        this._reader = null as any;
        this._readerResolve = null as any;
        this._pulling = false;
        this._pullAgain = false;

        const self = this; // eslint-disable-line no-this-alias
        const controller: any = {
            enqueue: function(chunk: any) {
                if (self._closed || self._errored) return;
                if (self._readerResolve) {
                    const resolve = self._readerResolve;
                    self._readerResolve = null;
                    resolve({ value: chunk, done: false });
                } else {
                    self._queue.push(chunk);
                    if (isByteStream && chunk && typeof chunk.byteLength === 'number') {
                        self._queueSize += chunk.byteLength;
                    } else {
                        self._queueSize += 1;
                    }
                }
            },
            close: function() {
                if (self._closed || self._errored) return;
                self._closed = true;
                if (self._readerResolve) {
                    const resolve = self._readerResolve;
                    self._readerResolve = null;
                    resolve({ value: undefined, done: true });
                }
            },
            error: function(e: any) {
                if (self._closed || self._errored) return;
                self._errored = true;
                self._error = e;
                if (self._readerResolve) {
                    const resolve = self._readerResolve;
                    self._readerResolve = null;
                    resolve(Promise.reject(e));
                }
            },
            get desiredSize(): number | null {
                if (self._errored) return null;
                if (self._closed) return 0;
                return self._highWaterMark - self._queueSize;
            },
            byobRequest: null,
        };
        this._controller = controller;
        this._underlyingSource = source;

        if (typeof source.start === 'function') {
            const startResult = source.start(controller);
            if (startResult && typeof startResult.then === 'function') {
                startResult.then(function() {
                    // start resolved — pull if needed
                }, function(err: any) {
                    controller.error(err);
                });
            }
        }
    };

    (globalThis as any).ReadableStream.prototype._callPull = function(this: any) {
        if (this._pulling || this._closed || this._errored) return;
        if (typeof this._underlyingSource.pull !== 'function') return;
        this._pulling = true;
        const self = this; // eslint-disable-line no-this-alias
        try {
            const result = this._underlyingSource.pull(this._controller);
            if (result && typeof result.then === 'function') {
                result.then(function() {
                    self._pulling = false;
                    if (self._pullAgain) {
                        self._pullAgain = false;
                        self._callPull();
                    }
                    // After pull completes, deliver queued data to pending reader
                    if (self._readerResolve && self._queue.length > 0) {
                        const resolve = self._readerResolve;
                        self._readerResolve = null;
                        const val = self._queue.shift();
                        if (self._isByteStream && val && typeof val.byteLength === 'number') {
                            self._queueSize -= val.byteLength;
                        } else {
                            self._queueSize = Math.max(0, self._queueSize - 1);
                        }
                        resolve({ value: val, done: false });
                    }
                    // If stream closed during async pull, resolve pending reader
                    if (self._readerResolve && self._closed) {
                        const resolve = self._readerResolve;
                        self._readerResolve = null;
                        resolve({ value: undefined, done: true });
                    }
                }, function(err: any) {
                    self._pulling = false;
                    self._controller.error(err);
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
        const self = this; // eslint-disable-line no-this-alias
        return {
            read: function() {
                if (self._errored) return Promise.reject(self._error);
                if (self._queue.length > 0) {
                    const value = self._queue.shift();
                    // Decrease queueSize for byte streams
                    if (self._isByteStream && value && typeof value.byteLength === 'number') {
                        self._queueSize -= value.byteLength;
                    } else {
                        self._queueSize = Math.max(0, self._queueSize - 1);
                    }
                    self._callPull();
                    return Promise.resolve({ value: value, done: false });
                }
                if (self._closed) {
                    return Promise.resolve({ value: undefined, done: true });
                }
                // Call pull to generate data
                self._callPull();
                // Check if pull synchronously enqueued or closed
                if (self._queue.length > 0) {
                    const val = self._queue.shift();
                    if (self._isByteStream && val && typeof val.byteLength === 'number') {
                        self._queueSize -= val.byteLength;
                    } else {
                        self._queueSize = Math.max(0, self._queueSize - 1);
                    }
                    return Promise.resolve({ value: val, done: false });
                }
                if (self._closed) {
                    return Promise.resolve({ value: undefined, done: true });
                }
                // Signal that we need another pull when the current one finishes.
                // Without this, the stream stalls: pull completes but nobody
                // re-invokes pull to deliver the data the reader is waiting for.
                if (self._pulling) {
                    self._pullAgain = true;
                }
                return new Promise(function(resolve) {
                    self._readerResolve = resolve;
                });
            },
            cancel: function() {
                self._closed = true;
                self._queue = [];
                self._queueSize = 0;
                if (typeof self._underlyingSource.cancel === 'function') {
                    self._underlyingSource.cancel();
                }
                return Promise.resolve();
            },
            releaseLock: function() {
                self._reader = null;
            },
        };
    };
}
