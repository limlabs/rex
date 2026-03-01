export interface NextURL {
  /** The full URL string. */
  href: string;
  /** The pathname portion of the URL (e.g. "/blog/hello"). */
  pathname: string;
  /** The search/query string (e.g. "?foo=bar"). */
  search: string;
  /** String representation of the URL. */
  toString(): string;
}

export interface NextRequest {
  /** HTTP method (e.g. "GET", "POST"). */
  method: string;
  /** The full request URL string. */
  url: string;
  /** Parsed URL object with pathname, search, etc. */
  nextUrl: NextURL;
  /** Request headers as a plain object. */
  headers: Record<string, string>;
  /** Request cookies as a plain object. */
  cookies: Record<string, string>;
}

export interface NextResponseInit {
  /** Headers to add to the response. */
  headers?: Record<string, string>;
  /** Headers/options to modify the downstream request. */
  request?: {
    headers?: Record<string, string>;
  };
}

export declare class NextResponse {
  /**
   * Continue to the next handler, optionally modifying response or request headers.
   */
  static next(init?: NextResponseInit): NextResponse;

  /**
   * Redirect the request to a different URL.
   * @param url - The target URL (string or URL object).
   * @param status - HTTP status code (default: 307).
   */
  static redirect(url: string | URL, status?: number): NextResponse;

  /**
   * Rewrite the request to a different URL without changing the browser URL.
   * @param url - The target URL (string or URL object).
   */
  static rewrite(url: string | URL): NextResponse;
}

export interface MiddlewareConfig {
  /**
   * Route patterns to match. Supports:
   * - Exact paths: "/about"
   * - Dynamic segments: "/blog/:slug"
   * - Catch-all: "/api/:path*"
   *
   * If omitted, middleware runs on all routes.
   */
  matcher?: string[];
}

/**
 * Middleware function signature.
 *
 * @example
 * ```ts
 * import { NextResponse } from '@limlabs/rex/middleware'
 * import type { MiddlewareFunction } from '@limlabs/rex/middleware'
 *
 * export const middleware: MiddlewareFunction = (request) => {
 *   if (request.nextUrl.pathname === '/old') {
 *     return NextResponse.redirect(new URL('/new', request.url))
 *   }
 *   return NextResponse.next()
 * }
 *
 * export const config = { matcher: ['/old', '/dashboard/:path*'] }
 * ```
 */
export type MiddlewareFunction = (request: NextRequest) => NextResponse;
