/* eslint-disable @typescript-eslint/no-explicit-any */
// crypto polyfill — randomUUID(), randomBytes(), createHash(), createHmac(), subtle
(function() {
    const _crypto = (globalThis as any).crypto || {};

    // randomUUID — V8 may already provide this via Web Crypto
    if (typeof _crypto.randomUUID !== 'function') {
        _crypto.randomUUID = function(): string {
            const h = '0123456789abcdef';
            let s = '';
            for (let i = 0; i < 36; i++) {
                if (i === 8 || i === 13 || i === 18 || i === 23) { s += '-'; }
                else if (i === 14) { s += '4'; }
                else if (i === 19) { s += h[(Math.random() * 4 | 0) + 8]; }
                else { s += h[Math.random() * 16 | 0]; }
            }
            return s;
        };
    }

    // randomBytes — returns a Buffer of n random bytes
    _crypto.randomBytes = function(size: number): any {
        if (typeof size !== 'number' || size < 0) {
            throw new Error('crypto.randomBytes: size must be a non-negative number');
        }
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) {
            bytes[i] = (Math.random() * 256) | 0;
        }
        if ((globalThis as any).Buffer) return (globalThis as any).Buffer.from(bytes);
        return bytes;
    };

    // ── SHA-256 (FIPS 180-4) ──────────────────────────────────────────────

    const _K256 = new Uint32Array([
        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
    ]);

    function _sha256(data: Uint8Array): Uint8Array {
        let h0 = 0x6a09e667, h1 = 0xbb67ae85, h2 = 0x3c6ef372, h3 = 0xa54ff53a;
        let h4 = 0x510e527f, h5 = 0x9b05688c, h6 = 0x1f83d9ab, h7 = 0x5be0cd19;
        const len = data.length;
        const bitLen = len * 8;
        const padded = len + 1 + 8;
        const blocks = Math.ceil(padded / 64) * 64;
        const msg = new Uint8Array(blocks);
        msg.set(data);
        msg[len] = 0x80;
        msg[blocks - 4] = (bitLen >>> 24) & 0xFF;
        msg[blocks - 3] = (bitLen >>> 16) & 0xFF;
        msg[blocks - 2] = (bitLen >>> 8) & 0xFF;
        msg[blocks - 1] = bitLen & 0xFF;

        const w = new Uint32Array(64);
        for (let offset = 0; offset < blocks; offset += 64) {
            for (let i = 0; i < 16; i++) {
                const j = offset + i * 4;
                w[i] = (msg[j] << 24) | (msg[j+1] << 16) | (msg[j+2] << 8) | msg[j+3];
            }
            for (let i = 16; i < 64; i++) {
                const s0 = ((w[i-15] >>> 7) | (w[i-15] << 25)) ^ ((w[i-15] >>> 18) | (w[i-15] << 14)) ^ (w[i-15] >>> 3);
                const s1 = ((w[i-2] >>> 17) | (w[i-2] << 15)) ^ ((w[i-2] >>> 19) | (w[i-2] << 13)) ^ (w[i-2] >>> 10);
                w[i] = (w[i-16] + s0 + w[i-7] + s1) | 0;
            }

            let a = h0, b = h1, c = h2, d = h3, e = h4, f = h5, g = h6, hv = h7;
            for (let i = 0; i < 64; i++) {
                const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
                const ch = (e & f) ^ (~e & g);
                const t1 = (hv + S1 + ch + _K256[i] + w[i]) | 0;
                const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
                const maj = (a & b) ^ (a & c) ^ (b & c);
                const t2 = (S0 + maj) | 0;
                hv = g; g = f; f = e; e = (d + t1) | 0;
                d = c; c = b; b = a; a = (t1 + t2) | 0;
            }
            h0 = (h0 + a) | 0; h1 = (h1 + b) | 0; h2 = (h2 + c) | 0; h3 = (h3 + d) | 0;
            h4 = (h4 + e) | 0; h5 = (h5 + f) | 0; h6 = (h6 + g) | 0; h7 = (h7 + hv) | 0;
        }

        const out = new Uint8Array(32);
        const hh = [h0, h1, h2, h3, h4, h5, h6, h7];
        for (let i = 0; i < 8; i++) {
            out[i*4]   = (hh[i] >>> 24) & 0xFF;
            out[i*4+1] = (hh[i] >>> 16) & 0xFF;
            out[i*4+2] = (hh[i] >>> 8)  & 0xFF;
            out[i*4+3] = hh[i] & 0xFF;
        }
        return out;
    }

    // ── MD5 (RFC 1321) — used by pg for legacy MD5 password auth ──────────

    function _md5(data: Uint8Array): Uint8Array {
        const S = [
            7,12,17,22, 7,12,17,22, 7,12,17,22, 7,12,17,22,
            5, 9,14,20, 5, 9,14,20, 5, 9,14,20, 5, 9,14,20,
            4,11,16,23, 4,11,16,23, 4,11,16,23, 4,11,16,23,
            6,10,15,21, 6,10,15,21, 6,10,15,21, 6,10,15,21
        ];
        const K = new Uint32Array(64);
        for (let i = 0; i < 64; i++) K[i] = Math.floor(Math.abs(Math.sin(i + 1)) * 0x100000000) >>> 0;

        let a0 = 0x67452301, b0 = 0xefcdab89, c0 = 0x98badcfe, d0 = 0x10325476;
        const len = data.length;
        const bitLen = len * 8;
        const padded = len + 1 + 8;
        const blocks = Math.ceil(padded / 64) * 64;
        const msg = new Uint8Array(blocks);
        msg.set(data);
        msg[len] = 0x80;
        // Little-endian bit length
        msg[blocks - 8] = bitLen & 0xFF;
        msg[blocks - 7] = (bitLen >>> 8) & 0xFF;
        msg[blocks - 6] = (bitLen >>> 16) & 0xFF;
        msg[blocks - 5] = (bitLen >>> 24) & 0xFF;

        for (let offset = 0; offset < blocks; offset += 64) {
            const M = new Uint32Array(16);
            for (let j = 0; j < 16; j++) {
                const p = offset + j * 4;
                M[j] = msg[p] | (msg[p+1] << 8) | (msg[p+2] << 16) | (msg[p+3] << 24);
            }
            let A = a0, B = b0, C = c0, D = d0;
            for (let i = 0; i < 64; i++) {
                let F: number, g: number;
                if (i < 16)      { F = (B & C) | (~B & D); g = i; }
                else if (i < 32) { F = (D & B) | (~D & C); g = (5 * i + 1) % 16; }
                else if (i < 48) { F = B ^ C ^ D; g = (3 * i + 5) % 16; }
                else             { F = C ^ (B | ~D); g = (7 * i) % 16; }
                F = (F + A + K[i] + M[g]) | 0;
                A = D; D = C; C = B;
                B = (B + ((F << S[i]) | (F >>> (32 - S[i])))) | 0;
            }
            a0 = (a0 + A) | 0; b0 = (b0 + B) | 0; c0 = (c0 + C) | 0; d0 = (d0 + D) | 0;
        }
        const out = new Uint8Array(16);
        for (let i = 0; i < 4; i++) {
            out[i]     = (a0 >>> (i * 8)) & 0xFF;
            out[i+4]   = (b0 >>> (i * 8)) & 0xFF;
            out[i+8]   = (c0 >>> (i * 8)) & 0xFF;
            out[i+12]  = (d0 >>> (i * 8)) & 0xFF;
        }
        return out;
    }

    // ── HMAC-SHA-256 ──────────────────────────────────────────────────────

    function _hmacSha256(key: Uint8Array, message: Uint8Array): Uint8Array {
        const blockSize = 64;
        let keyBlock = new Uint8Array(blockSize);
        if (key.length > blockSize) {
            keyBlock.set(_sha256(key));
        } else {
            keyBlock.set(key);
        }
        const ipad = new Uint8Array(blockSize + message.length);
        const opad = new Uint8Array(blockSize + 32);
        for (let i = 0; i < blockSize; i++) {
            ipad[i] = keyBlock[i] ^ 0x36;
            opad[i] = keyBlock[i] ^ 0x5c;
        }
        ipad.set(message, blockSize);
        const innerHash = _sha256(ipad);
        opad.set(innerHash, blockSize);
        return _sha256(opad);
    }

    // ── PBKDF2 (RFC 2898) with HMAC-SHA-256 ───────────────────────────────

    function _pbkdf2(password: Uint8Array, salt: Uint8Array, iterations: number, keyLen: number): Uint8Array {
        const result = new Uint8Array(keyLen);
        const numBlocks = Math.ceil(keyLen / 32);
        for (let blockNum = 1; blockNum <= numBlocks; blockNum++) {
            const saltBlock = new Uint8Array(salt.length + 4);
            saltBlock.set(salt);
            saltBlock[salt.length]     = (blockNum >>> 24) & 0xFF;
            saltBlock[salt.length + 1] = (blockNum >>> 16) & 0xFF;
            saltBlock[salt.length + 2] = (blockNum >>> 8) & 0xFF;
            saltBlock[salt.length + 3] = blockNum & 0xFF;
            let u = _hmacSha256(password, saltBlock);
            const block = new Uint8Array(u);
            for (let i = 1; i < iterations; i++) {
                u = _hmacSha256(password, u);
                for (let j = 0; j < 32; j++) block[j] ^= u[j];
            }
            const offset = (blockNum - 1) * 32;
            const len = Math.min(32, keyLen - offset);
            result.set(block.subarray(0, len), offset);
        }
        return result;
    }

    // ── Helper: convert input to Uint8Array ───────────────────────────────

    function _toBytes(data: any, encoding?: string): Uint8Array {
        if (typeof data === 'string') {
            if (encoding === 'hex') {
                const hlen = data.length >> 1;
                const buf = new Uint8Array(hlen);
                for (let i = 0; i < hlen; i++) buf[i] = parseInt(data.substr(i * 2, 2), 16);
                return buf;
            }
            return new (globalThis as any).TextEncoder().encode(data);
        }
        if (data instanceof Uint8Array) return data;
        if (data && data._isBuffer) return new Uint8Array(data.buffer, data.byteOffset, data.length);
        if (data instanceof ArrayBuffer) return new Uint8Array(data);
        return new Uint8Array(data);
    }

    function _digestToOutput(result: Uint8Array, encoding?: string): any {
        if (!encoding || encoding === 'buffer') {
            if ((globalThis as any).Buffer) return (globalThis as any).Buffer.from(result);
            return result;
        }
        if (encoding === 'hex') {
            let hex = '';
            for (let i = 0; i < result.length; i++) hex += (result[i] < 16 ? '0' : '') + result[i].toString(16);
            return hex;
        }
        if (encoding === 'base64') {
            if ((globalThis as any).Buffer) return (globalThis as any).Buffer.from(result).toString('base64');
            const b64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
            let out = '';
            for (let i = 0; i < result.length; i += 3) {
                const aa = result[i], bb = result[i+1], cc = result[i+2];
                out += b64[aa >> 2];
                out += b64[((aa & 3) << 4) | ((bb || 0) >> 4)];
                out += (i+1 < result.length) ? b64[((bb & 0xF) << 2) | ((cc || 0) >> 6)] : '=';
                out += (i+2 < result.length) ? b64[cc & 0x3F] : '=';
            }
            return out;
        }
        throw new Error('Unsupported encoding "' + encoding + '"');
    }

    // ── createHash ────────────────────────────────────────────────────────

    function _selectHash(alg: string): (data: Uint8Array) => Uint8Array {
        if (alg === 'sha256' || alg === 'sha-256') return _sha256;
        if (alg === 'md5') return _md5;
        throw new Error('createHash: unsupported algorithm "' + alg + '"');
    }

    _crypto.createHash = function(algorithm: string) {
        const hashFn = _selectHash(algorithm.toLowerCase());
        const chunks: Uint8Array[] = [];
        let totalLen = 0;
        const hashObj = {
            update(data: any, encoding?: string) {
                const buf = _toBytes(data, encoding);
                chunks.push(buf);
                totalLen += buf.length;
                return hashObj;
            },
            digest(encoding?: string) {
                const combined = new Uint8Array(totalLen);
                let pos = 0;
                for (const c of chunks) { combined.set(c, pos); pos += c.length; }
                return _digestToOutput(hashFn(combined), encoding);
            }
        };
        return hashObj;
    };

    // ── createHmac ────────────────────────────────────────────────────────

    _crypto.createHmac = function(algorithm: string, key: any) {
        const alg = algorithm.toLowerCase();
        if (alg !== 'sha256' && alg !== 'sha-256') {
            throw new Error('createHmac: unsupported algorithm "' + algorithm + '"');
        }
        const keyBytes = _toBytes(key);
        const chunks: Uint8Array[] = [];
        let totalLen = 0;
        const hmacObj = {
            update(data: any, encoding?: string) {
                const buf = _toBytes(data, encoding);
                chunks.push(buf);
                totalLen += buf.length;
                return hmacObj;
            },
            digest(encoding?: string) {
                const combined = new Uint8Array(totalLen);
                let pos = 0;
                for (const c of chunks) { combined.set(c, pos); pos += c.length; }
                return _digestToOutput(_hmacSha256(keyBytes, combined), encoding);
            }
        };
        return hmacObj;
    };

    // ── SubtleCrypto (WebCrypto API for pg SCRAM-SHA-256 auth) ────────────

    if (!_crypto.subtle) {
        _crypto.subtle = {
            async digest(algorithm: any, data: any): Promise<ArrayBuffer> {
                const alg = (typeof algorithm === 'string' ? algorithm : algorithm.name || '').replace('-', '').toLowerCase();
                const bytes = data instanceof Uint8Array ? data : new Uint8Array(data);
                if (alg === 'sha256') return _sha256(bytes).buffer as ArrayBuffer;
                if (alg === 'md5') return _md5(bytes).buffer as ArrayBuffer;
                throw new Error('subtle.digest: unsupported algorithm');
            },
            async importKey(_format: string, keyData: any, algorithm: any, extractable: boolean, usages: string[]): Promise<any> {
                const data = keyData instanceof Uint8Array ? keyData : new Uint8Array(keyData);
                return { type: 'raw', algorithm, data, extractable, usages };
            },
            async sign(algorithm: any, key: any, data: any): Promise<ArrayBuffer> {
                const alg = typeof algorithm === 'string' ? algorithm : (algorithm.name || '');
                if (alg === 'HMAC') {
                    const msg = data instanceof Uint8Array ? data : new Uint8Array(data);
                    return _hmacSha256(key.data, msg).buffer as ArrayBuffer;
                }
                throw new Error('subtle.sign: unsupported algorithm ' + alg);
            },
            async deriveBits(algorithm: any, key: any, length: number): Promise<ArrayBuffer> {
                if (algorithm.name === 'PBKDF2') {
                    const salt = algorithm.salt instanceof Uint8Array ? algorithm.salt : new Uint8Array(algorithm.salt);
                    return _pbkdf2(key.data, salt, algorithm.iterations, length / 8).buffer as ArrayBuffer;
                }
                throw new Error('subtle.deriveBits: unsupported algorithm ' + algorithm.name);
            },
        };
    }

    // ── pbkdf2Sync / pbkdf2 (Node.js crypto API) ────────────────────────
    // Used by postgres.js for SCRAM-SHA-256 authentication.

    _crypto.pbkdf2Sync = function(password: any, salt: any, iterations: number, keylen: number, _digest: string): any {
        const pwBytes = _toBytes(password);
        const saltBytes = _toBytes(salt);
        const result = _pbkdf2(pwBytes, saltBytes, iterations, keylen);
        if ((globalThis as any).Buffer) return (globalThis as any).Buffer.from(result);
        return result;
    };

    _crypto.pbkdf2 = function(password: any, salt: any, iterations: number, keylen: number, digest: string, callback: any): void {
        try {
            const result = _crypto.pbkdf2Sync(password, salt, iterations, keylen, digest);
            if (typeof callback === 'function') callback(null, result);
        } catch (e) {
            if (typeof callback === 'function') callback(e);
        }
    };

    // getRandomValues for SubtleCrypto usage
    if (typeof _crypto.getRandomValues !== 'function') {
        _crypto.getRandomValues = function(array: Uint8Array): Uint8Array {
            for (let i = 0; i < array.length; i++) {
                array[i] = (Math.random() * 256) | 0;
            }
            return array;
        };
    }

    (globalThis as any).crypto = _crypto;
})();
