// next/router stub for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export function useRouter(): any {
    // Set by __rex_render_page for the page being rendered (ssr-runtime.ts).
    const state = globalThis.__rex_router_state;
    const pathname = state ? state.pathname : '/';
    return {
        route: pathname,
        pathname: pathname,
        query: state ? state.query : {},
        asPath: state ? state.asPath : '/',
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
