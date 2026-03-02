import React from 'react';

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html>
      <body>
        <div>
          <nav>
            <a href="/">Home</a>
            {' | '}
            <a href="/about">About</a>
            {' | '}
            <a href="/blog/hello">Blog</a>
            {' | '}
            <a href="/dashboard">Dashboard</a>
          </nav>
          <main>{children}</main>
        </div>
      </body>
    </html>
  );
}
