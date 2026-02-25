// Rex Link Component - equivalent to next/link
// Works in both server (V8) and client (browser) environments.
(function() {
  var isServer = typeof window === 'undefined';
  var React = isServer ? globalThis.__React : window.React;

  function Link(props) {
    var href = props.href;
    var replace = props.replace || false;
    var children = props.children;
    var target = props.target;

    // Build <a> props, forwarding className, style, id, etc.
    var aProps = { href: href };
    if (props.className) aProps.className = props.className;
    if (props.style) aProps.style = props.style;
    if (props.id) aProps.id = props.id;
    if (target) aProps.target = target;

    if (!isServer) {
      aProps.onClick = function(e) {
        // Call user onClick if provided
        if (props.onClick) props.onClick(e);
        if (e.defaultPrevented) return;

        // Skip client-side navigation for:
        // - modifier keys (new tab/window)
        // - non-left clicks
        // - target="_blank" or external links
        // - hash-only links
        if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
        if (e.button !== 0) return;
        if (target && target !== '_self') return;

        // Check if it's an internal link
        try {
          var url = new URL(href, window.location.origin);
          if (url.origin !== window.location.origin) return;
        } catch (_) {
          return;
        }

        e.preventDefault();

        var router = window.__REX_ROUTER;
        if (router) {
          if (replace) {
            router.replace(href);
          } else {
            router.push(href);
          }
        } else {
          // Fallback: full page navigation
          window.location.href = href;
        }
      };

      aProps.onMouseEnter = function() {
        var router = window.__REX_ROUTER;
        if (router && router.prefetch) {
          router.prefetch(href);
        }
      };
    }

    return React.createElement('a', aProps, children);
  }

  // Register in the appropriate global
  if (isServer) {
    globalThis.__rex_link_component = Link;
  } else {
    window.__REX_LINK__ = Link;
  }
})();
