/**
 * rex/router - Client-side router
 * Handles SPA navigation by fetching page data and re-rendering.
 */

let _initialized = false;
let _currentPath = null;
let _buildId = null;

/**
 * Initialize the client-side router.
 * Called automatically on page load.
 */
export function initializeRouter() {
  if (_initialized) return;
  _initialized = true;

  _currentPath = window.location.pathname;
  _buildId = window.__REX_BUILD_ID__ || '';

  // Listen for browser back/forward
  window.addEventListener('popstate', function(event) {
    var path = window.location.pathname;
    if (path !== _currentPath) {
      _currentPath = path;
      loadPage(path, false);
    }
  });
}

/**
 * Navigate to a new path via client-side routing.
 */
export function navigateTo(path) {
  if (path === _currentPath) return;

  _currentPath = path;
  window.history.pushState(null, '', path);
  loadPage(path, true);
}

/**
 * Get the current route information (React hook style, but simplified).
 */
export function useRouter() {
  return {
    pathname: window.location.pathname,
    query: parseQuery(window.location.search),
    push: navigateTo,
    back: function() { window.history.back(); },
  };
}

async function loadPage(path, isNavigation) {
  try {
    // Fetch page data
    var dataPath = path === '/' ? '/index' : path;
    var dataUrl = '/_rex/data/' + _buildId + dataPath + '.json';

    var response = await fetch(dataUrl);

    if (response.status === 404) {
      // Build ID mismatch or route not found - full reload
      window.location.href = path;
      return;
    }

    if (!response.ok) {
      throw new Error('Failed to fetch page data: ' + response.status);
    }

    var data = await response.json();

    if (data.redirect) {
      navigateTo(data.redirect.destination);
      return;
    }

    if (data.notFound) {
      // Show 404
      window.location.href = path;
      return;
    }

    // For the prototype, we do a full page reload
    // A full implementation would dynamically import the page module
    // and re-render using window.__REX_ROOT__.render()
    window.location.href = path;

  } catch (err) {
    console.error('[Rex Router] Navigation error:', err);
    window.location.href = path;
  }
}

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

// Auto-initialize when loaded in browser
if (typeof window !== 'undefined') {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initializeRouter);
  } else {
    initializeRouter();
  }
}
