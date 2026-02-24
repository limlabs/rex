import React from 'react';

/**
 * rex/document exports
 * These components are meaningful during SSR and provide structure for the HTML document.
 * On the client side, they are essentially pass-throughs.
 */

export function Html({ children, ...props }) {
  return React.createElement('html', props, children);
}

export function Head({ children }) {
  return React.createElement('head', null, children);
}

export function Main() {
  // During SSR, this is replaced with the actual page content
  return React.createElement('div', { id: '__rex' });
}

export function NextScript() {
  // During SSR, this is replaced with the actual script tags
  return null;
}

// Default document component
export default function Document() {
  return React.createElement(
    Html,
    null,
    React.createElement(Head, null),
    React.createElement(
      'body',
      null,
      React.createElement(Main, null),
      React.createElement(NextScript, null)
    )
  );
}
