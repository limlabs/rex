// Node.js `events` module polyfill for Rex server bundles.
// Provides a minimal EventEmitter compatible with common usage patterns.

/* eslint-disable @typescript-eslint/no-explicit-any */

export class EventEmitter {
    private _events: Map<string, ((...args: any[]) => void)[]> = new Map();

    on(event: string, listener: (...args: any[]) => void): this {
        const listeners = this._events.get(event);
        if (listeners) {
            listeners.push(listener);
        } else {
            this._events.set(event, [listener]);
        }
        return this;
    }

    addListener(event: string, listener: (...args: any[]) => void): this {
        return this.on(event, listener);
    }

    once(event: string, listener: (...args: any[]) => void): this {
        const wrapper = (...args: any[]) => {
            this.removeListener(event, wrapper);
            listener.apply(this, args);
        };
        (wrapper as any)._originalListener = listener;
        return this.on(event, wrapper);
    }

    off(event: string, listener: (...args: any[]) => void): this {
        return this.removeListener(event, listener);
    }

    removeListener(event: string, listener: (...args: any[]) => void): this {
        const listeners = this._events.get(event);
        if (listeners) {
            const idx = listeners.findIndex(
                (fn) => fn === listener || (fn as any)._originalListener === listener,
            );
            if (idx !== -1) listeners.splice(idx, 1);
            if (listeners.length === 0) this._events.delete(event);
        }
        return this;
    }

    removeAllListeners(event?: string): this {
        if (event) {
            this._events.delete(event);
        } else {
            this._events.clear();
        }
        return this;
    }

    emit(event: string, ...args: any[]): boolean {
        const listeners = this._events.get(event);
        if (!listeners || listeners.length === 0) return false;
        const snapshot = listeners.slice();
        for (const listener of snapshot) {
            listener.apply(this, args);
        }
        return true;
    }

    listenerCount(event: string): number {
        return this._events.get(event)?.length ?? 0;
    }

    listeners(event: string): ((...args: any[]) => void)[] {
        return [...(this._events.get(event) ?? [])];
    }

    eventNames(): string[] {
        return [...this._events.keys()];
    }
}

export default EventEmitter;
