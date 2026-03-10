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

export interface UrlObject {
    protocol?: string | null;
    slashes?: boolean | null;
    auth?: string | null;
    host?: string | null;
    hostname?: string | null;
    port?: string | null;
    pathname?: string | null;
    search?: string | null;
    query?: string | Record<string, string> | null;
    hash?: string | null;
    path?: string | null;
    href?: string;
}

export function parse(urlString: string, parseQueryString?: boolean): UrlObject {
    // Try WHATWG URL first for well-formed URLs
    try {
        const u = new globalThis.URL(urlString);
        const result: UrlObject = {
            protocol: u.protocol,
            slashes: true,
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
            const params: Record<string, string> = {};
            u.searchParams.forEach((value: any, key: any) => {
                params[key] = value;
            });
            result.query = params;
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
            const params: Record<string, string> = {};
            const searchStr = result.search.slice(1);
            if (searchStr) {
                for (const pair of searchStr.split('&')) {
                    const eqIdx = pair.indexOf('=');
                    if (eqIdx === -1) {
                        params[decodeURIComponent(pair)] = '';
                    } else {
                        params[decodeURIComponent(pair.slice(0, eqIdx))] =
                            decodeURIComponent(pair.slice(eqIdx + 1));
                    }
                }
            }
            result.query = params;
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
        // Host
        const hostEnd = rest.indexOf('/');
        const hostStr = hostEnd === -1 ? rest : rest.slice(0, hostEnd);
        rest = hostEnd === -1 ? '' : rest.slice(hostEnd);
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
            : Object.entries(urlObj.query).map(([k, v]) =>
                encodeURIComponent(k) + '=' + encodeURIComponent(v)).join('&');
        result += '?' + q;
    }
    if (urlObj.hash) {
        result += urlObj.hash.startsWith('#') ? urlObj.hash : '#' + urlObj.hash;
    }
    return result;
}

export function resolve(from: string, to: string): string {
    return new globalThis.URL(to, from).href;
}

// Re-export WHATWG globals for convenience
export const URL: any = globalThis.URL;
export const URLSearchParams: any = globalThis.URLSearchParams;

export default { parse, format, resolve, URL, URLSearchParams };
