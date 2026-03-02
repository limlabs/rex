// Rex Client Router — client-side navigation without full page reloads.
// Reads window.__REX_MANIFEST__ (embedded in HTML) for route-to-chunk mapping.
// Exposes window.__REX_ROUTER with navigation methods, state, and events.
(function () {
  const manifest = window.__REX_MANIFEST__;
  if (!manifest) return;

  const buildId = manifest.build_id;
  const pages = manifest.pages;
  const prefetchCache: Record<string, Promise<GsspData>> = {};
  const loadingChunks: Record<string, Promise<void>> = {};

  // --- Event emitter ---

  const listeners: Record<string, Array<(...args: unknown[]) => void>> = {};

  const events: RexEvents = {
    on: function (event: string, fn: (...args: unknown[]) => void) {
      (listeners[event] = listeners[event] || []).push(fn);
    },
    off: function (event: string, fn: (...args: unknown[]) => void) {
      const arr = listeners[event];
      if (arr)
        listeners[event] = arr.filter(function (f) {
          return f !== fn;
        });
    },
    emit: function (event: string, ...args: unknown[]) {
      const arr = listeners[event];
      if (arr)
        for (let i = 0; i < arr.length; i++) arr[i].apply(null, args);
    },
  };

  // --- Types ---

  interface RouteMatch {
    pattern: string;
    params: Record<string, string>;
  }

  interface GsspData {
    props?: Record<string, unknown>;
    redirect?: { destination: string; permanent?: boolean };
    notFound?: boolean;
  }

  interface NavigateOpts {
    replace?: boolean;
    popstate?: boolean;
  }

  // --- Query string parsing ---

  function parseQuery(search: string): Record<string, string> {
    const query: Record<string, string> = {};
    if (!search || search.length < 2) return query;
    const pairs = search.substring(1).split("&");
    for (let i = 0; i < pairs.length; i++) {
      const idx = pairs[i].indexOf("=");
      if (idx > 0) {
        query[decodeURIComponent(pairs[i].substring(0, idx))] =
          decodeURIComponent(pairs[i].substring(idx + 1));
      }
    }
    return query;
  }

  // --- Route matching ---

  function matchRoute(pathname: string): RouteMatch | null {
    if (pages[pathname]) return { pattern: pathname, params: {} };
    for (const pattern in pages) {
      if (pattern.indexOf(":") === -1) continue;
      const params = matchDynamic(pattern, pathname);
      if (params) return { pattern, params };
    }
    return null;
  }

  function matchDynamic(
    pattern: string,
    pathname: string,
  ): Record<string, string> | null {
    const pp = pattern.split("/");
    const up = pathname.split("/");
    if (pp.length !== up.length) return null;
    const params: Record<string, string> = {};
    for (let i = 0; i < pp.length; i++) {
      if (pp[i][0] === ":") {
        params[pp[i].slice(1)] = decodeURIComponent(up[i]);
      } else if (pp[i] !== up[i]) {
        return null;
      }
    }
    return params;
  }

  // --- Router state ---

  const initialMatch = matchRoute(window.location.pathname);
  const initialQuery = parseQuery(window.location.search);
  if (initialMatch) {
    for (const k in initialMatch.params) initialQuery[k] = initialMatch.params[k];
  }

  const state: RexRouterState = {
    pathname: initialMatch ? initialMatch.pattern : window.location.pathname,
    asPath: window.location.pathname + window.location.search,
    query: initialQuery,
    route: initialMatch ? initialMatch.pattern : window.location.pathname,
  };

  function updateState(match: RouteMatch | null, url: string): void {
    const a = document.createElement("a");
    a.href = url;
    const query = parseQuery(a.search);
    if (match) {
      for (const k in match.params) query[k] = match.params[k];
    }
    state.pathname = match ? match.pattern : a.pathname;
    state.route = state.pathname;
    state.asPath = a.pathname + a.search;
    state.query = query;
  }

  // --- Data fetching ---

  function fetchPageData(pathname: string): Promise<GsspData> {
    const dataUrl = "/_rex/data/" + buildId + pathname + ".json";
    return fetch(dataUrl).then(function (res) {
      if (!res.ok) throw new Error("Data fetch failed: " + res.status);
      return res.json() as Promise<GsspData>;
    });
  }

  // --- Chunk loading ---

  function ensureChunk(pattern: string): Promise<void> {
    if (window.__REX_PAGES && window.__REX_PAGES[pattern]) {
      return Promise.resolve();
    }
    const js = pages[pattern] && pages[pattern].js;
    if (!js) return Promise.reject(new Error("No chunk for: " + pattern));

    if (!loadingChunks[js]) {
      window.__REX_NAVIGATING__ = true;
      loadingChunks[js] = import("/_rex/static/" + js).then(function () {
        delete loadingChunks[js];
      });
    }
    return loadingChunks[js];
  }

  // --- CSS management ---

  function updatePageCss(pattern: string): void {
    const entry = pages[pattern];
    if (!entry || !entry.css || !entry.css.length) return;

    for (let i = 0; i < entry.css.length; i++) {
      const href = "/_rex/static/" + entry.css[i];
      if (!document.querySelector('link[href="' + href + '"]')) {
        const link = document.createElement("link");
        link.rel = "stylesheet";
        link.href = href;
        document.head.appendChild(link);
      }
    }
  }

  // --- Navigation ---

  function navigate(url: string, opts?: NavigateOpts): Promise<void> {
    opts = opts || {};

    const a = document.createElement("a");
    a.href = url;
    const pathname = a.pathname;
    const fullUrl = a.pathname + a.search;

    const match = matchRoute(pathname);
    if (!match) {
      window.location.href = url;
      return Promise.resolve();
    }

    events.emit("routeChangeStart", fullUrl);

    const dataPromise = prefetchCache[pathname] || fetchPageData(pathname);
    delete prefetchCache[pathname];
    const chunkPromise = ensureChunk(match.pattern);

    return Promise.all([dataPromise, chunkPromise])
      .then(function (results) {
        const data = results[0];

        // Handle GSSP redirect
        if (data.redirect) {
          const dest = data.redirect.destination;
          return navigate(dest, { replace: data.redirect.permanent });
        }

        // Handle notFound — fall back to server
        if (data.notFound) {
          window.location.href = pathname;
          return;
        }

        const props = data.props || {};

        // Update URL (skip on popstate — browser already updated it)
        if (!opts!.popstate) {
          events.emit("beforeHistoryChange", fullUrl);
          if (opts!.replace) {
            history.replaceState({ __rex: pathname }, "", url);
          } else {
            history.pushState({ __rex: pathname }, "", url);
          }
        }

        // Update router state
        updateState(match, url);

        // Update data element
        const dataEl = document.getElementById("__REX_DATA__");
        if (dataEl) dataEl.textContent = JSON.stringify(props);

        // Load page CSS
        updatePageCss(match.pattern);

        // Re-render via the global render callback (set by page entry)
        const page = window.__REX_PAGES && window.__REX_PAGES[match.pattern];
        if (page && window.__REX_RENDER__) {
          window.__REX_RENDER__(page.default, props as Record<string, unknown>);
        }

        // Scroll to top (unless back/forward)
        if (!opts!.popstate) {
          window.scrollTo(0, 0);
        }

        events.emit("routeChangeComplete", fullUrl);
      })
      .catch(function (err) {
        console.error("Rex navigation failed:", err);
        events.emit("routeChangeError", err, fullUrl);
        window.location.href = url;
      });
  }

  // --- Popstate (back/forward) ---

  window.addEventListener("popstate", function (e: PopStateEvent) {
    if (e.state && (e.state as Record<string, unknown>).__rex) {
      navigate(window.location.href, { replace: true, popstate: true });
    }
  });

  // Mark initial page in history state
  history.replaceState(
    { __rex: window.location.pathname },
    "",
    window.location.href,
  );

  // --- Public API ---

  window.__REX_ROUTER = {
    // Navigation methods
    push: function (url: string) {
      return navigate(url);
    },
    replace: function (url: string) {
      return navigate(url, { replace: true });
    },
    back: function () {
      history.back();
    },
    forward: function () {
      history.forward();
    },
    reload: function () {
      window.location.reload();
    },
    prefetch: function (url: string) {
      const a = document.createElement("a");
      a.href = url;
      const pathname = a.pathname;
      if (!prefetchCache[pathname]) {
        prefetchCache[pathname] = fetchPageData(pathname);
      }
    },
    // State (mutable — updated on navigation)
    state: state,
    // Event system
    events: events,
  };
})();
