import React from 'react';
import Head from 'rex/head';
import Link from 'rex/link';

export default function About() {
  return (
    <div>
      <Head>
        <title>About - Rex</title>
        <meta name="description" content="Learn about Rex, a Next.js Pages Router in Rust." />
      </Head>
      <h1>About</h1>
      <p>Rex is a Next.js Pages Router reimplemented in Rust.</p>
      <Link href="/">Back to home</Link>
    </div>
  );
}
