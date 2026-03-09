// next/server stubs for Rex server bundles.

/* eslint-disable @typescript-eslint/no-explicit-any */

export class NextRequest extends Request {
    nextUrl: URL;
    cookies: any;

    constructor(input: RequestInfo | URL, init?: RequestInit) {
        super(input, init);
        this.nextUrl = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url);
        this.cookies = {
            get(_name: string) { return undefined; },
            getAll() { return []; },
            set() {},
            delete() {},
            has() { return false; },
        };
    }

    get geo() { return {}; }
    get ip() { return undefined; }
}

export class NextResponse extends Response {
    static json(body: any, init?: ResponseInit): NextResponse {
        return new NextResponse(JSON.stringify(body), {
            ...init,
            headers: { 'content-type': 'application/json', ...init?.headers },
        });
    }

    static redirect(url: string | URL, status?: number): NextResponse {
        return new NextResponse(null, {
            status: status || 307,
            headers: { Location: typeof url === 'string' ? url : url.href },
        });
    }

    static rewrite(destination: string | URL): NextResponse {
        return new NextResponse(null, {
            headers: { 'x-middleware-rewrite': typeof destination === 'string' ? destination : destination.href },
        });
    }

    static next(): NextResponse {
        return new NextResponse(null, { headers: { 'x-middleware-next': '1' } });
    }

    cookies: any = {
        get() { return undefined; },
        getAll() { return []; },
        set() {},
        delete() {},
        has() { return false; },
    };
}

const server = { NextRequest, NextResponse };
export default server;
