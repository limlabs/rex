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
var _chunks: (string | Uint8Array)[] = [];
var _streamDone = false;
var _flightString: string | null = null;
var _wantHtml = false;
var _htmlDone = false;
var _htmlResult: string | null = null;

// --- Helpers ---

function _buildElement(routeKey: string, propsJson: string): React.ReactElement | null {
    var props = JSON.parse(propsJson);
    var Page = globalThis.__rex_app_pages[routeKey];
    if (!Page) return null;

    var layouts = globalThis.__rex_app_layout_chains[routeKey] || [];

    // Build nested layout tree: Layout1(Layout2(Page))
    var element = __rex_createElement(Page, props);
    for (var i = layouts.length - 1; i >= 0; i--) {
        element = __rex_createElement(layouts[i], { children: element } as React.Attributes & { children: React.ReactElement });
    }
    return element;
}

function _assembleChunks(): string {
    var decoder = new TextDecoder();
    var parts: string[] = [];
    for (var i = 0; i < _chunks.length; i++) {
        var c = _chunks[i];
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
        var result = globalThis.__rex_rsc_flight_to_html(_flightString!);
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
    var element = _buildElement(routeKey, propsJson);
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

    var bundlerConfig = globalThis.__rex_webpack_bundler_config || {};
    var stream = __rex_renderToReadableStream(element, bundlerConfig);
    _startReading(stream.getReader());

    if (_streamDone) {
        return _flightString!;
    }

    return '__REX_RSC_ASYNC__';
};

// --- Public API: Two-pass render (flight + HTML) ---

globalThis.__rex_render_rsc_to_html = function(routeKey: string, propsJson: string): string {
    var element = _buildElement(routeKey, propsJson);
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

    var bundlerConfig = globalThis.__rex_webpack_bundler_config || {};
    var stream = __rex_renderToReadableStream(element, bundlerConfig);
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
                var ssrStatus = globalThis.__rex_resolve_ssr_pending();
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
    var result = _flightString || '';
    _flightString = null;
    return result;
};

globalThis.__rex_finalize_rsc_to_html = function(): string {
    var result = _htmlResult || JSON.stringify({ body: '', head: '', flight: _flightString || '' });
    _flightString = null;
    _htmlResult = null;
    return result;
};
