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
} from './fs.js';

export function readFile(path, options) {
    return Promise.resolve(readFileSync(path, options));
}

export function writeFile(path, data, options) {
    return Promise.resolve(writeFileSync(path, data, options));
}

export function readdir(path) {
    return Promise.resolve(readdirSync(path));
}

export function stat(path) {
    return Promise.resolve(statSync(path));
}

export function mkdir(path, options) {
    return Promise.resolve(mkdirSync(path, options));
}

export function access(path) {
    return existsSync(path)
        ? Promise.resolve()
        : Promise.reject(Object.assign(new Error('ENOENT: no such file or directory'), { code: 'ENOENT' }));
}

export function unlink(path) {
    return Promise.resolve(unlinkSync(path));
}

export function rm(path, options) {
    return Promise.resolve(rmSync(path, options));
}

var fsPromises = {
    readFile: readFile,
    writeFile: writeFile,
    readdir: readdir,
    stat: stat,
    mkdir: mkdir,
    access: access,
    unlink: unlink,
    rm: rm,
};

export default fsPromises;
