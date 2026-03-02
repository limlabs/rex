import type { ReactElement } from "react";

declare global {
  /** Manifest entry for a single page route */
  interface RexManifestPage {
    js: string;
    css?: string[];
  }

  /** Client manifest embedded in SSR HTML */
  interface RexManifest {
    build_id: string;
    pages: Record<string, RexManifestPage>;
  }

  /** Router state exposed on window.__REX_ROUTER */
  interface RexRouterState {
    pathname: string;
    asPath: string;
    query: Record<string, string>;
    route: string;
  }

  /** Event emitter API */
  interface RexEvents {
    on(event: string, fn: (...args: unknown[]) => void): void;
    off(event: string, fn: (...args: unknown[]) => void): void;
    emit(event: string, ...args: unknown[]): void;
  }

  /** Public client router API */
  interface RexRouter {
    push(url: string): Promise<void>;
    replace(url: string): Promise<void>;
    back(): void;
    forward(): void;
    reload(): void;
    prefetch(url: string): void;
    state: RexRouterState;
    events: RexEvents;
  }

  /** Page module registered on window.__REX_PAGES */
  interface RexPageModule {
    default: React.ComponentType<Record<string, unknown>>;
  }

  /** HMR update message */
  interface RexHmrMessage {
    type: "connected" | "update" | "full-reload" | "error";
    path?: string;
    manifest?: RexManifest;
    message?: string;
    file?: string;
  }

  interface Window {
    __REX_ROUTER?: RexRouter;
    __REX_MANIFEST__?: RexManifest;
    __REX_PAGES?: Record<string, RexPageModule>;
    __REX_RENDER__?: (
      component: React.ComponentType<Record<string, unknown>>,
      props: Record<string, unknown>,
    ) => void;
    __REX_NAVIGATING__?: boolean;
  }

  // Server-side globals (V8 SSR environment)
  var __rex_head_elements: ReactElement[];
  var __rex_doc_html_attrs: Record<string, string>;
  var __rex_doc_body_attrs: Record<string, string>;
  var __rex_doc_head_elements: ReactElement[];
  var __rex_render_document: () => string;
  var __rex_document: { default?: React.ComponentType } | undefined;
}

export {};
