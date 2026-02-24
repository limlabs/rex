// Rex Client Entry Template
// This template is instantiated per-page by the build system.
// Variables __REX_PAGE_IMPORT__ and __REX_APP_IMPORT__ are replaced at build time.
'use strict';

(function() {
  var dataEl = document.getElementById('__REX_DATA__');
  var pageProps = dataEl ? JSON.parse(dataEl.textContent) : {};
  var container = document.getElementById('__rex');

  if (container && window.ReactDOM && window.ReactDOM.hydrateRoot) {
    var Page = window.__REX_PAGE_COMPONENT__;
    var element = window.React.createElement(Page, pageProps);

    if (window.__REX_APP_COMPONENT__) {
      var App = window.__REX_APP_COMPONENT__;
      element = window.React.createElement(App, {
        Component: Page,
        pageProps: pageProps
      });
    }

    window.__REX_ROOT__ = window.ReactDOM.hydrateRoot(container, element);
  }
})();
