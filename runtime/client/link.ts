// Rex Link Component - Client-side navigation
// Equivalent to next/link: renders <a> with client-side nav on click.
import React from "react";

interface LinkProps {
  href: string;
  replace?: boolean;
  children?: React.ReactNode;
  target?: string;
  className?: string;
  style?: React.CSSProperties;
  id?: string;
  onClick?: (e: React.MouseEvent<HTMLAnchorElement>) => void;
}

export default function Link(props: LinkProps): React.ReactElement {
  const { href, replace = false, children, target } = props;

  const aProps: Record<string, unknown> = { href };
  if (props.className) aProps.className = props.className;
  if (props.style) aProps.style = props.style;
  if (props.id) aProps.id = props.id;
  if (target) aProps.target = target;

  aProps.onClick = function (e: React.MouseEvent<HTMLAnchorElement>) {
    if (props.onClick) props.onClick(e);
    if (e.defaultPrevented) return;
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    if (e.button !== 0) return;
    if (target && target !== "_self") return;

    try {
      const url = new URL(href, window.location.origin);
      if (url.origin !== window.location.origin) return;
    } catch {
      return;
    }

    e.preventDefault();

    const router = window.__REX_ROUTER;
    if (router) {
      if (replace) {
        router.replace(href);
      } else {
        router.push(href);
      }
    } else {
      window.location.href = href;
    }
  };

  aProps.onMouseEnter = function () {
    const router = window.__REX_ROUTER;
    if (router && router.prefetch) {
      router.prefetch(href);
    }
  };

  return React.createElement("a", aProps, children);
}
