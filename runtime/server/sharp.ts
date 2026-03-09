// sharp stub for Rex server bundles.
// sharp requires native libvips bindings which aren't available in V8 isolates.
// This stub provides a minimal API surface to prevent crashes during import.

/* eslint-disable @typescript-eslint/no-explicit-any */

function sharp(_input?: any, _options?: any): any {
    const instance: any = {
        resize: () => instance,
        rotate: () => instance,
        flip: () => instance,
        flop: () => instance,
        sharpen: () => instance,
        blur: () => instance,
        flatten: () => instance,
        gamma: () => instance,
        negate: () => instance,
        normalise: () => instance,
        normalize: () => instance,
        composite: () => instance,
        modulate: () => instance,
        trim: () => instance,
        extend: () => instance,
        extract: () => instance,
        withMetadata: () => instance,
        jpeg: () => instance,
        png: () => instance,
        webp: () => instance,
        avif: () => instance,
        tiff: () => instance,
        raw: () => instance,
        toFormat: () => instance,
        toBuffer: () => Promise.resolve(Buffer.alloc(0)),
        toFile: () => Promise.resolve({ width: 0, height: 0, channels: 3, size: 0 }),
        metadata: () => Promise.resolve({ width: 0, height: 0, channels: 3, format: 'unknown' }),
        stats: () => Promise.resolve({}),
        clone: () => sharp(),
        pipe: () => instance,
    };
    return instance;
}

sharp.libvipsVersion = () => '0.0.0';
sharp.format = {};
sharp.versions = {};
sharp.queue = { on: () => {} };
sharp.cache = () => {};
sharp.concurrency = () => 0;
sharp.counters = () => ({ queue: 0, process: 0 });
sharp.simd = () => false;

export default sharp;
export { sharp };
