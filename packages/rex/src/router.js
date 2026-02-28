/**
 * rex/router - Client-side router
 *
 * Thin wrapper around window.__REX_ROUTER (set up by the runtime IIFE at
 * /_rex/router.js). The bundler normally aliases `rex/router` to the runtime
 * files directly, so this module only runs when the npm package is used
 * without Rex's build pipeline.
 */

function getRouter() {
  return window.__REX_ROUTER || null;
}

/**
 * Navigate to a new path via client-side routing.
 */
export function navigateTo(path) {
  var r = getRouter();
  if (r) {
    r.push(path);
  } else {
    window.location.href = path;
  }
}

/**
 * Get the current route information.
 * Matches the API surface of runtime/client/use-router.js.
 */
export function useRouter() {
  var r = getRouter();
  var noop = function() {};

  if (r && r.state) {
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
      isReady: true
    };
  }

  return {
    pathname: window.location.pathname,
    asPath: window.location.pathname + window.location.search,
    query: parseQuery(window.location.search),
    route: window.location.pathname,
    push: function(url) { window.location.href = url; },
    replace: function(url) { window.location.replace(url); },
    back: function() { history.back(); },
    forward: function() { history.forward(); },
    reload: function() { window.location.reload(); },
    prefetch: noop,
    events: { on: noop, off: noop, emit: noop },
    isReady: false
  };
}

export default useRouter;

function parseQuery(search) {
  var query = {};
  if (!search || search.length <= 1) return query;
  var pairs = search.substring(1).split('&');
  for (var i = 0; i < pairs.length; i++) {
    var pair = pairs[i].split('=');
    query[decodeURIComponent(pair[0])] = decodeURIComponent(pair[1] || '');
  }
  return query;
}
