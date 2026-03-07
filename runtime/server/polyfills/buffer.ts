/* eslint-disable @typescript-eslint/no-explicit-any */
// Node.js Buffer polyfill for bare V8
if (typeof (globalThis as any).Buffer === 'undefined') {
    const _B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    const _B64L = new Uint8Array(256);
    for (let _bi = 0; _bi < _B64.length; _bi++) _B64L[_B64.charCodeAt(_bi)] = _bi;

    function _mkBuf(u8: any): any {
        const keys = Object.getOwnPropertyNames(Buffer.prototype);
        for (let i = 0; i < keys.length; i++) {
            if (typeof (Buffer.prototype as any)[keys[i]] === 'function') {
                u8[keys[i]] = (Buffer.prototype as any)[keys[i]];
            }
        }
        u8._isBuffer = true;
        return u8;
    }

    function _normEnc(enc: any): string {
        if (!enc) return 'utf8';
        const s = String(enc).toLowerCase();
        if (s === 'utf-8') return 'utf8';
        return s;
    }

    function _fromStr(str: string, encoding: any): any {
        const enc = _normEnc(encoding);
        if (enc === 'base64') {
            const raw = str.replace(/[^A-Za-z0-9+/]/g, '');
            const len = (raw.length * 3) >> 2;
            const buf = new Uint8Array(len);
            let p = 0;
            for (let i = 0; i < raw.length; i += 4) {
                const a = _B64L[raw.charCodeAt(i)];
                const b = _B64L[raw.charCodeAt(i + 1)];
                const c = _B64L[raw.charCodeAt(i + 2)];
                const d = _B64L[raw.charCodeAt(i + 3)];
                buf[p++] = (a << 2) | (b >> 4);
                if (i + 2 < raw.length) buf[p++] = ((b & 0x0F) << 4) | (c >> 2);
                if (i + 3 < raw.length) buf[p++] = ((c & 0x03) << 6) | d;
            }
            return _mkBuf(buf.subarray(0, p));
        }
        if (enc === 'hex') {
            const hlen = str.length >> 1;
            const hbuf = new Uint8Array(hlen);
            for (let hi = 0; hi < hlen; hi++) hbuf[hi] = parseInt(str.substr(hi * 2, 2), 16);
            return _mkBuf(hbuf);
        }
        if (enc === 'latin1' || enc === 'binary' || enc === 'ascii') {
            const lbuf = new Uint8Array(str.length);
            for (let li = 0; li < str.length; li++) lbuf[li] = str.charCodeAt(li) & 0xFF;
            return _mkBuf(lbuf);
        }
        // utf8 (default)
        return _mkBuf(new (globalThis as any).TextEncoder().encode(str));
    }

    function _toBase64(bytes: Uint8Array): string {
        let out = '';
        for (let i = 0; i < bytes.length; i += 3) {
            const a = bytes[i], b = bytes[i + 1], c = bytes[i + 2];
            out += _B64[a >> 2];
            out += _B64[((a & 0x03) << 4) | ((b || 0) >> 4)];
            out += (i + 1 < bytes.length) ? _B64[((b & 0x0F) << 2) | ((c || 0) >> 6)] : '=';
            out += (i + 2 < bytes.length) ? _B64[c & 0x3F] : '=';
        }
        return out;
    }

    function Buffer(this: any, arg: any, encodingOrOffset: any, length: any) {
        if (typeof arg === 'number') return Buffer.alloc(arg);
        return Buffer.from(arg, encodingOrOffset, length);
    }

    Buffer.isBuffer = function(obj: any): boolean { return !!(obj && obj._isBuffer); };

    Buffer.isEncoding = function(encoding: any): boolean {
        return ['utf8','utf-8','ascii','latin1','binary','base64','hex','ucs2','ucs-2','utf16le','utf-16le'].indexOf(String(encoding).toLowerCase()) !== -1;
    };

    Buffer.from = function(value: any, encodingOrOffset?: any, length?: number): any {
        if (typeof value === 'string') return _fromStr(value, encodingOrOffset || 'utf8');
        if (value instanceof ArrayBuffer) {
            const off = encodingOrOffset || 0;
            const len = length !== undefined ? length : value.byteLength - off;
            return _mkBuf(new Uint8Array(value, off, len));
        }
        if (value instanceof Uint8Array || Array.isArray(value)) {
            const ubuf = new Uint8Array(value.length);
            for (let ui = 0; ui < value.length; ui++) ubuf[ui] = value[ui];
            return _mkBuf(ubuf);
        }
        if (value && value._isBuffer) {
            const bbuf = new Uint8Array(value.length);
            for (let bi = 0; bi < value.length; bi++) bbuf[bi] = value[bi];
            return _mkBuf(bbuf);
        }
        throw new TypeError('First argument must be a string, Buffer, ArrayBuffer, Array, or array-like object');
    };

    Buffer.alloc = function(size: number, fill?: any, encoding?: any): any {
        const buf = _mkBuf(new Uint8Array(size));
        if (fill !== undefined) buf.fill(fill, 0, size, encoding);
        return buf;
    };

    Buffer.allocUnsafe = function(size: number): any { return Buffer.alloc(size); };

    Buffer.byteLength = function(string: any, encoding?: any): number {
        if (typeof string !== 'string') return string.length;
        return Buffer.from(string, encoding).length;
    };

    Buffer.concat = function(list: any[], totalLength?: number): any {
        if (totalLength === undefined) {
            totalLength = 0;
            for (let i = 0; i < list.length; i++) totalLength += list[i].length;
        }
        const buf = Buffer.alloc(totalLength);
        let pos = 0;
        for (let i = 0; i < list.length; i++) {
            for (let j = 0; j < list[i].length && pos < totalLength!; j++) {
                buf[pos++] = list[i][j];
            }
        }
        return buf;
    };

    Buffer.prototype.toString = function(this: any, encoding: any, start: number, end: number): string {
        start = start || 0;
        end = end !== undefined ? end : this.length;
        const slice = this.subarray(start, end);
        const enc = _normEnc(encoding);
        if (enc === 'base64') return _toBase64(slice);
        if (enc === 'hex') {
            let hout = '';
            for (let hi = 0; hi < slice.length; hi++) hout += (slice[hi] < 16 ? '0' : '') + slice[hi].toString(16);
            return hout;
        }
        if (enc === 'latin1' || enc === 'binary' || enc === 'ascii') {
            let lout = '';
            for (let li = 0; li < slice.length; li++) lout += String.fromCharCode(slice[li]);
            return lout;
        }
        return new (globalThis as any).TextDecoder().decode(slice);
    };

    Buffer.prototype.toJSON = function(this: any) {
        return { type: 'Buffer', data: Array.prototype.slice.call(this) };
    };

    Buffer.prototype.slice = function(this: any, start: number, end: number): any {
        return _mkBuf(this.subarray(start, end));
    };

    Buffer.prototype.copy = function(this: any, target: any, targetStart: number, sourceStart: number, sourceEnd: number): number {
        targetStart = targetStart || 0;
        sourceStart = sourceStart || 0;
        sourceEnd = sourceEnd !== undefined ? sourceEnd : this.length;
        const n = Math.min(sourceEnd - sourceStart, target.length - targetStart);
        for (let i = 0; i < n; i++) target[targetStart + i] = this[sourceStart + i];
        return n;
    };

    Buffer.prototype.equals = function(this: any, other: any): boolean {
        if (this.length !== other.length) return false;
        for (let i = 0; i < this.length; i++) {
            if (this[i] !== other[i]) return false;
        }
        return true;
    };

    Buffer.prototype.compare = function(this: any, other: any): number {
        const len = Math.min(this.length, other.length);
        for (let i = 0; i < len; i++) {
            if (this[i] !== other[i]) return this[i] < other[i] ? -1 : 1;
        }
        return this.length === other.length ? 0 : (this.length < other.length ? -1 : 1);
    };

    Buffer.prototype.write = function(this: any, string: string, offset: any, length: any, encoding: any): number {
        if (typeof offset === 'string') { encoding = offset; offset = 0; length = this.length; }
        else if (typeof length === 'string') { encoding = length; length = this.length - (offset || 0); }
        offset = offset || 0;
        length = length !== undefined ? length : this.length - offset;
        const src = Buffer.from(string, encoding || 'utf8');
        const n = Math.min(src.length, length, this.length - offset);
        for (let i = 0; i < n; i++) this[offset + i] = src[i];
        return n;
    };

    Buffer.prototype.fill = function(this: any, value: any, offset: number, end: number, encoding: any): any {
        offset = offset || 0;
        end = end !== undefined ? end : this.length;
        if (typeof value === 'number') {
            for (let i = offset; i < end; i++) this[i] = value & 0xFF;
        } else if (typeof value === 'string') {
            const src = Buffer.from(value, encoding || 'utf8');
            for (let i = offset; i < end; i++) this[i] = src[(i - offset) % src.length];
        }
        return this;
    };

    Buffer.prototype.indexOf = function(this: any, value: any, byteOffset: number, encoding: any): number {
        if (typeof value === 'number') {
            for (let i = byteOffset || 0; i < this.length; i++) {
                if (this[i] === (value & 0xFF)) return i;
            }
            return -1;
        }
        if (typeof value === 'string') value = Buffer.from(value, encoding || 'utf8');
        byteOffset = byteOffset || 0;
        if (value.length === 0) return byteOffset < this.length ? byteOffset : this.length;
        for (let i = byteOffset; i <= this.length - value.length; i++) {
            let found = true;
            for (let j = 0; j < value.length; j++) {
                if (this[i + j] !== value[j]) { found = false; break; }
            }
            if (found) return i;
        }
        return -1;
    };

    Buffer.prototype.includes = function(this: any, value: any, byteOffset: number, encoding: any): boolean {
        return this.indexOf(value, byteOffset, encoding) !== -1;
    };

    // Integer read/write methods
    Buffer.prototype.readUInt8 = function(this: any, off: number) { return this[off]; };
    Buffer.prototype.readUint8 = Buffer.prototype.readUInt8;
    Buffer.prototype.readInt8 = function(this: any, off: number) { const v = this[off]; return v > 127 ? v - 256 : v; };
    Buffer.prototype.readUInt16BE = function(this: any, off: number) { return (this[off] << 8) | this[off + 1]; };
    Buffer.prototype.readUint16BE = Buffer.prototype.readUInt16BE;
    Buffer.prototype.readUInt16LE = function(this: any, off: number) { return this[off] | (this[off + 1] << 8); };
    Buffer.prototype.readUint16LE = Buffer.prototype.readUInt16LE;
    Buffer.prototype.readInt16BE = function(this: any, off: number) { const v = this.readUInt16BE(off); return v > 0x7FFF ? v - 0x10000 : v; };
    Buffer.prototype.readInt16LE = function(this: any, off: number) { const v = this.readUInt16LE(off); return v > 0x7FFF ? v - 0x10000 : v; };
    Buffer.prototype.readUInt32BE = function(this: any, off: number) { return ((this[off] << 24) | (this[off+1] << 16) | (this[off+2] << 8) | this[off+3]) >>> 0; };
    Buffer.prototype.readUint32BE = Buffer.prototype.readUInt32BE;
    Buffer.prototype.readUInt32LE = function(this: any, off: number) { return ((this[off+3] << 24) | (this[off+2] << 16) | (this[off+1] << 8) | this[off]) >>> 0; };
    Buffer.prototype.readUint32LE = Buffer.prototype.readUInt32LE;
    Buffer.prototype.readInt32BE = function(this: any, off: number) { return (this[off] << 24) | (this[off+1] << 16) | (this[off+2] << 8) | this[off+3]; };
    Buffer.prototype.readInt32LE = function(this: any, off: number) { return (this[off+3] << 24) | (this[off+2] << 16) | (this[off+1] << 8) | this[off]; };
    Buffer.prototype.writeUInt8 = function(this: any, val: number, off: number) { this[off] = val & 0xFF; return off + 1; };
    Buffer.prototype.writeUint8 = Buffer.prototype.writeUInt8;
    Buffer.prototype.writeUInt16BE = function(this: any, val: number, off: number) { this[off] = (val >> 8) & 0xFF; this[off+1] = val & 0xFF; return off + 2; };
    Buffer.prototype.writeUint16BE = Buffer.prototype.writeUInt16BE;
    Buffer.prototype.writeUInt16LE = function(this: any, val: number, off: number) { this[off] = val & 0xFF; this[off+1] = (val >> 8) & 0xFF; return off + 2; };
    Buffer.prototype.writeUint16LE = Buffer.prototype.writeUInt16LE;
    Buffer.prototype.writeUInt32BE = function(this: any, val: number, off: number) { this[off] = (val >> 24) & 0xFF; this[off+1] = (val >> 16) & 0xFF; this[off+2] = (val >> 8) & 0xFF; this[off+3] = val & 0xFF; return off + 4; };
    Buffer.prototype.writeUint32BE = Buffer.prototype.writeUInt32BE;
    Buffer.prototype.writeUInt32LE = function(this: any, val: number, off: number) { this[off] = val & 0xFF; this[off+1] = (val >> 8) & 0xFF; this[off+2] = (val >> 16) & 0xFF; this[off+3] = (val >> 24) & 0xFF; return off + 4; };
    Buffer.prototype.writeUint32LE = Buffer.prototype.writeUInt32LE;
    Buffer.prototype.writeInt8 = function(this: any, val: number, off: number) { if (val < 0) val = 256 + val; this[off] = val & 0xFF; return off + 1; };
    Buffer.prototype.writeInt16BE = function(this: any, val: number, off: number) { if (val < 0) val = 0x10000 + val; return this.writeUInt16BE(val, off); };
    Buffer.prototype.writeInt16LE = function(this: any, val: number, off: number) { if (val < 0) val = 0x10000 + val; return this.writeUInt16LE(val, off); };
    Buffer.prototype.writeInt32BE = function(this: any, val: number, off: number) { if (val < 0) val = 0x100000000 + val; return this.writeUInt32BE(val >>> 0, off); };
    Buffer.prototype.writeInt32LE = function(this: any, val: number, off: number) { if (val < 0) val = 0x100000000 + val; return this.writeUInt32LE(val >>> 0, off); };

    (globalThis as any).Buffer = Buffer;
}
