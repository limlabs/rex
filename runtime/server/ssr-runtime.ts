/* eslint-disable @typescript-eslint/no-explicit-any */

declare var __rex_pages: Record<string, any>;
declare var __rex_app: { default?: any } | undefined;
declare var __rex_api_handlers: Record<string, any> | undefined;
declare var __rex_render_page: (routeKey: string, propsJson: string) => string;
declare var __rex_gssp_resolved: any;
declare var __rex_gssp_rejected: any;
declare var __rex_get_server_side_props: (routeKey: string, contextJson: string) => string;
declare var __rex_resolve_gssp: () => string;
declare var __rex_call_api_handler: (routeKey: string, reqJson: string) => string;
declare var __rex_api_resolved: any;
declare var __rex_api_rejected: any;
declare var __rex_resolve_api: () => string;
declare var __rex_gsp_resolved: any;
declare var __rex_gsp_rejected: any;
declare var __rex_get_static_props: (routeKey: string, contextJson: string) => string;
declare var __rex_resolve_gsp: () => string;

// SSR render function — returns JSON { body, head }
globalThis.__rex_render_page = function(routeKey: string, propsJson: string): string {
    const page = globalThis.__rex_pages[routeKey];
    if (!page) throw new Error('Page not found in registry: ' + routeKey);
    const Component = page.default;
    if (!Component) throw new Error('Page has no default export: ' + routeKey);

    const props = JSON.parse(propsJson);
    let element = __rex_createElement(Component, props);

    if (globalThis.__rex_app && globalThis.__rex_app.default) {
        element = __rex_createElement(globalThis.__rex_app.default, {
            Component: Component, pageProps: props
        });
    }

    globalThis.__rex_head_elements = [];
    const bodyHtml = __rex_renderToString(element);

    let headHtml = '';
    for (let i = 0; i < globalThis.__rex_head_elements.length; i++) {
        headHtml += __rex_renderToString(globalThis.__rex_head_elements[i]);
    }

    return JSON.stringify({ body: bodyHtml, head: headHtml });
};

globalThis.__rex_gssp_resolved = null;
globalThis.__rex_gssp_rejected = null;

globalThis.__rex_get_server_side_props = function(routeKey: string, contextJson: string): string {
    const page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) return JSON.stringify({ props: {} });

    const context = JSON.parse(contextJson);
    const result = page.getServerSideProps(context);

    if (result && typeof result.then === 'function') {
        globalThis.__rex_gssp_resolved = null;
        globalThis.__rex_gssp_rejected = null;
        result.then(
            function(v: unknown) { globalThis.__rex_gssp_resolved = v; },
            function(e: unknown) { globalThis.__rex_gssp_rejected = e; }
        );
        return '__REX_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gssp = function(): string {
    if (globalThis.__rex_gssp_rejected) throw globalThis.__rex_gssp_rejected;
    if (globalThis.__rex_gssp_resolved !== null) return JSON.stringify(globalThis.__rex_gssp_resolved);
    throw new Error('getServerSideProps promise did not resolve after microtask checkpoint');
};

globalThis.__rex_call_api_handler = function(routeKey: string, reqJson: string): string {
    const handlers = globalThis.__rex_api_handlers;
    if (!handlers) throw new Error('No API handlers registered');
    const handler = handlers[routeKey];
    if (!handler) throw new Error('API handler not found: ' + routeKey);
    const handlerFn = handler.default;
    if (!handlerFn) throw new Error('No default export for API route: ' + routeKey);

    const reqData = JSON.parse(reqJson);
    const res = {
        _statusCode: 200, _headers: {} as Record<string, string>, _body: '',
        status(code: number) { this._statusCode = code; return this; },
        setHeader(name: string, value: string) { this._headers[name.toLowerCase()] = value; return this; },
        json(data: unknown) { this._headers['content-type'] = 'application/json'; this._body = JSON.stringify(data); return this; },
        send(body: unknown) { if (typeof body === 'object' && !this._headers['content-type']) return this.json(body); this._body = typeof body === 'string' ? body : String(body); return this; },
        end(body?: unknown) { if (body !== undefined) this._body = String(body); return this; },
        redirect(statusOrUrl: number | string, maybeUrl?: string) { if (typeof statusOrUrl === 'string') { this._statusCode = 307; this._headers['location'] = statusOrUrl; } else { this._statusCode = statusOrUrl; this._headers['location'] = maybeUrl!; } return this; }
    };
    const req = { method: reqData.method, url: reqData.url, headers: reqData.headers || {}, query: reqData.query || {}, body: reqData.body, cookies: reqData.cookies || {} };

    const result = handlerFn(req, res);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_api_resolved = null;
        globalThis.__rex_api_rejected = null;
        result.then(function() { globalThis.__rex_api_resolved = { statusCode: res._statusCode, headers: res._headers, body: res._body }; }, function(e: unknown) { globalThis.__rex_api_rejected = e; });
        return '__REX_API_ASYNC__';
    }
    return JSON.stringify({ statusCode: res._statusCode, headers: res._headers, body: res._body });
};

globalThis.__rex_resolve_api = function(): string {
    if (globalThis.__rex_api_rejected) throw globalThis.__rex_api_rejected;
    if (globalThis.__rex_api_resolved !== null) return JSON.stringify(globalThis.__rex_api_resolved);
    throw new Error('API handler promise did not resolve');
};

// App router route handler (route.ts) — method-based dispatch.
// Route handlers export named HTTP methods: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS.
// Each handler receives (request, context) and returns a Response-like object.
declare var __rex_app_route_handlers: Record<string, any> | undefined;
declare var __rex_call_app_route_handler: (routePattern: string, reqJson: string) => string;
declare var __rex_app_route_resolved: any;
declare var __rex_app_route_rejected: any;
declare var __rex_resolve_app_route: () => string;

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

// getStaticProps execution (parallel structure to GSSP)
globalThis.__rex_gsp_resolved = null;
globalThis.__rex_gsp_rejected = null;

globalThis.__rex_get_static_props = function(routeKey: string, contextJson: string): string {
    const page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getStaticProps) return JSON.stringify({ props: {} });

    const context = JSON.parse(contextJson);
    const result = page.getStaticProps(context);

    if (result && typeof result.then === 'function') {
        globalThis.__rex_gsp_resolved = null;
        globalThis.__rex_gsp_rejected = null;
        result.then(
            function(v: unknown) { globalThis.__rex_gsp_resolved = v; },
            function(e: unknown) { globalThis.__rex_gsp_rejected = e; }
        );
        return '__REX_GSP_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gsp = function(): string {
    if (globalThis.__rex_gsp_rejected) throw globalThis.__rex_gsp_rejected;
    if (globalThis.__rex_gsp_resolved !== null) return JSON.stringify(globalThis.__rex_gsp_resolved);
    throw new Error('getStaticProps promise did not resolve after microtask checkpoint');
};
