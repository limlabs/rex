# Rex

A Rust-native reimplementation of the Next.js Pages Router. The server, router, build pipeline, and SSR engine are all Rust. You write React — `.tsx` pages with `getServerSideProps`, `_app`, `_document` — exactly like Next.js.

## Getting Started

### Prerequisites

- **Rust** (1.75+): https://rustup.rs
- **Node.js** (18+): needed for `react` and `react-dom` in your project's `node_modules`

### Install

```sh
git clone https://github.com/your-org/rex.git
cd rex
cargo build --release
```

The binary is at `target/release/rex`. Add it to your PATH or use `cargo run --` to invoke it.

### Create a project

```sh
mkdir my-app && cd my-app
npm init -y
npm install react react-dom
```

Create a `pages/` directory with your first page:

```sh
mkdir pages
```

**pages/index.tsx**
```tsx
import React from 'react';

export default function Home({ message }: { message: string }) {
  return (
    <div>
      <h1>Rex</h1>
      <p>{message}</p>
    </div>
  );
}

export function getServerSideProps() {
  return {
    props: {
      message: 'Hello from Rex!',
    },
  };
}
```

### Run the dev server

```sh
rex dev
```

Open http://localhost:3000. You'll see server-rendered HTML with your React component.

### Add more pages

**pages/about.tsx**
```tsx
import React from 'react';

export default function About() {
  return <h1>About</h1>;
}
```

Visit http://localhost:3000/about.

### Dynamic routes

File-based routing works the same as Next.js:

**pages/blog/[slug].tsx**
```tsx
import React from 'react';

export default function Post({ slug, title }: { slug: string; title: string }) {
  return (
    <div>
      <h1>{title}</h1>
      <p>Slug: {slug}</p>
    </div>
  );
}

export function getServerSideProps(context: { params: { slug: string } }) {
  return {
    props: {
      slug: context.params.slug,
      title: `Post: ${context.params.slug}`,
    },
  };
}
```

Visit http://localhost:3000/blog/hello-world.

### Route patterns

| File | URL |
|------|-----|
| `pages/index.tsx` | `/` |
| `pages/about.tsx` | `/about` |
| `pages/blog/index.tsx` | `/blog` |
| `pages/blog/[slug].tsx` | `/blog/:slug` |
| `pages/docs/[...path].tsx` | `/docs/*` |

### Production build

```sh
rex build
rex start
```

`rex build` compiles server and client bundles to `.rex/build/`. `rex start` serves them without file watching or HMR.

## CLI

```
rex dev [--port 3000] [--root .]    Start dev server with HMR
rex build [--root .]                Production build
rex start [--port 3000] [--root .]  Serve production build
```

Set `RUST_LOG=rex=debug` for verbose logging.

## How it works

1. **Route scanning** — walks `pages/` and builds a trie for URL matching (static > dynamic > catch-all priority)
2. **SWC transforms** — strips TypeScript, transforms JSX (automatic runtime), strips `getServerSideProps` from client bundles
3. **V8 SSR** — a pool of V8 isolates (one per thread) evaluates the server bundle, calls `getServerSideProps`, then `renderToString`
4. **Axum serving** — assembles the HTML document with SSR output, props JSON, and client script tags
5. **HMR** — file watcher triggers rebuild, V8 isolates reload, WebSocket pushes updates to the browser

## Architecture

```
rex/
  crates/
    rex_cli/        CLI binary (dev, build, start)
    rex_core/       Shared types and config
    rex_router/     File-system scanner + trie matcher
    rex_build/      SWC transforms + bundler
    rex_v8/         V8 isolate pool + SSR engine
    rex_server/     Axum HTTP server + document assembly
    rex_dev/        File watcher + HMR WebSocket
  runtime/          JS templates (HMR client, server/client entries)
  packages/rex/     npm package (rex/document, rex/link, rex/router)
```

## Supported features

- [x] File-based routing (`pages/`)
- [x] Server-side rendering via V8
- [x] `getServerSideProps` with params, query, headers
- [x] `getServerSideProps` redirect and notFound
- [x] Dynamic routes (`[slug]`, `[...slug]`, `[[...slug]]`)
- [x] `_app` wrapper component
- [x] TypeScript / TSX
- [x] Dev server with HMR
- [x] Client hydration
- [x] Data endpoint for client-side navigation (`/_rex/data/`)

## Not yet implemented

- Static generation (SSG / ISR)
- API routes (`pages/api/`)
- CSS / CSS Modules
- Image optimization
- Middleware
- `_document` custom rendering
- `next/head` equivalent

## License

MIT
