import React from 'react';
import Link from 'next/link';

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html>
      <body>
        <div>
          <nav>
            <Link href="/">Home</Link>
            {' | '}
            <Link href="/about">About</Link>
          </nav>
          {children}
        </div>
      </body>
    </html>
  );
}
