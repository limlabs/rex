// Node.js `child_process` stub for Rex server bundles.
// V8 cannot spawn child processes.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function exec(_cmd: string, _opts?: any, cb?: any): any {
    const callback = typeof _opts === 'function' ? _opts : cb;
    if (callback) callback(new Error('child_process.exec is not supported in Rex V8 runtime'));
    return {};
}

export function execSync(_cmd: string, _opts?: any): any {
    throw new Error('child_process.execSync is not supported in Rex V8 runtime');
}

export function spawn(_cmd: string, _args?: any, _opts?: any): any {
    throw new Error('child_process.spawn is not supported in Rex V8 runtime');
}

export function fork(_modulePath: string, _args?: any, _opts?: any): any {
    throw new Error('child_process.fork is not supported in Rex V8 runtime');
}

export function execFile(_file: string, _args?: any, _opts?: any, cb?: any): any {
    const callback = typeof _args === 'function' ? _args : typeof _opts === 'function' ? _opts : cb;
    if (callback) callback(new Error('child_process.execFile is not supported in Rex V8 runtime'));
    return {};
}

const child_process = { exec, execSync, spawn, fork, execFile };
export default child_process;
