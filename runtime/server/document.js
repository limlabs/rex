// Server-side _document component helpers for rex/document
// Side-channel collectors capture attributes during SSR rendering.
import { createElement } from 'react';
import { renderToString } from 'react-dom/server';

globalThis.__rex_doc_html_attrs = {};
globalThis.__rex_doc_body_attrs = {};
globalThis.__rex_doc_head_elements = [];

export function Html(props) {
    if (props) {
        var attrs = {};
        for (var k in props) {
            if (k !== 'children' && props.hasOwnProperty(k)) attrs[k] = props[k];
        }
        globalThis.__rex_doc_html_attrs = attrs;
    }
    return props && props.children ? props.children : null;
}

export function Head(props) {
    if (props && props.children) {
        var children = Array.isArray(props.children) ? props.children : [props.children];
        for (var i = 0; i < children.length; i++) {
            if (children[i]) globalThis.__rex_doc_head_elements.push(children[i]);
        }
    }
    return null;
}

export function Main() { return null; }
export function NextScript() { return null; }

export default Html;

// Render a user's _document component and extract descriptor
globalThis.__rex_render_document = function() {
    var doc = globalThis.__rex_document;
    if (!doc || !doc.default) {
        return JSON.stringify({ htmlAttrs: {}, bodyAttrs: {}, headContent: '' });
    }
    globalThis.__rex_doc_html_attrs = {};
    globalThis.__rex_doc_body_attrs = {};
    globalThis.__rex_doc_head_elements = [];

    var element = createElement(doc.default, {});
    renderToString(element);

    var headContent = '';
    for (var i = 0; i < globalThis.__rex_doc_head_elements.length; i++) {
        headContent += renderToString(globalThis.__rex_doc_head_elements[i]);
    }

    var bodyAttrs = {};
    var rawBody = globalThis.__rex_doc_body_attrs;
    for (var k in rawBody) {
        if (rawBody.hasOwnProperty(k)) {
            bodyAttrs[k === 'className' ? 'class' : k] = rawBody[k];
        }
    }

    return JSON.stringify({
        htmlAttrs: globalThis.__rex_doc_html_attrs,
        bodyAttrs: bodyAttrs,
        headContent: headContent
    });
};
