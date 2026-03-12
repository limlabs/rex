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

  const _require = (globalThis as Record<string, unknown>).require as ((id: string) => unknown) | undefined;
  const React = window.React || (typeof _require === 'function' && _require('react')) as typeof import('react') | false;
  const ReactDOM = window.ReactDOM || (typeof _require === 'function' && _require('react-dom/client')) as typeof import('react-dom/client') | false;

  if (!React || !ReactDOM) {
    console.warn('[Rex RSC] React/ReactDOM not found, RSC runtime disabled');
    return;
  }

  // Module map: { entries: { refId -> { chunk_url, export_name } } }
  const rawMap = window.__REX_RSC_MODULE_MAP__ || {};
  const moduleMap = ((rawMap.entries || rawMap) as Record<string, RexRscModuleMapEntry>);

  // Cache of loaded client component modules
  const moduleCache: Record<string, React.ComponentType> = {};

  // --- Flight Data Parser ---

  interface FlightData {
    models: Record<string, unknown>;
    modules: Record<string, { id: string; name: string }>;
    errors: Record<string, { message: string; stack?: string }>;
    rootId: string | null;
  }

  function parseFlightData(flightString: string): FlightData {
    const rows = flightString.split('\n');
    const models: Record<string, unknown> = {};
    const modules: Record<string, { id: string; name: string }> = {};
    const errors: Record<string, { message: string; stack?: string }> = {};
    let rootId: string | null = null;

    for (let i = 0; i < rows.length; i++) {
      const row = rows[i].trim();
      if (!row) continue;

      const colonIdx = row.indexOf(':');
      if (colonIdx === -1) continue;

      const type = row.substring(0, colonIdx);
      const rest = row.substring(colonIdx + 1);

      if (type === 'R') {
        rootId = rest;
        continue;
      }

      const secondColon = rest.indexOf(':');
      if (secondColon === -1) continue;

      const id = rest.substring(0, secondColon);
      const payload = rest.substring(secondColon + 1);

      try {
        const parsed = JSON.parse(payload);
        if (type === 'J') {
          models[id] = parsed;
        } else if (type === 'M') {
          modules[id] = parsed as { id: string; name: string };
        } else if (type === 'E') {
          errors[id] = parsed as { message: string; stack?: string };
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
  function resolveValue(value: unknown, flight: FlightData): unknown {
    if (value === null || value === undefined) return value;

    // String reference: "$<id>" or "$M<id>" or "$E<id>"
    if (typeof value === 'string') {
      if (value.length > 1 && value[0] === '$') {
        if (value[1] === 'M') {
          // Client module reference
          const modId = value.substring(2);
          return resolveModuleReference(modId, flight);
        }
        if (value[1] === 'E') {
          // Error reference
          const errId = value.substring(2);
          const err = flight.errors[errId];
          throw new Error(err ? err.message : 'Unknown RSC error');
        }
        // Model reference
        const refId = value.substring(1);
        if (flight.models[refId] !== undefined) {
          return resolveValue(flight.models[refId], flight);
        }
      }
      return value;
    }

    if (typeof value === 'number' || typeof value === 'boolean') return value;

    // Array
    if (Array.isArray(value)) {
      return value.map(function (item: unknown) { return resolveValue(item, flight); });
    }

    // Element node: { t: type, p: props }
    if (value && typeof value === 'object' && 't' in value) {
      const elementNode = value as { t: string; p?: Record<string, unknown> };
      const nodeType = elementNode.t;
      const props = resolveProps(elementNode.p || {}, flight);

      // Client module reference type: "$M<id>"
      if (typeof nodeType === 'string' && nodeType.length > 2 && nodeType[0] === '$' && nodeType[1] === 'M') {
        const moduleRefId = nodeType.substring(2);
        const mod = flight.modules[moduleRefId];
        if (mod) {
          const Component = getClientComponent(mod.id, mod.name);
          return React.createElement(Component, props);
        }
      }

      // HTML element
      if (typeof nodeType === 'string') {
        return React.createElement(nodeType, props);
      }

      return null;
    }

    // Plain object (props sub-object)
    if (typeof value === 'object') {
      const result: Record<string, unknown> = {};
      for (const key in value as Record<string, unknown>) {
        if (Object.prototype.hasOwnProperty.call(value, key)) {
          result[key] = resolveValue((value as Record<string, unknown>)[key], flight);
        }
      }
      return result;
    }

    return value;
  }

  function resolveProps(props: Record<string, unknown>, flight: FlightData): Record<string, unknown> {
    const resolved: Record<string, unknown> = {};
    for (const key in props) {
      if (!Object.prototype.hasOwnProperty.call(props, key)) continue;
      resolved[key] = resolveValue(props[key], flight);
    }
    return resolved;
  }

  // Resolve a module reference element: looks up the component, returns React element
  function resolveModuleReference(modId: string, flight: FlightData): React.ComponentType | null {
    const mod = flight.modules[modId];
    if (!mod) return null;

    return getClientComponent(mod.id, mod.name);
  }

  // --- Client Component Loading ---

  // Get (or lazy-load) a client component by reference ID and export name.
  function getClientComponent(refId: string, exportName: string): React.ComponentType {
    const cacheKey = refId + '#' + exportName;

    if (moduleCache[cacheKey]) {
      return moduleCache[cacheKey];
    }

    const entry = moduleMap[refId];
    if (!entry) {
      // Fallback: return a placeholder
      const Placeholder: React.FC = function (_props) {
        return React.createElement('div', {
          'data-rsc-missing': refId,
          style: { border: '2px dashed red', padding: '8px' }
        }, 'Missing client component: ' + refId);
      };
      moduleCache[cacheKey] = Placeholder;
      return Placeholder;
    }

    // Create a lazy component that loads the chunk
    const LazyComponent = React.lazy(function () {
      return import(entry.chunk_url).then(function (mod: Record<string, unknown>) {
        let Component = (exportName === 'default' ? mod.default : mod[exportName]) as React.ComponentType | undefined;
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

  function flightToReactTree(flightString: string): React.ReactElement | null {
    const flight = parseFlightData(flightString);

    if (!flight.rootId) {
      console.error('[Rex RSC] No root marker in flight data');
      return null;
    }

    const rootValue = flight.models[flight.rootId];
    if (rootValue === undefined) {
      console.error('[Rex RSC] Root model not found:', flight.rootId);
      return null;
    }

    return resolveValue(rootValue, flight) as React.ReactElement | null;
  }

  // --- Hydration ---

  let rscRoot: ReturnType<typeof ReactDOM.hydrateRoot> | ReturnType<typeof ReactDOM.createRoot> | null = null;

  function hydrateFromInlineData(): void {
    const dataEl = document.getElementById('__REX_RSC_DATA__');
    if (!dataEl) return;

    const flightData = dataEl.textContent;
    if (!flightData) return;

    const tree = flightToReactTree(flightData);
    if (!tree) return;

    const container = document.getElementById('__rex');
    if (!container) return;

    // Wrap in Suspense for lazy-loaded client components
    const wrapped = React.createElement(React.Suspense, { fallback: null }, tree);

    try {
      rscRoot = ReactDOM.hydrateRoot(container, wrapped);
    } catch (e) {
      console.error('[Rex RSC] Hydration failed, falling back to render:', e);
      rscRoot = ReactDOM.createRoot(container);
      rscRoot.render(wrapped);
    }
  }

  // --- Client Navigation ---

  function navigateRsc(pathname: string): Promise<void> {
    const manifest = window.__REX_MANIFEST__;
    if (!manifest) return Promise.reject(new Error('No manifest'));

    const buildId = manifest.build_id;
    const suffix = pathname === '/' ? '' : pathname;
    const url = '/_rex/rsc/' + buildId + suffix;

    return fetch(url).then(function (res) {
      if (!res.ok) throw new Error('RSC fetch failed: ' + res.status);
      return res.text();
    }).then(function (flightData) {
      const tree = flightToReactTree(flightData);
      if (!tree) throw new Error('Failed to parse flight data');

      if (rscRoot) {
        const wrapped = React.createElement(React.Suspense, { fallback: null }, tree);
        rscRoot.render(wrapped);
      }

    }).then(function() { /* void */ });
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
