# Next.js Pages Router to Rex Migration

**Difficulty: Low** — Rex is API-compatible with the Pages Router pattern.

## File Structure

The `pages/` directory structure is identical. No renaming needed:

| Next.js | Rex | Notes |
|---------|-----|-------|
| `pages/index.tsx` | `pages/index.tsx` | Same |
| `pages/about.tsx` | `pages/about.tsx` | Same |
| `pages/blog/[slug].tsx` | `pages/blog/[slug].tsx` | Same |
| `pages/[...path].tsx` | `pages/[...path].tsx` | Same |
| `pages/_app.tsx` | `pages/_app.tsx` | Same |
| `pages/_document.tsx` | `pages/_document.tsx` | Same |
| `pages/_error.tsx` | `pages/_error.tsx` | Same |
| `pages/404.tsx` | `pages/404.tsx` | Same |
| `pages/api/*.ts` | `pages/api/*.ts` | Same |

## Imports

Rex provides compatibility shims — `next/head`, `next/link`, `next/router` all resolve automatically. You can optionally update them:

```tsx
// Both work:
import Head from 'next/head';   // compat shim
import Head from 'rex/head';    // native
```

## Data Fetching

`getServerSideProps` works identically:

```tsx
export async function getServerSideProps(context) {
  // context.params, context.query, context.req — all same
  return { props: { ... } };
}
```

`getStaticProps` is also supported with the same API.

**Not supported:** `getStaticPaths` (Rex doesn't do static generation at build time).

## Config Migration

Convert `next.config.js` to `rex.config.json`:

```js
// next.config.js
module.exports = {
  async redirects() {
    return [{ source: '/old', destination: '/new', permanent: true }];
  },
  async rewrites() {
    return [{ source: '/api/:path*', destination: '/api/v2/:path' }];
  },
};
```

Becomes:

```json
{
  "redirects": [
    { "source": "/old", "destination": "/new", "permanent": true }
  ],
  "rewrites": [
    { "source": "/api/:path", "destination": "/api/v2/:path" }
  ]
}
```

## package.json Scripts

```json
{
  "scripts": {
    "dev": "rex dev",
    "build": "rex build",
    "start": "rex start"
  }
}
```

## What to Remove

- `next.config.js` / `next.config.mjs` (after migrating to `rex.config.json`)
- `next-env.d.ts`
- `.next/` directory
- `next` from dependencies

## Known Differences

- No `next/image` optimization — use `<img>` tags or `rex/image` (basic wrapper)
- No middleware support
- No ISR — use `getServerSideProps` for dynamic data
- No `next/font` — use standard CSS `@font-face` or Google Fonts CDN
- API routes run in V8, not Node.js — no `fs`, `path`, etc. available in API handlers
