// Rex RSC Client Hydration Runtime
//
// Uses react-server-dom-webpack/client to parse React's flight format
// and hydrate the server-rendered HTML with interactive client components.
//
// This file is bundled by rolldown as an ESM entry alongside client component
// chunks. It replaces the old custom rsc-runtime.ts parser.
//
// Module loading strategy:
//   The flight I rows use ref_id hashes as module identifiers with empty chunks
//   (so SSR can resolve synchronously). On the client, we pre-load all client
//   modules using window.__REX_RSC_MODULE_MAP__ which maps ref_id → chunk_url.
//   After all modules are loaded into the webpack cache, we hydrate.

import { createFromReadableStream, createFromFetch } from 'react-server-dom-webpack/client';
import React from 'react';
import ReactDOM from 'react-dom/client';

// --- callServer: server action RPC ---
// Must be defined before module loading since stubs reference it at module scope.
function callServer(id: string, args: unknown[]): Promise<unknown> {
  const manifest = window.__REX_MANIFEST__;
  const buildId = manifest ? manifest.build_id : '';
  return fetch('/_rex/action/' + buildId + '/' + id, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(args),
  }).then(function(r: Response) {
    return r.json();
  }).then(function(d: { result?: unknown; error?: string }) {
    if (d.error) throw new Error(d.error);
    return d.result;
  });
}

(window as any).__REX_CALL_SERVER = callServer;

// --- Client-side webpack shims ---
// The actual __webpack_require__ and __webpack_chunk_load__ are set up by an
// inline <script> in the HTML (before this module loads) because
// react-server-dom-webpack/client accesses __webpack_require__ during CJS
// factory initialization, before our module-level code can run.
//
// We just reference the shared module cache for pre-loading:
const moduleCache: Record<string, unknown> = window.__rexModuleCache || {};

// --- Pre-load client modules ---
// Load all client component modules into the webpack cache before hydrating.
// The module map maps ref_id → { chunk_url, export_name }.
// After loading, __webpack_require__(ref_id) returns the module.

function preloadClientModules(): Promise<void | unknown[]> {
  const rawMap = window.__REX_RSC_MODULE_MAP__ || {};
  const entries = (rawMap.entries || rawMap) as Record<string, RexRscModuleMapEntry>;
  const promises: Promise<void>[] = [];

  for (const refId in entries) {
    if (!Object.prototype.hasOwnProperty.call(entries, refId)) continue;
    const entry = entries[refId];
    // Use an IIFE to capture refId in closure
    (function(id: string, url: string) {
      promises.push(
        import(url).then(function(mod: Record<string, unknown>) {
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

function buildSSRManifest(): Record<string, Record<string, { id: string; chunks: string[]; name: string }>> {
  const rawMap = window.__REX_RSC_MODULE_MAP__ || {};
  const entries = (rawMap.entries || rawMap) as Record<string, RexRscModuleMapEntry>;
  const moduleMap: Record<string, Record<string, { id: string; chunks: string[]; name: string }>> = {};

  for (const refId in entries) {
    if (!Object.prototype.hasOwnProperty.call(entries, refId)) continue;
    const entry = entries[refId];
    const exportMap: Record<string, { id: string; chunks: string[]; name: string }> = {};
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

let rscRoot: ReturnType<typeof ReactDOM.hydrateRoot> | ReturnType<typeof ReactDOM.createRoot> | null = null;

function stringToReadableStream(str: string): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  return new ReadableStream({
    start: function(controller: ReadableStreamDefaultController<Uint8Array>) {
      controller.enqueue(encoder.encode(str));
      controller.close();
    }
  });
}

function hydrateFromInlineData(): void {
  const dataEl = document.getElementById('__REX_RSC_DATA__');
  if (!dataEl) return;

  const flightData = dataEl.textContent;
  if (!flightData) return;

  // Build the SSR manifest for React's flight decoder
  const ssrManifest = buildSSRManifest();

  // Pre-load all client modules, then hydrate
  preloadClientModules().then(function() {
    const stream = stringToReadableStream(flightData);
    // Wrap in Promise.resolve() because React's createFromReadableStream returns
    // a custom thenable whose .then() doesn't return a chainable Promise
    const treePromise = Promise.resolve(
      createFromReadableStream(stream, {
        ssrManifest: {
          moduleMap: ssrManifest,
          moduleLoading: null
        },
        callServer: callServer
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
    }).catch(function(e: unknown) {
      console.error('[Rex RSC] Failed to process flight data:', e);
    });
  }).catch(function(e: unknown) {
    console.error('[Rex RSC] Module preload failed:', e);
  });
}

// --- Client-side navigation ---

function navigateRsc(pathname: string): Promise<void> {
  const manifest = window.__REX_MANIFEST__;
  if (!manifest) return Promise.reject(new Error('No manifest'));

  const buildId = manifest.build_id;
  const url = '/_rex/rsc/' + buildId + pathname;
  const ssrManifest = buildSSRManifest();

  const treePromise = Promise.resolve(
    createFromFetch(fetch(url), {
      ssrManifest: { moduleMap: ssrManifest, moduleLoading: null },
      callServer: callServer
    })
  );

  return treePromise.then(function(tree) {
    if (rscRoot) {
      React.startTransition(function() {
        rscRoot!.render(
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
