// Web API polyfills for bare V8 — Blob, File, Event, EventTarget
// Required by undici and other packages that rely on web platform APIs.

/* eslint-disable @typescript-eslint/no-explicit-any */

// ---------- Blob ----------
if (typeof (globalThis as any).Blob === 'undefined') {
    (globalThis as any).Blob = class Blob {
        private _parts: Uint8Array[];
        readonly size: number;
        readonly type: string;

        constructor(parts?: any[], options?: { type?: string }) {
            this.type = options?.type || '';
            this._parts = [];
            let totalSize = 0;
            if (parts) {
                for (const part of parts) {
                    let bytes: Uint8Array;
                    if (part instanceof Uint8Array) {
                        bytes = part;
                    } else if (typeof part === 'string') {
                        bytes = new (globalThis as any).TextEncoder().encode(part);
                    } else if (part instanceof ArrayBuffer) {
                        bytes = new Uint8Array(part);
                    } else if (part && part._parts) {
                        // Another Blob
                        for (const p of part._parts) {
                            this._parts.push(p);
                            totalSize += p.byteLength;
                        }
                        continue;
                    } else {
                        bytes = new (globalThis as any).TextEncoder().encode(String(part));
                    }
                    this._parts.push(bytes);
                    totalSize += bytes.byteLength;
                }
            }
            this.size = totalSize;
        }

        async text(): Promise<string> {
            const buf = await this.arrayBuffer();
            return new (globalThis as any).TextDecoder().decode(buf);
        }

        async arrayBuffer(): Promise<ArrayBuffer> {
            const result = new Uint8Array(this.size);
            let offset = 0;
            for (const part of this._parts) {
                result.set(part, offset);
                offset += part.byteLength;
            }
            return result.buffer;
        }

        slice(start?: number, end?: number, contentType?: string): any {
            const buf = new Uint8Array(this.size);
            let offset = 0;
            for (const part of this._parts) {
                buf.set(part, offset);
                offset += part.byteLength;
            }
            const sliced = buf.slice(start || 0, end || this.size);
            return new (globalThis as any).Blob([sliced], { type: contentType || this.type });
        }

        stream(): any {
            const data = this._parts;
            return new ReadableStream({
                start(controller: any) {
                    for (const chunk of data) controller.enqueue(chunk);
                    controller.close();
                },
            });
        }
    };
}

// ---------- File ----------
if (typeof (globalThis as any).File === 'undefined') {
    (globalThis as any).File = class File extends (globalThis as any).Blob {
        readonly name: string;
        readonly lastModified: number;

        constructor(parts: any[], name: string, options?: { type?: string; lastModified?: number }) {
            super(parts, options);
            this.name = name;
            this.lastModified = options?.lastModified || Date.now();
        }
    };
}

// ---------- Event ----------
if (typeof (globalThis as any).Event === 'undefined') {
    (globalThis as any).Event = class Event {
        readonly type: string;
        readonly bubbles: boolean;
        readonly cancelable: boolean;
        readonly composed: boolean;
        defaultPrevented: boolean;
        readonly target: any;
        readonly currentTarget: any;
        readonly timeStamp: number;
        cancelBubble: boolean;
        returnValue: boolean;
        readonly eventPhase: number;
        readonly isTrusted: boolean;

        constructor(type: string, options?: { bubbles?: boolean; cancelable?: boolean; composed?: boolean }) {
            this.type = type;
            this.bubbles = options?.bubbles || false;
            this.cancelable = options?.cancelable || false;
            this.composed = options?.composed || false;
            this.defaultPrevented = false;
            this.target = null;
            this.currentTarget = null;
            this.timeStamp = Date.now();
            this.cancelBubble = false;
            this.returnValue = true;
            this.eventPhase = 0;
            this.isTrusted = false;
        }

        preventDefault() { this.defaultPrevented = true; }
        stopPropagation() { this.cancelBubble = true; }
        stopImmediatePropagation() { this.cancelBubble = true; }
        composedPath() { return []; }
    };
}

// ---------- EventTarget ----------
if (typeof (globalThis as any).EventTarget === 'undefined') {
    (globalThis as any).EventTarget = class EventTarget {
        private _listeners: Map<string, Set<any>>;

        constructor() {
            this._listeners = new Map();
        }

        addEventListener(type: string, listener: any, _options?: any) {
            if (!listener) return;
            let set = this._listeners.get(type);
            if (!set) {
                set = new Set();
                this._listeners.set(type, set);
            }
            set.add(typeof listener === 'object' ? listener.handleEvent.bind(listener) : listener);
        }

        removeEventListener(type: string, listener: any, _options?: any) {
            const set = this._listeners.get(type);
            if (set) set.delete(listener);
        }

        dispatchEvent(event: any): boolean {
            const set = this._listeners.get(event.type);
            if (!set) return true;
            for (const listener of set) {
                try { listener.call(this, event); } catch { /* ignore */ }
            }
            return !event.defaultPrevented;
        }
    };
}

// ---------- DOMException ----------
if (typeof (globalThis as any).DOMException === 'undefined') {
    (globalThis as any).DOMException = class DOMException extends Error {
        readonly code: number;
        readonly name: string;
        constructor(message?: string, name?: string) {
            super(message);
            this.name = name || 'Error';
            this.code = 0;
        }
    };
}

