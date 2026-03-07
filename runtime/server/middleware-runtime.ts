/* eslint-disable @typescript-eslint/no-explicit-any */

declare var __rex_middleware: any | undefined;
declare var __rex_mw_resolved: any;
declare var __rex_mw_rejected: any;
declare var __rex_run_middleware: (reqJson: string) => string;
declare var __rex_resolve_middleware: () => string;

interface MiddlewareResult {
    _action?: string;
    _url?: string;
    _status?: number;
    _requestHeaders?: Record<string, string>;
    _responseHeaders?: Record<string, string>;
}

function __rex_serialize_mw(res: MiddlewareResult | null | undefined) {
    if (!res || !res._action) return { action: 'next' };
    return {
        action: res._action,
        url: res._url || null,
        status: res._status || 307,
        request_headers: res._requestHeaders || {},
        response_headers: res._responseHeaders || {}
    };
}

globalThis.__rex_mw_resolved = null;
globalThis.__rex_mw_rejected = null;

globalThis.__rex_run_middleware = function(reqJson: string): string {
    const mw = globalThis.__rex_middleware;
    if (!mw) return JSON.stringify({ action: 'next' });

    const middlewareFn = mw.middleware || mw.default;
    if (!middlewareFn) return JSON.stringify({ action: 'next' });

    const reqData = JSON.parse(reqJson);
    const request = {
        method: reqData.method,
        url: reqData.url,
        headers: reqData.headers || {},
        cookies: reqData.cookies || {},
        nextUrl: { pathname: reqData.pathname || reqData.url }
    };

    const result = middlewareFn(request);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_mw_resolved = null;
        globalThis.__rex_mw_rejected = null;
        result.then(
            function(v: unknown) { globalThis.__rex_mw_resolved = v; },
            function(e: unknown) { globalThis.__rex_mw_rejected = e; }
        );
        return '__REX_MW_ASYNC__';
    }
    return JSON.stringify(__rex_serialize_mw(result));
};

globalThis.__rex_resolve_middleware = function(): string {
    if (globalThis.__rex_mw_rejected) throw globalThis.__rex_mw_rejected;
    if (globalThis.__rex_mw_resolved !== null) return JSON.stringify(__rex_serialize_mw(globalThis.__rex_mw_resolved));
    throw new Error('Middleware promise did not resolve');
};
