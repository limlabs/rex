# Rex

A Rust-native reimplementation of Next.js. The server, router, build pipeline, and SSR engine are all Rust — you write React pages with `getServerSideProps` exactly like Next.js, but requests are handled by Axum, rendered in V8, and bundled by Rolldown.

## Quick Start

```sh
npx @limlabs/rex init my-app
cd my-app
npm install
npx rex dev
```

Open http://localhost:3000.

## Install

### npm (recommended)

```sh
npm install -D @limlabs/rex
```

This installs the `rex` binary for your platform (macOS arm64/x64, Linux x64/arm64).

### From source

```sh
git clone https://github.com/limlabs/rex.git
cd rex
cargo build --release
# Binary at target/release/rex
```

### Docker

```sh
docker build -t rex .
docker run -v $(pwd):/app -w /app -p 3000:3000 rex
```

## Pages & Routing

Rex uses file-system routing from the `pages/` directory, identical to Next.js:

| File | URL |
|------|-----|
| `pages/index.tsx` | `/` |
| `pages/about.tsx` | `/about` |
| `pages/blog/index.tsx` | `/blog` |
| `pages/blog/[slug].tsx` | `/blog/:slug` |
| `pages/docs/[...path].tsx` | `/docs/*` (catch-all) |
| `pages/docs/[[...path]].tsx` | `/docs/*` (optional catch-all) |

### Basic page

```tsx
export default function Home({ message }: { message: string }) {
  return <h1>{message}</h1>;
}

export function getServerSideProps() {
  return {
    props: { message: "Hello from Rex!" },
  };
}
```

### Dynamic routes

```tsx
export default function Post({ slug }: { slug: string }) {
  return <h1>Post: {slug}</h1>;
}

export function getServerSideProps(context: { params: { slug: string } }) {
  return {
    props: { slug: context.params.slug },
  };
}
```

### getServerSideProps

Runs on every request. Receives `context` with `params`, `query`, `headers`, and `cookies`. Can return:

- **props** — `{ props: { ... } }` passed to the page component
- **redirect** — `{ redirect: { destination: "/login", permanent: false } }`
- **notFound** — `{ notFound: true }` renders a 404

Supports `async` for data fetching with the built-in `fetch` API.

### API routes

Files in `pages/api/` export a request handler instead of a React component:

```ts
// pages/api/hello.ts
export default function handler(req, res) {
  res.json({ message: "Hello" });
}
```

## Features

### Custom `_app`

Wrap all pages with a shared layout:

```tsx
// pages/_app.tsx
export default function App({ Component, pageProps }) {
  return (
    <div className="layout">
      <Component {...pageProps} />
    </div>
  );
}
```

### Custom `_document`

Control the HTML shell:

```tsx
// pages/_document.tsx
import { Html, Head, Main, Script } from "@limlabs/rex/document";

export default function Document() {
  return (
    <Html lang="en">
      <Head />
      <body>
        <Main />
        <Script />
      </body>
    </Html>
  );
}
```

### Head

Set `<head>` tags per page:

```tsx
import Head from "@limlabs/rex/head";

export default function About() {
  return (
    <>
      <Head>
        <title>About</title>
        <meta name="description" content="About page" />
      </Head>
      <h1>About</h1>
    </>
  );
}
```

### Link

Client-side navigation with prefetching:

```tsx
import Link from "@limlabs/rex/link";

export default function Nav() {
  return <Link href="/about">About</Link>;
}
```

### Image

Optimized image component with blur placeholders and format negotiation:

```tsx
import Image from "@limlabs/rex/image";

export default function Hero() {
  return <Image src="/hero.jpg" width={800} height={400} alt="Hero" />;
}
```

### CSS Modules

`.module.css` files are scoped automatically:

```tsx
import styles from "./button.module.css";

export default function Button() {
  return <button className={styles.primary}>Click</button>;
}
```

Class names are scoped to `{File}_{class}_{hash}` to prevent collisions.

### Tailwind CSS

Rex automatically detects Tailwind and processes it during builds. Install `@tailwindcss/cli` and add your config — no extra setup needed.

### MDX

Write pages in MDX with full component support:

```mdx
// pages/docs/intro.mdx
import { Chart } from "../components/Chart";

# Introduction

Here's an interactive chart:

<Chart data={[1, 2, 3]} />
```

### Google Fonts

Rex optimizes Google Fonts automatically — downloading font files at build time, generating `@font-face` CSS, and adding preload hints.

### Middleware

Add a `middleware.ts` at the project root to intercept requests:

```ts
// middleware.ts
import { NextRequest, NextResponse } from "@limlabs/rex/middleware";

export function middleware(request: NextRequest) {
  if (!request.cookies.get("session")) {
    return NextResponse.redirect("/login");
  }
  return NextResponse.next();
}
```

