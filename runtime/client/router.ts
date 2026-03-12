// Rex Client Router — client-side navigation without full page reloads.
// Reads window.__REX_MANIFEST__ (embedded in HTML) for route-to-chunk mapping.
// Exposes window.__REX_ROUTER with navigation methods, state, and events.
(function () {
  const manifest = window.__REX_MANIFEST__;
  if (!manifest) return;

  const buildId = manifest.build_id;
  const pages = manifest.pages;
  const appRoutes = manifest.app_routes || {};
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

  // --- Base path ---

  const basePath = ((window.__REX_BASE_PATH || "") as string).replace(/\/+$/, "");

  // Strip the base path prefix and any trailing slash from a browser pathname
  // so it can be matched against manifest route patterns (which never include
  // the base path).  e.g. "/rex/features/routing/" → "/features/routing"
  function stripBasePath(pathname: string): string {
    let p = pathname;
    if (basePath && p.startsWith(basePath)) {
      p = p.slice(basePath.length) || "/";
    }
    // Normalise trailing slash (GitHub Pages redirects to trailing slash for
    // directory/index.html files)
    if (p.length > 1 && p.endsWith("/")) {
      p = p.slice(0, -1);
    }
    return p;
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

  function matchAppRoute(pathname: string): RouteMatch | null {
    if (appRoutes[pathname]) return { pattern: pathname, params: {} };
    for (const pattern in appRoutes) {
      if (pattern.indexOf(":") === -1) continue;
      const params = matchDynamic(pattern, pathname);
      if (params) return { pattern, params };
    }
    return null;
  }

  // --- Router state ---

  const initialPathname = stripBasePath(window.location.pathname);
  const initialMatch =
    matchRoute(initialPathname) ||
    matchAppRoute(initialPathname);
  const initialQuery = parseQuery(window.location.search);
  if (initialMatch) {
    for (const k in initialMatch.params) initialQuery[k] = initialMatch.params[k];
  }

  const state: RexRouterState = {
    pathname: initialMatch ? initialMatch.pattern : initialPathname,
    asPath: window.location.pathname + window.location.search,
    query: initialQuery,
    route: initialMatch ? initialMatch.pattern : initialPathname,
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
    // Static export uses index.json for root to avoid dotfile that static servers skip
    const file = window.__REX_STATIC_EXPORT && pathname === "/" ? "/index.json" : pathname + ".json";
    const basePath = ((window.__REX_BASE_PATH || "") as string).replace(/\/+$/, "");
    const dataUrl = basePath + "/_rex/data/" + buildId + file;
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

  // --- App route (RSC) navigation ---

  function navigateApp(
    fullUrl: string,
    pathname: string,
    match: RouteMatch,
    opts: NavigateOpts,
  ): Promise<void> {
    events.emit("routeChangeStart", fullUrl);

    const rscNavigate = window.__REX_RSC_NAVIGATE;
    if (!rscNavigate) {
      window.location.href = fullUrl;
      return Promise.resolve();
    }

    return rscNavigate(pathname)
      .then(function () {
        if (!opts.popstate) {
          events.emit("beforeHistoryChange", fullUrl);
          if (opts.replace) {
            history.replaceState({ __rex: pathname }, "", fullUrl);
          } else {
            history.pushState({ __rex: pathname }, "", fullUrl);
          }
        }
        updateState(match, fullUrl);
        if (!opts.popstate) window.scrollTo(0, 0);
        events.emit("routeChangeComplete", fullUrl);
      })
      .catch(function (err: Error) {
        events.emit("routeChangeError", err, fullUrl);
        window.location.href = fullUrl;
      });
  }

  // --- Navigation ---

  function navigate(url: string, opts?: NavigateOpts): Promise<void> {
    opts = opts || {};

    const a = document.createElement("a");
    a.href = url;
    const rawPathname = a.pathname;
    const pathname = stripBasePath(rawPathname);
    const fullUrl = rawPathname + a.search;

    const match = matchRoute(pathname);
    if (!match) {
      const appMatch = matchAppRoute(pathname);
      if (appMatch) {
        return navigateApp(fullUrl, pathname, appMatch, opts || {});
      }
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
      const pathname = stripBasePath(a.pathname);
      if (matchRoute(pathname)) {
        if (!prefetchCache[pathname]) {
          prefetchCache[pathname] = fetchPageData(pathname);
        }
      }
      // App routes: no-op for now (flight data requires streaming consumer)
    },
    // State (mutable — updated on navigation)
    state: state,
    // Event system
    events: events,
  };
})();
