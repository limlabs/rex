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
//   - __rex_decodeReply: from react-server-dom-webpack/server (when server actions exist)
//   - __rex_decodeAction: from react-server-dom-webpack/server (when server actions exist)
//   - __rex_server_action_manifest: dispatch table for decodeReply/decodeAction

// --- State ---
let _chunks: (string | Uint8Array)[] = [];
let _rawChunks: Uint8Array[] = []; // Raw bytes for SSR pass (avoids UTF-8 round-trip)
let _streamDone = false;
let _flightString: string | null = null;
let _wantHtml = false;
let _htmlDone = false;
let _htmlResult: string | null = null;

// --- Metadata State ---
let _metadataHeadHtml = '';
let _metadataDone = true;

// --- Sentinel detection for notFound() and redirect() ---
// Next.js throws errors with a `digest` property for flow control.
// Rex's own stubs (next-navigation.ts) and the real next/navigation both use this pattern.
function _isNotFound(e: unknown): boolean {
    if (e && typeof e === 'object') {
        const obj = e as Record<string, unknown>;
        return obj.digest === 'NEXT_NOT_FOUND' || obj.__rex_type === '__rex_notFound__';
    }
    return false;
}

function _isRedirect(e: unknown): { url: string; status: number } | null {
    if (e && typeof e === 'object') {
        const obj = e as Record<string, unknown>;
        if (obj.digest === 'NEXT_REDIRECT' || obj.__rex_type === '__rex_redirect__') {
            return {
                url: String(obj.url || '/'),
                status: Number(obj.status || 307),
            };
        }
    }
    return null;
}

// --- Metadata Resolution ---

// Resolve metadata from the layout chain + page for a route.
// Sources are module namespaces that may have `metadata` or `generateMetadata` exports.
// generateMetadata receives { params, searchParams } like the page component.
function _resolveMetadata(routeKey: string, propsJson: string): void {
    const sources = globalThis.__rex_app_metadata_sources?.[routeKey];
    if (!sources || sources.length === 0) {
        _metadataHeadHtml = '';
        _metadataDone = true;
        return;
    }

    const raw = JSON.parse(propsJson);
    // Next.js 15 passes params/searchParams as Promises to generateMetadata too
    const props = {
        ...raw,
        params: Promise.resolve(raw.params || {}),
        searchParams: Promise.resolve(raw.searchParams || {}),
    };
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const chain: any[] = [];
    let hasAsync = false;

    // Collect metadata from each source (layouts first, then page)
    for (let i = 0; i < sources.length; i++) {
        const mod = sources[i];
        if (typeof mod.generateMetadata === 'function') {
            const result = mod.generateMetadata(props);
            if (result && typeof result.then === 'function') {
                hasAsync = true;
                chain.push(result);
            } else {
                chain.push(result);
            }
        } else if (mod.metadata) {
            chain.push(mod.metadata);
        } else {
            chain.push(null);
        }
    }

    if (!hasAsync) {
        // All synchronous — resolve immediately
        const resolved = chain.filter(Boolean);
        _metadataHeadHtml = resolved.length > 0 ? metadataToHtml(resolved) : '';
        _metadataDone = true;
        return;
    }

    // Some async generateMetadata — use Promise.all
    _metadataDone = false;
    Promise.all(chain).then(function(results) {
        const resolved = results.filter(Boolean);
        _metadataHeadHtml = resolved.length > 0 ? metadataToHtml(resolved) : '';
        _metadataDone = true;
    }, function() {
        _metadataHeadHtml = '';
        _metadataDone = true;
    });
}

// --- Helpers ---

// Safely stringify an error, handling cases where .stack or .message
// access throws (e.g. malformed error objects in React internals).
function _safeErrorString(e: unknown): string {
    try {
        if (e instanceof Error) return e.message || 'Unknown error';
        return String(e);
    } catch {
        return 'Unknown error (serialization failed)';
    }
}

function _buildElement(routeKey: string, propsJson: string): React.ReactElement | null {
    const props = JSON.parse(propsJson);
    const Page = globalThis.__rex_app_pages[routeKey];
    if (!Page) return null;

    // Next.js 15 passes params and searchParams as Promises.
    // Wrap them so page components can `await params` / `await searchParams`.
    const pageProps = {
        ...props,
        params: Promise.resolve(props.params || {}),
        searchParams: Promise.resolve(props.searchParams || {}),
    };

    const layouts = globalThis.__rex_app_layout_chains[routeKey] || [];

    // Build nested layout tree: Layout1(Layout2(Page))
    let element = __rex_createElement(Page, pageProps);

    for (let i = layouts.length - 1; i >= 0; i--) {
        const layoutProps = {
            params: pageProps.params,
            children: element,
        };
        element = __rex_createElement(layouts[i], layoutProps as React.Attributes & { children: React.ReactElement });
    }
    return element;
}

