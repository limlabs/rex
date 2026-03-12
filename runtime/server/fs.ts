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

// Stub WriteStream class — PayloadCMS imports it but actual file streaming
// isn't supported in V8. Provides shape-compatibility only.
export class WriteStream {
    writable = true;
    path = '';
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    write(_chunk: any): boolean { return true; }
    end(): void { /* noop */ }
    on(_event: string, _cb: (...args: unknown[]) => void): this { return this; }
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export class ReadStream {
    readable = true;
    path = '';
    on(_event: string, _cb: (...args: unknown[]) => void): this { return this; }
    pipe<T>(dest: T): T { return dest; }
}

export function createWriteStream(path: string): WriteStream {
    const s = new WriteStream();
    s.path = path;
    return s;
}

export function createReadStream(path: string): ReadStream {
    const s = new ReadStream();
    s.path = path;
    return s;
}

export function realpathSync(path: string): string {
    return path; // No symlink resolution in V8 — passthrough
}

// Inline promises object to avoid circular import with fs-promises.ts
export const promises = {
    readFile(path: string, options?: string | { encoding?: string }): Promise<string | Uint8Array> {
        return Promise.resolve(readFileSync(path, options));
    },
    writeFile(path: string, data: string | Uint8Array, _options?: unknown): Promise<void> {
        return Promise.resolve(writeFileSync(path, data, _options));
    },
    readdir(path: string): Promise<string[]> {
        return Promise.resolve(readdirSync(path));
    },
    stat(path: string): Promise<StatResult> {
        return Promise.resolve(statSync(path));
    },
    mkdir(path: string, options?: { recursive?: boolean }): Promise<void> {
        return Promise.resolve(mkdirSync(path, options));
    },
    access(path: string): Promise<void> {
        return existsSync(path)
            ? Promise.resolve()
            : Promise.reject(Object.assign(new Error('ENOENT: no such file or directory'), { code: 'ENOENT' }));
    },
    unlink(path: string): Promise<void> {
        return Promise.resolve(unlinkSync(path));
    },
    rm(path: string, options?: { recursive?: boolean; force?: boolean }): Promise<void> {
        return Promise.resolve(rmSync(path, options));
    },
    realpath(path: string): Promise<string> {
        return Promise.resolve(path);
    },
};

const fs = {
    readFileSync,
    writeFileSync,
    readdirSync,
    statSync,
    mkdirSync,
    existsSync,
    unlinkSync,
    rmSync,
    createWriteStream,
    createReadStream,
    realpathSync,
    promises,
    WriteStream,
    ReadStream,
};

export default fs;
