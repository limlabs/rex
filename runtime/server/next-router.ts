// next/router stub for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function useRouter(): any {
    return {
        route: '/',
        pathname: '/',
        query: {},
        asPath: '/',
        basePath: '',
        locale: undefined,
        push() { return Promise.resolve(true); },
        replace() { return Promise.resolve(true); },
        reload() {},
        back() {},
        forward() {},
        prefetch() { return Promise.resolve(); },
        events: { on() {}, off() {}, emit() {} },
        isFallback: false,
        isReady: true,
        isPreview: false,
    };
}

export function withRouter(Component: any): any {
    return Component;
}

const router = { useRouter, withRouter };
export default router;
