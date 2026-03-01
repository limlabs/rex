import React from 'react';
import { navigateTo } from './router.js';
/**
 * rex/link - Client-side navigation link.
 * Renders an <a> tag that intercepts clicks for SPA navigation.
 */
export default function Link({ href, children, target, ...rest }) {
    function handleClick(e) {
        if (e.metaKey ||
            e.ctrlKey ||
            e.shiftKey ||
            e.altKey ||
            target === '_blank' ||
            (href && (href.startsWith('http://') || href.startsWith('https://')))) {
            return;
        }
        e.preventDefault();
        navigateTo(href);
    }
    return React.createElement('a', {
        href,
        onClick: handleClick,
        target,
        ...rest,
    }, children);
}
//# sourceMappingURL=link.js.map