// RSC SSR Pass — Converts flight data to HTML
//
// Uses React's official APIs:
//   - createFromReadableStream() from react-server-dom-webpack/client
//   - renderToString() from react-dom/server
//
// The virtual entry sets these globals before this code runs:
//   - __rex_createFromReadableStream: from react-server-dom-webpack/client
//   - __rex_renderToString: from react-dom/server
//   - __rex_webpack_ssr_manifest: client-side module map for resolving client refs

var _ssrPending = false;
var _ssrResult: string | null = null;

globalThis.__rex_rsc_flight_to_html = function(flightString: string): string {
    _ssrPending = true;
    _ssrResult = null;

    // Wrap flight string in a ReadableStream for createFromReadableStream
    var encoder = new TextEncoder();
    var encoded = encoder.encode(flightString);
    var stream = new ReadableStream<Uint8Array>({
        start: function(controller: ReadableStreamDefaultController<Uint8Array>) {
            controller.enqueue(encoded);
            controller.close();
        }
    });

    var ssrManifest = globalThis.__rex_webpack_ssr_manifest || {};
    var treeResult: unknown;
    try {
        treeResult = __rex_createFromReadableStream(stream, {
            ssrManifest: {
                moduleMap: ssrManifest,
                moduleLoading: null
            }
        });
    } catch(e) {
        _ssrResult = JSON.stringify({
            body: '<div style="color:red">RSC SSR Error: ' + String(e).replace(/</g, '&lt;') + '</div>',
            head: '',
            flight: flightString
        });
        _ssrPending = false;
        return _ssrResult;
    }

    // createFromReadableStream returns a thenable
    // For synchronous flight data, it may resolve immediately after microtask pump
    if (treeResult && typeof (treeResult as PromiseLike<unknown>).then === 'function') {
        (treeResult as PromiseLike<unknown>).then(function(tree: unknown) {
            try {
                var html = __rex_renderToString(tree);
                _ssrResult = JSON.stringify({ body: html, head: '', flight: flightString });
            } catch(e) {
                _ssrResult = JSON.stringify({
                    body: '<div style="color:red">SSR render error: ' + String(e).replace(/</g, '&lt;') + '</div>',
                    head: '',
                    flight: flightString
                });
            }
            _ssrPending = false;
        }, function(err: unknown) {
            _ssrResult = JSON.stringify({
                body: '<div style="color:red">SSR error: ' + String(err).replace(/</g, '&lt;') + '</div>',
                head: '',
                flight: flightString
            });
            _ssrPending = false;
        });

        if (!_ssrPending) {
            return _ssrResult!;
        }
        return '__REX_SSR_ASYNC__';
    }

    // Synchronous result — render directly
    try {
        var html = __rex_renderToString(treeResult);
        _ssrResult = JSON.stringify({ body: html, head: '', flight: flightString });
    } catch(e) {
        _ssrResult = JSON.stringify({
            body: '<div style="color:red">SSR render error: ' + String(e).replace(/</g, '&lt;') + '</div>',
            head: '',
            flight: flightString
        });
    }
    _ssrPending = false;
    return _ssrResult!;
};

globalThis.__rex_resolve_ssr_pending = function(): "pending" | "done" {
    return _ssrPending ? 'pending' : 'done';
};

globalThis.__rex_finalize_ssr = function(): string {
    return _ssrResult || JSON.stringify({ body: '', head: '', flight: '' });
};
