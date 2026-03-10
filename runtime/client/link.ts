"use client";
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

// Base path for sites deployed under a subpath (e.g. "/rex" for user.github.io/rex/).
// Set by `rex export --base-path` via an inline <script> in the HTML head.
function getBasePath(): string {
  return (typeof window !== "undefined" && window.__REX_BASE_PATH) || "";
}

function withBasePath(href: string): string {
  const bp = getBasePath();
  if (!bp || !href.startsWith("/") || href.startsWith("//")) return href;
  return bp + href;
}

export default function Link(props: LinkProps): React.ReactElement {
  const { href, replace = false, children, target } = props;
  const resolvedHref = withBasePath(href);

  const aProps: Record<string, unknown> = { href: resolvedHref };
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
      const url = new URL(resolvedHref, window.location.origin);
      if (url.origin !== window.location.origin) return;
    } catch {
      return;
    }

    e.preventDefault();

    // RSC app router navigation (fetches flight data, re-renders in place)
    const rscNavigate = window.__REX_RSC_NAVIGATE;
    if (rscNavigate) {
      if (replace) {
        history.replaceState(null, "", resolvedHref);
      } else {
        history.pushState(null, "", resolvedHref);
      }
      rscNavigate(resolvedHref);
      return;
    }

    // Pages router navigation
    const router = window.__REX_ROUTER;
    if (router) {
      if (replace) {
        router.replace(resolvedHref);
      } else {
        router.push(resolvedHref);
      }
    } else {
      window.location.href = resolvedHref;
    }
  };

  aProps.onMouseEnter = function () {
    // RSC routes: no prefetch yet (could add flight data prefetch later)
    if (window.__REX_RSC_NAVIGATE) return;
    const router = window.__REX_ROUTER;
    if (router && router.prefetch) {
      router.prefetch(resolvedHref);
    }
  };

  return React.createElement("a", aProps, children);
}
