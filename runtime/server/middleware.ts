// Rex middleware runtime — server-side NextResponse implementation
// Imported via `import { NextResponse } from 'rex/middleware'`

interface NextResponseOpts {
  url?: string;
  status?: number;
}

interface NextOpts {
  headers?: Record<string, string>;
  request?: {
    headers?: Record<string, string>;
  };
}

export class NextResponse {
  _action: string;
  _url: string | null;
  _status: number;
  _requestHeaders: Record<string, string>;
  _responseHeaders: Record<string, string>;

  constructor(action: string, opts?: NextResponseOpts) {
    this._action = action;
    this._url = (opts && opts.url) || null;
    this._status = (opts && opts.status) || 307;
    this._requestHeaders = {};
    this._responseHeaders = {};
  }

  static next(opts?: NextOpts): NextResponse {
    const res = new NextResponse("next", {});
    if (opts && opts.headers) {
      for (const k in opts.headers) {
        if (Object.prototype.hasOwnProperty.call(opts.headers, k)) {
          res._responseHeaders[k] = opts.headers[k];
        }
      }
    }
    if (opts && opts.request && opts.request.headers) {
      for (const k in opts.request.headers) {
        if (Object.prototype.hasOwnProperty.call(opts.request.headers, k)) {
          res._requestHeaders[k] = opts.request.headers[k];
        }
      }
    }
    return res;
  }

  static redirect(url: string | URL, status?: number): NextResponse {
    const urlStr =
      typeof url === "object" && url.toString ? url.toString() : String(url);
    return new NextResponse("redirect", { url: urlStr, status: status || 307 });
  }

  static rewrite(url: string | URL): NextResponse {
    const urlStr =
      typeof url === "object" && url.toString ? url.toString() : String(url);
    return new NextResponse("rewrite", { url: urlStr });
  }
}
