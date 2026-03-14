// next/link → Rex Link stub for server bundles.
// Renders a plain <a> tag for SSR.
import { createElement, type ReactElement } from "react";

/* eslint-disable @typescript-eslint/no-explicit-any */

function Link(props: any): ReactElement {
    const { href, children, prefetch: _prefetch, replace: _replace, scroll: _scroll, shallow: _shallow, passHref: _passHref, legacyBehavior: _legacyBehavior, ...rest } = props;
    return createElement('a', { href, ...rest }, children);
}

Link.displayName = 'Link';

export default Link;
export { Link };
