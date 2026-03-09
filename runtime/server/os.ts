// Node.js `os` module stub for Rex server bundles.
// Provides minimal os module APIs used by common packages.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function tmpdir(): string {
    return '/tmp';
}

export function homedir(): string {
    return (globalThis as any).process?.env?.HOME || '/root';
}

export function hostname(): string {
    return (globalThis as any).process?.env?.HOSTNAME || 'localhost';
}

export function platform(): string {
    return 'linux';
}

export function arch(): string {
    return 'x64';
}

export function cpus(): any[] {
    return [{ model: 'unknown', speed: 0, times: {} }];
}

export function totalmem(): number {
    return 1024 * 1024 * 1024; // 1 GB
}

export function freemem(): number {
    return 512 * 1024 * 1024; // 512 MB
}

export function type(): string {
    return 'Linux';
}

export function release(): string {
    return '5.0.0';
}

export function networkInterfaces(): any {
    return {};
}

export function endianness(): string {
    return 'LE';
}

export const EOL = '\n';

const os = {
    tmpdir, homedir, hostname, platform, arch, cpus,
    totalmem, freemem, type, release, networkInterfaces,
    endianness, EOL,
};
export default os;
