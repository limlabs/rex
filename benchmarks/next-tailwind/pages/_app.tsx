import React from 'react';
import type { AppProps } from 'next/app';
import Link from 'next/link';
import '../styles/globals.css';

export default function App({ Component, pageProps }: AppProps) {
  return (
    <div className="min-h-screen bg-gray-50">
      <nav className="bg-white shadow-sm border-b border-gray-200 px-6 py-3 flex gap-4">
        <Link href="/">Home</Link>
        <Link href="/about">About</Link>
      </nav>
      <main className="max-w-4xl mx-auto px-6 py-8">
        <Component {...pageProps} />
      </main>
    </div>
  );
}
