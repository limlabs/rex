// Node.js `tls` module stub for Rex server bundles.
// Empty stub — TLS is handled by the cloudflare:sockets startTls() API.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function connect(_options: any, _callback?: any) {
    throw new Error('tls.connect is not supported in Rex server runtime — use cloudflare:sockets startTls()');
}

export function createServer() {
    throw new Error('tls.createServer is not supported in Rex server runtime');
}

export function createSecureContext() {
    return {};
}

const tls = { connect, createServer, createSecureContext };
export default tls;
