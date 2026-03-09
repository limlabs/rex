// Node.js `dns` module stub for Rex server bundles.
// pg uses dns.lookup() to resolve hostnames before connecting.
// In Rex, TCP connections go through Rust's tokio which handles DNS natively,
// so we pass the hostname through unchanged.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function lookup(hostname: string, options: any, callback?: any) {
    const cb = typeof options === 'function' ? options : callback;
    // Pass hostname through — Rust/tokio handles actual DNS resolution
    if (typeof cb === 'function') {
        (globalThis as any).queueMicrotask(() => cb(null, hostname, 4));
    }
}

export function resolve(hostname: string, callback: any) {
    if (typeof callback === 'function') {
        (globalThis as any).queueMicrotask(() => callback(null, [hostname]));
    }
}

export function resolve4(hostname: string, callback: any) {
    resolve(hostname, callback);
}

export function resolve6(hostname: string, callback: any) {
    resolve(hostname, callback);
}

export const promises = {
    lookup(hostname: string) {
        return Promise.resolve({ address: hostname, family: 4 });
    },
    resolve(hostname: string) {
        return Promise.resolve([hostname]);
    },
};

const dns = { lookup, resolve, resolve4, resolve6, promises };
export default dns;
