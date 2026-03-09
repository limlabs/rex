// Node.js `events` module polyfill for Rex server bundles.
// Minimal EventEmitter implementation sufficient for pg, pg-cloudflare,
// and other database drivers that rely on EventEmitter.

/* eslint-disable @typescript-eslint/no-explicit-any */

type Listener = (...args: any[]) => void;

interface ListenerEntry {
    fn: Listener;
    once: boolean;
}

export class EventEmitter {
    private _events: Map<string, ListenerEntry[]>;
    private _maxListeners: number;

    constructor() {
        this._events = new Map();
        this._maxListeners = 10;
    }

    on(event: string, listener: Listener): this {
        return this._addListener(event, listener, false);
    }

    addListener(event: string, listener: Listener): this {
        return this.on(event, listener);
    }

    once(event: string, listener: Listener): this {
        return this._addListener(event, listener, true);
    }

    off(event: string, listener: Listener): this {
        return this.removeListener(event, listener);
    }

    removeListener(event: string, listener: Listener): this {
        const entries = this._events.get(event);
        if (!entries) return this;
        const idx = entries.findIndex(e => e.fn === listener);
        if (idx !== -1) {
            entries.splice(idx, 1);
            if (entries.length === 0) this._events.delete(event);
        }
        return this;
    }

    removeAllListeners(event?: string): this {
        if (event !== undefined) {
            this._events.delete(event);
        } else {
            this._events.clear();
        }
        return this;
    }

    emit(event: string, ...args: any[]): boolean {
        const entries = this._events.get(event);
        if (!entries || entries.length === 0) return false;
        // Copy to allow mutation during iteration
        const copy = entries.slice();
        for (const entry of copy) {
            if (entry.once) {
                this.removeListener(event, entry.fn);
            }
            entry.fn.apply(this, args);
        }
        return true;
    }

    listeners(event: string): Listener[] {
        const entries = this._events.get(event);
        if (!entries) return [];
        return entries.map(e => e.fn);
    }

    listenerCount(event: string): number {
        const entries = this._events.get(event);
        return entries ? entries.length : 0;
    }

    eventNames(): string[] {
        return Array.from(this._events.keys());
    }

    setMaxListeners(n: number): this {
        this._maxListeners = n;
        return this;
    }

    getMaxListeners(): number {
        return this._maxListeners;
    }

    prependListener(event: string, listener: Listener): this {
        return this._addListener(event, listener, false, true);
    }

    prependOnceListener(event: string, listener: Listener): this {
        return this._addListener(event, listener, true, true);
    }

    rawListeners(event: string): Listener[] {
        return this.listeners(event);
    }

    private _addListener(event: string, listener: Listener, once: boolean, prepend = false): this {
        let entries = this._events.get(event);
        if (!entries) {
            entries = [];
            this._events.set(event, entries);
        }
        const entry: ListenerEntry = { fn: listener, once };
        if (prepend) {
            entries.unshift(entry);
        } else {
            entries.push(entry);
        }
        this.emit('newListener', event, listener);
        return this;
    }
}

// Static property for default max listeners
(EventEmitter as any).defaultMaxListeners = 10;

// Allow EventEmitter to be used as a base class
(EventEmitter as any).EventEmitter = EventEmitter;

export default EventEmitter;
