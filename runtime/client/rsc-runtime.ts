// Rex RSC Client Runtime
//
// Parses flight data (from inline <script> or fetch response),
// resolves client component references via the module map,
// and renders/hydrates the React tree.
//
// Flight format (newline-delimited):
//   J:<id>:<json>  — JSON model node
//   M:<id>:<json>  — Client module reference { id, name }
//   E:<id>:<json>  — Error { message, stack }
//   R:<id>         — Root marker (points to root node)

(function () {
  'use strict';

  var React = window.React || (typeof require === 'function' && require('react'));
  var ReactDOM = window.ReactDOM || (typeof require === 'function' && require('react-dom/client'));

  if (!React || !ReactDOM) {
    console.warn('[Rex RSC] React/ReactDOM not found, RSC runtime disabled');
    return;
  }

  // Module map: { entries: { refId -> { chunk_url, export_name } } }
  var rawMap = window.__REX_RSC_MODULE_MAP__ || {};
  var moduleMap = rawMap.entries || rawMap;

  // Cache of loaded client component modules
  var moduleCache = {};

  // --- Flight Data Parser ---

  function parseFlightData(flightString) {
    var rows = flightString.split('\n');
    var models = {};   // id -> parsed JSON
    var modules = {};  // id -> { id, name }
    var errors = {};   // id -> { message, stack }
    var rootId = null;

    for (var i = 0; i < rows.length; i++) {
      var row = rows[i].trim();
      if (!row) continue;

      var colonIdx = row.indexOf(':');
      if (colonIdx === -1) continue;

      var type = row.substring(0, colonIdx);
      var rest = row.substring(colonIdx + 1);

      if (type === 'R') {
        rootId = rest;
        continue;
      }

      var secondColon = rest.indexOf(':');
      if (secondColon === -1) continue;

      var id = rest.substring(0, secondColon);
      var payload = rest.substring(secondColon + 1);

      try {
        var parsed = JSON.parse(payload);
        if (type === 'J') {
          models[id] = parsed;
        } else if (type === 'M') {
          modules[id] = parsed;
        } else if (type === 'E') {
          errors[id] = parsed;
        }
      } catch (e) {
        console.warn('[Rex RSC] Failed to parse row:', row, e);
      }
    }

    return { models: models, modules: modules, errors: errors, rootId: rootId };
  }

  // --- Reference Resolution ---

  // Resolve a value from parsed flight data into a React element tree.
  // Client module references become lazy-loaded components.
  function resolveValue(value, flight) {
    if (value === null || value === undefined) return value;

    // String reference: "$<id>" or "$M<id>" or "$E<id>"
    if (typeof value === 'string') {
      if (value.length > 1 && value[0] === '$') {
        if (value[1] === 'M') {
          // Client module reference
          var modId = value.substring(2);
          return resolveModuleReference(modId, flight);
        }
        if (value[1] === 'E') {
          // Error reference
          var errId = value.substring(2);
          var err = flight.errors[errId];
          throw new Error(err ? err.message : 'Unknown RSC error');
        }
        // Model reference
        var refId = value.substring(1);
        if (flight.models[refId] !== undefined) {
          return resolveValue(flight.models[refId], flight);
        }
      }
      return value;
    }

    if (typeof value === 'number' || typeof value === 'boolean') return value;

    // Array
    if (Array.isArray(value)) {
      return value.map(function (item) { return resolveValue(item, flight); });
    }

    // Element node: { t: type, p: props }
    if (value && typeof value === 'object' && value.t !== undefined) {
      var type = value.t;
      var props = resolveProps(value.p || {}, flight);

      // Client module reference type: "$M<id>"
      if (typeof type === 'string' && type.length > 2 && type[0] === '$' && type[1] === 'M') {
        var moduleRefId = type.substring(2);
        var mod = flight.modules[moduleRefId];
        if (mod) {
          var Component = getClientComponent(mod.id, mod.name);
          return React.createElement(Component, props);
        }
      }

      // HTML element
      if (typeof type === 'string') {
        return React.createElement(type, props);
      }

      return null;
    }

    // Plain object (props sub-object)
    if (typeof value === 'object') {
      var result = {};
      for (var key in value) {
        if (Object.prototype.hasOwnProperty.call(value, key)) {
          result[key] = resolveValue(value[key], flight);
        }
      }
      return result;
    }

    return value;
  }

  function resolveProps(props, flight) {
    var resolved = {};
    for (var key in props) {
      if (!Object.prototype.hasOwnProperty.call(props, key)) continue;
      resolved[key] = resolveValue(props[key], flight);
    }
    return resolved;
  }

  // Resolve a module reference element: looks up the component, returns React element
  function resolveModuleReference(modId, flight) {
    var mod = flight.modules[modId];
    if (!mod) return null;

    // Find the corresponding J row that has this module as its type
    // The J row contains { t: "$M<modId>", p: props }
    // But we're being called from resolveValue when we encounter "$<jId>"
    // where the J row IS { t: "$M<modId>", p: props }
    // So this function is called when we see a raw "$M<id>" string reference
    var Component = getClientComponent(mod.id, mod.name);
    return Component;
  }

  // --- Client Component Loading ---

  // Get (or lazy-load) a client component by reference ID and export name.
  function getClientComponent(refId, exportName) {
    var cacheKey = refId + '#' + exportName;

    if (moduleCache[cacheKey]) {
      return moduleCache[cacheKey];
    }

    var entry = moduleMap[refId];
    if (!entry) {
      // Fallback: return a placeholder
      var Placeholder = function (_props) {
        return React.createElement('div', {
          'data-rsc-missing': refId,
          style: { border: '2px dashed red', padding: '8px' }
        }, 'Missing client component: ' + refId);
      };
      moduleCache[cacheKey] = Placeholder;
      return Placeholder;
    }

    // Create a lazy component that loads the chunk
    var LazyComponent = React.lazy(function () {
      return import(entry.chunk_url).then(function (mod) {
        var Component = exportName === 'default' ? mod.default : mod[exportName];
        if (!Component) {
          Component = function () {
            return React.createElement('div', null, 'Export not found: ' + exportName);
          };
        }
        // Cache the resolved component for future renders
        moduleCache[cacheKey] = Component;
        return { default: Component };
      });
    });

    moduleCache[cacheKey] = LazyComponent;
    return LazyComponent;
  }

  // --- Flight to React Tree ---

  function flightToReactTree(flightString) {
    var flight = parseFlightData(flightString);

    if (!flight.rootId) {
      console.error('[Rex RSC] No root marker in flight data');
      return null;
    }

    var rootValue = flight.models[flight.rootId];
    if (rootValue === undefined) {
      console.error('[Rex RSC] Root model not found:', flight.rootId);
      return null;
    }

    return resolveValue(rootValue, flight);
  }

  // --- Hydration ---

  var rscRoot = null;

  function hydrateFromInlineData() {
    var dataEl = document.getElementById('__REX_RSC_DATA__');
    if (!dataEl) return;

    var flightData = dataEl.textContent;
    if (!flightData) return;

    var tree = flightToReactTree(flightData);
    if (!tree) return;

    var container = document.getElementById('__rex');
    if (!container) return;

    // Wrap in Suspense for lazy-loaded client components
    var wrapped = React.createElement(React.Suspense, { fallback: null }, tree);

    try {
      rscRoot = ReactDOM.hydrateRoot(container, wrapped);
    } catch (e) {
      console.error('[Rex RSC] Hydration failed, falling back to render:', e);
      rscRoot = ReactDOM.createRoot(container);
      rscRoot.render(wrapped);
    }
  }

  // --- Client Navigation ---

  function navigateRsc(pathname) {
    var manifest = window.__REX_MANIFEST__;
    if (!manifest) return Promise.reject(new Error('No manifest'));

    var buildId = manifest.build_id;
    var url = '/_rex/rsc/' + buildId + pathname;

    return fetch(url).then(function (res) {
      if (!res.ok) throw new Error('RSC fetch failed: ' + res.status);
      return res.text();
    }).then(function (flightData) {
      var tree = flightToReactTree(flightData);
      if (!tree) throw new Error('Failed to parse flight data');

      if (rscRoot) {
        var wrapped = React.createElement(React.Suspense, { fallback: null }, tree);
        rscRoot.render(wrapped);
      }

      return tree;
    });
  }

  // --- Public API ---

  window.__REX_RSC_INIT = hydrateFromInlineData;
  window.__REX_RSC_NAVIGATE = navigateRsc;
  window.__REX_RSC_PARSE_FLIGHT = flightToReactTree;

  // Auto-init on DOMContentLoaded if flight data is present
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', function () {
      if (document.getElementById('__REX_RSC_DATA__')) {
        hydrateFromInlineData();
      }
    });
  } else {
    if (document.getElementById('__REX_RSC_DATA__')) {
      hydrateFromInlineData();
    }
  }
})();
