// Node.js `https` module polyfill for Rex server bundles.
// Wraps http module but defaults protocol to 'https:'.

/* eslint-disable @typescript-eslint/no-explicit-any */

import {
    request as httpRequest,
    get as httpGet,
    createServer,
    ClientRequest,
    IncomingMessage,
    METHODS,
    STATUS_CODES,
} from './http';

function ensureHttps(args: any[]): any[] {
    const first = args[0];
    if (first && typeof first === 'object' && !(first instanceof URL) && !first.protocol) {
        args[0] = { ...first, protocol: 'https:' };
    }
    return args;
}

export function request(...args: any[]): ClientRequest {
    return (httpRequest as any)(...ensureHttps(args));
}

export function get(...args: any[]): ClientRequest {
    return (httpGet as any)(...ensureHttps(args));
}

export { createServer, ClientRequest, IncomingMessage, METHODS, STATUS_CODES };

export default {
    request, get, createServer, ClientRequest, IncomingMessage,
    METHODS, STATUS_CODES,
};
