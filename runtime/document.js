(function() {
    'use strict';

    // Side-channel collectors for _document rendering
    globalThis.__rex_doc_html_attrs = {};
    globalThis.__rex_doc_body_attrs = {};
    globalThis.__rex_doc_head_elements = [];

    // Html component — captures html attributes (lang, dir, etc.)
    function Html(props) {
        if (props) {
            var attrs = {};
            for (var k in props) {
                if (k !== 'children' && props.hasOwnProperty(k)) {
                    attrs[k] = props[k];
                }
            }
            globalThis.__rex_doc_html_attrs = attrs;
        }
        return props && props.children ? props.children : null;
    }
    Html.displayName = 'Html';

    // Head component — captures extra head elements (links, meta, etc.)
    function Head(props) {
        if (props && props.children) {
            var children = Array.isArray(props.children) ? props.children : [props.children];
            for (var i = 0; i < children.length; i++) {
                if (children[i]) {
                    globalThis.__rex_doc_head_elements.push(children[i]);
                }
            }
        }
        return null;
    }
    Head.displayName = 'Head';

    // Main component — placeholder, Rust handles the actual content injection
    function Main() {
        return null;
    }
    Main.displayName = 'Main';

    // NextScript component — placeholder, Rust handles script injection
    function NextScript() {
        return null;
    }
    NextScript.displayName = 'NextScript';

    // Export for require('rex/document')
    globalThis.__rex_document_components = {
        Html: Html,
        Head: Head,
        Main: Main,
        NextScript: NextScript
    };

    // __rex_render_document: call the user's _document component to extract attrs
    // Returns JSON descriptor: { htmlAttrs, bodyAttrs, headContent }
    globalThis.__rex_render_document = function() {
        var React = globalThis.__React;
        var ReactDOMServer = globalThis.__ReactDOMServer;
        if (!React || !ReactDOMServer) {
            throw new Error('React not loaded for document rendering');
        }

        var doc = globalThis.__rex_document;
        if (!doc || !doc.default) {
            return JSON.stringify({ htmlAttrs: {}, bodyAttrs: {}, headContent: '' });
        }

        // Reset collectors
        globalThis.__rex_doc_html_attrs = {};
        globalThis.__rex_doc_body_attrs = {};
        globalThis.__rex_doc_head_elements = [];

        // Render the document component — this triggers the collectors
        var element = React.createElement(doc.default, {});
        ReactDOMServer.renderToString(element);

        // Render collected head elements to HTML
        var headContent = '';
        for (var i = 0; i < globalThis.__rex_doc_head_elements.length; i++) {
            headContent += ReactDOMServer.renderToString(globalThis.__rex_doc_head_elements[i]);
        }

        // Convert className to class for HTML output
        var bodyAttrs = {};
        var rawBody = globalThis.__rex_doc_body_attrs;
        for (var k in rawBody) {
            if (rawBody.hasOwnProperty(k)) {
                if (k === 'className') {
                    bodyAttrs['class'] = rawBody[k];
                } else {
                    bodyAttrs[k] = rawBody[k];
                }
            }
        }

        return JSON.stringify({
            htmlAttrs: globalThis.__rex_doc_html_attrs,
            bodyAttrs: bodyAttrs,
            headContent: headContent
        });
    };
})();
