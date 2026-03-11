// Node.js `net` module polyfill for Rex server bundles.
// Provides a push-based Socket implementation for pg (node-postgres).
//
// Architecture:
// - Socket.connect() calls __rex_tcp_connect() synchronously (blocking TCP connect)
// - Socket.write() calls __rex_tcp_write() synchronously
// - Reads are PUSH-BASED: Rust polls sockets with non-blocking reads in the IO loop
//   and calls __rex_tcp_push_data/__rex_tcp_push_eof/__rex_tcp_push_error on globalThis
// - These global callbacks look up the Socket in _socketRegistry and emit events
//
// This approach is modeled on how Bun implements net.Socket for V8-like runtimes.
// It eliminates the promise-per-read problem that caused idle connection deadlocks.

import { EventEmitter } from './events';

/* eslint-disable @typescript-eslint/no-explicit-any */

const _g = globalThis as any;

// Socket registry — maps connId → Socket for push-based data from Rust
const _socketRegistry = new Map<number, Socket>();

// Global callbacks for Rust to push data/events to JS.
// These are called from poll_tcp_sockets() in the Rust IO loop.
_g.__rex_tcp_push_data = function(connId: number, data: Uint8Array) {
    const socket = _socketRegistry.get(connId);
    if (socket) {
        socket._onData(data);
    }
};

_g.__rex_tcp_push_eof = function(connId: number) {
    const socket = _socketRegistry.get(connId);
    if (socket) {
        socket._onEnd();
    }
};

_g.__rex_tcp_push_error = function(connId: number, message: string) {
    const socket = _socketRegistry.get(connId);
    if (socket) {
        socket._onError(new Error(message));
    }
};

export class Socket extends EventEmitter {
    private _connId: number | null = null;
    writable: boolean = false;
    readable: boolean = false;
    destroyed: boolean = false;
    private _connecting: boolean = false;

    constructor() {
        super();
    }

    connect(...args: any[]): this {
        let port: number;
        let host: string = '127.0.0.1';
        let callback: (() => void) | undefined;

        if (typeof args[0] === 'object') {
            port = args[0].port;
            host = args[0].host || '127.0.0.1';
            callback = args[1];
        } else {
            port = args[0];
            if (typeof args[1] === 'string') {
                host = args[1];
                callback = args[2];
            } else {
                callback = args[1];
            }
        }

        if (callback) this.once('connect', callback);

        this._connecting = true;

        try {
            this._connId = _g.__rex_tcp_connect(host, port);
            this.writable = true;
            this.readable = true;
            _socketRegistry.set(this._connId!, this);

            // Defer 'connect' event so pg can attach listeners first.
            // pg calls socket.connect() then socket.once('connect', handler).
            // The handler sets up the protocol parser, so it must run before
            // any data is pushed. We use Promise.resolve().then() to defer to
            // the next microtask, which runs during perform_microtask_checkpoint().
            Promise.resolve().then(() => {
                if (this.destroyed || !this._connId) return;
                this._connecting = false;
                if (_g.__rex_tcp_debug) {
                    _g.__rex_tcp_debug('socket EMIT connect connId=' + this._connId);
                }
                this.emit('connect');
                // After connect event, pg has set up its protocol parser.
                // Enable Rust-side polling for this socket.
                _g.__rex_tcp_enable_polling(this._connId);
            });
        } catch (e: any) {
            this._connecting = false;
            Promise.resolve().then(() => this.emit('error', e));
        }

        return this;
    }

    write(data: any, encoding?: any, callback?: any): boolean {
        if (!this.writable || this._connId === null) {
            if (_g.__rex_tcp_debug) {
                _g.__rex_tcp_debug('socket.write BLOCKED connId=' + this._connId + ' writable=' + this.writable);
            }
            return false;
        }

        const cb = typeof encoding === 'function' ? encoding : callback;

        let bytes: Uint8Array;
        if (typeof data === 'string') {
            bytes = new TextEncoder().encode(data);
        } else if (data instanceof Uint8Array) {
            bytes = data;
        } else if (_g.Buffer && _g.Buffer.isBuffer(data)) {
            bytes = new Uint8Array(data.buffer, data.byteOffset, data.length);
        } else {
            bytes = new Uint8Array(data);
        }

        try {
            _g.__rex_tcp_write(this._connId, bytes);
            if (typeof cb === 'function') cb();
        } catch (e: any) {
            this.emit('error', e);
            return false;
        }

        return true;
    }

