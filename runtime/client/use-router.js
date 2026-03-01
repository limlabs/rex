// Rex useRouter() hook — client-side implementation.
// Subscribes to window.__REX_ROUTER events for reactive updates.
import { useState, useEffect } from 'react';

function getSnapshot() {
  var r = window.__REX_ROUTER;
  if (!r || !r.state) {
    return {
      pathname: window.location.pathname,
      asPath: window.location.pathname + window.location.search,
      query: {},
      route: window.location.pathname
    };
  }
  return {
    pathname: r.state.pathname,
    asPath: r.state.asPath,
    query: r.state.query,
    route: r.state.route
  };
}

export function useRouter() {
  var snap = getSnapshot();
  var state = useState(snap);
  var routerState = state[0];
  var setRouterState = state[1];

  useEffect(function() {
    var r = window.__REX_ROUTER;
    if (!r || !r.events) return;

    function onRouteChange() {
      setRouterState(getSnapshot());
    }

    r.events.on('routeChangeComplete', onRouteChange);
    return function() {
      r.events.off('routeChangeComplete', onRouteChange);
    };
  }, []);

  var r = window.__REX_ROUTER;
  var noop = function() {};

  return {
    pathname: routerState.pathname,
    asPath: routerState.asPath,
    query: routerState.query,
    route: routerState.route,
    push: r ? r.push : function(url) { window.location.href = url; },
    replace: r ? r.replace : function(url) { window.location.replace(url); },
    back: r ? r.back : function() { history.back(); },
    forward: r ? r.forward : function() { history.forward(); },
    reload: r ? r.reload : function() { window.location.reload(); },
    prefetch: r ? r.prefetch : noop,
    events: r ? r.events : { on: noop, off: noop, emit: noop },
    isReady: true
  };
}

export default useRouter;
