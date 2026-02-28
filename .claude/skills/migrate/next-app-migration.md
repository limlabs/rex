# Next.js App Router to Rex Migration

**Difficulty: High** — The App Router uses fundamentally different patterns (RSC, layouts, server actions) that must be restructured.

## Directory Restructuring

The `app/` directory must be converted to `pages/`:

| App Router | Rex Pages Router |
|-----------|-----------------|
| `app/page.tsx` | `pages/index.tsx` |
| `app/about/page.tsx` | `pages/about.tsx` |
| `app/blog/[slug]/page.tsx` | `pages/blog/[slug].tsx` |
| `app/layout.tsx` | `pages/_app.tsx` |
| `app/not-found.tsx` | `pages/404.tsx` |
| `app/error.tsx` | `pages/_error.tsx` |
| `app/loading.tsx` | (remove — use component-level loading states) |
| `app/api/hello/route.ts` | `pages/api/hello.ts` |

## Layout to _app

Convert the root layout to `_app.tsx`:

```tsx
// app/layout.tsx (before)
export default function RootLayout({ children }) {
  return (
    <html>
      <body>{children}</body>
    </html>
  );
}

// pages/_app.tsx (after)
export default function App({ Component, pageProps }) {
  return <Component {...pageProps} />;
}
```

HTML/body wrapping is handled by Rex's document system. If you need custom `<head>` content, use `rex/head` in individual pages.

Nested layouts must be flattened or implemented as wrapper components.

## Server Components to Client Components

Rex doesn't use React Server Components. Remove all `'use client'` directives (everything is a client component by default). Convert server-only data fetching:

```tsx
// app/page.tsx (before — Server Component)
async function Page() {
  const data = await fetch('https://api.example.com/data');
  return <div>{data.title}</div>;
}

// pages/index.tsx (after)
export default function Page({ data }) {
  return <div>{data.title}</div>;
}

export async function getServerSideProps() {
  const res = await fetch('https://api.example.com/data');
  const data = await res.json();
  return { props: { data } };
}
```

## Server Actions

Server Actions must be converted to API routes:

```tsx
// app/page.tsx (before)
async function submitForm(formData: FormData) {
  'use server';
  await db.insert(formData);
}

// pages/api/submit.ts (after)
export default async function handler(req, res) {
  await db.insert(req.body);
  res.status(200).json({ ok: true });
}
```

Then call the API route from the client component.

## Route Handlers to API Routes

```tsx
// app/api/hello/route.ts (before)
export async function GET(request: Request) {
  return Response.json({ message: 'hello' });
}

// pages/api/hello.ts (after)
export default function handler(req, res) {
  res.status(200).json({ message: 'hello' });
}
```

## Metadata API

Replace the Metadata API with `rex/head`:

```tsx
// app/page.tsx (before)
export const metadata = { title: 'Home', description: '...' };

// pages/index.tsx (after)
import Head from 'rex/head';

export default function Page() {
  return (
    <>
      <Head>
        <title>Home</title>
        <meta name="description" content="..." />
      </Head>
      {/* page content */}
    </>
  );
}
```

## What to Remove

- `app/` directory (after migration)
- `'use client'` / `'use server'` directives
- `next.config.js` (after migrating to `rex.config.json`)
- `next` from dependencies
- `next-env.d.ts`, `.next/`

## Unsupported Patterns

- React Server Components — convert to client components + `getServerSideProps`
- `generateStaticParams` — not needed, Rex handles dynamic routes at request time
- Parallel routes (`@slot`) — not supported
- Intercepting routes (`(.)`, `(..)`) — not supported
- Route groups (`(group)`) — not supported, use flat file structure
- `loading.tsx` / streaming — not supported as a file convention
