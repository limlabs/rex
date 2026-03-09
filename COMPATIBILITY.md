# Compatibility Matrix

Feature-by-feature comparison of Rex, Next.js, and [Vinext](https://github.com/cloudflare/vinext).

**Legend:** Yes = supported, Partial = partially implemented, No = not yet implemented

## Pages Router

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| File-system routing (`pages/`) | Yes | Yes | Yes |
| `getServerSideProps` | Yes | Yes | Yes |
| `getStaticProps` | Partial | Yes | Yes |
| `getStaticPaths` | No | Yes | Yes |
| Incremental Static Regeneration (ISR) | No | Yes | Yes |
| Dynamic routes (`[slug]`) | Yes | Yes | Yes |
| Catch-all routes (`[...slug]`) | Yes | Yes | Yes |
| Optional catch-all (`[[...slug]]`) | Yes | Yes | Yes |
| API routes (`pages/api/`) | Yes | Yes | Yes |
| Custom `_app` | Yes | Yes | Yes |
| Custom `_document` | Yes | Yes | Yes |
| Custom `_error` | Yes | Yes | Yes |
| Client-side navigation | Yes | Yes | Yes |

## App Router

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| Nested layouts (`layout.tsx`) | Yes | Yes | Yes |
| React Server Components | Yes | Yes | Yes |
| `"use client"` boundary | Yes | Yes | Yes |
| Streaming SSR / Suspense | Yes | Yes | Yes |
| `loading.tsx` | Yes | Yes | Yes |
| `error.tsx` | Yes | Yes | Yes |
| `not-found.tsx` | Yes | Yes | Yes |
| Route groups `(group)` | Yes | Yes | Yes |
| `generateMetadata` | Yes | Yes | Yes |
| Server Actions (`"use server"`) | Yes | Yes | Yes |
| Route handlers (`route.ts`) | No | Yes | Yes |
| Parallel routes (`@folder`) | No | Yes | Yes |
| Intercepting routes (`(.)folder`) | No | Yes | Yes |

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
| `<Image>` (optimization + blur placeholder) | Yes | Yes | Partial |
| `<Head>` (per-page head tags) | Yes | Yes | Yes |
| `<Script>` (third-party scripts) | No | Yes | Yes |

## Data Fetching

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| `getServerSideProps` | Yes | Yes | Yes |
| `getStaticProps` | Partial | Yes | Yes |
| `getStaticPaths` / `generateStaticParams` | No | Yes | Yes |
| Async Server Components (app router) | Yes | Yes | Yes |
| Server Actions | Yes | Yes | Yes |
| `fetch` in server context | Yes | Yes | Yes |

## Build & Tooling

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| TypeScript / TSX | Yes | Yes | Yes |
| MDX | Yes | Yes | Partial |
| Built-in linter | Yes (oxlint) | Yes (eslint config) | No |
| Built-in formatter | Yes (oxfmt) | No | No |
| Built-in type checker | Yes (tsc wrapper) | Yes | No |
| Google Fonts optimization | Yes (self-hosted) | Yes (self-hosted) | Partial (CDN) |
| Image optimization | Yes | Yes | Partial |
| Code splitting | Yes | Yes | Yes |
| Source maps | Yes | Yes | Yes |

## Server & Deployment

| Feature | Rex | Next.js | Vinext |
|---------|-----|---------|--------|
| Middleware | Yes | Yes | Yes |
| Redirects / rewrites / headers config | Yes | Yes | Yes |
| Docker | Yes | Yes | Yes |
| Static export (`output: 'export'`) | No | Yes | Yes |
| Edge runtime | No | Yes | Yes (Cloudflare Workers) |
| i18n routing | No | Yes | Yes |
| Custom server | Yes | Yes | Yes |

## Notes

- **Rex** is a Rust-native engine — Axum HTTP, V8 SSR, Rolldown bundler. The focus is on raw throughput and single-binary deployment.
- **Vinext** ([source](https://github.com/cloudflare/vinext)) reimplements ~94% of the Next.js API surface on Vite. It has native Cloudflare Workers support and multi-platform deployment via Nitro. Image optimization uses `@unpic/react` for remote images (no build-time resizing). Google Fonts are loaded from CDN rather than self-hosted. The project is experimental.
- **Rex** `getStaticProps` is marked "Partial" — the type is recognized and pages without `getServerSideProps` or dynamic segments are automatically statically optimized, but full ISR/revalidation is not yet implemented.
- **Parallel routes** and **intercepting routes** are planned for future Rex releases.
