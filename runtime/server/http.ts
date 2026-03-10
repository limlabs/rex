// Node.js `http` module polyfill for Rex server bundles.
// Implements http.request() and http.get() using the global fetch() API
// available in V8 isolates, similar to edge runtime shims.

/* eslint-disable @typescript-eslint/no-explicit-any */

import { EventEmitter } from './events';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface RequestOptions {
    protocol?: string;
    hostname?: string;
    host?: string;
    port?: number | string;
    path?: string;
    method?: string;
    headers?: Record<string, string | string[]>;
    timeout?: number;
}

// ---------------------------------------------------------------------------
// IncomingMessage — wraps a fetch Response for callback consumers
// ---------------------------------------------------------------------------

export class IncomingMessage extends EventEmitter {
    statusCode: number;
    statusMessage: string;
    headers: Record<string, string>;
    complete: boolean = false;

    constructor(response: Response) {
        super();
        this.statusCode = response.status;
        this.statusMessage = response.statusText || '';
        this.headers = {};
        response.headers.forEach((value: string, key: string) => {
            this.headers[key.toLowerCase()] = value;
        });
    }

    /** Compatibility — collects all body data into a callback */
    setEncoding(_enc: string): this {
        return this; // fetch returns text by default
    }
}

// ---------------------------------------------------------------------------
// ClientRequest — wraps fetch for the http.request() return value
// ---------------------------------------------------------------------------

export class ClientRequest extends EventEmitter {
    private _options: RequestOptions;
    private _callback?: (res: IncomingMessage) => void;
    private _body: (string | Uint8Array)[] = [];
    private _headers: Record<string, string> = {};
    private _ended: boolean = false;
    private _abortController: AbortController = new AbortController();
    private _timeoutId: ReturnType<typeof setTimeout> | null = null;

    constructor(options: RequestOptions, callback?: (res: IncomingMessage) => void) {
        super();
        this._options = options;
        this._callback = callback;
        if (options.headers) {
            for (const [key, val] of Object.entries(options.headers)) {
                this._headers[key.toLowerCase()] = Array.isArray(val) ? val.join(', ') : val;
            }
        }
    }

    setHeader(name: string, value: string | string[]): void {
        this._headers[name.toLowerCase()] = Array.isArray(value) ? value.join(', ') : value;
    }

    getHeader(name: string): string | undefined {
        return this._headers[name.toLowerCase()];
    }

    removeHeader(name: string): void {
        delete this._headers[name.toLowerCase()];
    }

    write(data: string | Uint8Array): boolean {
        this._body.push(data);
        return true;
    }

    end(
        data?: string | Uint8Array | (() => void),
        encodingOrCallback?: string | (() => void),
        callback?: () => void,
    ): void {
        if (this._ended) return;
        this._ended = true;

        if (typeof data === 'function') {
            callback = data;
            data = undefined;
        } else if (typeof encodingOrCallback === 'function') {
            callback = encodingOrCallback;
        }
        if (data) this.write(data);

        const finish = callback;
        this._execute(finish).catch((err: any) => {
            if ((err as Error).name !== 'AbortError') {
                this.emit('error', err);
            }
        });
    }

    private _buildBody(): string | Uint8Array | undefined {
        if (this._body.length === 0) return undefined;
        // All strings — join them
        if (this._body.every((c) => typeof c === 'string')) {
            return (this._body as string[]).join('');
        }
        // Mix of strings and Uint8Arrays — concatenate as bytes
        const encoder = new TextEncoder();
        const buffers = this._body.map((c) =>
            typeof c === 'string' ? encoder.encode(c) : c,
        );
        let totalLen = 0;
        for (const b of buffers) totalLen += b.byteLength;
        const merged = new Uint8Array(totalLen);
        let offset = 0;
        for (const b of buffers) {
            merged.set(b, offset);
            offset += b.byteLength;
        }
        return merged;
    }

    private async _execute(onFlushed?: () => void): Promise<void> {
        const opts = this._options;
        const protocol = opts.protocol || 'http:';
        const hostname = opts.hostname || opts.host || 'localhost';
        const port = opts.port ? ':' + opts.port : '';
        const path = opts.path || '/';
        const url = protocol + '//' + hostname + port + (path.startsWith('/') ? path : '/' + path);
        const method = (opts.method || 'GET').toUpperCase();

        const fetchInit: RequestInit = {
            method,
            headers: this._headers,
            signal: this._abortController.signal,
        };
        const body = this._buildBody();
        if (body !== undefined && method !== 'GET' && method !== 'HEAD') {
            fetchInit.body = body as any;
        }

        const response = await fetch(url, fetchInit);

        // Signal that the request has been flushed (Node.js end callback semantics)
        if (onFlushed) onFlushed();

        const msg = new IncomingMessage(response);

        if (this._callback) {
            this._callback(msg);
        }
        this.emit('response', msg);

        // Read body as raw bytes and emit as Uint8Array to preserve binary data
        try {
            const buffer = await response.arrayBuffer();
            if (buffer.byteLength > 0) {
                msg.emit('data', new Uint8Array(buffer));
            }
        } catch (bodyErr: any) {
            msg.emit('error', bodyErr);
        }
        if (this._timeoutId !== null) {
            clearTimeout(this._timeoutId);
            this._timeoutId = null;
        }
        msg.complete = true;
        msg.emit('end');
    }

