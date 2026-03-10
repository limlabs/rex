// Node.js `url` module polyfill for Rex server bundles.
// Provides url.parse(), url.format(), and url.resolve() on top of the
// WHATWG URL/URLSearchParams globals installed by V8 polyfills banner.

/* eslint-disable @typescript-eslint/no-explicit-any */
/* eslint-disable no-shadow-restricted-names -- augmenting globalThis for V8 runtime bindings */

declare const globalThis: {
    URL: any;
    URLSearchParams: any;
};
/* eslint-enable no-shadow-restricted-names */

function safeDecode(str: string): string {
    try {
        return decodeURIComponent(str);
    } catch {
        return str;
    }
}

export interface UrlObject {
    protocol?: string | null;
    slashes?: boolean | null;
    auth?: string | null;
    host?: string | null;
    hostname?: string | null;
    port?: string | null;
    pathname?: string | null;
    search?: string | null;
    query?: string | Record<string, string | string[]> | null;
    hash?: string | null;
    path?: string | null;
    href?: string;
}

function parseQueryParams(searchStr: string): Record<string, string | string[]> {
    const params: Record<string, string | string[]> = {};
    if (!searchStr) return params;
    for (const pair of searchStr.split('&')) {
        const eqIdx = pair.indexOf('=');
        let key: string;
        let value: string;
        if (eqIdx === -1) {
            key = safeDecode(pair);
            value = '';
        } else {
            key = safeDecode(pair.slice(0, eqIdx));
            value = safeDecode(pair.slice(eqIdx + 1));
        }
        const existing = params[key];
        if (existing === undefined) {
            params[key] = value;
        } else if (Array.isArray(existing)) {
            existing.push(value);
        } else {
            params[key] = [existing, value];
        }
    }
    return params;
}

// Protocols that use // slashes (non-exhaustive, covers common cases)
const SLASHED_PROTOCOLS = new Set([
    'http:', 'https:', 'ftp:', 'ftps:', 'gopher:', 'ws:', 'wss:', 'file:',
]);

export function parse(urlString: string, parseQueryString?: boolean): UrlObject {
    // Try WHATWG URL first for well-formed URLs
    try {
        const u = new globalThis.URL(urlString);
        const hasSlashes = SLASHED_PROTOCOLS.has(u.protocol);
        const result: UrlObject = {
            protocol: u.protocol,
            slashes: hasSlashes,
            auth: u.username ? (u.password ? u.username + ':' + u.password : u.username) : null,
            host: u.host,
            hostname: u.hostname,
            port: u.port || null,
            pathname: u.pathname,
            search: u.search || null,
            hash: u.hash || null,
            href: u.href,
            path: u.pathname + (u.search || ''),
            query: null,
        };
        if (parseQueryString && u.search) {
            result.query = parseQueryParams(u.search.slice(1));
        } else {
            result.query = u.search ? u.search.slice(1) : null;
        }
        return result;
    } catch {
        // Fall back to manual parsing for relative URLs or non-standard formats
        return parseFallback(urlString, parseQueryString);
    }
}