    end(data?: any, encoding?: any, callback?: any): this {
        if (this.destroyed || !this._connId) return this;

        const cb = typeof data === 'function' ? data :
                   (typeof encoding === 'function' ? encoding : callback);

        if (typeof data !== 'function' && data != null) {
            this.write(data, typeof encoding === 'string' ? encoding : undefined);
        }

        this.writable = false;

        if (this._connId !== null) {
            _g.__rex_tcp_disable_polling(this._connId);
            _socketRegistry.delete(this._connId);
            _g.__rex_tcp_close(this._connId);
            this._connId = null;
        }

        this.readable = false;
        if (typeof cb === 'function') cb();
        this.emit('end');
        this.emit('close');

        return this;
    }

    destroy(error?: Error): this {
        if (this.destroyed) return this;
        this.destroyed = true;
        this.writable = false;
        this.readable = false;

        if (this._connId !== null) {
            _g.__rex_tcp_disable_polling(this._connId);
            _socketRegistry.delete(this._connId);
            _g.__rex_tcp_close(this._connId);
            this._connId = null;
        }

        if (error) this.emit('error', error);
        this.emit('close');

        return this;
    }

    setNoDelay(_noDelay?: boolean): this { return this; }
    setKeepAlive(_enable?: boolean, _initialDelay?: number): this { return this; }
    ref(): this { return this; }
    unref(): this { return this; }
    setTimeout(_timeout: number, _callback?: () => void): this { return this; }

    // Push-based data from Rust (called via __rex_tcp_push_data)
    _onData(data: Uint8Array) {
        const buf = _g.Buffer ? _g.Buffer.from(data) : data;
        if (_g.__rex_tcp_debug) {
            _g.__rex_tcp_debug('socket._onData connId=' + this._connId + ' bytes=' + data.length + ' firstByte=' + String.fromCharCode(data[0]));
        }
        try {
            this.emit('data', buf);
        } catch (e: any) {
            if (_g.__rex_tcp_debug) {
                _g.__rex_tcp_debug('socket._onData EXCEPTION connId=' + this._connId + ': ' + (e.message || e) + ' stack=' + (e.stack || ''));
            }
        }
    }

    // EOF from Rust (called via __rex_tcp_push_eof)
    _onEnd() {
        if (_g.__rex_tcp_debug) {
            _g.__rex_tcp_debug('socket._onEnd connId=' + this._connId);
        }
        this.readable = false;
        if (this._connId !== null) {
            _g.__rex_tcp_disable_polling(this._connId);
            _socketRegistry.delete(this._connId);
            this._connId = null;
        }
        this.writable = false;
        this.destroyed = true;
        this.emit('end');
        this.emit('close');
    }

    // Error from Rust (called via __rex_tcp_push_error)
    _onError(error: Error) {
        if (_g.__rex_tcp_debug) {
            _g.__rex_tcp_debug('socket._onError connId=' + this._connId + ' error=' + error.message);
        }
        this.emit('error', error);
        this.destroy();
    }

    // Expose connId for tls.connect() to use
    get _rexConnId(): number | null { return this._connId; }

    // Replace connId (used by tls.connect for TLS upgrade)
    _setConnId(id: number) {
        if (this._connId !== null) {
            _socketRegistry.delete(this._connId);
        }
        this._connId = id;
        this.writable = true;
        this.readable = true;
        this.destroyed = false;
        _socketRegistry.set(id, this);
    }
}

export function createServer() {
    throw new Error('net.createServer is not supported in Rex server runtime');
}

export function createConnection(...args: any[]) {
    const socket = new Socket();
    return socket.connect(...args);
}

export function connect(...args: any[]) {
    return createConnection(...args);
}

export const isIP = (input: string): number => {
    if (/^(\d{1,3}\.){3}\d{1,3}$/.test(input)) return 4;
    if (input.includes(':')) return 6;
    return 0;
};

export const isIPv4 = (input: string): boolean => isIP(input) === 4;
export const isIPv6 = (input: string): boolean => isIP(input) === 6;

const net = { Socket, createServer, createConnection, connect, isIP, isIPv4, isIPv6 };
export default net;
