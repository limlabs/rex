
globalThis.__rex_mw_resolved = null;
globalThis.__rex_mw_rejected = null;

globalThis.__rex_run_middleware = function(reqJson) {
    var mw = globalThis.__rex_middleware;
    if (!mw) return JSON.stringify({ action: 'next' });

    var middlewareFn = mw.middleware || mw.default;
    if (!middlewareFn) return JSON.stringify({ action: 'next' });

    var reqData = JSON.parse(reqJson);
    var request = {
        method: reqData.method,
        url: reqData.url,
        headers: reqData.headers || {},
        cookies: reqData.cookies || {},
        nextUrl: { pathname: reqData.pathname || reqData.url }
    };

    var result = middlewareFn(request);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_mw_resolved = null;
        globalThis.__rex_mw_rejected = null;
        result.then(
            function(v) { globalThis.__rex_mw_resolved = v; },
            function(e) { globalThis.__rex_mw_rejected = e; }
        );
        return '__REX_MW_ASYNC__';
    }
    return JSON.stringify(__rex_serialize_mw(result));
};

globalThis.__rex_resolve_middleware = function() {
    if (globalThis.__rex_mw_rejected) throw globalThis.__rex_mw_rejected;
    if (globalThis.__rex_mw_resolved !== null) return JSON.stringify(__rex_serialize_mw(globalThis.__rex_mw_resolved));
    throw new Error('Middleware promise did not resolve');
};

function __rex_serialize_mw(res) {
    if (!res || !res._action) return { action: 'next' };
    return {
        action: res._action,
        url: res._url || null,
        status: res._status || 307,
        request_headers: res._requestHeaders || {},
        response_headers: res._responseHeaders || {}
    };
}
