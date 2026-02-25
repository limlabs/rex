// Rex Head Component - Server-side implementation
// Equivalent to next/head: collects <title>, <meta>, etc. during SSR
// and passes them to the document assembly for injection into <head>.
(function() {
    'use strict';

    // Reset before each render; read after renderToString completes
    globalThis.__rex_head_elements = [];

    function Head(props) {
        if (props.children) {
            var children = Array.isArray(props.children) ? props.children : [props.children];
            for (var i = 0; i < children.length; i++) {
                if (children[i]) {
                    globalThis.__rex_head_elements.push(children[i]);
                }
            }
        }
        return null;
    }

    Head.displayName = 'Head';

    // Available via require('rex/head')
    globalThis.__rex_head_component = Head;
})();
