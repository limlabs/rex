// next/headers stubs for Rex server bundles.
// Reads request context from globals set by Rust before each render.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function cookies(): any {
    const cookieObj: Record<string, string> =
        (globalThis as any).__rex_request_cookies || {};
    return {
        get(name: string) {
            const val = cookieObj[name];
            return val !== undefined ? { name, value: val } : undefined;
        },
        getAll() {
            return Object.entries(cookieObj).map(([name, value]) => ({ name, value }));
        },
        set(_name: string, _value: string) {},
        delete(_name: string) {},
        has(name: string) { return name in cookieObj; },
    };
}

export function headers(): any {
    const raw: Record<string, string> =
        (globalThis as any).__rex_request_headers || {};
    const h = new Headers();
    for (const [k, v] of Object.entries(raw)) {
        h.set(k, v);
    }
    return h;
}

export function draftMode(): any {
    return { isEnabled: false, enable() {}, disable() {} };
}

const nextHeaders = { cookies, headers, draftMode };
export default nextHeaders;
