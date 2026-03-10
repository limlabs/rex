// Node.js `https` module polyfill for Rex server bundles.
// Re-exports http — the global fetch() handles both HTTP and HTTPS natively.

export {
    request,
    get,
    createServer,
    ClientRequest,
    IncomingMessage,
    METHODS,
    STATUS_CODES,
} from './http';

export { default } from './http';
