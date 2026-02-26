import React from 'react';
import type { AppProps } from 'next/app';
import Link from 'next/link';

export default function App({ Component, pageProps }: AppProps) {
  return (
    <div>
      <nav>
        <Link href="/">Home</Link>
        {' | '}
        <Link href="/about">About</Link>
      </nav>
      <Component {...pageProps} />
    </div>
  );
}
