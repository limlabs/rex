export interface RouterEvents {
  on(event: string, handler: (...args: unknown[]) => void): void;
  off(event: string, handler: (...args: unknown[]) => void): void;
  emit(event: string, ...args: unknown[]): void;
}

export interface NextRouter {
  /** Current pathname (e.g. "/blog/hello") */
  pathname: string;
  /** Resolved path including query string */
  asPath: string;
  /** Parsed query parameters */
  query: Record<string, string>;
  /** Route pattern (e.g. "/blog/[slug]") */
  route: string;
  /** Navigate to a new URL */
  push(url: string): void;
  /** Replace the current URL without adding to history */
  replace(url: string): void;
  /** Go back in history */
  back(): void;
  /** Go forward in history */
  forward(): void;
  /** Reload the current page */
  reload(): void;
  /** Prefetch a route for faster navigation */
  prefetch(url: string): void;
  /** Router event emitter */
  events: RouterEvents;
  /** Whether the router is ready */
  isReady: boolean;
}

/**
 * React hook that returns the current router instance.
 */
export function useRouter(): NextRouter;

/**
 * Navigate to a new path via client-side routing.
 */
export function navigateTo(path: string): void;

export default useRouter;
