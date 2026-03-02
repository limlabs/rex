// Server-side _document component helpers for rex/document
// Side-channel collectors capture attributes during SSR rendering.
import { createElement, type ReactElement, type ReactNode } from "react";
import { renderToString } from "react-dom/server";

interface DocumentProps {
  children?: ReactNode;
  [key: string]: unknown;
}

globalThis.__rex_doc_html_attrs = {};
globalThis.__rex_doc_body_attrs = {};
globalThis.__rex_doc_head_elements = [];

export function Html(props: DocumentProps): ReactNode {
  if (props) {
    const attrs: Record<string, string> = {};
    for (const k in props) {
      if (k !== "children" && Object.prototype.hasOwnProperty.call(props, k))
        attrs[k] = props[k] as string;
    }
    globalThis.__rex_doc_html_attrs = attrs;
  }
  return props && props.children ? (props.children as ReactNode) : null;
}

export function Head(props: DocumentProps): null {
  if (props && props.children) {
    const children = Array.isArray(props.children)
      ? props.children
      : [props.children];
    for (let i = 0; i < children.length; i++) {
      if (children[i])
        globalThis.__rex_doc_head_elements.push(children[i] as ReactElement);
    }
  }
  return null;
}

export function Main(): null {
  return null;
}
export function NextScript(): null {
  return null;
}

export default Html;

// Render a user's _document component and extract descriptor
globalThis.__rex_render_document = function (): string {
  const doc = globalThis.__rex_document;
  if (!doc || !doc.default) {
    return JSON.stringify({ htmlAttrs: {}, bodyAttrs: {}, headContent: "" });
  }
  globalThis.__rex_doc_html_attrs = {};
  globalThis.__rex_doc_body_attrs = {};
  globalThis.__rex_doc_head_elements = [];

  const element = createElement(doc.default, {});
  renderToString(element);

  let headContent = "";
  for (let i = 0; i < globalThis.__rex_doc_head_elements.length; i++) {
    headContent += renderToString(globalThis.__rex_doc_head_elements[i]);
  }

  const bodyAttrs: Record<string, string> = {};
  const rawBody = globalThis.__rex_doc_body_attrs;
  for (const k in rawBody) {
    if (Object.prototype.hasOwnProperty.call(rawBody, k)) {
      bodyAttrs[k === "className" ? "class" : k] = rawBody[k];
    }
  }

  return JSON.stringify({
    htmlAttrs: globalThis.__rex_doc_html_attrs,
    bodyAttrs: bodyAttrs,
    headContent: headContent,
  });
};
