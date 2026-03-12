// FormData polyfill for bare V8 (needed by React's decodeReply/decodeAction)
if (typeof (globalThis as any).FormData === 'undefined') { // eslint-disable-line @typescript-eslint/no-explicit-any
    (globalThis as any).FormData = function FormData(this: any) { // eslint-disable-line @typescript-eslint/no-explicit-any
        this._entries = [] as [string, any][]; // eslint-disable-line @typescript-eslint/no-explicit-any
    };
    (globalThis as any).FormData.prototype.append = function(this: any, key: string, value: any) { // eslint-disable-line @typescript-eslint/no-explicit-any
        this._entries.push([String(key), value]);
    };
    (globalThis as any).FormData.prototype.set = function(this: any, key: string, value: any) { // eslint-disable-line @typescript-eslint/no-explicit-any
        this._entries = this._entries.filter(function(e: [string, any]) { return e[0] !== key; }); // eslint-disable-line @typescript-eslint/no-explicit-any
        this._entries.push([String(key), value]);
    };
    (globalThis as any).FormData.prototype.get = function(this: any, key: string): any { // eslint-disable-line @typescript-eslint/no-explicit-any
        for (let i = 0; i < this._entries.length; i++) {
            if (this._entries[i][0] === key) return this._entries[i][1];
        }
        return null;
    };
    (globalThis as any).FormData.prototype.getAll = function(this: any, key: string): any[] { // eslint-disable-line @typescript-eslint/no-explicit-any
        const result = [] as any[]; // eslint-disable-line @typescript-eslint/no-explicit-any
        for (let i = 0; i < this._entries.length; i++) {
            if (this._entries[i][0] === key) result.push(this._entries[i][1]);
        }
        return result;
    };
    (globalThis as any).FormData.prototype.has = function(this: any, key: string): boolean { // eslint-disable-line @typescript-eslint/no-explicit-any
        for (let i = 0; i < this._entries.length; i++) {
            if (this._entries[i][0] === key) return true;
        }
        return false;
    };
    (globalThis as any).FormData.prototype.delete = function(this: any, key: string) { // eslint-disable-line @typescript-eslint/no-explicit-any
        this._entries = this._entries.filter(function(e: [string, any]) { return e[0] !== key; }); // eslint-disable-line @typescript-eslint/no-explicit-any
    };
    (globalThis as any).FormData.prototype.forEach = function(this: any, callback: (value: any, key: string, parent: any) => void) { // eslint-disable-line @typescript-eslint/no-explicit-any
        for (let i = 0; i < this._entries.length; i++) {
            callback(this._entries[i][1], this._entries[i][0], this);
        }
    };
    (globalThis as any).FormData.prototype.entries = function(this: any) { // eslint-disable-line @typescript-eslint/no-explicit-any
        let idx = 0;
        const entries = this._entries;
        return {
            next: function() {
                if (idx >= entries.length) return { done: true, value: undefined };
                return { done: false, value: entries[idx++] };
            },
            [Symbol.iterator]: function() { return this; }
        };
    };
    (globalThis as any).FormData.prototype.keys = function(this: any) { // eslint-disable-line @typescript-eslint/no-explicit-any
        let idx = 0;
        const entries = this._entries;
        return {
            next: function() {
                if (idx >= entries.length) return { done: true, value: undefined };
                return { done: false, value: entries[idx++][0] };
            },
            [Symbol.iterator]: function() { return this; }
        };
    };
    (globalThis as any).FormData.prototype.values = function(this: any) { // eslint-disable-line @typescript-eslint/no-explicit-any
        let idx = 0;
        const entries = this._entries;
        return {
            next: function() {
                if (idx >= entries.length) return { done: true, value: undefined };
                return { done: false, value: entries[idx++][1] };
            },
            [Symbol.iterator]: function() { return this; }
        };
    };
    (globalThis as any).FormData.prototype[Symbol.iterator] = (globalThis as any).FormData.prototype.entries; // eslint-disable-line @typescript-eslint/no-explicit-any
}
