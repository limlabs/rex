export interface RouterEvents {
  on(event: string, handler: (...args: unknown[]) => void): void;
  off(event: string, handler: (...args: unknown[]) => void): void;
  emit(event: string, ...args: unknown[]): void;
}

export interface RexRouter {
  pathname: string;
  asPath: string;
  query: Record<string, string>;
  route: string;
  push(url: string): void;
  replace(url: string): void;
  back(): void;
  forward(): void;
  reload(): void;
  prefetch(url: string): void;
  events: RouterEvents;
  isReady: boolean;
}

interface InternalRouter {
  push(path: string): void;
  replace(path: string): void;
  back(): void;
  forward(): void;
  reload(): void;
  prefetch(url: string): void;
  events: RouterEvents;
  state: {
    pathname: string;
    asPath: string;
    query: Record<string, string>;
    route: string;
  };
}

declare const window: Window & {
  __REX_ROUTER?: InternalRouter;
};

function getRouter(): InternalRouter | null {
  return window.__REX_ROUTER ?? null;
}

function getBasePath(): string {
  return ((window as any).__REX_BASE_PATH || '').replace(/\/+$/, '');
}

export function withBasePath(href: string): string {
  const bp = getBasePath();
  if (!bp || !href.startsWith('/') || href.startsWith('//')) return href;
  return bp + href;
}

/**
 * Navigate to a new path via client-side routing.
 */
export function navigateTo(path: string): void {
  // RSC app router navigation (fetches flight data, re-renders in place)
  const rscNavigate = (window as any).__REX_RSC_NAVIGATE;
  if (rscNavigate) {
    history.pushState(null, '', withBasePath(path));
    rscNavigate(path);
    return;
  }
  // Pages router fallback
  const r = getRouter();
  if (r) {
    r.push(path);
  } else {
    window.location.href = path;
  }
}

function parseQuery(search: string): Record<string, string> {
  const query: Record<string, string> = {};
  if (!search || search.length <= 1) return query;
  const pairs = search.substring(1).split('&');
  for (const pair of pairs) {
    const [key, value] = pair.split('=');
    query[decodeURIComponent(key)] = decodeURIComponent(value ?? '');
  }
  return query;
}

/**
 * React hook that returns the current router instance.
 */
export function useRouter(): RexRouter {
  const r = getRouter();
  const noop = (): void => {};

  if (r?.state) {
    return {
      pathname: r.state.pathname,
      asPath: r.state.asPath,
      query: r.state.query,
      route: r.state.route,
      push: r.push,
      replace: r.replace,
      back: r.back,
      forward: r.forward,
      reload: r.reload,
      prefetch: r.prefetch,
      events: r.events,
      isReady: true,
    };
  }

  return {
    pathname: window.location.pathname,
    asPath: window.location.pathname + window.location.search,
    query: parseQuery(window.location.search),
    route: window.location.pathname,
    push: (url: string) => { window.location.href = url; },
    replace: (url: string) => { window.location.replace(url); },
    back: () => { history.back(); },
    forward: () => { history.forward(); },
    reload: () => { window.location.reload(); },
    prefetch: noop,
    events: { on: noop, off: noop, emit: noop },
    isReady: false,
  };
}

export default useRouter;
