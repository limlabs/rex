import React from "react";
import Link from "rex/link";

export const metadata = {
  title: "Rex — Rust-Native React Framework",
};

export default function Home() {
  return (
    <div>
      <div className="mb-10">
        <h1 className="text-4xl font-bold text-slate-900 mb-4 border-0 pb-0">
          Rex
        </h1>
        <p className="text-xl text-slate-600 mb-6">
          A Rust-native React framework with Next.js compatibility. Fast builds,
          lightweight runtime, familiar conventions.
        </p>
        <div className="flex gap-3">
          <Link
            href="/getting-started"
            className="inline-flex items-center px-4 py-2 rounded-lg bg-emerald-600 text-white text-sm font-medium hover:bg-emerald-700 transition-colors no-underline"
          >
            Get Started
          </Link>
          <a
            href="https://github.com/limlabs/rex"
            className="inline-flex items-center px-4 py-2 rounded-lg border border-slate-300 text-slate-700 text-sm font-medium hover:bg-slate-50 transition-colors no-underline"
          >
            GitHub
          </a>
        </div>
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        <Card
          title="Rust-Powered"
          description="Server, router, and build pipeline written in Rust for maximum performance and minimal resource usage."
        />
        <Card
          title="Next.js Compatible"
          description="Pages Router and App Router support. Use getServerSideProps, getStaticProps, and React Server Components."
        />
        <Card
          title="V8 SSR"
          description="Server-side rendering via V8 isolates — no Node.js runtime needed for SSR."
        />
        <Card
          title="Rolldown + OXC"
          description="Built on Rolldown for bundling and OXC for parsing. Fast builds with minimal dependencies."
        />
        <Card
          title="Batteries Included"
          description="Built-in auth, linting, formatting, and MCP server support out of the box."
        />
        <Card
          title="Light Footprint"
          description="Single binary deployment. No node_modules on the server. Docker images under 50MB."
        />
      </div>
    </div>
  );
}

function Card({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <div className="rounded-lg border border-slate-200 p-5 hover:border-emerald-300 transition-colors">
      <h3 className="font-semibold text-slate-900 mb-1">{title}</h3>
      <p className="text-sm text-slate-600">{description}</p>
    </div>
  );
}
