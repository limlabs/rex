// Node.js `crypto` module polyfill for Rex server bundles.
// Re-exports the crypto global that is installed by V8 polyfills banner.
//
// The actual crypto implementation lives in the V8_POLYFILLS banner
// and is set on globalThis before any bundled code runs. This module
// exists so that `import crypto from 'crypto'` resolves correctly
// via rolldown resolve aliases.

/* eslint-disable no-shadow-restricted-names -- augmenting globalThis for V8 runtime bindings */
declare const globalThis: {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    crypto: any;
};
/* eslint-enable no-shadow-restricted-names */

export const randomUUID = globalThis.crypto.randomUUID.bind(globalThis.crypto);
export const randomBytes = globalThis.crypto.randomBytes;
export const createHash = globalThis.crypto.createHash;

export default globalThis.crypto;
