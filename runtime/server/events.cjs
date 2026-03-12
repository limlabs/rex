// Node.js `events` module polyfill for Rex server bundles (CJS format).
// CJS is required so `require('events')` returns the class directly,
// not a namespace object (which can't be extended with `class extends`).

class EventEmitter {
    constructor() {
        this._events = new Map();
        this._maxListeners = 10;
    }

    _ensureEvents() {
        if (!this._events) this._events = new Map();
        return this._events;
    }

    on(event, listener) { return this._addListener(event, listener, false); }
    addListener(event, listener) { return this.on(event, listener); }
    once(event, listener) { return this._addListener(event, listener, true); }
    off(event, listener) { return this.removeListener(event, listener); }

    removeListener(event, listener) {
        const entries = this._ensureEvents().get(event);
        if (!entries) return this;
        const idx = entries.findIndex(e => e.fn === listener);
        if (idx !== -1) { entries.splice(idx, 1); if (entries.length === 0) this._events.delete(event); }
        return this;
    }

    removeAllListeners(event) {
        const events = this._ensureEvents();
        if (event !== undefined) { events.delete(event); } else { events.clear(); }
        return this;
    }

    emit(event, ...args) {
        const entries = this._ensureEvents().get(event);
        if (!entries || entries.length === 0) return false;
        for (const entry of entries.slice()) {
            if (entry.once) this.removeListener(event, entry.fn);
            entry.fn.apply(this, args);
        }
        return true;
    }

    listeners(event) { return (this._ensureEvents().get(event) || []).map(e => e.fn); }
    listenerCount(event) { return (this._ensureEvents().get(event) || []).length; }
    eventNames() { return Array.from(this._ensureEvents().keys()); }
    setMaxListeners(n) { this._maxListeners = n; return this; }
    getMaxListeners() { return this._maxListeners || 10; }
    prependListener(event, listener) { return this._addListener(event, listener, false, true); }
    prependOnceListener(event, listener) { return this._addListener(event, listener, true, true); }
    rawListeners(event) { return this.listeners(event); }

    _addListener(event, listener, once, prepend) {
        const events = this._ensureEvents();
        let entries = events.get(event);
        if (!entries) { entries = []; events.set(event, entries); }
        const entry = { fn: listener, once };
        if (prepend) { entries.unshift(entry); } else { entries.push(entry); }
        return this;
    }
}

EventEmitter.defaultMaxListeners = 10;
EventEmitter.EventEmitter = EventEmitter;

// Static helpers
EventEmitter.once = function(emitter, event) {
    return new Promise(resolve => emitter.once(event, (...args) => resolve(args)));
};
EventEmitter.on = function() {
    return { [Symbol.asyncIterator]() { return { next() { return new Promise(() => {}); } }; } };
};
EventEmitter.getMaxListeners = function(emitter) { return emitter.getMaxListeners(); };
EventEmitter.setMaxListeners = function(n, ...emitters) { for (const e of emitters) e.setMaxListeners(n); };

module.exports = EventEmitter;
