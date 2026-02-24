// Rex Client Chunk - Auto-generated
(function() {
'use strict';
import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
export default function About() {
    return /*#__PURE__*/ _jsxs("div", {
        children: [
            /*#__PURE__*/ _jsx("h1", {
                children: "About"
            }),
            /*#__PURE__*/ _jsx("p", {
                children: "Rex is a Next.js Pages Router reimplemented in Rust."
            }),
            /*#__PURE__*/ _jsx("a", {
                href: "/",
                children: "Back to home"
            })
        ]
    });
}


  var React = window.React;
  var ReactDOM = window.ReactDOM;
  if (typeof exports !== 'undefined' && exports.default) {
    var dataEl = document.getElementById('__REX_DATA__');
    var pageProps = dataEl ? JSON.parse(dataEl.textContent) : {};
    var container = document.getElementById('__rex');
    if (container && ReactDOM.hydrateRoot) {
      var element = React.createElement(exports.default, pageProps);
      window.__REX_ROOT__ = ReactDOM.hydrateRoot(container, element);
    }
  }
})();
