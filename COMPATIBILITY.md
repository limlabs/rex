# Compatibility Matrix

Feature-by-feature comparison of Rex, Next.js, and Vinext.

**Legend:** Yes = supported, Partial = partially implemented, No = not yet implemented, N/A = not applicable to that framework

## Pages Router

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| File-system routing (`pages/`) | Yes | Yes | N/A |
| `getServerSideProps` | Yes | Yes | N/A |
| `getStaticProps` | Partial | Yes | N/A |
| `getStaticPaths` | No | Yes | N/A |
| Incremental Static Regeneration (ISR) | No | Yes | N/A |
| Dynamic routes (`[slug]`) | Yes | Yes | N/A |
| Catch-all routes (`[...slug]`) | Yes | Yes | N/A |
| Optional catch-all (`[[...slug]]`) | Yes | Yes | N/A |
| API routes (`pages/api/`) | Yes | Yes | N/A |
| Custom `_app` | Yes | Yes | N/A |
| Custom `_document` | Yes | Yes | N/A |
| Custom `_error` | Yes | Yes | N/A |
| Client-side navigation | Yes | Yes | N/A |

## App Router

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| Nested layouts (`layout.tsx`) | Yes | Yes | Yes |
| React Server Components | Yes | Yes | Yes |
| `"use client"` boundary | Yes | Yes | Yes |
| Streaming SSR / Suspense | Yes | Yes | Yes |
| `loading.tsx` | Yes | Yes | No |
| `error.tsx` | Yes | Yes | No |
| `not-found.tsx` | Yes | Yes | No |
| Route groups `(group)` | Yes | Yes | No |
| `generateMetadata` | Yes | Yes | No |
| Server Actions (`"use server"`) | Yes | Yes | No |
| Route handlers (`route.ts`) | No | Yes | Yes |
| Parallel routes (`@folder`) | No | Yes | No |
| Intercepting routes (`(.)folder`) | No | Yes | No |

## Styling

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| CSS Modules (`.module.css`) | Yes | Yes | Yes |
| Tailwind CSS | Yes | Yes | Yes |
| Global CSS | Yes | Yes | Yes |
| CSS-in-JS (styled-components, etc.) | No | Yes | Yes |
| Sass/SCSS | No | Yes | Yes |

## Built-in Components

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| `<Link>` (client-side navigation) | Yes | Yes | Yes |
| `<Image>` (optimization + blur placeholder) | Yes | Yes | No |
| `<Head>` (per-page head tags) | Yes | Yes | N/A |
| `<Script>` (third-party scripts) | No | Yes | No |

## Data Fetching

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| `getServerSideProps` | Yes | Yes | N/A |
| `getStaticProps` | Partial | Yes | N/A |
| `getStaticPaths` / `generateStaticParams` | No | Yes | N/A |
| Async Server Components (app router) | Yes | Yes | Yes |
| Server Actions | Yes | Yes | No |
| `fetch` in V8 / server context | Yes | Yes | Yes |

## Build & Tooling

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| TypeScript / TSX | Yes | Yes | Yes |
| MDX | Yes | Yes | No |
| Built-in linter | Yes (oxlint) | Yes (eslint config) | No |
| Built-in formatter | Yes (oxfmt) | No | No |
| Built-in type checker | Yes (tsc wrapper) | Yes | No |
| Google Fonts optimization | Yes | Yes | No |
| Image optimization | Yes | Yes | No |
| Code splitting | Yes | Yes | Yes |
| Source maps | Yes | Yes | Yes |

## Server & Deployment

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| Middleware | Yes | Yes | No |
| Redirects / rewrites / headers config | Yes | Yes | No |
| Docker | Yes | Yes | Yes |
| Static export (`output: 'export'`) | No | Yes | No |
| Edge runtime | No | Yes | No |
| i18n routing | No | Yes | No |
| Custom server | Yes | Yes | Yes |

## Notes

- **Vinext** is a Vite-based Next.js alternative focused on the App Router. It does not support the Pages Router.
- **Rex** `getStaticProps` is marked "Partial" — the type is recognized and pages without `getServerSideProps` or dynamic segments are automatically statically optimized, but full ISR/revalidation is not yet implemented.
- **Parallel routes** and **intercepting routes** are planned for future Rex releases.