function _setStreamError(err: unknown, prefix: string): void {
    _streamDone = true;
    _flightString = '0:{"error":' + JSON.stringify(_safeErrorString(err)) + '}\n';
    if (_wantHtml && !_htmlDone) {
        _htmlResult = JSON.stringify({
            body: '<div style="color:red">' + prefix + _safeErrorString(err).replace(/</g, '&lt;') + '</div>',
            head: '', flight: _flightString
        });
        _htmlDone = true;
    }
}

function _assembleChunks(): string {
    const decoder = new TextDecoder();
    const parts: string[] = [];
    for (let i = 0; i < _chunks.length; i++) {
        const c = _chunks[i];
        // stream: true so multi-byte chars spanning chunk boundaries decode correctly
        parts.push(typeof c === 'string' ? c : decoder.decode(c, { stream: true }));
    }
    parts.push(decoder.decode()); // flush remaining bytes
    _chunks = [];
    return parts.join('');
}

function _startReading(reader: ReadableStreamDefaultReader<Uint8Array>): void {
    // Trampoline pattern: avoids stack overflow when promises resolve synchronously
    // (which happens in bare V8 where queueMicrotask/setTimeout call fn() immediately).
    function drain(): void {
        let sync = true;
        let loop = true;
        while (loop) {
            loop = false;
            reader.read().then(function(result: ReadableStreamReadResult<Uint8Array>) {
                try {
                    if (result.done) {
                        _streamDone = true;
                        _flightString = _assembleChunks();
                        // Store raw bytes on globalThis for SSR pass (avoids UTF-8 round-trip
                        // that corrupts binary flight chunks like TypedArrays)
                        globalThis.__rex_flight_raw_chunks = _rawChunks;
                        if (_wantHtml) {
                            _startHtmlPass();
                        }
                    } else {
                        _chunks.push(result.value);
                        // Keep a copy of raw bytes for the SSR pass
                        if (result.value instanceof Uint8Array) {
                            _rawChunks.push(result.value);
                        }
                        if (sync) {
                            loop = true; // continue the while loop instead of recursing
                        } else {
                            drain(); // truly async — safe to recurse
                        }
                    }
                } catch (e) {
                    // Prevent silent failures that leave _streamDone = false forever.
                    _setStreamError(e, 'RSC Read Error: ');
                }
            }, function(err: unknown) {
                _streamDone = true;
                // Check for notFound/redirect sentinels from async components
                if (_isNotFound(err)) {
                    _flightString = '';
                    if (_wantHtml && !_htmlDone) {
                        _htmlResult = '__REX_NOT_FOUND__';
                        _htmlDone = true;
                    }
                    return;
                }
                const redir = _isRedirect(err);
                if (redir) {
                    _flightString = '';
                    if (_wantHtml && !_htmlDone) {
                        _htmlResult = '__REX_REDIRECT__:' + redir.status + ':' + redir.url;
                        _htmlDone = true;
                    }
                    return;
                }
                _setStreamError(err, 'RSC Stream Error: ');
            });
        }
        sync = false;
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
    _rawChunks = [];
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

    try {
        const stream = __rex_renderToReadableStream(element, bundlerConfig, renderOptions);
        _startReading(stream.getReader());
    } catch (e) {
        _streamDone = true;
        if (_isNotFound(e)) {
            _flightString = '__REX_NOT_FOUND__';
        } else {
            const redir = _isRedirect(e);
            if (redir) {
                _flightString = '__REX_REDIRECT__:' + redir.status + ':' + redir.url;
            } else {
                _flightString = '0:{"error":' + JSON.stringify(_safeErrorString(e)) + '}\n';
            }
        }
    }

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
    _rawChunks = [];
    _streamDone = false;
    _flightString = null;
    _wantHtml = true;
    _htmlDone = false;
    _htmlResult = null;

    // Resolve metadata concurrently with RSC rendering
    _resolveMetadata(routeKey, propsJson);

    const bundlerConfig = globalThis.__rex_webpack_bundler_config || {};
    const renderOptions: Record<string, unknown> = {};
    if (globalThis.__rex_server_actions) {
        renderOptions.serverManifest = bundlerConfig;
    }

    try {
        const stream = __rex_renderToReadableStream(element, bundlerConfig, renderOptions);
        _startReading(stream.getReader());
    } catch (e) {
        if (_isNotFound(e)) {
            _streamDone = true; _flightString = '';
            _htmlResult = '__REX_NOT_FOUND__'; _htmlDone = true;
        } else {
            const redir = _isRedirect(e);
            if (redir) {
                _streamDone = true; _flightString = '';
                _htmlResult = '__REX_REDIRECT__:' + redir.status + ':' + redir.url; _htmlDone = true;
            } else {
                _setStreamError(e, 'RSC Error: ');
            }
        }
    }

    if (_htmlDone && _metadataDone) {
        // Inject metadata head into the result
        if (_metadataHeadHtml && _htmlResult) {
            _injectMetadataHead();
        }
        return _htmlResult!;
    }

    return '__REX_RSC_HTML_ASYNC__';
};

// --- Public API: Async resolution ---

globalThis.__rex_resolve_rsc_pending = function(): "pending" | "done" {
    // Phase 1: Stream still reading
    if (!_streamDone) return 'pending';

    // Phase 2: Metadata still resolving (async generateMetadata)
    if (!_metadataDone) return 'pending';

    // Phase 3: HTML pass pending (two-pass mode)
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
                    // Inject metadata head into the finalized HTML result
                    if (_metadataHeadHtml) {
                        _injectMetadataHead();
                    }
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

// Inject resolved metadata head HTML into the _htmlResult JSON.
// The SSR pass returns { body, head, flight } — we set `head` to the metadata HTML.
function _injectMetadataHead(): void {
    if (!_htmlResult || !_metadataHeadHtml) return;
    try {
        const parsed = JSON.parse(_htmlResult);
        parsed.head = _metadataHeadHtml;
        _htmlResult = JSON.stringify(parsed);
    } catch {
        // If parsing fails, leave _htmlResult as-is
    }
}

globalThis.__rex_finalize_rsc_to_html = function(): string {
    // Inject metadata before returning
    if (_metadataHeadHtml) {
        _injectMetadataHead();
    }
    const result = _htmlResult || JSON.stringify({ body: '', head: '', flight: _flightString || '' });
    _flightString = null;
    _htmlResult = null;
    _metadataHeadHtml = '';
    return result;
};

// --- Server Action Dispatch ---

let _actionResult: string | null = null;
let _actionDone = false;

// --- redirect() and notFound() sentinel errors ---

const REDIRECT_TYPE = '__rex_redirect__';
const NOT_FOUND_TYPE = '__rex_notFound__';

globalThis.__rex_redirect = function(url: string, status?: number): never {
    throw { __rex_type: REDIRECT_TYPE, url: url, status: status || 303 };
};

globalThis.__rex_notFound = function(): never {
    throw { __rex_type: NOT_FOUND_TYPE };
};

// --- Request context (set by Rust before action execution) ---

globalThis.__rex_request_headers = {};
globalThis.__rex_request_cookies = {};

// --- Flight serialization for action results ---

function _storeActionResult(val: unknown): void {
    const bundlerConfig = globalThis.__rex_webpack_bundler_config || {};
    try {
        // renderToReadableStream can serialize any value, not just ReactElements
        const stream = __rex_renderToReadableStream(val as React.ReactElement, bundlerConfig);
        const reader = stream.getReader();
        const chunks: (string | Uint8Array)[] = [];

        // Trampoline pattern: avoids stack overflow with sync promise resolution
        function drain(): void {
            let sync = true;
            let loop = true;
            while (loop) {
                loop = false;
                reader.read().then(function(result: ReadableStreamReadResult<Uint8Array>) {
                    if (result.done) {
                        const decoder = new TextDecoder();
                        const parts: string[] = [];
                        for (let i = 0; i < chunks.length; i++) {
                            const c = chunks[i];
                            parts.push(typeof c === 'string' ? c : decoder.decode(c));
                        }
                        _actionResult = JSON.stringify({ flight: parts.join('') });
                        _actionDone = true;
                    } else {
                        chunks.push(result.value);
                        if (sync) {
                            loop = true;
                        } else {
                            drain();
                        }
                    }
                }, function(err: unknown) {
                    _actionResult = JSON.stringify({ error: 'Flight serialization failed: ' + String(err) });
                    _actionDone = true;
                });
            }
            sync = false;
        }
        drain();
    } catch {
        // Fallback to JSON if flight serialization not available
        _actionResult = JSON.stringify({ result: val });
        _actionDone = true;
    }
}

function _handleActionError(err: unknown): void {
    if (err && typeof err === 'object') {
        const sentinel = err as Record<string, unknown>;
        if (sentinel.__rex_type === REDIRECT_TYPE) {
            _actionResult = JSON.stringify({
                redirect: sentinel.url,
                redirectStatus: sentinel.status
            });
            _actionDone = true;
            return;
        }
        if (sentinel.__rex_type === NOT_FOUND_TYPE) {
            _actionResult = JSON.stringify({ notFound: true });
            _actionDone = true;
            return;
        }
    }
    _actionResult = JSON.stringify({ error: String(err) });
    _actionDone = true;
}

function _handleActionValue(val: unknown): void {
    _storeActionResult(val);
}

// Legacy JSON-only path (backward compat for plain JSON args)
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
            (result as Promise<unknown>).then(
                _handleActionValue,
                _handleActionError
            );
            return '__REX_ACTION_ASYNC__';
        }
        _handleActionValue(result);
        return '__REX_ACTION_ASYNC__';
    } catch (e) {
        _handleActionError(e);
        return '__REX_ACTION_ASYNC__';
    }
};

