/* eslint-disable @typescript-eslint/no-explicit-any */
// TextEncoder / TextDecoder polyfills for bare V8
if (typeof globalThis.TextEncoder === 'undefined') {
    (globalThis as any).TextEncoder = function() {};
    (globalThis as any).TextEncoder.prototype.encode = function(str: string): Uint8Array {
        const arr: number[] = [];
        for (let i = 0; i < str.length; i++) {
            const c = str.charCodeAt(i);
            if (c < 0x80) {
                arr.push(c);
            } else if (c < 0x800) {
                arr.push(0xC0 | (c >> 6), 0x80 | (c & 0x3F));
            } else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < str.length) {
                const next = str.charCodeAt(i + 1);
                if (next >= 0xDC00 && next <= 0xDFFF) {
                    const cp = ((c - 0xD800) << 10) + (next - 0xDC00) + 0x10000;
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
    (globalThis as any).TextDecoder = function() {};
    (globalThis as any).TextDecoder.prototype.decode = function(buf: ArrayBuffer): string {
        const bytes = new Uint8Array(buf);
        let out = '';
        let i = 0;
        while (i < bytes.length) {
            const b = bytes[i];
            if (b < 0x80) { out += String.fromCharCode(b); i++; }
            else if ((b & 0xE0) === 0xC0) {
                out += String.fromCharCode(((b & 0x1F) << 6) | (bytes[i+1] & 0x3F));
                i += 2;
            } else if ((b & 0xF0) === 0xE0) {
                out += String.fromCharCode(((b & 0x0F) << 12) | ((bytes[i+1] & 0x3F) << 6)
                    | (bytes[i+2] & 0x3F));
                i += 3;
            } else if ((b & 0xF8) === 0xF0) {
                let cp = ((b & 0x07) << 18) | ((bytes[i+1] & 0x3F) << 12)
                    | ((bytes[i+2] & 0x3F) << 6) | (bytes[i+3] & 0x3F);
                cp -= 0x10000;
                out += String.fromCharCode(0xD800 + (cp >> 10), 0xDC00 + (cp & 0x3FF));
                i += 4;
            } else { out += '\uFFFD'; i++; }
        }
        return out;
    };
}
