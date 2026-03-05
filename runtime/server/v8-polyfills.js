
if (typeof globalThis.process === 'undefined') {
    globalThis.process = { env: { NODE_ENV: 'production' } };
}
if (typeof globalThis.setTimeout === 'undefined') {
    globalThis.setTimeout = function(fn) { fn(); return 0; };
    globalThis.clearTimeout = function() {};
}
if (typeof globalThis.queueMicrotask === 'undefined') {
    globalThis.queueMicrotask = function(fn) { fn(); };
}
if (typeof globalThis.MessageChannel === 'undefined') {
    globalThis.MessageChannel = function() {
        var cb = null;
        this.port1 = {};
        this.port2 = { postMessage: function() { if (cb) cb({ data: undefined }); } };
        Object.defineProperty(this.port1, 'onmessage', {
            set: function(fn) { cb = fn; }, get: function() { return cb; }
        });
    };
}
if (typeof globalThis.TextEncoder === 'undefined') {
    globalThis.TextEncoder = function() {};
    globalThis.TextEncoder.prototype.encode = function(str) {
        var arr = [];
        for (var i = 0; i < str.length; i++) {
            var c = str.charCodeAt(i);
            if (c < 0x80) {
                arr.push(c);
            } else if (c < 0x800) {
                arr.push(0xC0 | (c >> 6), 0x80 | (c & 0x3F));
            } else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < str.length) {
                var next = str.charCodeAt(i + 1);
                if (next >= 0xDC00 && next <= 0xDFFF) {
                    var cp = ((c - 0xD800) << 10) + (next - 0xDC00) + 0x10000;
                    arr.push(0xF0 | (cp >> 18), 0x80 | ((cp >> 12) & 0x3F),
                             0x80 | ((cp >> 6) & 0x3F), 0x80 | (cp & 0x3F));
                    i++;
                }
            } else {
                arr.push(0xE0 | (c >> 12), 0x80 | ((c >> 6) & 0x3F), 0x80 | (c & 0x3F));
            }
        }
        return new Uint8Array(arr);
    };
}
if (typeof globalThis.TextDecoder === 'undefined') {
    globalThis.TextDecoder = function() {};
    globalThis.TextDecoder.prototype.decode = function(buf) {
        var bytes = new Uint8Array(buf);
        var out = '', i = 0;
        while (i < bytes.length) {
            var b = bytes[i];
            if (b < 0x80) { out += String.fromCharCode(b); i++; }
            else if ((b & 0xE0) === 0xC0) {
                out += String.fromCharCode(((b & 0x1F) << 6) | (bytes[i+1] & 0x3F));
                i += 2;
            } else if ((b & 0xF0) === 0xE0) {
                out += String.fromCharCode(((b & 0x0F) << 12) | ((bytes[i+1] & 0x3F) << 6)
                    | (bytes[i+2] & 0x3F));
                i += 3;
            } else if ((b & 0xF8) === 0xF0) {
                var cp = ((b & 0x07) << 18) | ((bytes[i+1] & 0x3F) << 12)
                    | ((bytes[i+2] & 0x3F) << 6) | (bytes[i+3] & 0x3F);
                cp -= 0x10000;
                out += String.fromCharCode(0xD800 + (cp >> 10), 0xDC00 + (cp & 0x3FF));
                i += 4;
            } else { out += '\uFFFD'; i++; }
        }
        return out;
    };
}
if (typeof globalThis.performance === 'undefined') {
    globalThis.performance = { now: function() { return Date.now(); } };
}
if (typeof globalThis.URL === 'undefined') {
    globalThis.URL = function(path, base) {
        if (base) {
            // Simple URL joining: extract origin from base, append path
            var m = String(base).match(/^(https?:[/][/][^/]+)/);
            var origin = m ? m[1] : '';
            var p = String(path);
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
        // Parse pathname from href
        var withoutProto = this.href.replace(/^https?:[/][/][^/]+/, '');
        this.pathname = withoutProto ? withoutProto.split('?')[0].split('#')[0] : '/';
        if (!this.pathname.startsWith('/')) this.pathname = '/' + this.pathname;
        this.search = '';
        var qi = this.href.indexOf('?');
        if (qi !== -1) this.search = this.href.substring(qi).split('#')[0];
    };
    globalThis.URL.prototype.toString = function() { return this.href; };
}
if (typeof globalThis.ReadableStream === 'undefined') {
    globalThis.ReadableStream = function ReadableStream(underlyingSource) {
        this._queue = [];
        this._closed = false;
        this._errored = false;
        this._error = undefined;
        this._reader = null;
        this._readerResolve = null;
        this._pulling = false;
        this._pullAgain = false;
        var self = this;
        var controller = {
            enqueue: function(chunk) {
                if (self._closed || self._errored) return;
                if (self._readerResolve) {
                    var resolve = self._readerResolve;
                    self._readerResolve = null;
                    resolve({ value: chunk, done: false });
                } else {
                    self._queue.push(chunk);
                }
            },
            close: function() {
                if (self._closed || self._errored) return;
                self._closed = true;
                if (self._readerResolve) {
                    var resolve = self._readerResolve;
                    self._readerResolve = null;
                    resolve({ value: undefined, done: true });
                }
            },
            error: function(e) {
                if (self._closed || self._errored) return;
                self._errored = true;
                self._error = e;
                if (self._readerResolve) {
                    var resolve = self._readerResolve;
                    self._readerResolve = null;
                    resolve(Promise.reject(e));
                }
            },
            desiredSize: 1
        };
        this._controller = controller;
        this._underlyingSource = underlyingSource || {};
        if (typeof this._underlyingSource.start === 'function') {
            this._underlyingSource.start(controller);
        }
    };
    ReadableStream.prototype._callPull = function() {
        if (this._pulling || this._closed || this._errored) return;
        if (typeof this._underlyingSource.pull !== 'function') return;
        this._pulling = true;
        var self = this;
        try {
            var result = this._underlyingSource.pull(this._controller);
            if (result && typeof result.then === 'function') {
                result.then(function() {
                    self._pulling = false;
                    if (self._pullAgain) {
                        self._pullAgain = false;
                        self._callPull();
                    }
                }, function(err) {
                    self._pulling = false;
                    self._controller.error(err);
                });
            } else {
                this._pulling = false;
            }
        } catch(e) {
            this._pulling = false;
            this._controller.error(e);
        }
    };
    globalThis.ReadableStream.prototype.getReader = function() {
        var stream = this;
        stream._reader = true;
        return {
            read: function() {
                if (stream._errored) return Promise.reject(stream._error);
                if (stream._queue.length > 0) {
                    var value = stream._queue.shift();
                    stream._callPull();
                    return Promise.resolve({ value: value, done: false });
                }
                if (stream._closed) {
                    return Promise.resolve({ value: undefined, done: true });
                }
                // No data available — call pull (may enqueue synchronously)
                stream._callPull();
                // Re-check after pull in case data was enqueued synchronously
                if (stream._queue.length > 0) {
                    var value = stream._queue.shift();
                    return Promise.resolve({ value: value, done: false });
                }
                if (stream._closed) {
                    return Promise.resolve({ value: undefined, done: true });
                }
                // Still no data — wait for async enqueue
                return new Promise(function(resolve) {
                    stream._readerResolve = resolve;
                });
            },
            cancel: function() {
                stream._closed = true;
                stream._queue = [];
                return Promise.resolve();
            },
            releaseLock: function() {
                stream._reader = null;
            }
        };
    };
}

// AbortController/AbortSignal polyfill for bare V8
if (typeof globalThis.AbortController === 'undefined') {
    function AbortSignal() {
        this.aborted = false;
        this.reason = undefined;
        this._listeners = [];
    }
    AbortSignal.prototype.addEventListener = function(type, listener) {
        if (type === 'abort') this._listeners.push(listener);
    };
    AbortSignal.prototype.removeEventListener = function(type, listener) {
        if (type === 'abort') {
            this._listeners = this._listeners.filter(function(l) { return l !== listener; });
        }
    };
    AbortSignal.prototype.throwIfAborted = function() {
        if (this.aborted) throw this.reason;
    };

    globalThis.AbortController = function AbortController() {
        this.signal = new AbortSignal();
    };
    globalThis.AbortController.prototype.abort = function(reason) {
        if (this.signal.aborted) return;
        this.signal.aborted = true;
        this.signal.reason = reason || new DOMException('The operation was aborted.', 'AbortError');
        var listeners = this.signal._listeners.slice();
        for (var i = 0; i < listeners.length; i++) {
            try { listeners[i]({ type: 'abort', target: this.signal }); } catch { /* intentionally empty */ }
        }
    };

    // DOMException polyfill if not available
    if (typeof globalThis.DOMException === 'undefined') {
        globalThis.DOMException = function DOMException(message, name) {
            this.message = message || '';
            this.name = name || 'Error';
        };
        globalThis.DOMException.prototype = Object.create(Error.prototype);
    }
}

// Buffer polyfill for bare V8 — Node.js Buffer API (base64, hex, binary data)
if (typeof globalThis.Buffer === 'undefined') {
    var _B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    var _B64L = new Uint8Array(256);
    for (var _bi = 0; _bi < _B64.length; _bi++) _B64L[_B64.charCodeAt(_bi)] = _bi;

    function _mkBuf(u8) {
        var keys = Object.getOwnPropertyNames(Buffer.prototype);
        for (var i = 0; i < keys.length; i++) {
            if (typeof Buffer.prototype[keys[i]] === 'function') {
                u8[keys[i]] = Buffer.prototype[keys[i]];
            }
        }
        u8._isBuffer = true;
        return u8;
    }

    function _normEnc(enc) {
        if (!enc) return 'utf8';
        var s = String(enc).toLowerCase();
        if (s === 'utf-8') return 'utf8';
        return s;
    }

    function _fromStr(str, encoding) {
        var enc = _normEnc(encoding);
        if (enc === 'base64') {
            var raw = str.replace(/[^A-Za-z0-9+/]/g, '');
            var len = (raw.length * 3) >> 2;
            var buf = new Uint8Array(len);
            var p = 0;
            for (var i = 0; i < raw.length; i += 4) {
                var a = _B64L[raw.charCodeAt(i)];
                var b = _B64L[raw.charCodeAt(i + 1)];
                var c = _B64L[raw.charCodeAt(i + 2)];
                var d = _B64L[raw.charCodeAt(i + 3)];
                buf[p++] = (a << 2) | (b >> 4);
                if (i + 2 < raw.length) buf[p++] = ((b & 0x0F) << 4) | (c >> 2);
                if (i + 3 < raw.length) buf[p++] = ((c & 0x03) << 6) | d;
            }
            return _mkBuf(buf.subarray(0, p));
        }
        if (enc === 'hex') {
            var hlen = str.length >> 1;
            var buf = new Uint8Array(hlen);
            for (var i = 0; i < hlen; i++) buf[i] = parseInt(str.substr(i * 2, 2), 16);
            return _mkBuf(buf);
        }
        if (enc === 'latin1' || enc === 'binary' || enc === 'ascii') {
            var buf = new Uint8Array(str.length);
            for (var i = 0; i < str.length; i++) buf[i] = str.charCodeAt(i) & 0xFF;
            return _mkBuf(buf);
        }
        // utf8 (default)
        return _mkBuf(new globalThis.TextEncoder().encode(str));
    }

    function _toBase64(bytes) {
        var out = '';
        for (var i = 0; i < bytes.length; i += 3) {
            var a = bytes[i], b = bytes[i + 1], c = bytes[i + 2];
            out += _B64[a >> 2];
            out += _B64[((a & 0x03) << 4) | ((b || 0) >> 4)];
            out += (i + 1 < bytes.length) ? _B64[((b & 0x0F) << 2) | ((c || 0) >> 6)] : '=';
            out += (i + 2 < bytes.length) ? _B64[c & 0x3F] : '=';
        }
        return out;
    }

    function Buffer(arg, encodingOrOffset, length) {
        if (typeof arg === 'number') return Buffer.alloc(arg);
        return Buffer.from(arg, encodingOrOffset, length);
    }

    Buffer.isBuffer = function(obj) { return !!(obj && obj._isBuffer); };

    Buffer.isEncoding = function(encoding) {
        return ['utf8','utf-8','ascii','latin1','binary','base64','hex','ucs2','ucs-2','utf16le','utf-16le'].indexOf(String(encoding).toLowerCase()) !== -1;
    };

    Buffer.from = function(value, encodingOrOffset, length) {
        if (typeof value === 'string') return _fromStr(value, encodingOrOffset || 'utf8');
        if (value instanceof ArrayBuffer) {
            var off = encodingOrOffset || 0;
            var len = length !== undefined ? length : value.byteLength - off;
            return _mkBuf(new Uint8Array(value, off, len));
        }
        if (value instanceof Uint8Array || Array.isArray(value)) {
            var buf = new Uint8Array(value.length);
            for (var i = 0; i < value.length; i++) buf[i] = value[i];
            return _mkBuf(buf);
        }
        if (value && value._isBuffer) {
            var buf = new Uint8Array(value.length);
            for (var i = 0; i < value.length; i++) buf[i] = value[i];
            return _mkBuf(buf);
        }
        throw new TypeError('First argument must be a string, Buffer, ArrayBuffer, Array, or array-like object');
    };

    Buffer.alloc = function(size, fill, encoding) {
        var buf = _mkBuf(new Uint8Array(size));
        if (fill !== undefined) buf.fill(fill, 0, size, encoding);
        return buf;
    };

    Buffer.allocUnsafe = function(size) { return Buffer.alloc(size); };

    Buffer.byteLength = function(string, encoding) {
        if (typeof string !== 'string') return string.length;
        return Buffer.from(string, encoding).length;
    };

    Buffer.concat = function(list, totalLength) {
        if (totalLength === undefined) {
            totalLength = 0;
            for (var i = 0; i < list.length; i++) totalLength += list[i].length;
        }
        var buf = Buffer.alloc(totalLength);
        var pos = 0;
        for (var i = 0; i < list.length; i++) {
            for (var j = 0; j < list[i].length && pos < totalLength; j++) {
                buf[pos++] = list[i][j];
            }
        }
        return buf;
    };

    Buffer.prototype.toString = function(encoding, start, end) {
        start = start || 0;
        end = end !== undefined ? end : this.length;
        var slice = this.subarray(start, end);
        var enc = _normEnc(encoding);
        if (enc === 'base64') return _toBase64(slice);
        if (enc === 'hex') {
            var out = '';
            for (var i = 0; i < slice.length; i++) out += (slice[i] < 16 ? '0' : '') + slice[i].toString(16);
            return out;
        }
        if (enc === 'latin1' || enc === 'binary' || enc === 'ascii') {
            var out = '';
            for (var i = 0; i < slice.length; i++) out += String.fromCharCode(slice[i]);
            return out;
        }
        return new globalThis.TextDecoder().decode(slice);
    };

    Buffer.prototype.toJSON = function() {
        return { type: 'Buffer', data: Array.prototype.slice.call(this) };
    };

    Buffer.prototype.slice = function(start, end) {
        return _mkBuf(this.subarray(start, end));
    };

    Buffer.prototype.copy = function(target, targetStart, sourceStart, sourceEnd) {
        targetStart = targetStart || 0;
        sourceStart = sourceStart || 0;
        sourceEnd = sourceEnd !== undefined ? sourceEnd : this.length;
        var n = Math.min(sourceEnd - sourceStart, target.length - targetStart);
        for (var i = 0; i < n; i++) target[targetStart + i] = this[sourceStart + i];
        return n;
    };

    Buffer.prototype.equals = function(other) {
        if (this.length !== other.length) return false;
        for (var i = 0; i < this.length; i++) {
            if (this[i] !== other[i]) return false;
        }
        return true;
    };

    Buffer.prototype.compare = function(other) {
        var len = Math.min(this.length, other.length);
        for (var i = 0; i < len; i++) {
            if (this[i] !== other[i]) return this[i] < other[i] ? -1 : 1;
        }
        return this.length === other.length ? 0 : (this.length < other.length ? -1 : 1);
    };

    Buffer.prototype.write = function(string, offset, length, encoding) {
        if (typeof offset === 'string') { encoding = offset; offset = 0; length = this.length; }
        else if (typeof length === 'string') { encoding = length; length = this.length - (offset || 0); }
        offset = offset || 0;
        length = length !== undefined ? length : this.length - offset;
        var src = Buffer.from(string, encoding || 'utf8');
        var n = Math.min(src.length, length, this.length - offset);
        for (var i = 0; i < n; i++) this[offset + i] = src[i];
        return n;
    };

    Buffer.prototype.fill = function(value, offset, end, encoding) {
        offset = offset || 0;
        end = end !== undefined ? end : this.length;
        if (typeof value === 'number') {
            for (var i = offset; i < end; i++) this[i] = value & 0xFF;
        } else if (typeof value === 'string') {
            var src = Buffer.from(value, encoding || 'utf8');
            for (var i = offset; i < end; i++) this[i] = src[(i - offset) % src.length];
        }
        return this;
    };

    Buffer.prototype.indexOf = function(value, byteOffset, encoding) {
        if (typeof value === 'number') {
            for (var i = byteOffset || 0; i < this.length; i++) {
                if (this[i] === (value & 0xFF)) return i;
            }
            return -1;
        }
        if (typeof value === 'string') value = Buffer.from(value, encoding || 'utf8');
        byteOffset = byteOffset || 0;
        if (value.length === 0) return byteOffset < this.length ? byteOffset : this.length;
        for (var i = byteOffset; i <= this.length - value.length; i++) {
            var found = true;
            for (var j = 0; j < value.length; j++) {
                if (this[i + j] !== value[j]) { found = false; break; }
            }
            if (found) return i;
        }
        return -1;
    };

    Buffer.prototype.includes = function(value, byteOffset, encoding) {
        return this.indexOf(value, byteOffset, encoding) !== -1;
    };

    // Integer read/write methods
    Buffer.prototype.readUInt8 = function(off) { return this[off]; };
    Buffer.prototype.readUint8 = Buffer.prototype.readUInt8;
    Buffer.prototype.readInt8 = function(off) { var v = this[off]; return v > 127 ? v - 256 : v; };
    Buffer.prototype.readUInt16BE = function(off) { return (this[off] << 8) | this[off + 1]; };
    Buffer.prototype.readUint16BE = Buffer.prototype.readUInt16BE;
    Buffer.prototype.readUInt16LE = function(off) { return this[off] | (this[off + 1] << 8); };
    Buffer.prototype.readUint16LE = Buffer.prototype.readUInt16LE;
    Buffer.prototype.readInt16BE = function(off) { var v = this.readUInt16BE(off); return v > 0x7FFF ? v - 0x10000 : v; };
    Buffer.prototype.readInt16LE = function(off) { var v = this.readUInt16LE(off); return v > 0x7FFF ? v - 0x10000 : v; };
    Buffer.prototype.readUInt32BE = function(off) { return ((this[off] << 24) | (this[off+1] << 16) | (this[off+2] << 8) | this[off+3]) >>> 0; };
    Buffer.prototype.readUint32BE = Buffer.prototype.readUInt32BE;
    Buffer.prototype.readUInt32LE = function(off) { return ((this[off+3] << 24) | (this[off+2] << 16) | (this[off+1] << 8) | this[off]) >>> 0; };
    Buffer.prototype.readUint32LE = Buffer.prototype.readUInt32LE;
    Buffer.prototype.readInt32BE = function(off) { return (this[off] << 24) | (this[off+1] << 16) | (this[off+2] << 8) | this[off+3]; };
    Buffer.prototype.readInt32LE = function(off) { return (this[off+3] << 24) | (this[off+2] << 16) | (this[off+1] << 8) | this[off]; };
    Buffer.prototype.writeUInt8 = function(val, off) { this[off] = val & 0xFF; return off + 1; };
    Buffer.prototype.writeUint8 = Buffer.prototype.writeUInt8;
    Buffer.prototype.writeUInt16BE = function(val, off) { this[off] = (val >> 8) & 0xFF; this[off+1] = val & 0xFF; return off + 2; };
    Buffer.prototype.writeUint16BE = Buffer.prototype.writeUInt16BE;
    Buffer.prototype.writeUInt16LE = function(val, off) { this[off] = val & 0xFF; this[off+1] = (val >> 8) & 0xFF; return off + 2; };
    Buffer.prototype.writeUint16LE = Buffer.prototype.writeUInt16LE;
    Buffer.prototype.writeUInt32BE = function(val, off) { this[off] = (val >> 24) & 0xFF; this[off+1] = (val >> 16) & 0xFF; this[off+2] = (val >> 8) & 0xFF; this[off+3] = val & 0xFF; return off + 4; };
    Buffer.prototype.writeUint32BE = Buffer.prototype.writeUInt32BE;
    Buffer.prototype.writeUInt32LE = function(val, off) { this[off] = val & 0xFF; this[off+1] = (val >> 8) & 0xFF; this[off+2] = (val >> 16) & 0xFF; this[off+3] = (val >> 24) & 0xFF; return off + 4; };
    Buffer.prototype.writeUint32LE = Buffer.prototype.writeUInt32LE;
    Buffer.prototype.writeInt8 = function(val, off) { if (val < 0) val = 256 + val; this[off] = val & 0xFF; return off + 1; };
    Buffer.prototype.writeInt16BE = function(val, off) { if (val < 0) val = 0x10000 + val; return this.writeUInt16BE(val, off); };
    Buffer.prototype.writeInt16LE = function(val, off) { if (val < 0) val = 0x10000 + val; return this.writeUInt16LE(val, off); };
    Buffer.prototype.writeInt32BE = function(val, off) { if (val < 0) val = 0x100000000 + val; return this.writeUInt32BE(val >>> 0, off); };
    Buffer.prototype.writeInt32LE = function(val, off) { if (val < 0) val = 0x100000000 + val; return this.writeUInt32LE(val >>> 0, off); };

    globalThis.Buffer = Buffer;
}
