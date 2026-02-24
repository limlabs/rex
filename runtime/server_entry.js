// Rex Server Entry Template
// This is the template used by rex_build to generate the actual server entry.
// In the build process, page imports are injected dynamically.
'use strict';

globalThis.__rex_pages = globalThis.__rex_pages || {};

// __rex_render_page(routeKey, propsJson) -> HTML string
globalThis.__rex_render_page = function(routeKey, propsJson) {
    var React = globalThis.__React;
    var ReactDOMServer = globalThis.__ReactDOMServer;

    var page = globalThis.__rex_pages[routeKey];
    if (!page) {
        throw new Error('Page not found: ' + routeKey);
    }

    var Component = page.default;
    var props = JSON.parse(propsJson);
    var element = React.createElement(Component, props);

    if (globalThis.__rex_app && globalThis.__rex_app.default) {
        var App = globalThis.__rex_app.default;
        element = React.createElement(App, { Component: Component, pageProps: props });
    }

    return ReactDOMServer.renderToString(element);
};

// __rex_get_server_side_props(routeKey, contextJson) -> JSON string
globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) {
        return JSON.stringify({ props: {} });
    }

    var context = JSON.parse(contextJson);
    var result = page.getServerSideProps(context);

    if (result && typeof result.then === 'function') {
        var resolved;
        result.then(function(v) { resolved = v; });
        if (resolved !== undefined) {
            return JSON.stringify(resolved);
        }
        throw new Error('getServerSideProps returned an unresolved promise');
    }

    return JSON.stringify(result);
};
