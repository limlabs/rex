import React from "react";
import Link from "rex/link";
import Mascot from "../components/Mascot";

export const metadata = {
  title: "Rex — Rust-Native React Framework",
};

export default function Home() {
  return (
    <div>
      <div className="mb-10">
        <div className="flex flex-col sm:flex-row items-center sm:items-stretch gap-6 sm:gap-8 mb-20">
          <Mascot />
          <div className="text-center sm:text-left">
            <h1 className="!text-5xl sm:!text-8xl font-bold text-slate-900 border-0 pb-0 !mb-2">
              Rex
            </h1>
            <p className="!text-lg sm:!text-xl text-slate-600">
              A next-generation React framework with Next.js compatibility.
              <br />
              <br />
              Superior performance, minimal dependencies, and a delightful
              development experience for human developers and agents alike!
            </p>
            <div className="flex justify-center sm:justify-start gap-3">
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
          description="Built-in linting, formatting, type checking, and MCP server support out of the box."
        />
        <Card
          title="Light Footprint"
          description="Single binary deployment. No node_modules on the server. Compact Docker images."
        />
      </div>
    </div>
  );
}

function Card({ title, description }: { title: string; description: string }) {
  return (
    <div className="rounded-lg border border-slate-200 p-5 hover:border-emerald-300 transition-colors">
      <h3 className="font-semibold text-slate-900 mb-1">{title}</h3>
      <p className="text-sm text-slate-600">{description}</p>
    </div>
  );
}
