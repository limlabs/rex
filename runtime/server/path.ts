// Node.js `path` module polyfill for Rex server bundles.
// Pure string manipulation — POSIX semantics only (forward slashes).
// Rex targets Linux/macOS; Windows path support is not needed.

/* eslint-disable no-shadow-restricted-names -- accessing globalThis for V8 runtime bindings */
declare const globalThis: { __rex_project_root: string };
/* eslint-enable no-shadow-restricted-names */

export const sep = '/';
export const delimiter = ':';

function normalizeSegments(parts: string[]): string[] {
    const result: string[] = [];
    for (const part of parts) {
        if (part === '.' || part === '') continue;
        if (part === '..') {
            if (result.length > 0 && result[result.length - 1] !== '..') {
                result.pop();
            } else {
                result.push(part);
            }
        } else {
            result.push(part);
        }
    }
    return result;
}

export function normalize(p: string): string {
    if (p === '') return '.';
    const isAbsolute = p.charCodeAt(0) === 47; // '/'
    const segments = normalizeSegments(p.split('/'));
    let normalized = segments.join('/');
    if (isAbsolute) {
        normalized = '/' + normalized;
    }
    return normalized || (isAbsolute ? '/' : '.');
}

export function join(...paths: string[]): string {
    if (paths.length === 0) return '.';
    const joined = paths.filter(p => p !== '').join('/');
    return normalize(joined);
}

export function resolve(...paths: string[]): string {
    let resolved = '';
    // Walk right-to-left; stop once we have an absolute path
    for (let i = paths.length - 1; i >= 0; i--) {
        const segment = paths[i];
        if (segment === '') continue;
        resolved = segment + (resolved ? '/' + resolved : '');
        if (segment.charCodeAt(0) === 47) break; // absolute — done
    }
    // If still relative, prepend project root as cwd
    if (resolved.charCodeAt(0) !== 47) {
        const cwd = (typeof globalThis !== 'undefined' && globalThis.__rex_project_root) || '/';
        resolved = cwd + '/' + resolved;
    }
    return normalize(resolved);
}

export function basename(p: string, ext?: string): string {
    // Strip trailing slashes
    let end = p.length;
    while (end > 1 && p.charCodeAt(end - 1) === 47) end--;
    const start = p.lastIndexOf('/', end - 1) + 1;
    let base = p.slice(start, end);
    if (ext && base.length >= ext.length && base.slice(base.length - ext.length) === ext) {
        base = base.slice(0, base.length - ext.length);
    }
    return base;
}

export function dirname(p: string): string {
    if (p === '' || p === '/') return p || '.';
    // Strip trailing slashes
    let end = p.length;
    while (end > 1 && p.charCodeAt(end - 1) === 47) end--;
    const idx = p.lastIndexOf('/', end - 1);
    if (idx < 0) return '.';
    if (idx === 0) return '/';
    return p.slice(0, idx);
}

export function extname(p: string): string {
    const base = basename(p);
    const dot = base.lastIndexOf('.');
    if (dot <= 0) return '';
    return base.slice(dot);
}

export function isAbsolute(p: string): boolean {
    return p.length > 0 && p.charCodeAt(0) === 47;
}

export function relative(from: string, to: string): string {
    const fromParts = resolve(from).split('/').filter(Boolean);
    const toParts = resolve(to).split('/').filter(Boolean);

    let common = 0;
    while (common < fromParts.length && common < toParts.length && fromParts[common] === toParts[common]) {
        common++;
    }

    const ups: string[] = [];
    for (let i = common; i < fromParts.length; i++) ups.push('..');
    return ups.concat(toParts.slice(common)).join('/') || '.';
}

const posix = { sep, delimiter, normalize, join, resolve, basename, dirname, extname, isAbsolute, relative };

const path = { sep, delimiter, normalize, join, resolve, basename, dirname, extname, isAbsolute, relative, posix };

export { posix };
export default path;
