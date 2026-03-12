// next/dynamic stub for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export default function dynamic(loader: any, _options?: any): any {
    // On server: just return the component (no code splitting in SSR)
    if (typeof loader === 'function') {
        return loader;
    }
    return () => null;
}
