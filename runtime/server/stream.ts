// Node.js `stream` module stub for Rex server bundles.
// Provides minimal Stream, Readable, Writable, Duplex, Transform, PassThrough
// base classes that extend EventEmitter. Used by pg and other packages
// that inherit from stream classes.

import { EventEmitter } from './events';

/* eslint-disable @typescript-eslint/no-explicit-any */

export class Stream extends EventEmitter {
    pipe(dest: any): any {
        this.on('data', (chunk: any) => dest.write(chunk));
        this.on('end', () => { if (typeof dest.end === 'function') dest.end(); });
        return dest;
    }
}

export class Readable extends Stream {
    readable: boolean;
    _readableState: any;

    constructor(options?: any) {
        super();
        this.readable = true;
        this._readableState = { flowing: null, ended: false, ...options };
    }

    read(_size?: number): any {
        return null;
    }

    push(chunk: any): boolean {
        if (chunk === null) {
            this._readableState.ended = true;
            this.emit('end');
            return false;
        }
        this.emit('data', chunk);
        return true;
    }

    resume(): this {
        this._readableState.flowing = true;
        return this;
    }

    pause(): this {
        this._readableState.flowing = false;
        return this;
    }

    destroy(err?: any): this {
        if (err) this.emit('error', err);
        this.emit('close');
        return this;
    }

    setEncoding(_encoding: string): this {
        return this;
    }

    unpipe(_dest?: any): this {
        return this;
    }
}

export class Writable extends Stream {
    writable: boolean;
    _writableState: any;

    constructor(options?: any) {
        super();
        this.writable = true;
        this._writableState = { ended: false, ...options };
    }

    write(chunk: any, encoding?: any, callback?: any): boolean {
        const cb = typeof encoding === 'function' ? encoding : callback;
        if (typeof this._write === 'function') {
            this._write(chunk, typeof encoding === 'string' ? encoding : 'utf8', cb || (() => {}));
        }
        return true;
    }

    _write(_chunk: any, _encoding: string, callback: () => void): void {
        callback();
    }

    end(chunk?: any, encoding?: any, callback?: any): this {
        if (chunk) this.write(chunk, encoding);
        this._writableState.ended = true;
        const cb = typeof chunk === 'function' ? chunk : (typeof encoding === 'function' ? encoding : callback);
        this.emit('finish');
        if (typeof cb === 'function') cb();
        return this;
    }

    destroy(err?: any): this {
        if (err) this.emit('error', err);
        this.emit('close');
        return this;
    }

    cork(): void {}
    uncork(): void {}

    setDefaultEncoding(_encoding: string): this {
        return this;
    }
}

export class Duplex extends Readable {
    writable: boolean;
    _writableState: any;

    constructor(options?: any) {
        super(options);
        this.writable = true;
        this._writableState = { ended: false };
    }

    write(chunk: any, encoding?: any, callback?: any): boolean {
        return Writable.prototype.write.call(this, chunk, encoding, callback);
    }

    end(chunk?: any, encoding?: any, callback?: any): this {
        Writable.prototype.end.call(this, chunk, encoding, callback);
        return this;
    }
}

export class Transform extends Duplex {
    constructor(options?: any) {
        super(options);
    }

    _transform(chunk: any, _encoding: string, callback: (err?: any, data?: any) => void): void {
        callback(null, chunk);
    }
}

export class PassThrough extends Transform {
    _transform(chunk: any, _encoding: string, callback: (err?: any, data?: any) => void): void {
        callback(null, chunk);
    }
}

const stream = { Stream, Readable, Writable, Duplex, Transform, PassThrough };
export default stream;
