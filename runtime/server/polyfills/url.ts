/* eslint-disable @typescript-eslint/no-explicit-any */
// URLSearchParams polyfill for bare V8
if (typeof globalThis.URLSearchParams === 'undefined') {
    class URLSearchParamsPolyfill {
        private _entries: [string, string][] = [];

        constructor(init?: string | Record<string, string> | [string, string][] | URLSearchParamsPolyfill) {
            if (!init) return;
            if (typeof init === 'string') {
                const qs = init.startsWith('?') ? init.slice(1) : init;
                if (qs) {
                    for (const pair of qs.split('&')) {
                        const eqIdx = pair.indexOf('=');
                        if (eqIdx === -1) {
                            this._entries.push([decodeComp(pair), '']);
                        } else {
                            this._entries.push([
                                decodeComp(pair.slice(0, eqIdx)),
                                decodeComp(pair.slice(eqIdx + 1)),
                            ]);
                        }
                    }
                }
            } else if (Array.isArray(init)) {
                for (const [k, v] of init) {
                    this._entries.push([String(k), String(v)]);
                }
            } else if (init instanceof URLSearchParamsPolyfill) {
                this._entries = [...init._entries];
            } else if (typeof init === 'object') {
                for (const key of Object.keys(init)) {
                    this._entries.push([key, String((init as any)[key])]);
                }
            }
        }

        append(name: string, value: string): void {
            this._entries.push([String(name), String(value)]);
        }

        delete(name: string): void {
            this._entries = this._entries.filter(([k]) => k !== name);
        }

        get(name: string): string | null {
            const entry = this._entries.find(([k]) => k === name);
            return entry ? entry[1] : null;
        }

        getAll(name: string): string[] {
            return this._entries.filter(([k]) => k === name).map(([, v]) => v);
        }

        has(name: string): boolean {
            return this._entries.some(([k]) => k === name);
        }

        set(name: string, value: string): void {
            const sName = String(name);
            const sValue = String(value);
            let found = false;
            this._entries = this._entries.reduce<[string, string][]>((acc, [k, v]) => {
                if (k === sName) {
                    if (!found) {
                        found = true;
                        acc.push([sName, sValue]);
                    }
                    // drop subsequent duplicates
                } else {
                    acc.push([k, v]);
                }
                return acc;
            }, []);
            if (!found) {
                this._entries.push([sName, sValue]);
            }
        }

        sort(): void {
            this._entries.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
        }

        keys(): IterableIterator<string> {
            return this._entries.map(([k]) => k)[Symbol.iterator]();
        }

        values(): IterableIterator<string> {
            return this._entries.map(([, v]) => v)[Symbol.iterator]();
        }

        entries(): IterableIterator<[string, string]> {
            return this._entries[Symbol.iterator]();
        }

        forEach(callback: (value: string, key: string, parent: URLSearchParamsPolyfill) => void): void {
            for (const [k, v] of this._entries) {
                callback(v, k, this);
            }
        }

        [Symbol.iterator](): IterableIterator<[string, string]> {
            return this.entries();
        }

        get size(): number {
            return this._entries.length;
        }

        toString(): string {
            return this._entries
                .map(([k, v]) => encodeComp(k) + '=' + encodeComp(v))
                .join('&');
        }
    }

    function encodeComp(s: string): string {
        return encodeURIComponent(s).replace(/%20/g, '+');
    }

    function decodeComp(s: string): string {
        return decodeURIComponent(s.replace(/\+/g, '%20'));
    }

    (globalThis as any).URLSearchParams = URLSearchParamsPolyfill;
}

// URL constructor polyfill for bare V8
if (typeof globalThis.URL === 'undefined') {
    const URL_REGEX = /^([a-z][a-z0-9+\-.]*):\/\/(?:([^:@]*)(?::([^@]*))?@)?([^:/?#]*)(?::(\d+))?(\/[^?#]*)?(\?[^#]*)?(#.*)?$/i;

    class URLPolyfill {
        protocol: string = '';
        username: string = '';
        password: string = '';
        hostname: string = '';
        port: string = '';
        pathname: string = '/';
        search: string = '';
        hash: string = '';
        searchParams: any;

        constructor(url: string, base?: string) {
            let resolved: string;

            if (base) {
                const baseMatch = String(base).match(URL_REGEX);
                if (!baseMatch) throw new TypeError(`Invalid base URL: ${base}`);
                const u = String(url);
                if (u.match(/^[a-z][a-z0-9+\-.]*:\/\//i)) {
                    resolved = u;
                } else if (u.startsWith('/')) {
                    resolved = baseMatch[1] + '://' +
                        (baseMatch[2] ? baseMatch[2] + (baseMatch[3] ? ':' + baseMatch[3] : '') + '@' : '') +
                        baseMatch[4] + (baseMatch[5] ? ':' + baseMatch[5] : '') + u;
                } else {
                    const basePath = (baseMatch[6] || '/').replace(/\/[^/]*$/, '/');
                    resolved = baseMatch[1] + '://' +
                        (baseMatch[2] ? baseMatch[2] + (baseMatch[3] ? ':' + baseMatch[3] : '') + '@' : '') +
                        baseMatch[4] + (baseMatch[5] ? ':' + baseMatch[5] : '') + basePath + u;
                }
            } else {
                resolved = String(url);
            }

            const m = resolved.match(URL_REGEX);
            if (!m) throw new TypeError(`Invalid URL: ${url}`);

            this.protocol = m[1].toLowerCase() + ':';
            this.username = m[2] ? decodeURIComponent(m[2]) : '';
            this.password = m[3] ? decodeURIComponent(m[3]) : '';
            this.hostname = m[4] || '';
            this.port = m[5] || '';
            this.pathname = m[6] || '/';
            this.search = m[7] || '';
            this.hash = m[8] || '';
            this.searchParams = new (globalThis as any).URLSearchParams(this.search);
        }

        get host(): string {
            return this.port ? this.hostname + ':' + this.port : this.hostname;
        }

        get origin(): string {
            return this.protocol + '//' + this.host;
        }

        get href(): string {
            let url = this.protocol + '//';
            if (this.username) {
                url += this.username;
                if (this.password) url += ':' + this.password;
                url += '@';
            }
            url += this.host + this.pathname + this.search + this.hash;
            return url;
        }

        toString(): string {
            return this.href;
        }

        toJSON(): string {
            return this.href;
        }
    }

    (globalThis as any).URL = URLPolyfill;
}
