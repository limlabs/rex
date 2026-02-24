// Rex Server Bundle - Auto-generated
'use strict';

globalThis.__rex_pages = globalThis.__rex_pages || {};

// Page: blog/[slug]
globalThis.__rex_pages['blog/[slug]'] = (function() {
  var exports = {};
  var module = { exports: exports };
  (function(exports, module) {
    module.exports.default = function BlogPost({ slug, title }) {
        return /*#__PURE__*/ React.createElement("div", null, /*#__PURE__*/ React.createElement("h1", null, title), /*#__PURE__*/ React.createElement("p", null, "Slug: ", slug), /*#__PURE__*/ React.createElement("a", {
            href: "/"
        }, "Back to home"));
    }
    module.exports.getServerSideProps = async function getServerSideProps(context) {
        return {
            props: {
                slug: context.params.slug,
                title: `Blog Post: ${context.params.slug}`
            }
        };
    }
  })(exports, module);
  // Re-export
  if (module.exports.default) exports.default = module.exports.default;
  return exports;
})();

// Page: about
globalThis.__rex_pages['about'] = (function() {
  var exports = {};
  var module = { exports: exports };
  (function(exports, module) {
    module.exports.default = function About() {
        return /*#__PURE__*/ React.createElement("div", null, /*#__PURE__*/ React.createElement("h1", null, "About"), /*#__PURE__*/ React.createElement("p", null, "Rex is a Next.js Pages Router reimplemented in Rust."), /*#__PURE__*/ React.createElement("a", {
            href: "/"
        }, "Back to home"));
    }
  })(exports, module);
  // Re-export
  if (module.exports.default) exports.default = module.exports.default;
  return exports;
})();

// Page: index
globalThis.__rex_pages['index'] = (function() {
  var exports = {};
  var module = { exports: exports };
  (function(exports, module) {
    module.exports.default = function Home({ message, timestamp }) {
        return /*#__PURE__*/ React.createElement("div", null, /*#__PURE__*/ React.createElement("h1", null, "Rex"), /*#__PURE__*/ React.createElement("p", null, message), /*#__PURE__*/ React.createElement("p", null, "Rendered at: ", new Date(timestamp).toISOString()));
    }
    module.exports.getServerSideProps = async function getServerSideProps() {
        return {
            props: {
                message: 'Hello from Rex!',
                timestamp: Date.now()
            }
        };
    }
  })(exports, module);
  // Re-export
  if (module.exports.default) exports.default = module.exports.default;
  return exports;
})();


// SSR render function
globalThis.__rex_render_page = function(routeKey, propsJson) {
    var React = globalThis.__React;
    var ReactDOMServer = globalThis.__ReactDOMServer;
    if (!React || !ReactDOMServer) {
        throw new Error('React/ReactDOMServer not loaded. Ensure react runtime is evaluated first.');
    }

    var page = globalThis.__rex_pages[routeKey];
    if (!page) {
        throw new Error('Page not found in registry: ' + routeKey);
    }

    var Component = page.default;
    if (!Component) {
        throw new Error('Page has no default export: ' + routeKey);
    }

    var props = JSON.parse(propsJson);
    var element = React.createElement(Component, props);

    // Wrap with _app if present
    if (globalThis.__rex_app && globalThis.__rex_app.default) {
        var App = globalThis.__rex_app.default;
        element = React.createElement(App, { Component: Component, pageProps: props });
    }

    return ReactDOMServer.renderToString(element);
};

// getServerSideProps executor
globalThis.__rex_gssp_resolved = null;
globalThis.__rex_gssp_rejected = null;

globalThis.__rex_get_server_side_props = function(routeKey, contextJson) {
    var page = globalThis.__rex_pages[routeKey];
    if (!page || !page.getServerSideProps) {
        return JSON.stringify({ props: {} });
    }

    var context = JSON.parse(contextJson);
    var result = page.getServerSideProps(context);

    // Handle sync result or immediately-resolved promise
    if (result && typeof result.then === 'function') {
        globalThis.__rex_gssp_resolved = null;
        globalThis.__rex_gssp_rejected = null;
        result.then(
            function(v) { globalThis.__rex_gssp_resolved = v; },
            function(e) { globalThis.__rex_gssp_rejected = e; }
        );
        // Return sentinel — Rust will pump the microtask queue and call the resolver
        return '__REX_ASYNC__';
    }

    return JSON.stringify(result);
};

globalThis.__rex_resolve_gssp = function() {
    if (globalThis.__rex_gssp_rejected) {
        throw globalThis.__rex_gssp_rejected;
    }
    if (globalThis.__rex_gssp_resolved !== null) {
        return JSON.stringify(globalThis.__rex_gssp_resolved);
    }
    throw new Error('getServerSideProps promise did not resolve after microtask checkpoint');
};
