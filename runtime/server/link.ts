// Server-side Link component for rex/link
// Renders a plain <a> tag during SSR.
import { createElement, type ReactElement, type ReactNode } from "react";

interface LinkProps {
  href: string;
  children?: ReactNode;
  className?: string;
  style?: Record<string, string>;
  id?: string;
  target?: string;
}

export default function Link(props: LinkProps): ReactElement {
  const aProps: Record<string, unknown> = { href: props.href };
  if (props.className) aProps.className = props.className;
  if (props.style) aProps.style = props.style;
  if (props.id) aProps.id = props.id;
  if (props.target) aProps.target = props.target;
  return createElement("a", aProps, props.children);
}
