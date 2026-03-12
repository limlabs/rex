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
    private _events!: Map<string, ListenerEntry[]>;
    private _maxListeners!: number;

    constructor() {
        this._events = new Map();
        this._maxListeners = 10;
    }

    // Lazy init for subclasses that don't call super()
    private _ensureEvents(): Map<string, ListenerEntry[]> {
        if (!this._events) this._events = new Map();
        return this._events;
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
        const events = this._ensureEvents();
        const entries = events.get(event);
        if (!entries) return this;
        const idx = entries.findIndex(e => e.fn === listener);
        if (idx !== -1) {
            entries.splice(idx, 1);
            if (entries.length === 0) events.delete(event);
        }
        return this;
    }

    removeAllListeners(event?: string): this {
        const events = this._ensureEvents();
        if (event !== undefined) {
            events.delete(event);
        } else {
            events.clear();
        }
        return this;
    }

    emit(event: string, ...args: any[]): boolean {
        const events = this._ensureEvents();
        const entries = events.get(event);

        if (!entries || entries.length === 0) {
            return false;
        }
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
        const entries = this._ensureEvents().get(event);
        if (!entries) return [];
        return entries.map(e => e.fn);
    }

    listenerCount(event: string): number {
        const entries = this._ensureEvents().get(event);
        return entries ? entries.length : 0;
    }

    eventNames(): string[] {
        return Array.from(this._ensureEvents().keys());
    }

    setMaxListeners(n: number): this {
        this._maxListeners = n;
        return this;
    }

    getMaxListeners(): number {
        return this._maxListeners || 10;
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
        const events = this._ensureEvents();
        let entries = events.get(event);
        if (!entries) {
            entries = [];
            events.set(event, entries);
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

// Static helpers for CJS interop
export function getMaxListeners(emitter: EventEmitter): number {
    return emitter.getMaxListeners();
}
export function setMaxListeners(n: number, ...emitters: EventEmitter[]): void {
    for (const e of emitters) e.setMaxListeners(n);
}
export const defaultMaxListeners = 10;

export function once(emitter: EventEmitter, event: string): Promise<any[]> {
    return new Promise((resolve) => {
        emitter.once(event, (...args: any[]) => resolve(args));
    });
}

export function on(_emitter: EventEmitter, _event: string): AsyncIterable<any> {
    return { [Symbol.asyncIterator]() { return { next() { return new Promise(() => {}); } }; } };
}

export default EventEmitter;
