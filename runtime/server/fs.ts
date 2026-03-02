// Node.js `fs` module polyfill for Rex server bundles.
// Wraps globalThis.__rex_fs_* Rust callbacks into a Node.js-compatible API.
// The Rust callbacks take project_root as the first arg and return error
// sentinel strings instead of throwing, so this shim handles error detection.

/* eslint-disable no-shadow-restricted-names -- augmenting globalThis for V8 runtime bindings */
declare const globalThis: {
    __rex_project_root: string;
    __rex_fs_read_file_sync(root: string, path: string, encoding?: string): string | Uint8Array;
    __rex_fs_write_file_sync(root: string, path: string, data: string | Uint8Array): void;
    __rex_fs_readdir_sync(root: string, path: string): string;
    __rex_fs_stat_sync(root: string, path: string): string;
    __rex_fs_mkdir_sync(root: string, path: string, options?: { recursive?: boolean }): void;
    __rex_fs_exists_sync(root: string, path: string): boolean;
    __rex_fs_unlink_sync(root: string, path: string): void;
    __rex_fs_rm_sync(root: string, path: string, options?: { recursive?: boolean; force?: boolean }): void;
};
/* eslint-enable no-shadow-restricted-names */

interface NodeError extends Error {
    code: string;
}

export interface StatResult {
    size: number;
    mtimeMs: number;
    mtime: Date;
    isFile(): boolean;
    isDirectory(): boolean;
    isSymbolicLink(): boolean;
}

const ERROR_PREFIX = '__REX_FS_ERR__';

function checkResult<T>(result: T): T {
    if (typeof result === 'string' && result.indexOf(ERROR_PREFIX) === 0) {
        const errData = JSON.parse(result.slice(ERROR_PREFIX.length));
        const err = new Error(errData.message) as NodeError;
        err.code = errData.code;
        throw err;
    }
    return result;
}

const root = globalThis.__rex_project_root;

export function readFileSync(path: string, options?: string | { encoding?: string }): string | Uint8Array {
    const encoding = typeof options === 'string' ? options : (options && options.encoding);
    return checkResult(globalThis.__rex_fs_read_file_sync(root, path, encoding || undefined));
}

export function writeFileSync(path: string, data: string | Uint8Array, _options?: unknown): void {
    checkResult(globalThis.__rex_fs_write_file_sync(root, path, data));
}

export function readdirSync(path: string): string[] {
    const json = checkResult(globalThis.__rex_fs_readdir_sync(root, path));
    return JSON.parse(json);
}

export function statSync(path: string): StatResult {
    const json = checkResult(globalThis.__rex_fs_stat_sync(root, path));
    const raw = JSON.parse(json);
    return {
        size: raw.size,
        mtimeMs: raw.mtimeMs,
        mtime: new Date(raw.mtimeMs),
        isFile() { return raw.isFile; },
        isDirectory() { return raw.isDirectory; },
        isSymbolicLink() { return raw.isSymbolicLink; },
    };
}

export function mkdirSync(path: string, options?: { recursive?: boolean }): void {
    checkResult(globalThis.__rex_fs_mkdir_sync(root, path, options || undefined));
}

export function existsSync(path: string): boolean {
    return globalThis.__rex_fs_exists_sync(root, path);
}

export function unlinkSync(path: string): void {
    checkResult(globalThis.__rex_fs_unlink_sync(root, path));
}

export function rmSync(path: string, options?: { recursive?: boolean; force?: boolean }): void {
    checkResult(globalThis.__rex_fs_rm_sync(root, path, options || undefined));
}

const fs = {
    readFileSync,
    writeFileSync,
    readdirSync,
    statSync,
    mkdirSync,
    existsSync,
    unlinkSync,
    rmSync,
};

export default fs;
