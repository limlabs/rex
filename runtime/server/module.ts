// Node.js `module` stub for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function createRequire(_filename: string): any {
    return function require(id: string): any {
        // Delegate to global require polyfill which has Node.js builtin stubs
        if (typeof (globalThis as any).require === 'function') {
            return (globalThis as any).require(id);
        }
        throw new Error(`require("${id}") is not supported in Rex V8 runtime`);
    };
}

export const builtinModules: string[] = [];

export function isBuiltin(_moduleName: string): boolean {
    return false;
}

const mod = { createRequire, builtinModules, isBuiltin };
export default mod;
