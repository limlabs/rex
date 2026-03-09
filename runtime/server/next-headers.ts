// next/headers stubs for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function cookies(): any {
    return {
        get(_name: string) { return undefined; },
        getAll() { return []; },
        set(_name: string, _value: string) {},
        delete(_name: string) {},
        has(_name: string) { return false; },
    };
}

export function headers(): any {
    return new Headers();
}

export function draftMode(): any {
    return { isEnabled: false, enable() {}, disable() {} };
}

const nextHeaders = { cookies, headers, draftMode };
export default nextHeaders;
