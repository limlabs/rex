import React from 'react';
import { navigateTo } from './router.js';

interface LinkProps extends React.AnchorHTMLAttributes<HTMLAnchorElement> {
  href: string;
}

/**
 * rex/link - Client-side navigation link.
 * Renders an <a> tag that intercepts clicks for SPA navigation.
 */
export default function Link({ href, children, target, ...rest }: LinkProps): React.ReactElement {
  function handleClick(e: React.MouseEvent<HTMLAnchorElement>): void {
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
      href,
      onClick: handleClick,
      target,
      ...rest,
    },
    children
  );
}
