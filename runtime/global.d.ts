import type { ReactElement } from "react";

declare global {
  /** Manifest entry for a single page route */
  interface RexManifestPage {
    js: string;
    css?: string[];
  }

  /** Manifest entry for an app route */
  interface RexManifestAppRoute {
    client_chunks: string[];
  }

  /** Client manifest embedded in SSR HTML */
  interface RexManifest {
    build_id: string;
    pages: Record<string, RexManifestPage>;
    app_routes?: Record<string, RexManifestAppRoute>;
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
    type: "connected" | "full-reload" | "module-update" | "error" | "tsc-error" | "tsc-clear";
    path?: string;
    manifest?: RexManifest;
    message?: string;
    file?: string;
    /** Error origin: "build", "server", or "client" */
    kind?: "build" | "server" | "client";
    /** TypeScript diagnostics (for tsc-error type) */
    errors?: RexTscDiagnostic[];
    /** Module URL for module-update messages */
    url?: string;
    /** Timestamp for cache-busting module reimport */
    timestamp?: number;
    /** Route pattern affected (for module-update on page files) */
    route?: string;
  }

  /** A single TypeScript diagnostic from tsc --watch */
  interface RexTscDiagnostic {
    file: string;
    line: number;
    col: number;
    code: string;
    message: string;
  }

  /** RSC module map entry */
  interface RexRscModuleMapEntry {
    chunk_url: string;
    export_name: string;
  }

  /** RSC module map embedded in HTML */
  interface RexRscModuleMap {
    entries?: Record<string, RexRscModuleMapEntry>;
    [refId: string]: RexRscModuleMapEntry | Record<string, RexRscModuleMapEntry> | undefined;
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
    __REX_BASE_PATH?: string;
    __REX_STATIC_EXPORT?: boolean;
    __REX_STATIC_HTML_EXT?: boolean;
    __REX_RSC_NAVIGATE?: (pathname: string) => Promise<void>;
    __REX_RSC_INIT?: () => void;
    __REX_RSC_PARSE_FLIGHT?: (flight: string) => ReactElement | null;
    __REX_RSC_MODULE_MAP__?: RexRscModuleMap;
    __rexModuleCache?: Record<string, unknown>;
    __REX_CALL_SERVER?: (id: string, args: unknown[]) => Promise<unknown>;
    React?: typeof React;
    ReactDOM?: typeof import("react-dom/client");
  }

  // getStaticPaths globals (V8 SSR environment)
  var __rex_gsp_paths_resolved: unknown;
  var __rex_gsp_paths_rejected: unknown;
  var __rex_get_static_paths: (routeKey: string) => string;
  var __rex_resolve_static_paths: () => string;

  // Server-side globals (V8 SSR environment)
  var __rex_head_elements: ReactElement[];
  var __rex_doc_html_attrs: Record<string, string>;
  var __rex_doc_body_attrs: Record<string, string>;
  var __rex_doc_head_elements: ReactElement[];
  var __rex_render_document: () => string;
  var __rex_document: { default?: React.ComponentType } | undefined;

  // RSC server-side globals (V8 environment — set by virtual entry)
  var __rex_createElement: typeof React.createElement;
  var __rex_renderToReadableStream: (
    element: ReactElement,
    bundlerConfig: Record<string, unknown>,
    options?: Record<string, unknown>,
  ) => ReadableStream<Uint8Array>;
  var __rex_createFromReadableStream: (
    stream: ReadableStream<Uint8Array>,
    options: { ssrManifest: { moduleMap: Record<string, unknown>; moduleLoading: null } },
  ) => unknown;
  var __rex_renderToReadableStream_ssr: (
    element: unknown,
    options?: Record<string, unknown>,
  ) => unknown;
  var __rex_renderToString: (element: unknown) => string;

  // RSC globalThis properties (V8 environment)
  // eslint-disable-next-line no-var
  var __rex_app_pages: Record<string, React.ComponentType>;
  var __rex_app_layout_chains: Record<string, React.ComponentType[]>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  var __rex_app_metadata_sources: Record<string, Record<string, any>[]>;
  var __rex_webpack_bundler_config: Record<string, unknown>;
  var __rex_webpack_ssr_manifest: Record<string, unknown>;
  var __rex_webpack_server_module_map: Record<string, unknown>;
  var __rex_client_modules__: Record<string, unknown>;
  var __rex_ssr_modules__: Record<string, unknown>;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  var __webpack_require__: ((id: string) => any) & {
    u?: (chunkId: string) => string;
  };
  var __webpack_chunk_load__: (chunkId: string) => Promise<void>;

  // RSC flight/SSR public API (set by runtime scripts)
  var __rex_render_flight: (routeKey: string, propsJson: string) => string;
  var __rex_render_rsc_to_html: (routeKey: string, propsJson: string) => string;
  var __rex_resolve_rsc_pending: () => "pending" | "done";
  var __rex_finalize_rsc_flight: () => string;
  var __rex_finalize_rsc_to_html: () => string;
  var __rex_rsc_flight_to_html: (flightString: string) => string;
  var __rex_resolve_ssr_pending: () => "pending" | "done";
  var __rex_finalize_ssr: () => string;

  // Raw flight bytes for SSR pass (avoids UTF-8 round-trip corruption)
  var __rex_flight_raw_chunks: Uint8Array[] | undefined;

  // Server action globals (V8 environment — set by flight.ts runtime)
  var __rex_server_actions: Record<string, (...args: unknown[]) => unknown>;
  var __rex_server_action_manifest: Record<string, unknown>;
  var __rex_call_server_action: (actionId: string, argsJson: string) => string;
  var __rex_call_server_action_encoded: (actionId: string, body: string, isFormFields?: boolean) => string;
  var __rex_call_form_action: (fieldsJson: string) => string;
  var __rex_resolve_action_pending: () => "pending" | "done";
  var __rex_finalize_action: () => string;
  var __rex_decodeReply: (body: string | FormData, manifest: Record<string, unknown>) => PromiseLike<unknown[]>;
  var __rex_decodeAction: (body: FormData, manifest: Record<string, unknown>) => Promise<(() => unknown) | null>;
  var __rex_decodeFormState: (result: unknown, body: FormData, manifest: Record<string, unknown>) => Promise<unknown>;

  // Server action utilities (V8 environment — set by flight.ts runtime)
  var __rex_redirect: (url: string, status?: number) => never;
  var __rex_notFound: () => never;
  var __rex_request_headers: Record<string, string>;
  var __rex_request_cookies: Record<string, string>;
}

export {};
