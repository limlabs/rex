export interface RexOptions {
  /** Path to the project root directory (containing pages/). */
  root: string;
  /** Whether to run in dev mode (enables HMR, error overlays). */
  dev?: boolean;
}

export interface RouteMatch {
  /** The route pattern, e.g. "/blog/:slug" */
  pattern: string;
  /** The module name, e.g. "blog/[slug]" */
  moduleName: string;
  /** Matched params, e.g. { slug: "hello" } */
  params: Record<string, string>;
}

export interface PageResult {
  /** Full HTML document. */
  html: string;
  /** HTTP status code. */
  status: number;
  /** Response headers. */
  headers: Array<{ key: string; value: string }>;
}

export interface RexInstance {
  /** Whether this instance is running in dev mode. */
  readonly isDev: boolean;
  /** The current build ID. */
  readonly buildId: string;
  /** The path to the static files directory (client JS/CSS bundles). */
  readonly staticDir: string;

  /**
   * Match a URL path against the route trie.
   * Returns the matched route info with params, or null if no match.
   */
  matchRoute(path: string): RouteMatch | null;

  /**
   * Run getServerSideProps for a given path and return the result.
   */
  getServerSideProps(path: string): Promise<Record<string, unknown>>;

  /**
   * Render a page to an HTML string with the given props.
   */
  renderToString(path: string, props: Record<string, unknown>): Promise<string>;

  /**
   * Render a full page (GSSP + SSR + document assembly).
   * Returns HTML, status code, and headers.
   */
  renderPage(path: string): Promise<PageResult>;

  /**
   * Get a request handler function compatible with the Web Fetch API.
   * Returns `(req: Request) => Promise<Response>`.
   *
   * Works with Bun.serve, Deno.serve, and Node.js 18+.
   *
   * @example
   * ```js
   * const handler = rex.getRequestHandler()
   * Bun.serve({ fetch: handler })
   * ```
   */
  getRequestHandler(): (req: Request) => Promise<Response>;

  /**
   * Shut down the Rex instance, releasing V8 isolates and other resources.
   */
  close(): Promise<void>;
}

/**
 * Create a new Rex application instance.
 *
 * Scans the pages directory, builds bundles, initializes the V8 isolate pool,
 * and returns a ready-to-use RexInstance.
 *
 * @example
 * ```js
 * import { createRex } from '@limlabs/rex/server'
 *
 * const rex = await createRex({ root: './my-app' })
 * const handle = rex.getRequestHandler()
 *
 * Bun.serve({ fetch: handle })
 * ```
 */
export function createRex(options: RexOptions): Promise<RexInstance>;
