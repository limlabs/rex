function getRouter() {
    return window.__REX_ROUTER ?? null;
}
/**
 * Navigate to a new path via client-side routing.
 */
export function navigateTo(path) {
    const r = getRouter();
    if (r) {
        r.push(path);
    }
    else {
        window.location.href = path;
    }
}
function parseQuery(search) {
    const query = {};
    if (!search || search.length <= 1)
        return query;
    const pairs = search.substring(1).split('&');
    for (const pair of pairs) {
        const [key, value] = pair.split('=');
        query[decodeURIComponent(key)] = decodeURIComponent(value ?? '');
    }
    return query;
}
/**
 * React hook that returns the current router instance.
 */
export function useRouter() {
    const r = getRouter();
    const noop = () => { };
    if (r?.state) {
        return {
            pathname: r.state.pathname,
            asPath: r.state.asPath,
            query: r.state.query,
            route: r.state.route,
            push: r.push,
            replace: r.replace,
            back: r.back,
            forward: r.forward,
            reload: r.reload,
            prefetch: r.prefetch,
            events: r.events,
            isReady: true,
        };
    }
    return {
        pathname: window.location.pathname,
        asPath: window.location.pathname + window.location.search,
        query: parseQuery(window.location.search),
        route: window.location.pathname,
        push: (url) => { window.location.href = url; },
        replace: (url) => { window.location.replace(url); },
        back: () => { history.back(); },
        forward: () => { history.forward(); },
        reload: () => { window.location.reload(); },
        prefetch: noop,
        events: { on: noop, off: noop, emit: noop },
        isReady: false,
    };
}
export default useRouter;
//# sourceMappingURL=router.js.map