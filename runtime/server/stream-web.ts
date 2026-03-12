// Node.js `stream/web` module polyfill for Rex server bundles.
// Re-exports Web Streams API from globalThis (provided by V8 polyfills).

/* eslint-disable @typescript-eslint/no-explicit-any */

const _g = globalThis as any;

export const ReadableStream = _g.ReadableStream;
export const WritableStream = _g.WritableStream;
export const TransformStream = _g.TransformStream;

export default { ReadableStream, WritableStream, TransformStream };
