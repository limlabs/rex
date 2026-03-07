/* eslint-disable @typescript-eslint/no-explicit-any */
// URL constructor polyfill for bare V8
if (typeof globalThis.URL === 'undefined') {
    (globalThis as any).URL = function(this: any, path: string, base?: string) {
        if (base) {
            const m = String(base).match(/^(https?:[/][/][^/]+)/);
            const origin = m ? m[1] : '';
            const p = String(path);
            if (p.startsWith('/')) {
                this.href = origin + p;
            } else if (p.startsWith('http://') || p.startsWith('https://')) {
                this.href = p;
            } else {
                this.href = origin + '/' + p;
            }
        } else {
            this.href = String(path);
        }
        const withoutProto = this.href.replace(/^https?:[/][/][^/]+/, '');
        this.pathname = withoutProto ? withoutProto.split('?')[0].split('#')[0] : '/';
        if (!this.pathname.startsWith('/')) this.pathname = '/' + this.pathname;
        this.search = '';
        const qi = this.href.indexOf('?');
        if (qi !== -1) this.search = this.href.substring(qi).split('#')[0];
    };
    (globalThis as any).URL.prototype.toString = function(this: any) { return this.href; };
}
