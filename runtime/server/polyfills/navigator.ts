/* eslint-disable @typescript-eslint/no-explicit-any */
// navigator polyfill — identify as Cloudflare Workers runtime so pg uses
// the pg-cloudflare path (cloudflare:sockets → Rex TCP bridge) instead of
// requiring a full net.Socket implementation.
if (typeof (globalThis as any).navigator === 'undefined') {
    (globalThis as any).navigator = { userAgent: 'Cloudflare-Workers' };
}
