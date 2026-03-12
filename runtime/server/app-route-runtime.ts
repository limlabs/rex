// App router route handler (route.ts) — method-based dispatch.
// Route handlers export named HTTP methods: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS.
// Each handler receives (request, context) and returns a Response-like object.
/* eslint-disable no-var -- ambient declarations require `declare var` in TypeScript */
declare var __rex_app_route_handlers: Record<string, any> | undefined;
declare var __rex_call_app_route_handler: (routePattern: string, reqJson: string) => string;
declare var __rex_app_route_resolved: any;
declare var __rex_app_route_rejected: any;
declare var __rex_resolve_app_route: () => string;
/* eslint-enable no-var */

globalThis.__rex_call_app_route_handler = function(routePattern: string, reqJson: string): string {
    const handlers = globalThis.__rex_app_route_handlers;
    if (!handlers) throw new Error('No app route handlers registered');
    const routeModule = handlers[routePattern];
    if (!routeModule) throw new Error('App route handler not found: ' + routePattern);

    const reqData = JSON.parse(reqJson);
    const method = (reqData.method || 'GET').toUpperCase();
    const handlerFn = routeModule[method];
    if (!handlerFn) {
        // Return 405 Method Not Allowed with Allow header listing available methods
        const allowed = ['GET','HEAD','POST','PUT','DELETE','PATCH','OPTIONS'].filter(function(m: string) { return typeof routeModule[m] === 'function'; });
        return JSON.stringify({ statusCode: 405, headers: { allow: allowed.join(', ') }, body: 'Method Not Allowed' });
    }

    // Build a Request-like object
    const request = {
        method: method,
        url: reqData.url || '/',
        headers: reqData.headers || {},
        json: function() { return typeof reqData.body === 'string' ? JSON.parse(reqData.body) : reqData.body; },
        text: function() { return typeof reqData.body === 'string' ? reqData.body : JSON.stringify(reqData.body); },
        formData: function() { return reqData.body; },
        nextUrl: { pathname: reqData.url || '/', searchParams: reqData.query || {} }
    };
    // Route context with params
    const context = { params: reqData.params || {} };

    function serializeResponse(resp: any): string {
        if (resp && typeof resp === 'object') {
            const statusCode = resp.status || resp.statusCode || 200;
            const headers = resp.headers || {};
            let body = '';
            if (resp.body !== undefined && resp.body !== null) {
                body = typeof resp.body === 'string' ? resp.body : JSON.stringify(resp.body);
            } else if (resp._body !== undefined) {
                body = resp._body;
            }
            return JSON.stringify({ statusCode: statusCode, headers: headers, body: body });
        }
        return JSON.stringify({ statusCode: 200, headers: {}, body: String(resp || '') });
    }

    const result = handlerFn(request, context);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_app_route_resolved = null;
        globalThis.__rex_app_route_rejected = null;
        result.then(
            function(v: unknown) { globalThis.__rex_app_route_resolved = v; },
            function(e: unknown) { globalThis.__rex_app_route_rejected = e; }
        );
        return '__REX_APP_ROUTE_ASYNC__';
    }
    return serializeResponse(result);
};

globalThis.__rex_resolve_app_route = function(): string {
    if (globalThis.__rex_app_route_rejected) throw globalThis.__rex_app_route_rejected;
    if (globalThis.__rex_app_route_resolved !== null) {
        const resp = globalThis.__rex_app_route_resolved;
        const statusCode = resp.status || resp.statusCode || 200;
        const headers = resp.headers || {};
        let body = '';
        if (resp.body !== undefined && resp.body !== null) {
            body = typeof resp.body === 'string' ? resp.body : JSON.stringify(resp.body);
        } else if (resp._body !== undefined) {
            body = resp._body;
        }
        return JSON.stringify({ statusCode: statusCode, headers: headers, body: body });
    }
    throw new Error('App route handler promise did not resolve');
};
