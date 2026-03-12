/* eslint-disable @typescript-eslint/no-explicit-any */
// navigator polyfill — identify as a Node.js-compatible runtime so packages
// like pg use the standard net.Socket path (which Rex polyfills with push-based IO).
if (typeof (globalThis as any).navigator === 'undefined') {
    (globalThis as any).navigator = { userAgent: 'Rex' };
}
