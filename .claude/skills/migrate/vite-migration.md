# Vite + React to Rex Migration

**Difficulty: Medium** — Vite SPAs don't have file-based routing or SSR data fetching, so these must be added.

## Key Differences

| Vite | Rex |
|------|-----|
| SPA with client-side routing | SSR with file-based routing |
| `index.html` entry point | No HTML entry — Rex generates it |
| `main.tsx` / `App.tsx` root | `pages/` directory |
| React Router / manual routes | File-system routes |
| Client-side data fetching | `getServerSideProps` (server-side) |
| `vite.config.ts` | `rex.config.json` |

## Step 1: Create Pages Directory

Convert your route components into the `pages/` directory:

```
# Before (Vite + React Router)
src/
  App.tsx
  main.tsx
  pages/
    Home.tsx
    About.tsx
    Blog.tsx
    BlogPost.tsx

# After (Rex)
pages/
  index.tsx        # Home
  about.tsx        # About
  blog/
    index.tsx      # Blog list
    [slug].tsx     # BlogPost
```

## Step 2: Remove SPA Entry Points

Delete these Vite-specific files:
- `index.html`
- `src/main.tsx` (or `src/main.jsx`)
- `src/App.tsx` (router setup)

## Step 3: Convert Routing

```tsx
// Before: React Router in App.tsx
<Routes>
  <Route path="/" element={<Home />} />
  <Route path="/about" element={<About />} />
  <Route path="/blog/:slug" element={<BlogPost />} />
</Routes>

// After: File-system routing (just create the files)
// pages/index.tsx — renders at /
// pages/about.tsx — renders at /about
// pages/blog/[slug].tsx — renders at /blog/:slug
```

## Step 4: Add SSR Data Fetching

Convert client-side data fetching (useEffect, React Query, SWR) to `getServerSideProps`:

```tsx
// Before: Vite client-side fetching
function BlogPost() {
  const { slug } = useParams();
  const [post, setPost] = useState(null);
  useEffect(() => {
    fetch(`/api/posts/${slug}`).then(r => r.json()).then(setPost);
  }, [slug]);
  return post ? <div>{post.title}</div> : <div>Loading...</div>;
}

// After: Rex server-side fetching
export default function BlogPost({ post }) {
  return <div>{post.title}</div>;
}

export async function getServerSideProps(context) {
  const res = await fetch(`https://api.example.com/posts/${context.params.slug}`);
  const post = await res.json();
  return { props: { post } };
}
```

## Step 5: Migrate Config

Convert `vite.config.ts` aliases to `rex.config.json` or `tsconfig.json`:

```ts
// vite.config.ts (before)
export default defineConfig({
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
      '@components': path.resolve(__dirname, './src/components'),
    },
  },
});
```

Option A — tsconfig.json (recommended):
```json
{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@/*": ["./src/*"],
      "@components/*": ["./src/components/*"]
    }
  }
}
```

Option B — rex.config.json:
```json
{
  "build": {
    "alias": {
      "@": "./src",
      "@components": "./src/components"
    }
  }
}
```

## Step 6: Shared Layout

If you have a shared layout component, create `pages/_app.tsx`:

```tsx
export default function App({ Component, pageProps }) {
  return (
    <div className="layout">
      <nav>{/* shared nav */}</nav>
      <Component {...pageProps} />
    </div>
  );
}
```

## What to Remove

- `vite.config.ts` / `vite.config.js`
- `index.html`
- `src/main.tsx`
- `vite` and `@vitejs/plugin-react` from devDependencies
- React Router packages (`react-router-dom`, etc.)
