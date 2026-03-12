// React shim for per-route-group RSC bundles.
//
// The core RSC bundle exports the React namespace to globalThis.__rex_react_ns.
// Group bundles alias "react" to this shim so they share the same React
// instance (and therefore the same hooks dispatcher, context, etc.).
//
// This file is NOT used directly — it's referenced via rolldown aliases
// in group IIFE builds.

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const _r = (globalThis as any).__rex_react_ns;
export default _r;

// Re-export commonly used APIs so named imports work:
//   import { createElement, useState } from 'react';
export const {
  Children,
  Component,
  Fragment,
  PureComponent,
  StrictMode,
  Suspense,
  cache,
  cloneElement,
  createContext,
  createElement,
  createRef,
  forwardRef,
  isValidElement,
  lazy,
  memo,
  startTransition,
  use,
  useCallback,
  useContext,
  useDebugValue,
  useDeferredValue,
  useEffect,
  useId,
  useImperativeHandle,
  useInsertionEffect,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  useSyncExternalStore,
  useTransition,
  version,
} = _r;
