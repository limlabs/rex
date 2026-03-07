// Type declarations for react-server-dom-webpack (used by RSC runtime)

declare module "react-server-dom-webpack/client" {
  import type { ReactElement } from "react";

  interface SsrManifest {
    moduleMap: Record<string, unknown>;
    moduleLoading: null;
  }

  interface ClientOptions {
    ssrManifest: SsrManifest;
    callServer?: (id: string, args: unknown[]) => Promise<unknown>;
  }

  export function createFromReadableStream(
    stream: ReadableStream<Uint8Array>,
    options: ClientOptions,
  ): PromiseLike<ReactElement>;

  export function createFromFetch(
    fetchPromise: Promise<Response>,
    options: ClientOptions,
  ): PromiseLike<ReactElement>;

  export function createServerReference(
    id: string,
    callServer: (...args: unknown[]) => Promise<unknown>,
  ): (...args: unknown[]) => Promise<unknown>;

  export function encodeReply(
    value: unknown,
    options?: { signal?: AbortSignal; temporaryReferences?: unknown },
  ): Promise<string | FormData>;
}

declare module "react-server-dom-webpack/server" {
  export function renderToReadableStream(
    model: unknown,
    webpackMap: Record<string, unknown>,
    options?: Record<string, unknown>,
  ): ReadableStream<Uint8Array>;

  export function registerServerReference(
    fn: Function,
    id: string,
    name: string,
  ): Function;

  export function decodeReply(
    body: string | FormData,
    webpackMap: Record<string, unknown>,
    options?: { temporaryReferences?: unknown; arraySizeLimit?: number },
  ): PromiseLike<unknown[]>;

  export function decodeAction(
    body: FormData,
    serverManifest: Record<string, unknown>,
  ): Promise<(() => unknown) | null>;

  export function decodeFormState(
    actionResult: unknown,
    body: FormData,
    serverManifest: Record<string, unknown>,
  ): Promise<unknown>;
}