// ---------- Headers ----------
if (typeof (globalThis as any).Headers === 'undefined') {
    (globalThis as any).Headers = class Headers {
        private _headers: Map<string, string[]>;
        constructor(init?: any) {
            this._headers = new Map();
            if (init) {
                if (init instanceof Headers || (init._headers instanceof Map)) {
                    (init._headers as Map<string, string[]>).forEach((v: string[], k: string) => this._headers.set(k, [...v]));
                } else if (Array.isArray(init)) {
                    for (const [k, v] of init) this.append(k, v);
                } else if (typeof init === 'object') {
                    for (const [k, v] of Object.entries(init)) this.append(k, String(v));
                }
            }
        }
        append(name: string, value: string) {
            const key = name.toLowerCase();
            const vals = this._headers.get(key) || [];
            vals.push(value);
            this._headers.set(key, vals);
        }
        delete(name: string) { this._headers.delete(name.toLowerCase()); }
        get(name: string): string | null {
            const vals = this._headers.get(name.toLowerCase());
            return vals ? vals.join(', ') : null;
        }
        has(name: string): boolean { return this._headers.has(name.toLowerCase()); }
        set(name: string, value: string) { this._headers.set(name.toLowerCase(), [value]); }
        entries(): IterableIterator<[string, string]> {
            const result: [string, string][] = [];
            this._headers.forEach((v, k) => result.push([k, v.join(', ')]));
            return result[Symbol.iterator]();
        }
        keys(): IterableIterator<string> {
            return this._headers.keys();
        }
        values(): IterableIterator<string> {
            const result: string[] = [];
            this._headers.forEach(v => result.push(v.join(', ')));
            return result[Symbol.iterator]();
        }
        forEach(cb: (value: string, key: string, parent: any) => void) {
            this._headers.forEach((v, k) => cb(v.join(', '), k, this));
        }
        [Symbol.iterator]() { return this.entries(); }
        getSetCookie(): string[] {
            return this._headers.get('set-cookie') || [];
        }
    };
}

// ---------- Request ----------
if (typeof (globalThis as any).Request === 'undefined') {
    (globalThis as any).Request = class Request {
        readonly url: string;
        readonly method: string;
        readonly headers: any;
        readonly body: any;
        readonly signal: any;
        readonly mode: string;
        readonly credentials: string;
        readonly redirect: string;
        readonly referrer: string;
        readonly integrity: string;
        constructor(input: any, init?: any) {
            this.url = typeof input === 'string' ? input : input?.url || '';
            this.method = init?.method || 'GET';
            this.headers = new (globalThis as any).Headers(init?.headers);
            this.body = init?.body || null;
            this.signal = init?.signal || null;
            this.mode = init?.mode || 'cors';
            this.credentials = init?.credentials || 'same-origin';
            this.redirect = init?.redirect || 'follow';
            this.referrer = init?.referrer || 'about:client';
            this.integrity = init?.integrity || '';
        }
        clone() { return new (globalThis as any).Request(this.url, { method: this.method, headers: this.headers, body: this.body }); }
        async text() { return typeof this.body === 'string' ? this.body : ''; }
        async json() { return JSON.parse(await this.text()); }
        async arrayBuffer() { return new ArrayBuffer(0); }
        async blob() { return new (globalThis as any).Blob([]); }
        async formData() { return new (globalThis as any).FormData(); }
    };
}

// ---------- Response ----------
if (typeof (globalThis as any).Response === 'undefined') {
    (globalThis as any).Response = class Response {
        readonly body: any;
        readonly headers: any;
        readonly ok: boolean;
        readonly status: number;
        readonly statusText: string;
        readonly type: string;
        readonly url: string;
        readonly redirected: boolean;
        readonly bodyUsed: boolean;
        constructor(body?: any, init?: any) {
            this.body = body || null;
            this.status = init?.status || 200;
            this.statusText = init?.statusText || '';
            this.headers = new (globalThis as any).Headers(init?.headers);
            this.ok = this.status >= 200 && this.status < 300;
            this.type = 'basic';
            this.url = '';
            this.redirected = false;
            this.bodyUsed = false;
        }
        clone() { return new (globalThis as any).Response(this.body, { status: this.status, statusText: this.statusText, headers: this.headers }); }
        async text() { return typeof this.body === 'string' ? this.body : ''; }
        async json() { return JSON.parse(await this.text()); }
        async arrayBuffer() { return new ArrayBuffer(0); }
        async blob() { return new (globalThis as any).Blob([]); }
        async formData() { return new (globalThis as any).FormData(); }
        static json(data: any, init?: any) { return new (globalThis as any).Response(JSON.stringify(data), { ...init, headers: { ...init?.headers, 'content-type': 'application/json' } }); }
        static redirect(url: string, status?: number) { return new (globalThis as any).Response(null, { status: status || 302, headers: { Location: url } }); }
        static error() { return new (globalThis as any).Response(null, { status: 0, type: 'error' }); }
    };
}

// ---------- fetch stub ----------
if (typeof (globalThis as any).fetch === 'undefined') {
    (globalThis as any).fetch = function fetch(_url: any, _init?: any): Promise<any> {
        return Promise.reject(new Error('fetch() is not available in this V8 isolate'));
    };
}
