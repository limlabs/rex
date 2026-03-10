import React from 'react';
import Link from 'rex/link';

export const metadata = {
  title: {
    default: 'Rex App',
    template: '%s | Rex App',
  },
  description: 'A Next.js-compatible framework powered by Rust',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className="app-root">
        <div>
          <nav>
            <Link href="/">Home</Link>
            {' | '}
            <Link href="/about">About</Link>
            {' | '}
            <Link href="/blog/hello">Blog</Link>
            {' | '}
            <Link href="/dashboard">Dashboard</Link>
          </nav>
          <main>{children}</main>
        </div>
      </body>
    </html>
  );
}
