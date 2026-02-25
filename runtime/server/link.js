// Server-side Link component for rex/link
// Renders a plain <a> tag during SSR.
import { createElement } from 'react';

export default function Link(props) {
    var aProps = { href: props.href };
    if (props.className) aProps.className = props.className;
    if (props.style) aProps.style = props.style;
    if (props.id) aProps.id = props.id;
    if (props.target) aProps.target = props.target;
    return createElement('a', aProps, props.children);
}
