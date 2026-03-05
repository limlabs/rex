// RSC Flight Runtime — React Reconciler Integration
//
// Uses React's official react-server-dom-webpack APIs:
//   - renderToReadableStream() for flight data generation
//   - Drains the stream synchronously for sync components,
//     or via microtask pump for async components.
//
// The virtual entry sets these globals before this code runs:
//   - __rex_renderToReadableStream: from react-server-dom-webpack/server
//   - __rex_createElement: from react
//   - __rex_app_pages: page component registry
//   - __rex_app_layout_chains: layout chains per route
//   - __rex_webpack_bundler_config: server-side bundler config for client refs

// --- State ---
let _chunks: (string | Uint8Array)[] = [];
let _streamDone = false;
let _flightString: string | null = null;
let _wantHtml = false;
let _htmlDone = false;
let _htmlResult: string | null = null;

// --- Helpers ---

function _buildElement(routeKey: string, propsJson: string): React.ReactElement | null {
    const props = JSON.parse(propsJson);
    const Page = globalThis.__rex_app_pages[routeKey];
    if (!Page) return null;

    const layouts = globalThis.__rex_app_layout_chains[routeKey] || [];

    // Build nested layout tree: Layout1(Layout2(Page))
    let element = __rex_createElement(Page, props);
    for (let i = layouts.length - 1; i >= 0; i--) {
        element = __rex_createElement(layouts[i], { children: element } as React.Attributes & { children: React.ReactElement });
    }
    return element;
}

function _assembleChunks(): string {
    const decoder = new TextDecoder();
    const parts: string[] = [];
    for (let i = 0; i < _chunks.length; i++) {
        const c = _chunks[i];
        parts.push(typeof c === 'string' ? c : decoder.decode(c));
    }
    _chunks = [];
    return parts.join('');
}

function _startReading(reader: ReadableStreamDefaultReader<Uint8Array>): void {
    function drain(): void {
        reader.read().then(function(result: ReadableStreamReadResult<Uint8Array>) {
            if (result.done) {
                _streamDone = true;
                _flightString = _assembleChunks();
                if (_wantHtml) {
                    _startHtmlPass();
                }
            } else {
                _chunks.push(result.value);
                drain();
            }
        }, function(err: unknown) {
            _streamDone = true;
            // Encode error as flight data
            _flightString = '0:{"error":' + JSON.stringify(String(err)) + '}\n';
        });
    }
    drain();
}

function _startHtmlPass(): void {
    // Call the SSR bundle's flight-to-HTML function
    if (typeof globalThis.__rex_rsc_flight_to_html === 'function') {
        const result = globalThis.__rex_rsc_flight_to_html(_flightString!);
        if (result === '__REX_SSR_ASYNC__') {
            // SSR is async — __rex_resolve_rsc_pending will check
            return;
        }
        _htmlResult = result;
        _htmlDone = true;
    } else {
        // SSR bundle not loaded — return flight data with empty HTML
        _htmlResult = JSON.stringify({ body: '', head: '', flight: _flightString });
        _htmlDone = true;
    }
}

// --- Public API: Flight-only render ---

globalThis.__rex_render_flight = function(routeKey: string, propsJson: string): string {
    const element = _buildElement(routeKey, propsJson);
    if (!element) {
        return '0:{"error":"Page not found: ' + routeKey + '"}\n';
    }

    // Reset state
    _chunks = [];
    _streamDone = false;
    _flightString = null;
    _wantHtml = false;
    _htmlDone = false;
    _htmlResult = null;

    const bundlerConfig = globalThis.__rex_webpack_bundler_config || {};
    const renderOptions: Record<string, unknown> = {};
    if (globalThis.__rex_server_actions) {
        renderOptions.serverManifest = bundlerConfig;
    }
    const stream = __rex_renderToReadableStream(element, bundlerConfig, renderOptions);
    _startReading(stream.getReader());

    if (_streamDone) {
        return _flightString!;
    }

    return '__REX_RSC_ASYNC__';
};

