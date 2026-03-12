// Node.js `tls` module polyfill for Rex server bundles.
// Supports tls.connect({ socket: existingNetSocket, servername }) for pg's SSL upgrade.
// Uses __rex_tcp_start_tls() to upgrade the underlying TCP connection in Rust.

import { Socket } from './net';

/* eslint-disable @typescript-eslint/no-explicit-any */

const _g = globalThis as any;

export function connect(options: any, callback?: () => void) {
    if (typeof options === 'number' || typeof options === 'string') {
        throw new Error('tls.connect with port/host is not supported — use { socket } option');
    }

    const existingSocket = options.socket;
    if (!existingSocket || typeof existingSocket._rexConnId === 'undefined') {
        throw new Error('tls.connect requires a net.Socket with an active connection');
    }

    const hostname = options.servername || options.host || 'localhost';
    const oldConnId = existingSocket._rexConnId;

    // Disable polling on the old connection (it's being consumed by TLS upgrade)
    _g.__rex_tcp_disable_polling(oldConnId);

    // Upgrade the TCP connection to TLS in Rust (synchronous handshake)
    const newConnId = _g.__rex_tcp_start_tls(oldConnId, hostname);

    // Create a new Socket wrapper for the TLS connection
    const tlsSocket = new Socket();
    tlsSocket._setConnId(newConnId);

    // Enable polling on the new TLS connection
    _g.__rex_tcp_enable_polling(newConnId);

    if (callback) tlsSocket.once('secureConnect', callback);

    // Defer secureConnect event (TLS handshake is already done synchronously)
    Promise.resolve().then(() => {
        tlsSocket.emit('secureConnect');
    });

    return tlsSocket;
}

export function createServer() {
    throw new Error('tls.createServer is not supported in Rex server runtime');
}

export function createSecureContext() {
    return {};
}

const tls = { connect, createServer, createSecureContext };
export default tls;
