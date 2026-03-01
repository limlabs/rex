// Rex middleware runtime — server-side NextResponse implementation
// Imported via `import { NextResponse } from 'rex/middleware'`

export class NextResponse {
  constructor(action, opts) {
    this._action = action;
    this._url = opts && opts.url || null;
    this._status = opts && opts.status || 307;
    this._requestHeaders = {};
    this._responseHeaders = {};
  }

  static next(opts) {
    var res = new NextResponse('next', {});
    if (opts && opts.headers) {
      for (var k in opts.headers) {
        if (opts.headers.hasOwnProperty(k)) {
          res._responseHeaders[k] = opts.headers[k];
        }
      }
    }
    if (opts && opts.request && opts.request.headers) {
      for (var k in opts.request.headers) {
        if (opts.request.headers.hasOwnProperty(k)) {
          res._requestHeaders[k] = opts.request.headers[k];
        }
      }
    }
    return res;
  }

  static redirect(url, status) {
    var urlStr = typeof url === 'object' && url.toString ? url.toString() : String(url);
    return new NextResponse('redirect', { url: urlStr, status: status || 307 });
  }

  static rewrite(url) {
    var urlStr = typeof url === 'object' && url.toString ? url.toString() : String(url);
    return new NextResponse('rewrite', { url: urlStr });
  }
}
