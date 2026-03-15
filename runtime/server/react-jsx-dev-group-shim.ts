// JSX dev-runtime shim for per-route-group RSC bundles.
//
// In development mode, OXC generates:
//   import { jsxDEV } from 'react/jsx-dev-runtime';
// This shim re-exports from the shared React jsx-dev-runtime on globalThis,
// set by the core bundle.

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const _j = (globalThis as any).__rex_react_jsxDEV_ns;

export const jsxDEV = _j.jsxDEV;
export const Fragment = _j.Fragment;
