// Rex RSC Client Hydration Runtime
//
// Uses react-server-dom-webpack/client to parse React's flight format
// and hydrate the server-rendered HTML with interactive client components.
//
// This file is bundled by rolldown as an ESM entry alongside client component
// chunks. It replaces the old custom rsc-runtime.js parser.
//
// Module loading strategy:
//   The flight I rows use ref_id hashes as module identifiers with empty chunks
//   (so SSR can resolve synchronously). On the client, we pre-load all client
//   modules using window.__REX_RSC_MODULE_MAP__ which maps ref_id → chunk_url.
//   After all modules are loaded into the webpack cache, we hydrate.

import { createFromReadableStream, createFromFetch } from 'react-server-dom-webpack/client';
import React from 'react';
import ReactDOM from 'react-dom/client';

// --- Client-side webpack shims ---
// The actual __webpack_require__ and __webpack_chunk_load__ are set up by an
// inline <script> in the HTML (before this module loads) because
// react-server-dom-webpack/client accesses __webpack_require__ during CJS
// factory initialization, before our module-level code can run.
//
// We just reference the shared module cache for pre-loading:
var moduleCache = window.__rexModuleCache || {};

// --- Pre-load client modules ---
// Load all client component modules into the webpack cache before hydrating.
// The module map maps ref_id → { chunk_url, export_name }.
// After loading, __webpack_require__(ref_id) returns the module.

function preloadClientModules() {
  var rawMap = window.__REX_RSC_MODULE_MAP__ || {};
  var entries = rawMap.entries || rawMap;
  var promises = [];

  for (var refId in entries) {
    if (!Object.prototype.hasOwnProperty.call(entries, refId)) continue;
    var entry = entries[refId];
    // Use an IIFE to capture refId in closure
    (function(id, url) {
      promises.push(
        import(url).then(function(mod) {
          moduleCache[id] = mod;
        })
      );
    })(refId, entry.chunk_url);
  }

  return promises.length > 0 ? Promise.all(promises) : Promise.resolve();
}

// --- Build bundler config ---
// Transform __REX_RSC_MODULE_MAP__ into the webpack consumer manifest format
// that React's flight decoder expects: { [refId]: { [exportName]: { id, chunks, name } } }

function buildSSRManifest() {
  var rawMap = window.__REX_RSC_MODULE_MAP__ || {};
  var entries = rawMap.entries || rawMap;
  var moduleMap = {};

  for (var refId in entries) {
    if (!Object.prototype.hasOwnProperty.call(entries, refId)) continue;
    var entry = entries[refId];
    var exportMap = {};
    exportMap[entry.export_name] = {
      id: refId,
      chunks: [],
      name: entry.export_name
    };
    // Wildcard fallback
    exportMap['*'] = {
      id: refId,
      chunks: [],
      name: ''
    };
    moduleMap[refId] = exportMap;
  }

  return moduleMap;
}

// --- Hydration ---

var rscRoot = null;

function stringToReadableStream(str) {
  var encoder = new TextEncoder();
  return new ReadableStream({
    start: function(controller) {
      controller.enqueue(encoder.encode(str));
      controller.close();
    }
  });
}

function hydrateFromInlineData() {
  var dataEl = document.getElementById('__REX_RSC_DATA__');
  if (!dataEl) return;

  var flightData = dataEl.textContent;
  if (!flightData) return;

  // Build the SSR manifest for React's flight decoder
  var ssrManifest = buildSSRManifest();

  // Pre-load all client modules, then hydrate
  preloadClientModules().then(function() {
    var stream = stringToReadableStream(flightData);
    // Wrap in Promise.resolve() because React's createFromReadableStream returns
    // a custom thenable whose .then() doesn't return a chainable Promise
    var treePromise = Promise.resolve(
      createFromReadableStream(stream, {
        ssrManifest: {
          moduleMap: ssrManifest,
          moduleLoading: null
        }
      })
    );

    treePromise.then(function(tree) {
      try {
        // Hydrate the entire document since the RSC tree includes <html> and <body>
        rscRoot = ReactDOM.hydrateRoot(document, tree);
      } catch(e) {
        console.error('[Rex RSC] Hydration failed, falling back to render:', e);
        rscRoot = ReactDOM.createRoot(document.body);
        rscRoot.render(tree);
      }
    }).catch(function(e) {
      console.error('[Rex RSC] Failed to process flight data:', e);
    });
  }).catch(function(e) {
    console.error('[Rex RSC] Module preload failed:', e);
  });
}

// --- Client-side navigation ---

function navigateRsc(pathname) {
  var manifest = window.__REX_MANIFEST__;
  if (!manifest) return Promise.reject(new Error('No manifest'));

  var buildId = manifest.build_id;
  var url = '/_rex/rsc/' + buildId + pathname;
  var ssrManifest = buildSSRManifest();

  var treePromise = Promise.resolve(
    createFromFetch(fetch(url), {
      ssrManifest: { moduleMap: ssrManifest, moduleLoading: null }
    })
  );

  return treePromise.then(function(tree) {
    if (rscRoot) {
      React.startTransition(function() {
        rscRoot.render(
          React.createElement(React.Suspense, { fallback: null }, tree)
        );
      });
    }
  });
}

// --- Public API ---

window.__REX_RSC_INIT = hydrateFromInlineData;
window.__REX_RSC_NAVIGATE = navigateRsc;

// Auto-hydrate on DOMContentLoaded if flight data is present
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', function() {
    if (document.getElementById('__REX_RSC_DATA__')) {
      hydrateFromInlineData();
    }
  });
} else {
  if (document.getElementById('__REX_RSC_DATA__')) {
    hydrateFromInlineData();
  }
}
