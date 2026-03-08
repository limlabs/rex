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
          </nav>
          {children}
        </div>
      </body>
    </html>
  );
}
