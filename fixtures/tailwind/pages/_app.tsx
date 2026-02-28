import React from 'react';
import Link from 'rex/link';
import '../styles/globals.css';

export default function App({ Component, pageProps }: { Component: any; pageProps: any }) {
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
