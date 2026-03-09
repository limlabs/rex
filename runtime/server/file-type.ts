// Stub polyfill for the `file-type` npm package.
// PayloadCMS imports `fileTypeFromFile` which is only in the `node` condition
// entry. Since V8 can't do real file I/O for binary detection, provide stubs.

/* eslint-disable @typescript-eslint/no-explicit-any */

export async function fileTypeFromFile(_path: string): Promise<any> {
    return undefined;
}

export async function fileTypeFromBuffer(_buffer: any): Promise<any> {
    return undefined;
}

export async function fileTypeFromBlob(_blob: any): Promise<any> {
    return undefined;
}

export async function fileTypeFromStream(_stream: any): Promise<any> {
    return undefined;
}

export async function fileTypeFromTokenizer(_tokenizer: any): Promise<any> {
    return undefined;
}

export const supportedExtensions = new Set<string>();
export const supportedMimeTypes = new Set<string>();

export default {
    fileTypeFromFile,
    fileTypeFromBuffer,
    fileTypeFromBlob,
    fileTypeFromStream,
    fileTypeFromTokenizer,
    supportedExtensions,
    supportedMimeTypes,
};
