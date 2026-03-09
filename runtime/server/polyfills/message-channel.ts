/* eslint-disable @typescript-eslint/no-explicit-any */
// MessageChannel / MessagePort polyfill for bare V8
// React 19 scheduler and undici need MessageChannel and MessagePort.

if (typeof (globalThis as any).MessagePort === 'undefined') {
    (globalThis as any).MessagePort = class MessagePort {
        onmessage: any;
        onmessageerror: any;
        private _otherPort: any;

        constructor() {
            this.onmessage = null;
            this.onmessageerror = null;
            this._otherPort = null;
        }

        postMessage(data: any) {
            if (this._otherPort && this._otherPort.onmessage) {
                this._otherPort.onmessage({ data });
            }
        }

        start() {}
        close() {}
        addEventListener(_type: string, _listener: any) {}
        removeEventListener(_type: string, _listener: any) {}
        dispatchEvent(_event: any) { return true; }
    };
}

if (typeof (globalThis as any).MessageChannel === 'undefined') {
    (globalThis as any).MessageChannel = function(this: any) {
        this.port1 = new (globalThis as any).MessagePort();
        this.port2 = new (globalThis as any).MessagePort();
        this.port1._otherPort = this.port2;
        this.port2._otherPort = this.port1;
    };
}
