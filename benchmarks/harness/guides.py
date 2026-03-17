"""
CLAUDE.md content for each condition. Extracted from sdk_runner.py to stay
under the 700-line file limit.
"""

REX_GUIDED = """\
# Rex Project

This is a Rex project — a Rust-native React framework with file-based routing.

## Quick Reference

- Pages go in `pages/` (e.g. `pages/about.tsx`, `pages/blog/[slug].tsx`)
- API routes go in `pages/api/` (e.g. `pages/api/hello.ts`)
- All pages must `import React from "react"` and export a default component
- Server-side data fetching uses `getServerSideProps(context)`:
  - `context.params` — dynamic route params
  - `context.query` — query string
  - Must return `{ props: { ... } }`
- The component receives props from getServerSideProps as its props argument

## Example Page

```tsx
import React from "react";

export default function AboutPage() {
  return <div><h1>About</h1></div>;
}
```

## Example with Data Fetching

```tsx
import React from "react";

export default function UserPage({ name }: { name: string }) {
  return <h1>Hello {name}</h1>;
}

export async function getServerSideProps(context: any) {
  return { props: { name: context.params.slug } };
}
```

## Example API Route

```ts
export default function handler(req: any, res: any) {
  res.status(200).json({ message: "hello" });
}
```
"""

REX_RAW = """\
# Rex Project

This is a Rex project. Pages go in `pages/`, API routes in `pages/api/`.
"""

REX_MCP = """\
# Rex Project

This is a Rex project — a Rust-native React framework with file-based routing.

## Quick Reference

- Pages go in `pages/` (e.g. `pages/about.tsx`, `pages/blog/[slug].tsx`)
- API routes go in `pages/api/` (e.g. `pages/api/hello.ts`)
- All pages must `import React from "react"` and export a default component
- Server-side data fetching uses `getServerSideProps(context)`:
  - `context.params` — dynamic route params
  - Must return `{ props: { ... } }`

## MCP Tools Available

You have two Rex-specific tools — USE THEM:

- **rex_check**: Build the project and get structured pass/fail with errors.
  Call this after creating or editing any page file.
- **rex_status**: See what pages exist and what routes they map to.
  Call this before starting work to orient yourself.

## Workflow

1. Call `rex_status` to see current project state
2. Create/edit page files
3. Call `rex_check` to verify the build passes
4. If check fails, fix the errors and check again

## Example Page

```tsx
import React from "react";

export default function AboutPage() {
  return <div><h1>About</h1></div>;
}
```

## Example with Data Fetching

```tsx
import React from "react";

export default function UserPage({ name }: { name: string }) {
  return <h1>Hello {name}</h1>;
}

export async function getServerSideProps(context: any) {
  return { props: { name: context.params.slug } };
}
```

## Example API Route

```ts
export default function handler(req: any, res: any) {
  res.status(200).json({ message: "hello" });
}
```
"""

NEXTJS_GUIDED = """\
# Next.js Project (Pages Router)

This is a Next.js 16 project using the Pages Router. Do NOT use the App Router.

## Quick Reference

- Pages go in `pages/` (e.g. `pages/about.tsx`, `pages/blog/[slug].tsx`)
- API routes go in `pages/api/` (e.g. `pages/api/hello.ts`)
- All pages export a default React component
- Server-side data fetching uses `getServerSideProps(context)`:
  - `context.params` — dynamic route params
  - `context.query` — query string
  - Must return `{ props: { ... } }`

## Example Page

```tsx
export default function AboutPage() {
  return <div><h1>About</h1></div>;
}
```

## Example with Data Fetching

```tsx
export default function UserPage({ name }: { name: string }) {
  return <h1>Hello {name}</h1>;
}

export async function getServerSideProps(context: any) {
  return { props: { name: context.params.slug } };
}
```

## Example API Route

```ts
// pages/api/hello.ts
export default function handler(req: any, res: any) {
  res.status(200).json({ message: "hello" });
}
```

## Build & Run

```bash
npx next build
npx next start --port 3000
```
"""

TANSTACK_GUIDED = """\
# TanStack Start Project

TanStack Start is a full-stack React framework built on TanStack Router and Vite.

> **CRITICAL**: TanStack Start is NOT Next.js. Do not use getServerSideProps,
> "use server" directives, or any Next.js/Remix patterns.

## Quick Reference

- Routes go in `src/routes/` using file-based routing
- Each route file exports `Route = createFileRoute('/path')({...})`
- Dynamic segments use `$` prefix: `src/routes/blog/$slug.tsx`
- The `createFileRoute` path string MUST match the file path
- Root route (`__root.tsx`) and router (`router.tsx`) are already set up
- After creating route files, run `npx tsr generate` to regenerate the route tree

## Example Page

```tsx
// src/routes/about.tsx
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/about')({
  component: AboutPage,
})

function AboutPage() {
  return <div><h1>About</h1></div>
}
```

## Example with Data Loading

```tsx
// src/routes/blog/$slug.tsx
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const getPost = createServerFn({ method: 'GET' })
  .validator((slug: string) => slug)
  .handler(async ({ input: slug }) => {
    return { title: slug, body: 'Post content' }
  })

export const Route = createFileRoute('/blog/$slug')({
  loader: ({ params }) => getPost({ data: params.slug }),
  component: BlogPost,
})

function BlogPost() {
  const post = Route.useLoaderData()
  return <div><h1>{post.title}</h1><p>{post.body}</p></div>
}
```

## Example API Route

```tsx
// src/routes/api/hello.tsx
import { createAPIFileRoute } from '@tanstack/react-start/api'
import { json } from '@tanstack/react-start'

export const APIRoute = createAPIFileRoute('/api/hello')({
  GET: async () => {
    return json({ message: 'hello' })
  },
})
```

## Build & Run

```bash
npx tsr generate   # regenerate route tree after adding routes
npx vite build
npx vite preview --port 3000 --host 127.0.0.1
```
"""

REMIX_GUIDED = """\
# React Router v7 (Remix) Project — Framework Mode

This project uses React Router v7 in framework mode with file-based routing.

## Quick Reference

- Routes go in `app/routes/` with flat file convention
- Index route: `app/routes/_index.tsx`
- Named routes: `app/routes/about.tsx` -> `/about`
- Dynamic segments: `app/routes/blog.$slug.tsx` -> `/blog/:slug`
- Nested paths use dots: `app/routes/dashboard.settings.tsx` -> `/dashboard/settings`
- Root layout (`app/root.tsx`) is already set up

## Example Page

```tsx
// app/routes/about.tsx
export default function AboutPage() {
  return <div><h1>About</h1></div>;
}
```

## Example with Data Loading

```tsx
// app/routes/blog.$slug.tsx
import type { Route } from "./+types/blog.$slug";
import { useLoaderData } from "react-router";

export async function loader({ params }: Route.LoaderArgs) {
  return { title: params.slug, body: "Post content" };
}

export default function BlogPost() {
  const { title, body } = useLoaderData<typeof loader>();
  return <div><h1>{title}</h1><p>{body}</p></div>;
}
```

## Example API-style Route (Resource Route)

```tsx
// app/routes/api.hello.tsx
export async function loader() {
  return Response.json({ message: "hello" });
}
```

## Build & Run

```bash
npx react-router build
npx react-router-serve ./build/server/index.js
```
"""
