// Node.js `fs` module polyfill for Rex server bundles.
// Wraps globalThis.__rex_fs_* Rust callbacks into a Node.js-compatible API.
// The Rust callbacks take project_root as the first arg and return error
// sentinel strings instead of throwing, so this shim handles error detection.

var ERROR_PREFIX = '__REX_FS_ERR__';

function checkResult(result) {
    if (typeof result === 'string' && result.indexOf(ERROR_PREFIX) === 0) {
        var errData = JSON.parse(result.slice(ERROR_PREFIX.length));
        var err = new Error(errData.message);
        err.code = errData.code;
        throw err;
    }
    return result;
}

var root = globalThis.__rex_project_root;

export function readFileSync(path, options) {
    var encoding = typeof options === 'string' ? options : (options && options.encoding);
    return checkResult(globalThis.__rex_fs_read_file_sync(root, path, encoding || undefined));
}

export function writeFileSync(path, data, _options) {
    checkResult(globalThis.__rex_fs_write_file_sync(root, path, data));
}

export function readdirSync(path) {
    var json = checkResult(globalThis.__rex_fs_readdir_sync(root, path));
    return JSON.parse(json);
}

export function statSync(path) {
    var json = checkResult(globalThis.__rex_fs_stat_sync(root, path));
    var raw = JSON.parse(json);
    return {
        size: raw.size,
        mtimeMs: raw.mtimeMs,
        mtime: new Date(raw.mtimeMs),
        isFile: function() { return raw.isFile; },
        isDirectory: function() { return raw.isDirectory; },
        isSymbolicLink: function() { return raw.isSymbolicLink; },
    };
}

export function mkdirSync(path, options) {
    checkResult(globalThis.__rex_fs_mkdir_sync(root, path, options || undefined));
}

export function existsSync(path) {
    return globalThis.__rex_fs_exists_sync(root, path);
}

export function unlinkSync(path) {
    checkResult(globalThis.__rex_fs_unlink_sync(root, path));
}

export function rmSync(path, options) {
    checkResult(globalThis.__rex_fs_rm_sync(root, path, options || undefined));
}

var fs = {
    readFileSync: readFileSync,
    writeFileSync: writeFileSync,
    readdirSync: readdirSync,
    statSync: statSync,
    mkdirSync: mkdirSync,
    existsSync: existsSync,
    unlinkSync: unlinkSync,
    rmSync: rmSync,
};

export default fs;
