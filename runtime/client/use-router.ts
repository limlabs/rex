// Rex useRouter() hook — client-side implementation.
// Subscribes to window.__REX_ROUTER events for reactive updates.
import { useState, useEffect } from "react";

interface RouterHookState {
  pathname: string;
  asPath: string;
  query: Record<string, string>;
  route: string;
}

interface UseRouterReturn extends RouterHookState {
  push: (url: string) => void;
  replace: (url: string) => void;
  back: () => void;
  forward: () => void;
  reload: () => void;
  prefetch: (url: string) => void;
  events: RexEvents;
  isReady: boolean;
}

function getSnapshot(): RouterHookState {
  const r = window.__REX_ROUTER;
  if (!r || !r.state) {
    return {
      pathname: window.location.pathname,
      asPath: window.location.pathname + window.location.search,
      query: {},
      route: window.location.pathname,
    };
  }
  return {
    pathname: r.state.pathname,
    asPath: r.state.asPath,
    query: r.state.query,
    route: r.state.route,
  };
}

export function useRouter(): UseRouterReturn {
  const snap = getSnapshot();
  const [routerState, setRouterState] = useState(snap);

  useEffect(function () {
    const r = window.__REX_ROUTER;
    if (!r || !r.events) return;

    function onRouteChange(): void {
      setRouterState(getSnapshot());
    }

    r.events.on("routeChangeComplete", onRouteChange);
    return function () {
      r.events.off("routeChangeComplete", onRouteChange);
    };
  }, []);

  const r = window.__REX_ROUTER;
  const noop = function () {};

  return {
    pathname: routerState.pathname,
    asPath: routerState.asPath,
    query: routerState.query,
    route: routerState.route,
    push: r
      ? r.push
      : function (url: string) {
          window.location.href = url;
        },
    replace: r
      ? r.replace
      : function (url: string) {
          window.location.replace(url);
        },
    back: r ? r.back : function () { history.back(); },
    forward: r ? r.forward : function () { history.forward(); },
    reload: r ? r.reload : function () { window.location.reload(); },
    prefetch: r ? r.prefetch : noop,
    events: r ? r.events : { on: noop, off: noop, emit: noop },
    isReady: true,
  };
}

export default useRouter;
