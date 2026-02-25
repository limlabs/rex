// Rex Client Router — client-side navigation without full page reloads.
// Reads window.__REX_MANIFEST__ (embedded in HTML) for route-to-chunk mapping.
// Used by rex/link's onClick handler via window.__REX_ROUTER.
(function() {
  var manifest = window.__REX_MANIFEST__;
  if (!manifest) return;

  var buildId = manifest.build_id;
  var pages = manifest.pages;
  var prefetchCache = {};
  var loadingChunks = {};

  // --- Route matching ---

  function matchRoute(pathname) {
    // Exact static match first
    if (pages[pathname]) return { pattern: pathname, params: {} };

    // Dynamic pattern match
    for (var pattern in pages) {
      if (pattern.indexOf(':') === -1) continue;
      var params = matchDynamic(pattern, pathname);
      if (params) return { pattern: pattern, params: params };
    }
    return null;
  }

  function matchDynamic(pattern, pathname) {
    var pp = pattern.split('/');
    var up = pathname.split('/');
    if (pp.length !== up.length) return null;
    var params = {};
    for (var i = 0; i < pp.length; i++) {
      if (pp[i][0] === ':') {
        params[pp[i].slice(1)] = decodeURIComponent(up[i]);
      } else if (pp[i] !== up[i]) {
        return null;
      }
    }
    return params;
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

    var match = matchRoute(pathname);
    if (!match) {
      window.location.href = url;
      return Promise.resolve();
    }

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
        if (opts.replace) {
          history.replaceState({ __rex: pathname }, '', url);
        } else {
          history.pushState({ __rex: pathname }, '', url);
        }
      }

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
    }).catch(function(err) {
      console.error('Rex navigation failed:', err);
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
    push: function(url) { navigate(url); },
    replace: function(url) { navigate(url, { replace: true }); },
    prefetch: function(url) {
      var a = document.createElement('a');
      a.href = url;
      var pathname = a.pathname;
      if (!prefetchCache[pathname]) {
        prefetchCache[pathname] = fetchPageData(pathname);
      }
    }
  };
})();
