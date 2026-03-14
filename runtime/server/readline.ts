// Node.js `readline` stub for Rex server bundles.
// V8 has no stdin/stdout for interactive line editing.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function createInterface(_opts?: any): any {
    return {
        on() { return this; },
        once() { return this; },
        close() {},
        question(_query: string, cb: any) { if (cb) cb(''); },
        prompt() {},
        write() {},
        [Symbol.asyncIterator]() { return { next: async () => ({ done: true, value: undefined }) }; },
    };
}

export function clearLine(_stream: any, _dir: number, _cb?: any): boolean {
    if (_cb) _cb();
    return true;
}

export function cursorTo(_stream: any, _x: number, _y?: any, _cb?: any): boolean {
    const cb = typeof _y === 'function' ? _y : _cb;
    if (cb) cb();
    return true;
}

export function moveCursor(_stream: any, _dx: number, _dy: number, _cb?: any): boolean {
    if (_cb) _cb();
    return true;
}

const readline = { createInterface, clearLine, cursorTo, moveCursor };
export default readline;
