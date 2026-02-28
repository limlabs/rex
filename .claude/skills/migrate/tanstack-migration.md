# TanStack Start to Rex Migration

**Difficulty: Medium** — TanStack Start has file-based routing and loaders, but with different conventions.

## Route File Mapping

| TanStack Start | Rex | Notes |
|---------------|-----|-------|
| `src/routes/index.tsx` | `pages/index.tsx` | |
| `src/routes/about.tsx` | `pages/about.tsx` | |
| `src/routes/blog/$slug.tsx` | `pages/blog/[slug].tsx` | `$slug` becomes `[slug]` |
| `src/routes/blog/$.tsx` | `pages/blog/[...path].tsx` | Splat route |
| `src/routes/__root.tsx` | `pages/_app.tsx` | Root layout |
| `src/routes/_layout.tsx` | (flatten into `_app.tsx`) | Nested layouts |

## Dynamic Segment Syntax

```
TanStack: $param      Rex: [param]
TanStack: $            Rex: [...path]
TanStack: ($optional)  Rex: [[...optional]]
```

## Loader to getServerSideProps

```tsx
// Before: TanStack loader
import { createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/blog/$slug')({
  loader: async ({ params }) => {
    const post = await fetchPost(params.slug);
    return { post };
  },
  component: BlogPost,
});

function BlogPost() {
  const { post } = Route.useLoaderData();
  return <h1>{post.title}</h1>;
}

// After: Rex getServerSideProps
export default function BlogPost({ post }) {
  return <h1>{post.title}</h1>;
}

export async function getServerSideProps(context) {
  const post = await fetchPost(context.params.slug);
  return { props: { post } };
}
```

## Root Layout

```tsx
// Before: __root.tsx
import { Outlet, createRootRoute } from '@tanstack/react-router';

export const Route = createRootRoute({
  component: () => (
    <div>
      <nav>...</nav>
      <Outlet />
    </div>
  ),
});

// After: pages/_app.tsx
export default function App({ Component, pageProps }) {
  return (
    <div>
      <nav>...</nav>
      <Component {...pageProps} />
    </div>
  );
}
```

## Search Params

```tsx
// Before: TanStack
const { search } = Route.useSearch();

// After: Rex (via getServerSideProps context or client-side)
export async function getServerSideProps(context) {
  const { query } = context; // search params as object
  return { props: { query } };
}
```

## Navigation

```tsx
// Before: TanStack
import { Link, useNavigate } from '@tanstack/react-router';
<Link to="/about">About</Link>

// After: Rex
import Link from 'rex/link';
<Link href="/about">About</Link>
```

Note: Rex's `Link` uses `href` instead of `to`.

## Config Migration

TanStack Start typically uses Vite under the hood. Migrate `vite.config.ts` aliases the same way as the Vite migration guide — either via `tsconfig.json` paths or `rex.config.json` `build.alias`.

## What to Remove

- `src/routes/` directory (after migrating to `pages/`)
- `src/routeTree.gen.ts` (auto-generated route tree)
- `vite.config.ts`
- `app.config.ts` (TanStack Start config)
- `@tanstack/react-router`, `@tanstack/start`, `@tanstack/react-router-devtools` from dependencies
- `vinxi` from dependencies
- `tsr.config.json` (TanStack Router config)
