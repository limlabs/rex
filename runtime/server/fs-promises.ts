// Node.js `fs/promises` module polyfill for Rex server bundles.
// Wraps the sync fs shim operations in Promise.resolve() for async compatibility.

import {
    readFileSync,
    writeFileSync,
    readdirSync,
    statSync,
    mkdirSync,
    existsSync,
    unlinkSync,
    rmSync,
} from './fs';

import type { StatResult } from './fs';

export function readFile(path: string, options?: string | { encoding?: string }): Promise<string | Uint8Array> {
    return Promise.resolve(readFileSync(path, options));
}

export function writeFile(path: string, data: string | Uint8Array, options?: unknown): Promise<void> {
    return Promise.resolve(writeFileSync(path, data, options));
}

export function readdir(path: string): Promise<string[]> {
    return Promise.resolve(readdirSync(path));
}

export function stat(path: string): Promise<StatResult> {
    return Promise.resolve(statSync(path));
}

export function mkdir(path: string, options?: { recursive?: boolean }): Promise<void> {
    return Promise.resolve(mkdirSync(path, options));
}

export function access(path: string): Promise<void> {
    return existsSync(path)
        ? Promise.resolve()
        : Promise.reject(Object.assign(new Error('ENOENT: no such file or directory'), { code: 'ENOENT' }));
}

export function unlink(path: string): Promise<void> {
    return Promise.resolve(unlinkSync(path));
}

export function rm(path: string, options?: { recursive?: boolean; force?: boolean }): Promise<void> {
    return Promise.resolve(rmSync(path, options));
}

const fsPromises = {
    readFile,
    writeFile,
    readdir,
    stat,
    mkdir,
    access,
    unlink,
    rm,
};

export default fsPromises;