// --- Public API: Two-pass render (flight + HTML) ---

globalThis.__rex_render_rsc_to_html = function(routeKey: string, propsJson: string): string {
    const element = _buildElement(routeKey, propsJson);
    if (!element) {
        return JSON.stringify({
            body: '<div>Page not found</div>',
            head: '',
            flight: ''
        });
    }

    // Reset state
    _chunks = [];
    _streamDone = false;
    _flightString = null;
    _wantHtml = true;
    _htmlDone = false;
    _htmlResult = null;

    const bundlerConfig = globalThis.__rex_webpack_bundler_config || {};
    const renderOptions: Record<string, unknown> = {};
    if (globalThis.__rex_server_actions) {
        renderOptions.serverManifest = bundlerConfig;
    }
    const stream = __rex_renderToReadableStream(element, bundlerConfig, renderOptions);
    _startReading(stream.getReader());

    if (_htmlDone) {
        return _htmlResult!;
    }

    return '__REX_RSC_HTML_ASYNC__';
};

// --- Public API: Async resolution ---

globalThis.__rex_resolve_rsc_pending = function(): "pending" | "done" {
    // Phase 1: Stream still reading
    if (!_streamDone) return 'pending';

    // Phase 2: HTML pass pending (two-pass mode)
    if (_wantHtml) {
        if (!_htmlDone) {
            // Check if SSR resolved
            if (typeof globalThis.__rex_resolve_ssr_pending === 'function') {
                const ssrStatus = globalThis.__rex_resolve_ssr_pending();
                if (ssrStatus === 'done') {
                    if (typeof globalThis.__rex_finalize_ssr === 'function') {
                        _htmlResult = globalThis.__rex_finalize_ssr();
                    }
                    _htmlDone = true;
                    return 'done';
                }
                return 'pending';
            }
            return 'pending';
        }
        return 'done';
    }

    return 'done';
};

globalThis.__rex_finalize_rsc_flight = function(): string {
    const result = _flightString || '';
    _flightString = null;
    return result;
};

globalThis.__rex_finalize_rsc_to_html = function(): string {
    const result = _htmlResult || JSON.stringify({ body: '', head: '', flight: _flightString || '' });
    _flightString = null;
    _htmlResult = null;
    return result;
};

// --- Server Action Dispatch ---

let _actionResult: string | null = null;
let _actionDone = false;

globalThis.__rex_call_server_action = function(actionId: string, argsJson: string): string {
    const actions = globalThis.__rex_server_actions || {};
    const fn = actions[actionId];
    if (!fn) {
        return JSON.stringify({ error: 'Server action not found: ' + actionId });
    }

    // Reset state
    _actionResult = null;
    _actionDone = false;

    let args: unknown[];
    try {
        args = JSON.parse(argsJson);
    } catch {
        return JSON.stringify({ error: 'Invalid arguments JSON' });
    }

    try {
        const result = fn.apply(null, args);
        if (result && typeof result === 'object' && typeof (result as Record<string, unknown>).then === 'function') {
            // Async — store promise resolution
            (result as Promise<unknown>).then(
                function(val: unknown) {
                    _actionResult = JSON.stringify({ result: val });
                    _actionDone = true;
                },
                function(err: unknown) {
                    _actionResult = JSON.stringify({ error: String(err) });
                    _actionDone = true;
                }
            );
            return '__REX_ACTION_ASYNC__';
        }
        return JSON.stringify({ result: result });
    } catch (e) {
        return JSON.stringify({ error: String(e) });
    }
};

globalThis.__rex_resolve_action_pending = function(): "pending" | "done" {
    return _actionDone ? 'done' : 'pending';
};

globalThis.__rex_finalize_action = function(): string {
    const result = _actionResult || JSON.stringify({ error: 'No action result' });
    _actionResult = null;
    _actionDone = false;
    return result;
};
