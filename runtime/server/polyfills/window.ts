/* eslint-disable @typescript-eslint/no-explicit-any */
// window / self polyfill for bare V8
//
// Many client-side libraries and "use client" components reference `window` or
// `self` at module-initialisation time (e.g. `window.addEventListener`,
// `self.crypto`).  In a real browser these are aliases for the global object.
// Without them V8 throws "window is not defined" during SSR.
//
// Setting them to `globalThis` lets typeof checks pass and property lookups
// degrade gracefully (returning `undefined`) instead of throwing.

// Event target no-op methods — must exist on globalThis before we alias
// window/self to it, since client code calls window.addEventListener() etc.
if (typeof (globalThis as any).addEventListener !== 'function') {
    (globalThis as any).addEventListener = function() {};
}
if (typeof (globalThis as any).removeEventListener !== 'function') {
    (globalThis as any).removeEventListener = function() {};
}
if (typeof (globalThis as any).dispatchEvent !== 'function') {
    (globalThis as any).dispatchEvent = function() { return true; };
}

// Location stub — many libs read window.location at init time
if (typeof (globalThis as any).location === 'undefined') {
    (globalThis as any).location = {
        href: 'http://localhost/',
        origin: 'http://localhost',
        protocol: 'http:',
        host: 'localhost',
        hostname: 'localhost',
        port: '',
        pathname: '/',
        search: '',
        hash: '',
        assign() {},
        replace() {},
        reload() {},
    };
}

if (typeof (globalThis as any).window === 'undefined') {
    (globalThis as any).window = globalThis;
}

if (typeof (globalThis as any).self === 'undefined') {
    (globalThis as any).self = globalThis;
}

// document — minimal stub so `typeof document !== 'undefined'` passes and
// simple property access doesn't throw.  SSR code should never rely on real
// DOM APIs, but many libraries perform feature-detection at init time.
if (typeof (globalThis as any).document === 'undefined') {
    const noopEl = () => ({
        setAttribute() {},
        getAttribute() { return null; },
        appendChild() {},
        removeChild() {},
        insertBefore() {},
        addEventListener() {},
        removeEventListener() {},
        classList: { add() {}, remove() {}, toggle() {}, contains() { return false; } },
        style: {},
        dataset: {},
    });
    (globalThis as any).document = {
        createElement() { return noopEl(); },
        createElementNS() { return noopEl(); },
        createTextNode() { return {}; },
        createComment() { return {}; },
        createDocumentFragment() { return { appendChild() {}, querySelectorAll() { return []; } }; },
        getElementById() { return null; },
        querySelector() { return null; },
        querySelectorAll() { return []; },
        getElementsByTagName() { return []; },
        getElementsByClassName() { return []; },
        head: noopEl(),
        body: noopEl(),
        documentElement: { style: {}, setAttribute() {}, getAttribute() { return null; } },
        addEventListener() {},
        removeEventListener() {},
        createEvent() { return { initEvent() {} }; },
    };
}
