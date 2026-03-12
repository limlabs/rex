// cloudflare:sockets polyfill for Rex server bundles.
// Implements the Cloudflare Workers TCP socket API used by pg-cloudflare.
// The connect() function returns an object with readable/writable streams
// that communicate with Rust TCP callbacks (__rex_tcp_*).

/* eslint-disable @typescript-eslint/no-explicit-any */

const _g = globalThis as any;

interface SocketAddress {
    hostname: string;
    port: number;
}

interface Socket {
    readable: ReadableStreamLike;
    writable: WritableStreamLike;
    closed: Promise<void>;
    startTls(options?: { servername?: string }): Socket;
}

interface ReadableStreamLike {
    getReader(): ReadableStreamReader;
}

interface ReadableStreamReader {
    read(): Promise<{ done: boolean; value?: Uint8Array }>;
    releaseLock(): void;
    cancel(): Promise<void>;
}

interface WritableStreamLike {
    getWriter(): WritableStreamWriter;
}

interface WritableStreamWriter {
    write(chunk: Uint8Array | string): Promise<void>;
    close(): Promise<void>;
    releaseLock(): void;
}

function createSocket(connId: number, address: SocketAddress): Socket {
    let closedResolve: () => void;
    const closed = new Promise<void>((resolve) => { closedResolve = resolve; });
    let isClosed = false;

    const readable: ReadableStreamLike = {
        getReader(): ReadableStreamReader {
            return {
                read(): Promise<{ done: boolean; value?: Uint8Array }> {
                    if (isClosed) {
                        return Promise.resolve({ done: true, value: undefined });
                    }
                    return _g.__rex_tcp_read(connId).then((result: { done: boolean; value?: Uint8Array }) => {
                        if (result.done) {
                            isClosed = true;
                            closedResolve();
                        }
                        return result;
                    });
                },
                releaseLock() {},
                cancel(): Promise<void> {
                    if (!isClosed) {
                        isClosed = true;
                        _g.__rex_tcp_close(connId);
                        closedResolve();
                    }
                    return Promise.resolve();
                },
            };
        },
    };

    const writable: WritableStreamLike = {
        getWriter(): WritableStreamWriter {
            return {
                write(chunk: Uint8Array | string): Promise<void> {
                    if (isClosed) {
                        return Promise.reject(new Error('Socket is closed'));
                    }
                    let data: Uint8Array;
                    if (typeof chunk === 'string') {
                        data = new TextEncoder().encode(chunk);
                    } else if (chunk instanceof Uint8Array) {
                        data = chunk;
                    } else if (_g.Buffer && _g.Buffer.isBuffer(chunk)) {
                        data = new Uint8Array((chunk as any).buffer, (chunk as any).byteOffset, (chunk as any).length);
                    } else {
                        data = new Uint8Array(chunk as any);
                    }
                    _g.__rex_tcp_write(connId, data);
                    return Promise.resolve();
                },
                close(): Promise<void> {
                    if (!isClosed) {
                        isClosed = true;
                        _g.__rex_tcp_close(connId);
                        closedResolve();
                    }
                    return Promise.resolve();
                },
                releaseLock() {},
            };
        },
    };

    return {
        readable,
        writable,
        closed,
        startTls(options?: { servername?: string }): Socket {
            const hostname = options?.servername || address.hostname;
            const newConnId = _g.__rex_tcp_start_tls(connId, hostname);
            isClosed = true; // Old connection is consumed
            return createSocket(newConnId, address);
        },
    };
}

export function connect(address: SocketAddress | string, _options?: any): Socket {
    let host: string;
    let port: number;

    if (typeof address === 'string') {
        const parts = address.split(':');
        host = parts[0];
        port = parseInt(parts[1] || '443', 10);
    } else {
        host = address.hostname;
        port = address.port;
    }

    const connId = _g.__rex_tcp_connect(host, port);
    return createSocket(connId, { hostname: host, port });
}

export default { connect };
