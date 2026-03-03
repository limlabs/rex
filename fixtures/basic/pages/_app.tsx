import React from 'react';
import Link from 'rex/link';
import '../styles/globals.css';

export default function App({ Component, pageProps }: { Component: React.ComponentType; pageProps: Record<string, unknown> }) {
  return (
    <div className="app-wrapper">
      <nav>
        <Link href="/">Home</Link>
        <Link href="/about">About</Link>
      </nav>
      <Component {...pageProps} />
    </div>
  );
}
