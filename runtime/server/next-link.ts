// next/link → Rex Link stub for server bundles.
// Renders a plain <a> tag for SSR.

/* eslint-disable @typescript-eslint/no-explicit-any */

function Link(props: any) {
    const { href, children, ...rest } = props;
    // Server-side: use React.createElement to produce a valid React element
    const createElement = globalThis.React?.createElement;
    if (createElement) {
        return createElement('a', { href, ...rest }, ...(Array.isArray(children) ? children : [children]));
    }
    // Fallback: plain object (matches React element shape)
    return { type: 'a', props: { href, ...rest }, children };
}

Link.displayName = 'Link';

export default Link;
export { Link };