function parseFallback(urlString: string, parseQueryString?: boolean): UrlObject {
    const result: UrlObject = {
        protocol: null, slashes: null, auth: null,
        host: null, hostname: null, port: null,
        pathname: null, search: null, query: null,
        hash: null, path: null, href: urlString,
    };

    let rest = urlString;

    // Hash
    const hashIdx = rest.indexOf('#');
    if (hashIdx !== -1) {
        result.hash = rest.slice(hashIdx);
        rest = rest.slice(0, hashIdx);
    }

    // Query
    const qIdx = rest.indexOf('?');
    if (qIdx !== -1) {
        result.search = rest.slice(qIdx);
        rest = rest.slice(0, qIdx);
        if (parseQueryString) {
            result.query = parseQueryParams(result.search.slice(1));
        } else {
            result.query = result.search.slice(1);
        }
    }

    // Protocol
    const protoMatch = rest.match(/^([a-z][a-z0-9+\-.]*:)/i);
    if (protoMatch) {
        result.protocol = protoMatch[1].toLowerCase();
        rest = rest.slice(protoMatch[1].length);
    }

    // Slashes + authority
    if (rest.startsWith('//')) {
        result.slashes = true;
        rest = rest.slice(2);
        // Auth
        const atIdx = rest.indexOf('@');
        const slashIdx = rest.indexOf('/');
        if (atIdx !== -1 && (slashIdx === -1 || atIdx < slashIdx)) {
            result.auth = rest.slice(0, atIdx);
            rest = rest.slice(atIdx + 1);
        }
        // Host — handle IPv6 brackets
        const hostEnd = rest.indexOf('/');
        const hostStr = hostEnd === -1 ? rest : rest.slice(0, hostEnd);
        rest = hostEnd === -1 ? '' : rest.slice(hostEnd);

        if (hostStr.startsWith('[')) {
            // IPv6 literal: [::1] or [::1]:port
            const bracketEnd = hostStr.indexOf(']');
            if (bracketEnd !== -1) {
                result.hostname = hostStr.slice(0, bracketEnd + 1);
                const afterBracket = hostStr.slice(bracketEnd + 1);
                if (afterBracket.startsWith(':')) {
                    result.port = afterBracket.slice(1);
                }
                result.host = hostStr;
            } else {
                result.hostname = hostStr;
                result.host = hostStr;
            }
        } else {
            const colonIdx = hostStr.lastIndexOf(':');
            if (colonIdx !== -1) {
                result.hostname = hostStr.slice(0, colonIdx);
                result.port = hostStr.slice(colonIdx + 1);
                result.host = hostStr;
            } else {
                result.hostname = hostStr;
                result.host = hostStr;
            }
        }
    }

    result.pathname = rest || (result.host ? '/' : null);
    result.path = (result.pathname || '') + (result.search || '');

    return result;
}

export function format(urlObj: UrlObject): string {
    let result = '';
    if (urlObj.protocol) {
        result += urlObj.protocol;
        if (!urlObj.protocol.endsWith(':')) result += ':';
    }
    if (urlObj.slashes || (urlObj.protocol && urlObj.host)) {
        result += '//';
    }
    if (urlObj.auth) {
        result += urlObj.auth + '@';
    }
    if (urlObj.host) {
        result += urlObj.host;
    } else {
        if (urlObj.hostname) result += urlObj.hostname;
        if (urlObj.port) result += ':' + urlObj.port;
    }
    if (urlObj.pathname) {
        result += urlObj.pathname;
    }
    if (urlObj.search) {
        result += urlObj.search.startsWith('?') ? urlObj.search : '?' + urlObj.search;
    } else if (urlObj.query) {
        const q = typeof urlObj.query === 'string'
            ? urlObj.query
            : Object.entries(urlObj.query).map(([k, v]) => {
                if (Array.isArray(v)) {
                    return v.map((item) => encodeURIComponent(k) + '=' + encodeURIComponent(item)).join('&');
                }
                return encodeURIComponent(k) + '=' + encodeURIComponent(v);
            }).join('&');
        result += '?' + q;
    }
    if (urlObj.hash) {
        result += urlObj.hash.startsWith('#') ? urlObj.hash : '#' + urlObj.hash;
    }
    return result;
}

export function resolve(from: string, to: string): string {
    // WHATWG URL requires an absolute base; if from is relative, prepend a
    // dummy origin so the resolution still works, then strip it afterward.
    try {
        return new globalThis.URL(to, from).href;
    } catch {
        const dummy = 'http://__rex_dummy__';
        const resolved = new globalThis.URL(to, dummy + from).href;
        return resolved.slice(dummy.length);
    }
}

// Re-export WHATWG globals for convenience
export const URL: any = globalThis.URL;
export const URLSearchParams: any = globalThis.URLSearchParams;

export default { parse, format, resolve, URL, URLSearchParams };
