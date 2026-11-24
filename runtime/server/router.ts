// Rex useRouter() hook — server-side stub.
// Returns the route of the page being rendered (set by __rex_render_page in
// ssr-runtime.ts); navigation methods are no-ops on the server.

interface UseRouterReturn {
  pathname: string;
  asPath: string;
  query: Record<string, string>;
  route: string;
  push: () => void;
  replace: () => void;
  back: () => void;
  forward: () => void;
  reload: () => void;
  prefetch: () => void;
  events: { on: () => void; off: () => void; emit: () => void };
  isReady: boolean;
}

const noop = function (): void {};
const noopEvents = { on: noop, off: noop, emit: noop };

export function useRouter(): UseRouterReturn {
  const state = globalThis.__rex_router_state;
  const pathname = state ? state.pathname : "/";
  return {
    pathname: pathname,
    asPath: state ? state.asPath : "/",
    query: state ? state.query : {},
    route: pathname,
    push: noop,
    replace: noop,
    back: noop,
    forward: noop,
    reload: noop,
    prefetch: noop,
    events: noopEvents,
    isReady: false,
  };
}

export default useRouter;
