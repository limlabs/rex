// JSX runtime shim for per-route-group RSC bundles.
//
// OXC's automatic JSX transform generates:
//   import { jsx, jsxs } from 'react/jsx-runtime';
// This shim re-exports from the shared React jsx-runtime on globalThis,
// set by the core bundle.
//
// IMPORTANT: jsx/jsxs are NOT createElement — they have different signatures:
//   jsx(type, { children, ...props }, key)   — children in props object
//   createElement(type, props, ...children)  — children as rest args
// Using createElement breaks child handling, causing every element to
// produce "missing key" warnings and corrupting RSC flight data.

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const _j = (globalThis as any).__rex_react_jsx_ns;

export const jsx = _j.jsx;
export const jsxs = _j.jsxs;
export const Fragment = _j.Fragment;
