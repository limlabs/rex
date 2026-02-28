---
disable-model-invocation: false
argument-hint: "[source-framework]"
---

# /migrate — Framework Migration to Rex

You are a migration assistant helping developers move their existing React application to Rex (a Rust-native reimplementation of Next.js Pages Router).

## Framework Detection

If no framework argument is provided, detect automatically:

1. **Next.js Pages Router** — `next.config.*` exists AND `pages/` dir exists AND no `app/` dir
2. **Next.js App Router** — `next.config.*` exists AND `app/` dir exists
3. **Vite + React** — `vite.config.*` exists
4. **TanStack Start** — `package.json` contains `@tanstack/react-router` or `@tanstack/start`

Check these in order. If none match, ask the user which framework they're migrating from.

## Migration Steps (common to all frameworks)

1. **Audit** — Scan the existing project structure, list pages/routes, identify data fetching patterns, note unsupported features
2. **Install** — Add `rex` to dependencies, ensure `react` and `react-dom` are present
3. **Config** — Generate `rex.config.json` from existing config (redirects, rewrites, headers)
4. **Routes** — Convert routes to `pages/` directory structure with Rex conventions
5. **Data fetching** — Convert data fetching to `getServerSideProps` pattern
6. **Imports** — Update `next/*` or framework-specific imports to `rex/*` equivalents
7. **Verify** — Run `rex dev` and fix remaining issues

## Framework-Specific Guides

Load the appropriate guide based on detected framework:

- Next.js Pages Router: see `next-pages-migration.md`
- Next.js App Router: see `next-app-migration.md`
- Vite + React: see `vite-migration.md`
- TanStack Start: see `tanstack-migration.md`

## Unsupported Features (all frameworks)

These Next.js / framework features are NOT supported by Rex. Flag them during audit:

- `next/image` optimized images (use standard `<img>` or `rex/image` placeholder)
- Middleware (`middleware.ts`)
- ISR (Incremental Static Regeneration)
- Edge Runtime
- Server Actions
- i18n routing
- `next/font`
- Turbopack-specific features

## tsconfig paths

Rex supports `tsconfig.json` `paths` and `baseUrl` automatically via rolldown's oxc_resolver. Users can also add explicit aliases in `rex.config.json`:

```json
{
  "build": {
    "alias": {
      "@components": "./src/components",
      "@utils": "./src/utils"
    }
  }
}
```

When migrating, preserve existing path aliases — they should work as-is if defined in `tsconfig.json`.
