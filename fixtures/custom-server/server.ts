import { createServer } from "node:http";
import { createRex } from "@limlabs/rex/server";

const rex = await createRex({ root: import.meta.dirname! });
const handle = rex.getRequestHandler();

const server = createServer(async (req, res) => {
  const url = new URL(req.url!, `http://${req.headers.host}`);

  // Example: add a custom health check route outside of Rex
  if (url.pathname === "/healthz") {
    res.writeHead(200);
    res.end("ok");
    return;
  }

  // Convert Node request to Web Request, let Rex handle it
  const webReq = new Request(url, {
    method: req.method,
    headers: req.headers as Record<string, string>,
  });

  const webRes = await handle(webReq);

  res.writeHead(webRes.status, Object.fromEntries(webRes.headers));
  res.end(await webRes.text());
});

server.listen(3000, () => {
  console.log("Custom server listening on http://localhost:3000");
});
