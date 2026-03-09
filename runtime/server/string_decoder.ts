// Node.js `string_decoder` module stub for Rex server bundles.
// Provides minimal StringDecoder for packages that use it to decode
// Buffer chunks into strings (e.g., readable streams in pg).

/* eslint-disable @typescript-eslint/no-explicit-any */

export class StringDecoder {
    private encoding: string;

    constructor(encoding?: string) {
        this.encoding = (encoding || 'utf8').toLowerCase().replace('-', '');
    }

    write(buffer: any): string {
        if (typeof buffer === 'string') return buffer;
        if (buffer && typeof buffer.toString === 'function') {
            return buffer.toString(this.encoding);
        }
        if (buffer instanceof Uint8Array) {
            return new (globalThis as any).TextDecoder().decode(buffer);
        }
        return String(buffer);
    }

    end(buffer?: any): string {
        if (buffer) return this.write(buffer);
        return '';
    }
}

const stringDecoder = { StringDecoder };
export default stringDecoder;
