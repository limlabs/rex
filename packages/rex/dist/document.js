import React from 'react';
export function Html({ children, ...props }) {
    return React.createElement('html', props, children);
}
export function Head({ children }) {
    return React.createElement('head', null, children);
}
export function Main() {
    return React.createElement('div', { id: '__rex' });
}
export function NextScript() {
    return null;
}
export default function Document() {
    return React.createElement(Html, null, React.createElement(Head, null), React.createElement('body', null, React.createElement(Main, null), React.createElement(NextScript, null)));
}
//# sourceMappingURL=document.js.map