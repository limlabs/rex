
// SSR render function — returns JSON { body, head }
globalThis.__rex_render_page = function(routeKey, propsJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page) throw new Error('Page not found in registry: ' + routeKey);
    var Component = page.default;
    if (!Component) throw new Error('Page has no default export: ' + routeKey);

    var props = JSON.parse(propsJson);
    var element = __rex_createElement(Component, props);

    if (globalThis.__rex_app && globalThis.__rex_app.default) {
        element = __rex_createElement(globalThis.__rex_app.default, {
            Component: Component, pageProps: props
        });
    }

    globalThis.__rex_head_elements = [];
    var bodyHtml = __rex_renderToString(element);

    var headHtml = '';
    for (var i = 0; i < globalThis.__rex_head_elements.length; i++) {
        headHtml += __rex_renderToString(globalThis.__rex_head_elements[i]);
    }

    return JSON.stringify({ body: bodyHtml, head: headHtml });
};

globalThis.__rex_gssp_resolved = null;
globalThis.__rex_gssp_rejected = null;

globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) return JSON.stringify({ props: {} });

    var context = JSON.parse(contextJson);
    var result = page.getServerSideProps(context);

    if (result && typeof result.then === 'function') {
        globalThis.__rex_gssp_resolved = null;
        globalThis.__rex_gssp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gssp_resolved = v; },
            function(e) { globalThis.__rex_gssp_rejected = e; }
        );
        return '__REX_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gssp = function() {
    if (globalThis.__rex_gssp_rejected) throw globalThis.__rex_gssp_rejected;
    if (globalThis.__rex_gssp_resolved !== null) return JSON.stringify(globalThis.__rex_gssp_resolved);
    throw new Error('getServerSideProps promise did not resolve after microtask checkpoint');
};

globalThis.__rex_call_api_handler = function(routeKey, reqJson) {
    var handlers = globalThis.__rex_api_handlers;
    if (!handlers) throw new Error('No API handlers registered');
    var handler = handlers[routeKey];
    if (!handler) throw new Error('API handler not found: ' + routeKey);
    var handlerFn = handler.default;
    if (!handlerFn) throw new Error('No default export for API route: ' + routeKey);

    var reqData = JSON.parse(reqJson);
    var res = {
        _statusCode: 200, _headers: {}, _body: '',
        status: function(code) { this._statusCode = code; return this; },
        setHeader: function(name, value) { this._headers[name.toLowerCase()] = value; return this; },
        json: function(data) { this._headers['content-type'] = 'application/json'; this._body = JSON.stringify(data); return this; },
        send: function(body) { if (typeof body === 'object' && !this._headers['content-type']) return this.json(body); this._body = typeof body === 'string' ? body : String(body); return this; },
        end: function(body) { if (body !== undefined) this._body = String(body); return this; },
        redirect: function(statusOrUrl, maybeUrl) { if (typeof statusOrUrl === 'string') { this._statusCode = 307; this._headers['location'] = statusOrUrl; } else { this._statusCode = statusOrUrl; this._headers['location'] = maybeUrl; } return this; }
    };
    var req = { method: reqData.method, url: reqData.url, headers: reqData.headers || {}, query: reqData.query || {}, body: reqData.body, cookies: reqData.cookies || {} };

    var result = handlerFn(req, res);
    if (result && typeof result.then === 'function') {
        globalThis.__rex_api_resolved = null;
        globalThis.__rex_api_rejected = null;
        result.then(function() { globalThis.__rex_api_resolved = { statusCode: res._statusCode, headers: res._headers, body: res._body }; }, function(e) { globalThis.__rex_api_rejected = e; });
        return '__REX_API_ASYNC__';
    }
    return JSON.stringify({ statusCode: res._statusCode, headers: res._headers, body: res._body });
};

globalThis.__rex_resolve_api = function() {
    if (globalThis.__rex_api_rejected) throw globalThis.__rex_api_rejected;
    if (globalThis.__rex_api_resolved !== null) return JSON.stringify(globalThis.__rex_api_resolved);
    throw new Error('API handler promise did not resolve');
};

// getStaticProps execution (parallel structure to GSSP)
globalThis.__rex_gsp_resolved = null;
globalThis.__rex_gsp_rejected = null;

globalThis.__rex_get_static_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getStaticProps) return JSON.stringify({ props: {} });

    var context = JSON.parse(contextJson);
    var result = page.getStaticProps(context);

    if (result && typeof result.then === 'function') {
        globalThis.__rex_gsp_resolved = null;
        globalThis.__rex_gsp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gsp_resolved = v; },
            function(e) { globalThis.__rex_gsp_rejected = e; }
        );
        return '__REX_GSP_ASYNC__';
    }
    return JSON.stringify(result);
};

globalThis.__rex_resolve_gsp = function() {
    if (globalThis.__rex_gsp_rejected) throw globalThis.__rex_gsp_rejected;
    if (globalThis.__rex_gsp_resolved !== null) return JSON.stringify(globalThis.__rex_gsp_resolved);
    throw new Error('getStaticProps promise did not resolve after microtask checkpoint');
};
