// Node.js `worker_threads` stub for Rex server bundles.
// V8 isolates are single-threaded — no worker support.

/* eslint-disable @typescript-eslint/no-explicit-any */

export const isMainThread = true;
export const parentPort = null;
export const workerData = null;
export const threadId = 0;

export class Worker {
    constructor(_filename: string, _opts?: any) {
        throw new Error('worker_threads.Worker is not supported in Rex V8 runtime');
    }
}

export class MessageChannel {
    port1 = { postMessage() {}, on() {}, close() {} };
    port2 = { postMessage() {}, on() {}, close() {} };
}

const worker_threads = {
    isMainThread, parentPort, workerData, threadId, Worker, MessageChannel,
};
export default worker_threads;
