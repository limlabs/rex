use rex_core::Route;
use rex_router::ScanResult;

/// Generate the server entry JS that registers all pages into a global registry
pub fn generate_server_entry(scan: &ScanResult) -> String {
    let mut js = String::new();

    js.push_str("// Rex Server Entry - Auto-generated\n");
    js.push_str("'use strict';\n\n");

    // Page registry
    js.push_str("globalThis.__rex_pages = {};\n\n");

    // Register each page
    for (i, route) in scan.routes.iter().enumerate() {
        let module_name = route.module_name();
        let var_name = format!("__page_{i}");
        // The bundler will resolve these imports
        js.push_str(&format!(
            "import * as {var_name} from '{}';\n",
            route.abs_path.display()
        ));
        js.push_str(&format!(
            "globalThis.__rex_pages['{}'] = {var_name};\n",
            module_name
        ));
    }

    // Register _app if present
    if let Some(app) = &scan.app {
        js.push_str(&format!(
            "\nimport * as __app from '{}';\n",
            app.abs_path.display()
        ));
        js.push_str("globalThis.__rex_app = __app;\n");
    }

    js.push_str("\n");

    // __rex_render_page function
    js.push_str(
        r#"
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

    // Wrap with _app if present
    if (globalThis.__rex_app) {
        var App = globalThis.__rex_app.default;
        element = React.createElement(App, { Component: Component, pageProps: props });
    }

    return ReactDOMServer.renderToString(element);
};

globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) {
        return JSON.stringify({ props: {} });
    }

    var context = JSON.parse(contextJson);
    var result = page.getServerSideProps(context);

    // Handle both sync and promise results
    if (result && typeof result.then === 'function') {
        // For sync execution in V8, we expect the value to resolve immediately
        var resolved;
        result.then(function(v) { resolved = v; });
        if (resolved !== undefined) {
            return JSON.stringify(resolved);
        }
        throw new Error('getServerSideProps returned an unresolved promise. Use synchronous operations or __rex_fetch_sync.');
    }

    return JSON.stringify(result);
};
"#,
    );

    js
}

/// Generate a client entry JS for a specific page
pub fn generate_client_entry(route: &Route, app: Option<&Route>, build_id: &str) -> String {
    let _module_name = route.module_name();
    let mut js = String::new();

    js.push_str("// Rex Client Entry - Auto-generated\n");
    js.push_str("import React from 'react';\n");
    js.push_str("import { hydrateRoot } from 'react-dom/client';\n");
    js.push_str(&format!(
        "import Page from '{}';\n",
        route.abs_path.display()
    ));

    if let Some(app) = app {
        js.push_str(&format!(
            "import App from '{}';\n",
            app.abs_path.display()
        ));
    }

    js.push_str(&format!(
        r#"
var dataEl = document.getElementById('__REX_DATA__');
var pageProps = dataEl ? JSON.parse(dataEl.textContent) : {{}};

var container = document.getElementById('__rex');
var element;
"#
    ));

    if app.is_some() {
        js.push_str(
            "element = React.createElement(App, { Component: Page, pageProps: pageProps });\n",
        );
    } else {
        js.push_str("element = React.createElement(Page, pageProps);\n");
    }

    js.push_str(&format!(
        r#"
var root = hydrateRoot(container, element);
window.__REX_ROOT__ = root;
window.__REX_BUILD_ID__ = '{}';
"#,
        build_id
    ));

    js
}

/// Generate a build ID based on current timestamp
pub fn generate_build_id() -> String {
    use sha2::{Digest, Sha256};
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let hash = Sha256::digest(timestamp.to_string().as_bytes());
    hex::encode(&hash[..8])
}
