import React from 'react';
import Head from 'rex/head';
import Link from 'rex/link';

export default function Static() {
  return (
    <div>
      <Head>
        <title>Static Page - Rex</title>
      </Head>
      <h1>Static Page</h1>
      <p>This page has no data fetching function.</p>
      <Link href="/">Back to home</Link>
    </div>
  );
}
