// @ts-nocheck — CJS bridge, not checked by tsc (processed by rolldown/OXC only)
// Minimal react-dom stub for the RSC server bundle.
// The RSC flight bundle uses react-server-dom-webpack (not react-dom) for
// rendering. However, some node_modules packages (e.g. react-datepicker via
// PayloadCMS) that leak into the server bundle import react-dom. This stub
// provides just enough to prevent "Class extends undefined" errors without
// pulling in real DOM code.

/* eslint-disable @typescript-eslint/no-explicit-any */

const _React = require('react')

// react-dom/server stubs
function renderToString() {
    return ''
}
function renderToStaticMarkup() {
    return ''
}
function renderToReadableStream() {
    return new ReadableStream({
        start(c: any) {
            c.close()
        },
    })
}

// react-dom client stubs
function createRoot() {
    return {
        render: function () {},
        unmount: function () {},
    }
}
function hydrateRoot() {
    return {
        render: function () {},
        unmount: function () {},
    }
}
function createPortal(children: any) {
    return children
}

// flushSync just runs the callback
function flushSync(fn: any) {
    if (typeof fn === 'function') return fn()
}

const reactDom: Record<string, any> = {
    // Core
    createPortal: createPortal,
    flushSync: flushSync,
    // Client
    createRoot: createRoot,
    hydrateRoot: hydrateRoot,
    // Server
    renderToString: renderToString,
    renderToStaticMarkup: renderToStaticMarkup,
    renderToReadableStream: renderToReadableStream,
    // Legacy
    render: function () {},
    hydrate: function () {},
    unmountComponentAtNode: function () {
        return false
    },
    findDOMNode: function () {
        return null
    },
    // Version
    version: _React.version || '19.0.0',
    // React 19 internal API — used by react-server-dom-webpack and react-dom/server
    __DOM_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE: {
        d: {
            f: function () {},
            r: function () {},
            D: function () {},
            C: function () {},
            L: function () {},
            m: function () {},
            X: function () {},
            S: function () {},
            M: function () {},
        },
        p: 0,
        findDOMNode: null,
    },
    // Legacy React 18 internal API
    __SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED: {
        Events: [],
        usingClientEntryPoint: false,
    },
}

module.exports = reactDom
