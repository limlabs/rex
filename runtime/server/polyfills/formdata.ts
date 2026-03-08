// FormData polyfill for bare V8 (needed by React's decodeReply/decodeAction)
if (typeof (globalThis as any).FormData === 'undefined') {
    (globalThis as any).FormData = function FormData(this: any) {
        this._entries = [] as [string, any][];
    };
    (globalThis as any).FormData.prototype.append = function(this: any, key: string, value: any) {
        this._entries.push([String(key), value]);
    };
    (globalThis as any).FormData.prototype.set = function(this: any, key: string, value: any) {
        this._entries = this._entries.filter(function(e: [string, any]) { return e[0] !== key; });
        this._entries.push([String(key), value]);
    };
    (globalThis as any).FormData.prototype.get = function(this: any, key: string): any {
        for (var i = 0; i < this._entries.length; i++) {
            if (this._entries[i][0] === key) return this._entries[i][1];
        }
        return null;
    };
    (globalThis as any).FormData.prototype.getAll = function(this: any, key: string): any[] {
        var result = [] as any[];
        for (var i = 0; i < this._entries.length; i++) {
            if (this._entries[i][0] === key) result.push(this._entries[i][1]);
        }
        return result;
    };
    (globalThis as any).FormData.prototype.has = function(this: any, key: string): boolean {
        for (var i = 0; i < this._entries.length; i++) {
            if (this._entries[i][0] === key) return true;
        }
        return false;
    };
    (globalThis as any).FormData.prototype.delete = function(this: any, key: string) {
        this._entries = this._entries.filter(function(e: [string, any]) { return e[0] !== key; });
    };
    (globalThis as any).FormData.prototype.forEach = function(this: any, callback: (value: any, key: string, parent: any) => void) {
        for (var i = 0; i < this._entries.length; i++) {
            callback(this._entries[i][1], this._entries[i][0], this);
        }
    };
    (globalThis as any).FormData.prototype.entries = function(this: any) {
        var idx = 0;
        var entries = this._entries;
        return {
            next: function() {
                if (idx >= entries.length) return { done: true, value: undefined };
                return { done: false, value: entries[idx++] };
            },
            [Symbol.iterator]: function() { return this; }
        };
    };
    (globalThis as any).FormData.prototype.keys = function(this: any) {
        var idx = 0;
        var entries = this._entries;
        return {
            next: function() {
                if (idx >= entries.length) return { done: true, value: undefined };
                return { done: false, value: entries[idx++][0] };
            },
            [Symbol.iterator]: function() { return this; }
        };
    };
    (globalThis as any).FormData.prototype.values = function(this: any) {
        var idx = 0;
        var entries = this._entries;
        return {
            next: function() {
                if (idx >= entries.length) return { done: true, value: undefined };
                return { done: false, value: entries[idx++][1] };
            },
            [Symbol.iterator]: function() { return this; }
        };
    };
    (globalThis as any).FormData.prototype[Symbol.iterator] = (globalThis as any).FormData.prototype.entries;
}
