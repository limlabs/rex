// Node.js `http2` module polyfill for Rex server bundles.
// Provides HTTP/2 constants used by undici and other HTTP clients.

/* eslint-disable @typescript-eslint/no-explicit-any */

export const constants = {
    HTTP2_HEADER_AUTHORITY: ':authority',
    HTTP2_HEADER_METHOD: ':method',
    HTTP2_HEADER_PATH: ':path',
    HTTP2_HEADER_SCHEME: ':scheme',
    HTTP2_HEADER_STATUS: ':status',
    HTTP2_HEADER_CONTENT_TYPE: 'content-type',
    HTTP2_HEADER_CONTENT_LENGTH: 'content-length',
    HTTP2_HEADER_ACCEPT: 'accept',
    HTTP2_HEADER_ACCEPT_ENCODING: 'accept-encoding',
    HTTP2_HEADER_ACCEPT_LANGUAGE: 'accept-language',
    HTTP2_HEADER_AUTHORIZATION: 'authorization',
    HTTP2_HEADER_CACHE_CONTROL: 'cache-control',
    HTTP2_HEADER_CONTENT_DISPOSITION: 'content-disposition',
    HTTP2_HEADER_CONTENT_ENCODING: 'content-encoding',
    HTTP2_HEADER_COOKIE: 'cookie',
    HTTP2_HEADER_DATE: 'date',
    HTTP2_HEADER_HOST: 'host',
    HTTP2_HEADER_IF_MODIFIED_SINCE: 'if-modified-since',
    HTTP2_HEADER_IF_NONE_MATCH: 'if-none-match',
    HTTP2_HEADER_LOCATION: 'location',
    HTTP2_HEADER_SET_COOKIE: 'set-cookie',
    HTTP2_HEADER_USER_AGENT: 'user-agent',
    HTTP2_HEADER_VARY: 'vary',
    HTTP2_METHOD_GET: 'GET',
    HTTP2_METHOD_POST: 'POST',
    HTTP2_METHOD_PUT: 'PUT',
    HTTP2_METHOD_DELETE: 'DELETE',
    HTTP2_METHOD_PATCH: 'PATCH',
    HTTP2_METHOD_HEAD: 'HEAD',
    HTTP2_METHOD_OPTIONS: 'OPTIONS',
    NGHTTP2_NO_ERROR: 0,
    NGHTTP2_CANCEL: 8,
};

export function connect() {
    throw new Error('http2.connect() is not supported in Rex server bundles');
}

export function createServer() {
    throw new Error('http2.createServer() is not supported in Rex server bundles');
}

export function createSecureServer() {
    throw new Error('http2.createSecureServer() is not supported in Rex server bundles');
}

export default { constants, connect, createServer, createSecureServer };
