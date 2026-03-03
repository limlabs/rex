// Webpack runtime shims for react-server-dom-webpack
//
// React's react-server-dom-webpack expects these webpack globals to be
// present. In Rex's V8 IIFE bundles, we provide minimal implementations.
//
// The server bundle populates __rex_client_modules__ from the bundler config.
// The SSR bundle uses __rex_ssr_modules__ for module resolution.
//
// NOTE: This file is injected as a banner (not processed by OXC), so it must
// be valid JavaScript.  Types come from global.d.ts declarations.

globalThis.__rex_client_modules__ = globalThis.__rex_client_modules__ || {};
globalThis.__rex_ssr_modules__ = globalThis.__rex_ssr_modules__ || {};

globalThis.__webpack_require__ = function(id) {
    return globalThis.__rex_client_modules__[id] || globalThis.__rex_ssr_modules__[id] || {};
};

// Chunk filename resolver — react-server-dom-webpack wraps this
globalThis.__webpack_require__.u = function(chunkId) {
    return chunkId;
};

globalThis.__webpack_chunk_load__ = function(_chunkId) {
    return Promise.resolve();
};