    abort(): void {
        this._abortController.abort();
        this.emit('abort');
    }

    setTimeout(ms: number, callback?: () => void): this {
        if (callback) this.once('timeout', callback);
        if (ms > 0) {
            this._timeoutId = setTimeout(() => this.emit('timeout'), ms);
        }
        return this;
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

function normalizeArgs(
    urlOrOptions: string | URL | RequestOptions,
    optionsOrCallback?: RequestOptions | ((res: IncomingMessage) => void),
    maybeCallback?: (res: IncomingMessage) => void,
): [RequestOptions, ((res: IncomingMessage) => void) | undefined] {
    let options: RequestOptions;
    let callback: ((res: IncomingMessage) => void) | undefined;

    if (typeof urlOrOptions === 'string' || urlOrOptions instanceof URL) {
        const u = typeof urlOrOptions === 'string' ? new URL(urlOrOptions) : urlOrOptions;
        options = {
            protocol: u.protocol,
            hostname: u.hostname,
            port: u.port || undefined,
            path: u.pathname + u.search,
        };
        if (typeof optionsOrCallback === 'function') {
            callback = optionsOrCallback;
        } else if (optionsOrCallback) {
            options = { ...options, ...optionsOrCallback };
            callback = maybeCallback;
        }
    } else {
        options = { ...urlOrOptions };
        if (typeof optionsOrCallback === 'function') {
            callback = optionsOrCallback;
        } else {
            callback = maybeCallback;
        }
    }
    return [options, callback];
}

export function request(
    urlOrOptions: string | URL | RequestOptions,
    optionsOrCallback?: RequestOptions | ((res: IncomingMessage) => void),
    maybeCallback?: (res: IncomingMessage) => void,
): ClientRequest {
    const [options, callback] = normalizeArgs(urlOrOptions, optionsOrCallback, maybeCallback);
    return new ClientRequest(options, callback);
}

export function get(
    urlOrOptions: string | URL | RequestOptions,
    optionsOrCallback?: RequestOptions | ((res: IncomingMessage) => void),
    maybeCallback?: (res: IncomingMessage) => void,
): ClientRequest {
    const [options, callback] = normalizeArgs(urlOrOptions, optionsOrCallback, maybeCallback);
    options.method = 'GET';
    const req = new ClientRequest(options, callback);
    req.end();
    return req;
}

export function createServer(): never {
    throw new Error(
        'http.createServer() is not available in Rex\'s V8 runtime. ' +
        'Rex handles HTTP serving natively. For API routes, export a handler function instead.',
    );
}

export const METHODS = [
    'ACL', 'BIND', 'CHECKOUT', 'CONNECT', 'COPY', 'DELETE', 'GET', 'HEAD',
    'LINK', 'LOCK', 'M-SEARCH', 'MERGE', 'MKACTIVITY', 'MKCALENDAR',
    'MKCOL', 'MOVE', 'NOTIFY', 'OPTIONS', 'PATCH', 'POST', 'PRI',
    'PROPFIND', 'PROPPATCH', 'PURGE', 'PUT', 'REBIND', 'REPORT', 'SEARCH',
    'SOURCE', 'SUBSCRIBE', 'TRACE', 'UNBIND', 'UNLINK', 'UNLOCK',
    'UNSUBSCRIBE',
];

export const STATUS_CODES: Record<number, string> = {
    100: 'Continue', 101: 'Switching Protocols', 102: 'Processing',
    200: 'OK', 201: 'Created', 202: 'Accepted',
    203: 'Non-Authoritative Information', 204: 'No Content',
    205: 'Reset Content', 206: 'Partial Content', 207: 'Multi-Status',
    300: 'Multiple Choices', 301: 'Moved Permanently', 302: 'Found',
    303: 'See Other', 304: 'Not Modified', 307: 'Temporary Redirect',
    308: 'Permanent Redirect',
    400: 'Bad Request', 401: 'Unauthorized', 402: 'Payment Required',
    403: 'Forbidden', 404: 'Not Found', 405: 'Method Not Allowed',
    406: 'Not Acceptable', 407: 'Proxy Authentication Required',
    408: 'Request Timeout', 409: 'Conflict', 410: 'Gone',
    411: 'Length Required', 412: 'Precondition Failed',
    413: 'Payload Too Large', 414: 'URI Too Long',
    415: 'Unsupported Media Type', 416: 'Range Not Satisfiable',
    417: 'Expectation Failed', 418: "I'm a Teapot",
    422: 'Unprocessable Entity', 425: 'Too Early',
    426: 'Upgrade Required', 428: 'Precondition Required',
    429: 'Too Many Requests', 431: 'Request Header Fields Too Large',
    451: 'Unavailable For Legal Reasons',
    500: 'Internal Server Error', 501: 'Not Implemented',
    502: 'Bad Gateway', 503: 'Service Unavailable',
    504: 'Gateway Timeout', 505: 'HTTP Version Not Supported',
    507: 'Insufficient Storage',
};

export default {
    request, get, createServer, ClientRequest, IncomingMessage,
    METHODS, STATUS_CODES,
};
