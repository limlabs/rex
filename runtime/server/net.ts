// Node.js `net` module stub for Rex server bundles.
// Intentionally does NOT export a working Socket class.
// This causes pg to fall back to pg-cloudflare's CloudflareSocket,
// which we polyfill via the `cloudflare:sockets` module.

/* eslint-disable @typescript-eslint/no-explicit-any */

// pg checks: `typeof net.Socket === 'function'`
// By not providing Socket, pg takes the CloudflareSocket path.

export function createServer() {
    throw new Error('net.createServer is not supported in Rex server runtime');
}

export function createConnection() {
    throw new Error('net.createConnection is not supported in Rex server runtime');
}

export function connect() {
    throw new Error('net.connect is not supported in Rex server runtime');
}

export const isIP = (input: string): number => {
    if (/^(\d{1,3}\.){3}\d{1,3}$/.test(input)) return 4;
    if (input.includes(':')) return 6;
    return 0;
};

export const isIPv4 = (input: string): boolean => isIP(input) === 4;
export const isIPv6 = (input: string): boolean => isIP(input) === 6;

const net = { createServer, createConnection, connect, isIP, isIPv4, isIPv6 };
export default net;
