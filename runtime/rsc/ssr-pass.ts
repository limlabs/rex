// RSC SSR Pass — Converts flight data to HTML
//
// Uses React's official APIs:
//   - createFromReadableStream() from react-server-dom-webpack/client
//   - renderToReadableStream() from react-dom/server (streaming, Suspense-aware)
//
// The virtual entry sets these globals before this code runs:
//   - __rex_createFromReadableStream: from react-server-dom-webpack/client
//   - __rex_renderToReadableStream_ssr: from react-dom/server
//   - __rex_webpack_ssr_manifest: client-side module map for resolving client refs

var _ssrPending = false;
var _ssrResult: string | null = null;

function _ssrSafeErrorString(e: unknown): string {
    try {
        if (e instanceof Error) {
            // Include stack for debugging
            if (e.stack) return e.stack;
            return e.message || 'Unknown error';
        }
        return String(e);
    } catch {
        return 'Unknown error (serialization failed)';
    }
}

function _ssrError(msg: string, flightString: string): string {
    return JSON.stringify({
        body: '<div style="color:red">' + msg.replace(/</g, '&lt;') + '</div>',
        head: '',
        flight: flightString
    });
}

// Drain a ReadableStream of HTML chunks into a single string using the
// trampoline pattern (avoids stack overflow when promises resolve sync in V8).
function _drainHtmlStream(
    reader: ReadableStreamDefaultReader<Uint8Array>,
    flightString: string,
): void {
    var chunks: (string | Uint8Array)[] = [];

    function drain(): void {
        var sync = true;
        var loop = true;
        while (loop) {
            loop = false;
            reader.read().then(function(result: ReadableStreamReadResult<Uint8Array>) {
                if (result.done) {
                    var decoder = new TextDecoder();
                    var parts: string[] = [];
                    for (var i = 0; i < chunks.length; i++) {
                        var c = chunks[i];
                        parts.push(typeof c === 'string' ? c : decoder.decode(c));
                    }
                    _ssrResult = JSON.stringify({
                        body: parts.join(''),
                        head: '',
                        flight: flightString
                    });
                    _ssrPending = false;
                } else {
                    chunks.push(result.value);
                    if (sync) {
                        loop = true;
                    } else {
                        drain();
                    }
                }
            }, function(err: unknown) {
                _ssrResult = _ssrError('SSR stream error: ' + _ssrSafeErrorString(err), flightString);
                _ssrPending = false;
            });
        }
        sync = false;
    }
    drain();
}

// Render a React tree to HTML using the streaming renderToReadableStream API.
// renderToReadableStream returns Promise<ReadableStream> where the stream has
// an `allReady` Promise that resolves when all Suspense boundaries are ready.
function _renderTreeToHtml(tree: unknown, flightString: string): void {
    try {
        // renderToReadableStream returns a Promise<ReadableStream>
        var streamPromise = __rex_renderToReadableStream_ssr(tree, {
            onError: function(err: unknown) {
                // Log to console for server-side debugging
                if (typeof console !== 'undefined') {
                    console.error('[Rex SSR onError]', err);
                }
            }
        });
        Promise.resolve(streamPromise).then(function(htmlStream: any) {
            // Wait for allReady (all Suspense boundaries resolved)
            htmlStream.allReady.then(function() {
                _drainHtmlStream(htmlStream.getReader(), flightString);
            }, function(err: unknown) {
                if (typeof console !== 'undefined') console.error('[Rex SSR allReady]', err instanceof Error ? err.stack : err);
                _ssrResult = _ssrError('SSR allReady error: ' + _ssrSafeErrorString(err), flightString);
                _ssrPending = false;
            });
        }, function(err: unknown) {
            if (typeof console !== 'undefined') console.error('[Rex SSR render]', err instanceof Error ? err.stack : err);
            _ssrResult = _ssrError('SSR render error: ' + _ssrSafeErrorString(err), flightString);
            _ssrPending = false;
        });
    } catch(e) {
        _ssrResult = _ssrError('SSR render error: ' + _ssrSafeErrorString(e), flightString);
        _ssrPending = false;
    }
}

globalThis.__rex_rsc_flight_to_html = function(flightString: string): string {
    _ssrPending = true;
    _ssrResult = null;

    // Use raw flight bytes if available (avoids UTF-8 round-trip that corrupts
    // binary flight chunks like TypedArrays). Falls back to re-encoding the string.
    var rawChunks: Uint8Array[] | null = globalThis.__rex_flight_raw_chunks || null;
    var stream: ReadableStream<Uint8Array>;

    if (rawChunks && rawChunks.length > 0) {
        var chunkIndex = 0;
        stream = new ReadableStream<Uint8Array>({
            pull: function(controller: ReadableStreamDefaultController<Uint8Array>) {
                if (chunkIndex < rawChunks!.length) {
                    controller.enqueue(rawChunks![chunkIndex++]);
                } else {
                    controller.close();
                }
            }
        });
    } else {
        var encoder = new TextEncoder();
        var encoded = encoder.encode(flightString);
        stream = new ReadableStream<Uint8Array>({
            start: function(controller: ReadableStreamDefaultController<Uint8Array>) {
                controller.enqueue(encoded);
                controller.close();
            }
        });
    }

    var ssrManifest = globalThis.__rex_webpack_ssr_manifest || {};
    var treeResult: unknown;
    try {
        treeResult = __rex_createFromReadableStream(stream, {
            // React 19.2+ uses serverConsumerManifest, older uses ssrManifest
            serverConsumerManifest: {
                moduleMap: ssrManifest,
                serverModuleMap: {},
                moduleLoading: null
            }
        });
    } catch(e) {
        _ssrResult = _ssrError('RSC SSR Error: ' + _ssrSafeErrorString(e), flightString);
        _ssrPending = false;
        return _ssrResult;
    }

    // createFromReadableStream returns a thenable
    if (treeResult && typeof (treeResult as PromiseLike<unknown>).then === 'function') {
        (treeResult as PromiseLike<unknown>).then(function(tree: unknown) {
            _renderTreeToHtml(tree, flightString);
        }, function(err: unknown) {
            _ssrResult = _ssrError('SSR error: ' + _ssrSafeErrorString(err), flightString);
            _ssrPending = false;
        });

        if (!_ssrPending) {
            return _ssrResult!;
        }
        return '__REX_SSR_ASYNC__';
    }

    // Synchronous result — render via streaming API
    _renderTreeToHtml(treeResult, flightString);
    if (!_ssrPending) {
        return _ssrResult!;
    }
    return '__REX_SSR_ASYNC__';
};

globalThis.__rex_resolve_ssr_pending = function(): "pending" | "done" {
    return _ssrPending ? 'pending' : 'done';
};

globalThis.__rex_finalize_ssr = function(): string {
    return _ssrResult || JSON.stringify({ body: '', head: '', flight: '' });
};
