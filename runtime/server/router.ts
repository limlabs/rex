// Rex useRouter() hook — server-side stub.
// Returns static defaults during SSR. Real state is only available on the client.

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
  return {
    pathname: "/",
    asPath: "/",
    query: {},
    route: "/",
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
