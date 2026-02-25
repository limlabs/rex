// Server-side Head component for rex/head
// Collects <title>, <meta>, etc. during SSR for injection into <head>.
globalThis.__rex_head_elements = [];

export default function Head(props) {
    if (props.children) {
        var children = Array.isArray(props.children) ? props.children : [props.children];
        for (var i = 0; i < children.length; i++) {
            if (children[i]) globalThis.__rex_head_elements.push(children[i]);
        }
    }
    return null;
}
