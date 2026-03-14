// Node.js `tty` polyfill for Rex server bundles.
// V8 has no TTY — always report non-interactive.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function isatty(_fd?: number): boolean {
    return false;
}

export class ReadStream {
    isTTY = false;
    isRaw = false;
    setRawMode(_mode: boolean): this { return this; }
}

export class WriteStream {
    isTTY = false;
    columns = 80;
    rows = 24;
    getColorDepth(): number { return 1; }
    hasColors(_count?: number): boolean { return false; }
}

const tty = { isatty, ReadStream, WriteStream };
export default tty;
