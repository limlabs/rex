// JSX runtime shim for per-route-group RSC bundles.
//
// OXC's automatic JSX transform generates:
//   import { jsx, jsxs } from 'react/jsx-runtime';
// This shim re-exports from the shared React instance on globalThis.

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const _r = (globalThis as any).__rex_react_ns;

// react/jsx-runtime exports jsx, jsxs, Fragment
export const jsx = _r.createElement;
export const jsxs = _r.createElement;
export const Fragment = _r.Fragment;
