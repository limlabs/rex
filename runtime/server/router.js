// Rex useRouter() hook — server-side stub.
// Returns static defaults during SSR. Real state is only available on the client.
var noop = function() {};
var noopEvents = { on: noop, off: noop, emit: noop };

export function useRouter() {
  return {
    pathname: '/',
    asPath: '/',
    query: {},
    route: '/',
    push: noop,
    replace: noop,
    back: noop,
    forward: noop,
    reload: noop,
    prefetch: noop,
    events: noopEvents,
    isReady: false
  };
}

export default useRouter;
