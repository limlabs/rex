// Node.js `buffer` module polyfill for Rex server bundles.
// Re-exports the Buffer global that is installed by V8 polyfills banner.
//
// The actual Buffer implementation lives in the V8_POLYFILLS banner
// (crates/rex_build/src/bundler.rs) and is set on globalThis before
// any bundled code runs.  This module exists so that
// `import { Buffer } from 'buffer'` resolves correctly via rolldown
// resolve aliases.

/* eslint-disable no-shadow-restricted-names -- augmenting globalThis for V8 runtime bindings */
declare const globalThis: {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    Buffer: any;
};
/* eslint-enable no-shadow-restricted-names */

// Re-export the global Buffer constructor as-is.
// We use `any` to avoid duplicating the full Node.js Buffer type surface.
export const Buffer = globalThis.Buffer;

export default { Buffer };
