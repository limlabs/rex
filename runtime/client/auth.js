// rex/auth — Client-side auth utilities
// Provides useSession hook, signIn/signOut helpers

let _sessionCache = null;
let _sessionPromise = null;
let _listeners = [];

function notifyListeners() {
  for (const cb of _listeners) {
    try { cb(); } catch { /* empty */ }
  }
}

/**
 * Fetch the current session from the server.
 * Results are cached and shared across all callers.
 */
function fetchSession() {
  if (_sessionPromise) return _sessionPromise;

  _sessionPromise = fetch('/_rex/auth/session', { credentials: 'same-origin' })
    .then(r => r.json())
    .then(data => {
      _sessionCache = data;
      _sessionPromise = null;
      notifyListeners();
      return data;
    })
    .catch(() => {
      _sessionCache = {};
      _sessionPromise = null;
      notifyListeners();
      return {};
    });

  return _sessionPromise;
}

/**
 * React hook for accessing the current user session.
 *
 * Returns `{ data, status }` where:
 * - `status` is `'loading'` | `'authenticated'` | `'unauthenticated'`
 * - `data` is the session object (with `user`, `expires`) or `null`
 */
function useSession() {
  var React = require('react');
  var _React$useState = React.useState(_sessionCache);
  var session = _React$useState[0];
  var setSession = _React$useState[1];

  var _React$useState2 = React.useState(!_sessionCache);
  var loading = _React$useState2[0];
  var setLoading = _React$useState2[1];

  React.useEffect(function () {
    if (!_sessionCache) {
      fetchSession().then(function (data) {
        setSession(data);
        setLoading(false);
      });
    }

    function onUpdate() {
      setSession(_sessionCache);
      setLoading(false);
    }

    _listeners.push(onUpdate);
    return function () {
      _listeners = _listeners.filter(function (cb) { return cb !== onUpdate; });
    };
  }, []);

  if (loading) {
    return { data: null, status: 'loading' };
  }

  if (session && session.user) {
    return { data: session, status: 'authenticated' };
  }

  return { data: null, status: 'unauthenticated' };
}

/**
 * Redirect the user to sign in with the given provider.
 * @param {string} provider - Provider ID (e.g., 'github', 'google')
 * @param {{ callbackUrl?: string }} [options]
 */
function signIn(provider, options) {
  var url = '/_rex/auth/signin?provider=' + encodeURIComponent(provider);
  if (options && options.callbackUrl) {
    url += '&callbackUrl=' + encodeURIComponent(options.callbackUrl);
  }
  window.location.href = url;
}

/**
 * Sign the user out and redirect to the home page.
 * @param {{ callbackUrl?: string }} [options]
 */
function signOut(options) {
  // POST to signout endpoint via form
  var form = document.createElement('form');
  form.method = 'POST';
  form.action = '/_rex/auth/signout';
  form.style.display = 'none';
  if (options && options.callbackUrl) {
    var input = document.createElement('input');
    input.type = 'hidden';
    input.name = 'callbackUrl';
    input.value = options.callbackUrl;
    form.appendChild(input);
  }
  document.body.appendChild(form);
  form.submit();
}

/**
 * Invalidate the cached session and re-fetch from the server.
 */
function refreshSession() {
  _sessionCache = null;
  _sessionPromise = null;
  return fetchSession();
}

exports.useSession = useSession;
exports.signIn = signIn;
exports.signOut = signOut;
exports.refreshSession = refreshSession;
