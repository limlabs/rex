// next/cache stubs for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function revalidatePath(_path: string): void { /* noop */ }
export function revalidateTag(_tag: string): void { /* noop */ }
export function unstable_cache(fn: any): any { return fn; }
export function unstable_noStore(): void { /* noop */ }

const cache = { revalidatePath, revalidateTag, unstable_cache, unstable_noStore };
export default cache;
