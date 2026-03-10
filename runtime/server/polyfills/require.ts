// Minimal `require` polyfill for bare V8.
// Some CJS dependencies emit runtime require() calls that rolldown doesn't
// transform. This shim provides stubs for known Node.js builtins.

/* eslint-disable @typescript-eslint/no-explicit-any */

if (typeof (globalThis as any).require !== 'function') {
    const modules: Record<string, any> = {
        'node:querystring': {
            stringify(obj: any) {
                return Object.entries(obj || {}).map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`).join('&');
            },
            parse(str: string) {
                const result: any = {};
                for (const pair of (str || '').split('&')) {
                    const [k, v] = pair.split('=');
                    if (k) result[decodeURIComponent(k)] = decodeURIComponent(v || '');
                }
                return result;
            },
            encode(obj: any) { return modules['node:querystring'].stringify(obj); },
            decode(str: string) { return modules['node:querystring'].parse(str); },
        },
        'querystring': undefined as any, // alias set below
        'node:diagnostics_channel': {
            channel() { return { subscribe() {}, unsubscribe() {}, hasSubscribers: false }; },
            subscribe() {},
            unsubscribe() {},
            Channel: class { subscribe() {} unsubscribe() {} get hasSubscribers() { return false; } },
        },
        'node:util': {
            inherits(ctor: any, superCtor: any) {
                if (superCtor) Object.setPrototypeOf(ctor.prototype, superCtor.prototype);
            },
            deprecate(fn: any) { return fn; },
            promisify(fn: any) { return fn; },
            inspect(obj: any) { try { return JSON.stringify(obj); } catch { return String(obj); } },
            types: { isDate: (v: any) => v instanceof Date },
        },
        'node:events': {
            EventEmitter: class {
                on() { return this; }
                once() { return this; }
                off() { return this; }
                emit() { return false; }
                removeListener() { return this; }
                addListener() { return this; }
                removeAllListeners() { return this; }
                listeners() { return []; }
                listenerCount() { return 0; }
            },
        },
        'node:assert': {
            ok(v: any, m?: string) { if (!v) throw new Error(m || 'Assertion failed'); },
            strictEqual(a: any, b: any, m?: string) { if (a !== b) throw new Error(m || 'Not equal'); },
        },
        'node:process': (globalThis as any).process || {},
        'node:http2': {
            constants: {
                HTTP2_HEADER_AUTHORITY: ':authority',
                HTTP2_HEADER_METHOD: ':method',
                HTTP2_HEADER_PATH: ':path',
                HTTP2_HEADER_SCHEME: ':scheme',
                HTTP2_HEADER_STATUS: ':status',
                HTTP2_HEADER_CONTENT_TYPE: 'content-type',
                HTTP2_HEADER_CONTENT_LENGTH: 'content-length',
                HTTP2_HEADER_ACCEPT: 'accept',
                HTTP2_HEADER_ACCEPT_ENCODING: 'accept-encoding',
                HTTP2_HEADER_AUTHORIZATION: 'authorization',
                HTTP2_HEADER_CACHE_CONTROL: 'cache-control',
                HTTP2_HEADER_CONTENT_ENCODING: 'content-encoding',
                HTTP2_HEADER_COOKIE: 'cookie',
                HTTP2_HEADER_HOST: 'host',
                HTTP2_HEADER_LOCATION: 'location',
                HTTP2_HEADER_SET_COOKIE: 'set-cookie',
                HTTP2_HEADER_USER_AGENT: 'user-agent',
                NGHTTP2_NO_ERROR: 0,
                NGHTTP2_CANCEL: 8,
            },
            connect() { throw new Error('http2.connect() not supported'); },
        },
        'node:perf_hooks': {
            performance: typeof performance !== 'undefined' ? performance : { now() { return Date.now(); } },
            PerformanceObserver: class { observe() {} disconnect() {} },
        },
        'node:async_hooks': {
            AsyncResource: class AsyncResource {
                type: string;
                constructor(type: string, _opts?: any) { this.type = type; }
                runInAsyncScope(fn: (...args: any[]) => any, thisArg: any, ...args: any[]) { return fn.apply(thisArg, args); }
                emitBefore() {}
                emitAfter() {}
                emitDestroy() {}
                asyncId() { return 0; }
                triggerAsyncId() { return 0; }
                bind(fn: any) { return fn; }
                static bind(fn: any) { return fn; }
            },
            AsyncLocalStorage: class AsyncLocalStorage {
                getStore() { return undefined; }
                run(_store: any, fn: (...args: any[]) => any, ...args: any[]) { return fn(...args); }
                enterWith() {}
                disable() {}
            },
            executionAsyncId() { return 0; },
            triggerAsyncId() { return 0; },
            createHook() { return { enable() {}, disable() {} }; },
        },
    };
    // drizzle-kit/api — used by PayloadCMS's @payloadcms/drizzle for schema
    // push/migration. During SSR we don't run migrations, but the require() call
    // still happens during initialization. Return stubs that act as no-ops.
    modules['drizzle-kit/api'] = {
        generateDrizzleJson: async (_schema: any) => ({ id: '', version: '7', tables: {}, enums: {}, schemas: {}, views: {}, sequences: {}, _meta: { tables: {}, columns: {} } }),
        generateMigration: async () => [],
        pushSchema: async () => ({ apply: async () => {}, hasDataLoss: false, warnings: [] }),
        upPgSnapshot: (snapshot: any) => snapshot,
    };
    modules['querystring'] = modules['node:querystring'];
    modules['diagnostics_channel'] = modules['node:diagnostics_channel'];
    modules['util'] = modules['node:util'];
    modules['events'] = modules['node:events'];
    modules['assert'] = modules['node:assert'];
    modules['process'] = modules['node:process'];
    modules['http2'] = modules['node:http2'];
    modules['perf_hooks'] = modules['node:perf_hooks'];
    modules['async_hooks'] = modules['node:async_hooks'];

    (globalThis as any).require = function require(id: string): any {
        if (id in modules) return modules[id];
        // Return an empty object for unknown modules to avoid crashing.
        // Dependencies often require() inside try/catch or feature detection.
        return {};
    };
}
