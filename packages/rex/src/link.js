import React from 'react';
import { navigateTo } from './router.js';

/**
 * rex/link - Client-side navigation link
 * Renders an <a> tag that intercepts clicks for SPA navigation.
 */
export default function Link({ href, children, target, rel, className, style, ...rest }) {
  function handleClick(e) {
    // Don't intercept if:
    // - modifier key is held (user wants new tab)
    // - target is set (external link behavior)
    // - href is external
    if (
      e.metaKey ||
      e.ctrlKey ||
      e.shiftKey ||
      e.altKey ||
      target === '_blank' ||
      (href && (href.startsWith('http://') || href.startsWith('https://')))
    ) {
      return;
    }

    e.preventDefault();
    navigateTo(href);
  }

  return React.createElement(
    'a',
    {
      href: href,
      onClick: handleClick,
      target: target,
      rel: rel,
      className: className,
      style: style,
      ...rest,
    },
    children
  );
}
