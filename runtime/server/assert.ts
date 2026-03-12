// Node.js `assert` stub for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function ok(value: any, message?: string): void {
    if (!value) throw new Error(message || 'Assertion failed');
}

export function strictEqual(actual: any, expected: any, message?: string): void {
    if (actual !== expected) {
        throw new Error(message || `Expected ${expected} but got ${actual}`);
    }
}

export function deepStrictEqual(actual: any, expected: any, message?: string): void {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(message || 'Deep strict equality assertion failed');
    }
}

export function notStrictEqual(actual: any, expected: any, message?: string): void {
    if (actual === expected) {
        throw new Error(message || `Expected values to be strictly unequal`);
    }
}

export function fail(message?: string): never {
    throw new Error(message || 'Assertion failed');
}

function assert(value: any, message?: string): void {
    ok(value, message);
}

assert.ok = ok;
assert.strictEqual = strictEqual;
assert.deepStrictEqual = deepStrictEqual;
assert.notStrictEqual = notStrictEqual;
assert.fail = fail;

export default assert;
