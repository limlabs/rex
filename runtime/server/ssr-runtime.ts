/* eslint-disable @typescript-eslint/no-explicit-any */

/* eslint-disable no-var -- ambient declarations require `declare var` in TypeScript */
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
/* eslint-enable no-var */

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

// getStaticPaths execution
globalThis.__rex_gsp_paths_resolved = null;
globalThis.__rex_gsp_paths_rejected = null;

globalThis.__rex_get_static_paths = function(routeKey: string): string {
    const page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getStaticPaths) return JSON.stringify({ paths: [], fallback: false });
    const result = page.getStaticPaths();
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gsp_paths_resolved = null;
        globalThis.__rex_gsp_paths_rejected = null;
        result.then(
            (v: unknown) => { globalThis.__rex_gsp_paths_resolved = v; },
            (e: unknown) => { globalThis.__rex_gsp_paths_rejected = e; }
        );
        return '__REX_GSP_PATHS_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_static_paths = function(): string {
    if (globalThis.__rex_gsp_paths_rejected) throw globalThis.__rex_gsp_paths_rejected;
    if (globalThis.__rex_gsp_paths_resolved !== null) return JSON.stringify(globalThis.__rex_gsp_paths_resolved);
    throw new Error('getStaticPaths promise did not resolve');
};
