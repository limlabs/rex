/* eslint-disable @typescript-eslint/no-explicit-any */
// process.env stub for bare V8
if (typeof (globalThis as any).process === 'undefined') {
    (globalThis as any).process = { env: { NODE_ENV: 'production' } };
}
