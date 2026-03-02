// Server-side Head component for rex/head
// Collects <title>, <meta>, etc. during SSR for injection into <head>.
import type { ReactElement, ReactNode } from "react";

interface HeadProps {
  children?: ReactNode;
}

globalThis.__rex_head_elements = [];

export default function Head(props: HeadProps): null {
  if (props.children) {
    const children = Array.isArray(props.children)
      ? props.children
      : [props.children];
    for (let i = 0; i < children.length; i++) {
      if (children[i])
        globalThis.__rex_head_elements.push(children[i] as ReactElement);
    }
  }
  return null;
}
