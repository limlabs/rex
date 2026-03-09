// Node.js `zlib` stub for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function createGzip(_opts?: any): any {
    throw new Error('zlib.createGzip is not supported in Rex V8 runtime');
}

export function createGunzip(_opts?: any): any {
    throw new Error('zlib.createGunzip is not supported in Rex V8 runtime');
}

export function createDeflate(_opts?: any): any {
    throw new Error('zlib.createDeflate is not supported in Rex V8 runtime');
}

export function createInflate(_opts?: any): any {
    throw new Error('zlib.createInflate is not supported in Rex V8 runtime');
}

export function gzip(_buf: any, _opts: any, cb?: any): void {
    const callback = typeof _opts === 'function' ? _opts : cb;
    if (callback) callback(new Error('zlib.gzip is not supported'));
}

export function gunzip(_buf: any, _opts: any, cb?: any): void {
    const callback = typeof _opts === 'function' ? _opts : cb;
    if (callback) callback(new Error('zlib.gunzip is not supported'));
}

export const constants = {
    Z_NO_FLUSH: 0,
    Z_PARTIAL_FLUSH: 1,
    Z_SYNC_FLUSH: 2,
    Z_FULL_FLUSH: 3,
    Z_FINISH: 4,
    Z_BLOCK: 5,
    Z_OK: 0,
    Z_STREAM_END: 1,
    Z_NEED_DICT: 2,
    Z_ERRNO: -1,
    Z_STREAM_ERROR: -2,
    Z_DATA_ERROR: -3,
    Z_MEM_ERROR: -4,
    Z_BUF_ERROR: -5,
    Z_VERSION_ERROR: -6,
    Z_DEFAULT_COMPRESSION: -1,
};

const zlib = { createGzip, createGunzip, createDeflate, createInflate, gzip, gunzip, constants };
export default zlib;
