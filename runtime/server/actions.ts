// Rex Server Actions Runtime API
//
// Provides Next.js-compatible server action utilities:
//   - redirect(url, status?) — redirect from a server action
//   - notFound() — return 404 from a server action
//   - cookies() — access request cookies in a server action
//   - headers() — access request headers in a server action
//
// These functions rely on globals set by the flight.ts runtime
// and the Rust request context injection.

/**
 * Redirect the user to a different URL from a server action.
 * Throws a sentinel error caught by the action dispatcher.
 *
 * @param url - The URL to redirect to
 * @param status - HTTP status code (default: 303)
 */
export function redirect(url: string, status?: number): never {
  return globalThis.__rex_redirect(url, status);
}

/**
 * Return a 404 Not Found response from a server action.
 * Throws a sentinel error caught by the action dispatcher.
 */
export function notFound(): never {
  return globalThis.__rex_notFound();
}

/**
 * Access request cookies inside a server action.
 * Returns a read-only map of cookie name → value.
 */
export function cookies(): Readonly<Record<string, string>> {
  return globalThis.__rex_request_cookies || {};
}

/**
 * Access request headers inside a server action.
 * Returns a read-only map of header name → value.
 */
export function headers(): Readonly<Record<string, string>> {
  return globalThis.__rex_request_headers || {};
}