### Server Actions

Use `"use server"` directives for form handling and mutations:

```tsx
async function submitForm(formData: FormData) {
  "use server";
  await saveToDatabase(formData.get("email"));
}

export default function SignUp() {
  return (
    <form action={submitForm}>
      <input name="email" type="email" />
      <button type="submit">Sign Up</button>
    </form>
  );
}
```

### Client-Side Router

The built-in router supports `push`, `replace`, and `prefetch`:

```tsx
import { useRouter } from "@limlabs/rex/router";

export default function Page() {
  const router = useRouter();
  return <button onClick={() => router.push("/dashboard")}>Go</button>;
}
```

Client navigations fetch data via `/_rex/data/{buildId}/{path}.json` without full page reloads.

## Configuration

Create `rex.config.json` (or `rex.config.toml`) in your project root:

```json
{
  "redirects": [
    {
      "source": "/old-blog/:slug",
      "destination": "/blog/:slug",
      "permanent": true
    }
  ],
  "rewrites": [
    {
      "source": "/api/:path*",
      "destination": "/api/v2/:path*"
    }
  ],
  "headers": [
    {
      "source": "/(.*)",
      "headers": [
        { "key": "X-Frame-Options", "value": "DENY" }
      ]
    }
  ],
  "build": {
    "alias": {
      "@components": "./src/components"
    },
    "sourcemap": true
  }
}
```

## CLI Reference

```
rex init <name>                    Create a new Rex project
rex dev  [--port 3000] [--root .]  Dev server with HMR
         [-H host] [--no-tui]
rex build [--root .]               Production build
rex start [--port 3000] [--root .] Serve production build
          [-H host]
rex lint  [--root .] [--fix]       Lint with oxlint
          [--deny-warnings]
rex typecheck [--root .]           Type-check with tsc
rex fmt [--root .] [--check]       Format with oxfmt
```

All port/host flags also read `$PORT` and `$HOST` environment variables.

Set `RUST_LOG=rex=debug` for verbose logging.

## How It Works

1. **Route scanning** — walks `pages/` and builds a trie for URL matching (static > dynamic > catch-all priority)
2. **Rolldown bundling** — OXC parses TSX/JSX and strips TypeScript; Rolldown produces an IIFE server bundle and ESM client bundles with code splitting
3. **V8 SSR** — a pool of V8 isolates (one per thread) evaluates the server bundle, calls `getServerSideProps`, then `renderToString`
4. **Axum serving** — assembles the HTML document with SSR markup, props JSON, and `<script type="module">` tags
5. **Client hydration** — React hydrates the server-rendered HTML; the client-side router takes over navigation
6. **HMR** — file watcher triggers incremental rebuilds, V8 isolates reload, WebSocket pushes full-page refresh to the browser

## Architecture

```
rex/
├── crates/
│   ├── rex_cli/      CLI (dev, build, start, lint, fmt, typecheck, init)
│   ├── rex_core/     Shared types and config
│   ├── rex_router/   File-system scanner + trie matcher
│   ├── rex_build/    Rolldown bundler (server IIFE + client ESM)
│   ├── rex_v8/       V8 isolate pool + SSR engine
│   ├── rex_server/   Axum HTTP server + document assembly
│   ├── rex_dev/      File watcher + HMR WebSocket
│   ├── rex_image/    Image optimization + blur placeholders
│   ├── rex_mdx/      MDX compiler
│   ├── rex_napi/     Node.js N-API bindings
│   ├── rex_python/   Python bindings (PyO3)
│   └── rex_e2e/      End-to-end tests
├── runtime/          JS runtime (SSR, hydration, router, HMR client)
└── packages/rex/     npm package (@limlabs/rex)
```

## Deployment

### Docker

```dockerfile
FROM node:20-slim AS deps
WORKDIR /app
COPY package*.json ./
RUN npm ci

FROM ghcr.io/limlabs/rex:latest
WORKDIR /app
COPY --from=deps /app/node_modules ./node_modules
COPY . .
RUN rex build
CMD ["start"]
```

### Railway

Rex includes a Railway deployment config. See `fixtures/railway/` for an example with `railway.toml`.

### Any server

```sh
rex build
rex start --port 8080
```

The production server binds to `0.0.0.0` by default for container compatibility.

## Contributing

```sh
git clone https://github.com/limlabs/rex.git
cd rex
cargo build
cargo test

# Run dev server against the test fixture
cd fixtures/basic && npm install && cd ../..
cargo run -- dev --root fixtures/basic
```

## License

[MIT](LICENSE) — Lim Labs
