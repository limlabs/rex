// Node.js `querystring` module polyfill for Rex server bundles.
// Implemented using the URLSearchParams API available in V8 polyfills.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function escape(str: string): string {
    return encodeURIComponent(str);
}

export function unescape(str: string): string {
    return decodeURIComponent(str.replace(/\+/g, ' '));
}

export function stringify(
    obj: Record<string, any>,
    sep: string = '&',
    eq: string = '=',
): string {
    if (!obj || typeof obj !== 'object') return '';
    const pairs: string[] = [];
    for (const key of Object.keys(obj)) {
        const value = obj[key];
        const encodedKey = escape(key);
        if (Array.isArray(value)) {
            for (const v of value) {
                pairs.push(encodedKey + eq + escape(String(v)));
            }
        } else if (value !== undefined && value !== null) {
            pairs.push(encodedKey + eq + escape(String(value)));
        } else {
            pairs.push(encodedKey + eq);
        }
    }
    return pairs.join(sep);
}

export const encode = stringify;

export function parse(
    str: string,
    sep: string = '&',
    eq: string = '=',
): Record<string, string | string[]> {
    const result: Record<string, string | string[]> = Object.create(null);
    if (typeof str !== 'string' || str.length === 0) return result;

    for (const pair of str.split(sep)) {
        const eqIdx = pair.indexOf(eq);
        let key: string;
        let value: string;
        if (eqIdx === -1) {
            key = unescape(pair);
            value = '';
        } else {
            key = unescape(pair.slice(0, eqIdx));
            value = unescape(pair.slice(eqIdx + eq.length));
        }
        const existing = result[key];
        if (existing !== undefined) {
            if (Array.isArray(existing)) {
                existing.push(value);
            } else {
                result[key] = [existing, value];
            }
        } else {
            result[key] = value;
        }
    }
    return result;
}

export const decode = parse;

export default { stringify, parse, encode, decode, escape, unescape };
