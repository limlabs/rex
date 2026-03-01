// Rex Client Router — client-side navigation without full page reloads.
// Reads window.__REX_MANIFEST__ (embedded in HTML) for route-to-chunk mapping.
// Exposes window.__REX_ROUTER with navigation methods, state, and events.
(function() {
  var manifest = window.__REX_MANIFEST__;
  if (!manifest) return;

  var buildId = manifest.build_id;
  var pages = manifest.pages;
  var appRoutes = manifest.app_routes || {};
  var prefetchCache = {};
  var loadingChunks = {};

  // --- Event emitter ---

  var listeners = {};

  var events = {
    on: function(event, fn) {
      (listeners[event] = listeners[event] || []).push(fn);
    },
    off: function(event, fn) {
      var arr = listeners[event];
      if (arr) listeners[event] = arr.filter(function(f) { return f !== fn; });
    },
    emit: function(event) {
      var args = Array.prototype.slice.call(arguments, 1);
      var arr = listeners[event];
      if (arr) for (var i = 0; i < arr.length; i++) arr[i].apply(null, args);
    }
  };

  // --- Query string parsing ---

  function parseQuery(search) {
    var query = {};
    if (!search || search.length < 2) return query;
    var pairs = search.substring(1).split('&');
    for (var i = 0; i < pairs.length; i++) {
      var idx = pairs[i].indexOf('=');
      if (idx > 0) {
        query[decodeURIComponent(pairs[i].substring(0, idx))] =
          decodeURIComponent(pairs[i].substring(idx + 1));
      }
    }
    return query;
  }

  // --- Route matching ---

  function matchRoute(pathname) {
    if (pages[pathname]) return { pattern: pathname, params: {} };
    for (var pattern in pages) {
      if (pattern.indexOf(':') === -1 && pattern.indexOf('*') === -1) continue;
      var params = matchDynamic(pattern, pathname);
      if (params) return { pattern: pattern, params: params };
    }
    return null;
  }

  function matchDynamic(pattern, pathname) {
    var pp = pattern.split('/');
    var up = pathname.split('/');
    var params = {};
    for (var i = 0; i < pp.length; i++) {
      if (pp[i][0] === '*') {
        // Catch-all: consume remaining segments
        params[pp[i].slice(1)] = up.slice(i).map(decodeURIComponent);
        return params;
      }
      if (i >= up.length) return null;
      if (pp[i][0] === ':') {
        params[pp[i].slice(1)] = decodeURIComponent(up[i]);
      } else if (pp[i] !== up[i]) {
        return null;
      }
    }
    if (pp.length !== up.length) return null;
    return params;
  }

  // Match against app/ routes (RSC)
  function matchAppRoute(pathname) {
    if (appRoutes[pathname]) return { pattern: pathname, params: {} };
    for (var pattern in appRoutes) {
      if (pattern.indexOf(':') === -1 && pattern.indexOf('*') === -1) continue;
      var params = matchDynamic(pattern, pathname);
      if (params) return { pattern: pattern, params: params };
    }
    return null;
  }

  function isAppRoute(pathname) {
    return matchAppRoute(pathname) !== null;
  }

  // --- Router state ---

  var initialMatch = matchRoute(window.location.pathname);
  var initialQuery = parseQuery(window.location.search);
  if (initialMatch) {
    for (var k in initialMatch.params) initialQuery[k] = initialMatch.params[k];
  }

  var state = {
    pathname: initialMatch ? initialMatch.pattern : window.location.pathname,
    asPath: window.location.pathname + window.location.search,
    query: initialQuery,
    route: initialMatch ? initialMatch.pattern : window.location.pathname
  };

  function updateState(match, url) {
    var a = document.createElement('a');
    a.href = url;
    var query = parseQuery(a.search);
    if (match) {
      for (var k in match.params) query[k] = match.params[k];
    }
    state.pathname = match ? match.pattern : a.pathname;
    state.route = state.pathname;
    state.asPath = a.pathname + a.search;
    state.query = query;
  }

  // --- Data fetching ---

  function fetchPageData(pathname) {
    var dataUrl = '/_rex/data/' + buildId + pathname + '.json';
    return fetch(dataUrl).then(function(res) {
      if (!res.ok) throw new Error('Data fetch failed: ' + res.status);
      return res.json();
    });
  }

  // --- Chunk loading ---

  function ensureChunk(pattern) {
    if (window.__REX_PAGES && window.__REX_PAGES[pattern]) {
      return Promise.resolve();
    }
    var js = pages[pattern] && pages[pattern].js;
    if (!js) return Promise.reject(new Error('No chunk for: ' + pattern));

    if (!loadingChunks[js]) {
      window.__REX_NAVIGATING__ = true;
      loadingChunks[js] = import('/_rex/static/' + js).then(function() {
        delete loadingChunks[js];
      });
    }
    return loadingChunks[js];
  }

  // --- CSS management ---

  function updatePageCss(pattern) {
    var entry = pages[pattern];
    if (!entry || !entry.css || !entry.css.length) return;

    for (var i = 0; i < entry.css.length; i++) {
      var href = '/_rex/static/' + entry.css[i];
      if (!document.querySelector('link[href="' + href + '"]')) {
        var link = document.createElement('link');
        link.rel = 'stylesheet';
        link.href = href;
        document.head.appendChild(link);
      }
    }
  }

  // --- Navigation ---

  function navigate(url, opts) {
    opts = opts || {};

    var a = document.createElement('a');
    a.href = url;
    var pathname = a.pathname;
    var fullUrl = a.pathname + a.search;

    // Check app routes (RSC) first, then pages routes
    var appMatch = matchAppRoute(pathname);
    if (appMatch && window.__REX_RSC_NAVIGATE) {
      return navigateAppRoute(pathname, fullUrl, url, appMatch, opts);
    }

    var match = matchRoute(pathname);
    if (!match) {
      window.location.href = url;
      return Promise.resolve();
    }

    events.emit('routeChangeStart', fullUrl);

    var dataPromise = prefetchCache[pathname] || fetchPageData(pathname);
    delete prefetchCache[pathname];
    var chunkPromise = ensureChunk(match.pattern);

    return Promise.all([dataPromise, chunkPromise]).then(function(results) {
      var data = results[0];

      // Handle GSSP redirect
      if (data.redirect) {
        var dest = data.redirect.destination;
        return navigate(dest, { replace: data.redirect.permanent });
      }

      // Handle notFound — fall back to server
      if (data.notFound) {
        window.location.href = pathname;
        return;
      }

      var props = data.props || {};

      // Update URL (skip on popstate — browser already updated it)
      if (!opts.popstate) {
        events.emit('beforeHistoryChange', fullUrl);
        if (opts.replace) {
          history.replaceState({ __rex: pathname }, '', url);
        } else {
          history.pushState({ __rex: pathname }, '', url);
        }
      }

      // Update router state
      updateState(match, url);

      // Update data element
      var dataEl = document.getElementById('__REX_DATA__');
      if (dataEl) dataEl.textContent = JSON.stringify(props);

      // Load page CSS
      updatePageCss(match.pattern);

      // Re-render via the global render callback (set by page entry)
      var page = window.__REX_PAGES && window.__REX_PAGES[match.pattern];
      if (page && window.__REX_RENDER__) {
        window.__REX_RENDER__(page.default, props);
      }

      // Scroll to top (unless back/forward)
      if (!opts.popstate) {
        window.scrollTo(0, 0);
      }

      events.emit('routeChangeComplete', fullUrl);
    }).catch(function(err) {
      console.error('Rex navigation failed:', err);
      events.emit('routeChangeError', err, fullUrl);
      window.location.href = url;
    });
  }

  // Navigate to an app/ route using RSC flight data
  function navigateAppRoute(pathname, fullUrl, url, match, opts) {
    events.emit('routeChangeStart', fullUrl);

    return window.__REX_RSC_NAVIGATE(pathname).then(function() {
      // Update URL
      if (!opts.popstate) {
        events.emit('beforeHistoryChange', fullUrl);
        if (opts.replace) {
          history.replaceState({ __rex: pathname, __rsc: true }, '', url);
        } else {
          history.pushState({ __rex: pathname, __rsc: true }, '', url);
        }
      }

      // Update router state
      updateState(match, url);

      // Scroll to top (unless back/forward)
      if (!opts.popstate) {
        window.scrollTo(0, 0);
      }

      events.emit('routeChangeComplete', fullUrl);
    }).catch(function(err) {
      console.error('Rex RSC navigation failed:', err);
      events.emit('routeChangeError', err, fullUrl);
      window.location.href = url;
    });
  }

  // --- Popstate (back/forward) ---

  window.addEventListener('popstate', function(e) {
    if (e.state && e.state.__rex) {
      navigate(window.location.href, { replace: true, popstate: true });
    }
  });

  // Mark initial page in history state
  history.replaceState({ __rex: window.location.pathname }, '', window.location.href);

  // --- Public API ---

  window.__REX_ROUTER = {
    // Navigation methods
    push: function(url) { return navigate(url); },
    replace: function(url) { return navigate(url, { replace: true }); },
    back: function() { history.back(); },
    forward: function() { history.forward(); },
    reload: function() { window.location.reload(); },
    prefetch: function(url) {
      var a = document.createElement('a');
      a.href = url;
      var pathname = a.pathname;
      if (!prefetchCache[pathname]) {
        prefetchCache[pathname] = fetchPageData(pathname);
      }
    },
    // State (mutable — updated on navigation)
    state: state,
    // Event system
    events: events
  };
})();
