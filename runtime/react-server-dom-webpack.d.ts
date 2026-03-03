// Type declarations for react-server-dom-webpack (used by RSC runtime)

declare module "react-server-dom-webpack/client" {
  import type { ReactElement } from "react";

  interface SsrManifest {
    moduleMap: Record<string, unknown>;
    moduleLoading: null;
  }

  export function createFromReadableStream(
    stream: ReadableStream<Uint8Array>,
    options: { ssrManifest: SsrManifest },
  ): PromiseLike<ReactElement>;

  export function createFromFetch(
    fetchPromise: Promise<Response>,
    options: { ssrManifest: SsrManifest },
  ): PromiseLike<ReactElement>;
}
