import React from 'react';
import '../styles/globals.css';

export default function App({ Component, pageProps }) {
  return (
    <div className="app-wrapper">
      <nav>
        <a href="/">Home</a>
        <a href="/about">About</a>
      </nav>
      <Component {...pageProps} />
    </div>
  );
}
