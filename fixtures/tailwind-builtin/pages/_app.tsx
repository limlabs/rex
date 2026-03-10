import React from 'react';
import '../styles/globals.css';

export default function App({ Component, pageProps }: { Component: any; pageProps: any }) {
  return (
    <div className="min-h-screen bg-gray-50">
      <main className="max-w-4xl mx-auto px-6 py-8">
        <Component {...pageProps} />
      </main>
    </div>
  );
}
