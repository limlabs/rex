import { createRex } from "@limlabs/rex/server";

const rex = await createRex({ root: import.meta.dirname });
const handle = rex.getRequestHandler();

const server = Bun.serve({
  port: 3000,
  async fetch(req) {
    const url = new URL(req.url);

    // Example: add a custom health check route outside of Rex
    if (url.pathname === "/healthz") {
      return new Response("ok");
    }

    // Let Rex handle everything else (pages, API routes, static assets)
    return handle(req);
  },
});

console.log(`Custom server listening on http://localhost:${server.port}`);
