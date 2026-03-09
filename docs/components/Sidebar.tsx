"use client";

import React, { useState } from "react";
import Link from "rex/link";

interface NavItem {
  title: string;
  href: string;
}

interface NavSection {
  title: string;
  items: NavItem[];
}

const navigation: NavSection[] = [
  {
    title: "Getting Started",
    items: [
      { title: "Quickstart", href: "/getting-started" },
      { title: "Installation", href: "/getting-started/installation" },
    ],
  },
  {
    title: "How Rex Works",
    items: [
      { title: "Architecture", href: "/architecture" },
      { title: "Differences from Next.js", href: "/architecture/differences" },
    ],
  },
  {
    title: "Features",
    items: [
      { title: "Routing", href: "/features/routing" },
      { title: "Data Fetching", href: "/features/data-fetching" },
      { title: "Styling", href: "/features/styling" },
      { title: "Middleware", href: "/features/middleware" },
      { title: "Custom Server", href: "/features/custom-server" },
      { title: "MDX", href: "/features/mdx" },
    ],
  },
  {
    title: "Reference",
    items: [
      { title: "CLI", href: "/cli" },
      { title: "Configuration", href: "/configuration" },
    ],
  },
  {
    title: "Deployment",
    items: [{ title: "Deploy Rex", href: "/deployment" }],
  },
];

export default function Sidebar() {
  const [open, setOpen] = useState(false);

  return (
    <>
      <button
        onClick={() => setOpen(!open)}
        className="lg:hidden fixed top-3 left-3 z-50 p-2 rounded-md bg-slate-800 text-white"
        aria-label="Toggle navigation"
      >
        <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
          {open ? (
            <path d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z" />
          ) : (
            <path d="M3 5h14M3 10h14M3 15h14" stroke="currentColor" strokeWidth="2" fill="none" />
          )}
        </svg>
      </button>

      {open && (
        <div
          className="lg:hidden fixed inset-0 bg-black/30 z-30"
          onClick={() => setOpen(false)}
        />
      )}

      <aside
        className={`fixed top-0 left-0 z-40 h-full w-64 bg-slate-900 text-slate-300 overflow-y-auto transition-transform lg:translate-x-0 ${
          open ? "translate-x-0" : "-translate-x-full"
        }`}
      >
        <div className="px-5 py-5 border-b border-slate-700">
          <Link href="/" className="flex items-center gap-2 no-underline">
            <span className="text-xl font-bold text-emerald-400">Rex</span>
            <span className="text-xs text-slate-500 font-mono mt-1">docs</span>
          </Link>
        </div>

        <nav className="px-3 py-4">
          {navigation.map((section) => (
            <div key={section.title} className="mb-5">
              <h3 className="px-2 mb-1 text-xs font-semibold uppercase tracking-wider text-slate-500">
                {section.title}
              </h3>
              <ul className="space-y-0.5">
                {section.items.map((item) => (
                  <li key={item.href}>
                    <Link
                      href={item.href}
                      className="block px-2 py-1.5 rounded text-sm text-slate-400 hover:text-white hover:bg-slate-800 transition-colors no-underline"
                    >
                      {item.title}
                    </Link>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </nav>

        <div className="px-5 py-4 border-t border-slate-700 mt-auto">
          <a
            href="https://github.com/limlabs/rex"
            className="text-xs text-slate-500 hover:text-slate-300 transition-colors no-underline"
          >
            GitHub
          </a>
        </div>
      </aside>
    </>
  );
}
