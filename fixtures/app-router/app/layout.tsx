import React from 'react';
import Link from 'rex/link';

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html>
      <body>
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
