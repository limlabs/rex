// Node.js `url` module polyfill for Rex server bundles.
// Provides fileURLToPath, pathToFileURL, and URL/URLSearchParams re-exports.
// Named url-module.ts to avoid collision with polyfills/url.ts (banner polyfill).

/* eslint-disable @typescript-eslint/no-explicit-any */

export function fileURLToPath(url: string | URL): string {
    const urlStr = typeof url === 'string' ? url : url.href;
    if (!urlStr.startsWith('file://')) {
        throw new TypeError('The URL must be of scheme file');
    }
    // Remove file:// prefix and decode percent-encoding
    let path = urlStr.slice(7);
    // Handle file:///path (three slashes = absolute path)
    if (path.startsWith('/')) {
        path = decodeURIComponent(path);
    }
    return path;
}

export function pathToFileURL(path: string): URL {
    // Ensure absolute path starts with /
    const absPath = path.startsWith('/') ? path : '/' + path;
    return new URL('file://' + encodeURI(absPath));
}

export function parse(urlString: string): any {
    try {
        const u = new URL(urlString);
        return {
            protocol: u.protocol,
            slashes: true,
            auth: u.username ? (u.password ? `${u.username}:${u.password}` : u.username) : null,
            host: u.host,
            port: u.port || null,
            hostname: u.hostname,
            hash: u.hash || null,
            search: u.search || null,
            query: u.search ? u.search.slice(1) : null,
            pathname: u.pathname,
            path: u.pathname + (u.search || ''),
            href: u.href,
        };
    } catch {
        return { href: urlString };
    }
}

export function resolve(from: string, to: string): string {
    return new URL(to, from).href;
}

export function format(urlObj: any): string {
    if (urlObj instanceof URL) return urlObj.href;
    if (typeof urlObj === 'string') return urlObj;
    let result = '';
    if (urlObj.protocol) result += urlObj.protocol + '//';
    if (urlObj.auth) result += urlObj.auth + '@';
    if (urlObj.hostname) result += urlObj.hostname;
    if (urlObj.port) result += ':' + urlObj.port;
    if (urlObj.pathname) result += urlObj.pathname;
    if (urlObj.search) result += urlObj.search;
    if (urlObj.hash) result += urlObj.hash;
    return result;
}

// Re-export built-in URL and URLSearchParams
const _URL = (globalThis as any).URL;
const _URLSearchParams = (globalThis as any).URLSearchParams;

export { _URL as URL, _URLSearchParams as URLSearchParams };

const url = { fileURLToPath, pathToFileURL, parse, resolve, format, URL: _URL, URLSearchParams: _URLSearchParams };
export default url;
