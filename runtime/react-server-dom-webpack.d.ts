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
}
