// Node.js `module` stub for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function createRequire(_filename: string): any {
    return function require(_id: string): any {
        throw new Error(`require("${_id}") is not supported in Rex V8 runtime`);
    };
}

export const builtinModules: string[] = [];

export function isBuiltin(_moduleName: string): boolean {
    return false;
}

const mod = { createRequire, builtinModules, isBuiltin };
export default mod;
