// Node.js `util` module stub for Rex server bundles.
// Provides minimal util APIs used by common packages.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function inherits(ctor: any, superCtor: any): void {
    if (superCtor) {
        ctor.super_ = superCtor;
        Object.setPrototypeOf(ctor.prototype, superCtor.prototype);
    }
}

export function deprecate(fn: any, _msg: string): any {
    return fn;
}

export function promisify(fn: any): any {
    return function (...args: any[]): Promise<any> {
        return new Promise((resolve, reject) => {
            fn(...args, (err: any, result: any) => {
                if (err) reject(err);
                else resolve(result);
            });
        });
    };
}

export function inspect(obj: any, _options?: any): string {
    try {
        return JSON.stringify(obj, null, 2);
    } catch {
        return String(obj);
    }
}

export function format(fmt: string, ...args: any[]): string {
    let i = 0;
    return fmt.replace(/%[sdjifoO%]/g, (match) => {
        if (match === '%%') return '%';
        if (i >= args.length) return match;
        const val = args[i++];
        switch (match) {
            case '%s': return String(val);
            case '%d': return Number(val).toString();
            case '%i': return parseInt(val, 10).toString();
            case '%f': return parseFloat(val).toString();
            case '%j': return JSON.stringify(val);
            case '%o': case '%O': return inspect(val);
            default: return match;
        }
    });
}

export function debuglog(_section: string): (...args: any[]) => void {
    return () => {};
}

export const types = {
    isDate: (val: any): boolean => val instanceof Date,
    isRegExp: (val: any): boolean => val instanceof RegExp,
    isArray: Array.isArray,
    isBuffer: (val: any): boolean => !!(val && val._isBuffer),
    isUint8Array: (val: any): boolean => val instanceof Uint8Array,
};

export function callbackify(fn: any): any {
    return function (...args: any[]) {
        const callback = args.pop();
        fn(...args).then(
            (result: any) => callback(null, result),
            (err: any) => callback(err),
        );
    };
}

const util = {
    inherits, deprecate, promisify, inspect, format, debuglog,
    types, callbackify,
};
export default util;
