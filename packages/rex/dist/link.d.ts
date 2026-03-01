import React from 'react';
interface LinkProps extends React.AnchorHTMLAttributes<HTMLAnchorElement> {
    href: string;
}
/**
 * rex/link - Client-side navigation link.
 * Renders an <a> tag that intercepts clicks for SPA navigation.
 */
export default function Link({ href, children, target, ...rest }: LinkProps): React.ReactElement;
export {};
//# sourceMappingURL=link.d.ts.map