// Encoded reply path: uses React's decodeReply to handle FormData, Blob, etc.
// body is either a string (from encodeReply) or a JSON array of [key, value] pairs
// when isFormFields is true (multipart from encodeReply returning FormData).
// Returns '__REX_ACTION_ASYNC__' since decodeReply is always async.
globalThis.__rex_call_server_action_encoded = function(actionId: string, body: string, isFormFields?: boolean): string {
    const actions = globalThis.__rex_server_actions || {};
    const fn = actions[actionId];
    if (!fn) {
        return JSON.stringify({ error: 'Server action not found: ' + actionId });
    }

    // Reset state
    _actionResult = null;
    _actionDone = false;

    const serverManifest = globalThis.__rex_server_action_manifest || {};

    // Build the body for decodeReply: either a raw string or reconstructed FormData
    let decodedBody: string | FormData;
    if (isFormFields) {
        let fields: [string, string][];
        try {
            fields = JSON.parse(body);
        } catch {
            return JSON.stringify({ error: 'Invalid form fields JSON' });
        }
        const formData = new FormData();
        for (let i = 0; i < fields.length; i++) {
            formData.append(fields[i][0], fields[i][1]);
        }
        decodedBody = formData;
    } else {
        decodedBody = body;
    }

    // decodeReply returns a thenable/Promise of the decoded args
    const decoded = globalThis.__rex_decodeReply(decodedBody, serverManifest);

    Promise.resolve(decoded).then(
        function(args: unknown) {
            const argArray = Array.isArray(args) ? args : [args];
            try {
                const result = fn.apply(null, argArray);
                if (result && typeof result === 'object' && typeof (result as Record<string, unknown>).then === 'function') {
                    return (result as Promise<unknown>).then(
                        _handleActionValue,
                        _handleActionError
                    );
                }
                _handleActionValue(result);
            } catch (e) {
                _handleActionError(e);
            }
        },
        function(err: unknown) {
            _actionResult = JSON.stringify({ error: 'decodeReply failed: ' + String(err) });
            _actionDone = true;
        }
    );

    return '__REX_ACTION_ASYNC__';
};

// Form action path: uses React's decodeAction to resolve action + args from FormData.
// fieldsJson is a JSON array of [key, value] pairs from Rust's multipart parsing.
// Returns '__REX_ACTION_ASYNC__' since decodeAction is always async.
globalThis.__rex_call_form_action = function(fieldsJson: string): string {
    // Reset state
    _actionResult = null;
    _actionDone = false;

    let fields: [string, string][];
    try {
        fields = JSON.parse(fieldsJson);
    } catch {
        return JSON.stringify({ error: 'Invalid form fields JSON' });
    }

    // Reconstruct FormData from parsed fields
    const formData = new FormData();
    for (let i = 0; i < fields.length; i++) {
        formData.append(fields[i][0], fields[i][1]);
    }

    const serverManifest = globalThis.__rex_server_action_manifest || {};

    const actionPromise = globalThis.__rex_decodeAction(formData, serverManifest);

    Promise.resolve(actionPromise).then(
        function(boundFn: unknown) {
            if (typeof boundFn !== 'function') {
                _actionResult = JSON.stringify({ error: 'decodeAction did not return a function' });
                _actionDone = true;
                return;
            }
            try {
                const result = (boundFn as () => unknown)();
                if (result && typeof result === 'object' && typeof (result as Record<string, unknown>).then === 'function') {
                    return (result as Promise<unknown>).then(
                        _handleActionValue,
                        _handleActionError
                    );
                }
                _handleActionValue(result);
            } catch (e) {
                _handleActionError(e);
            }
        },
        function(err: unknown) {
            _actionResult = JSON.stringify({ error: 'decodeAction failed: ' + String(err) });
            _actionDone = true;
        }
    );

    return '__REX_ACTION_ASYNC__';
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
