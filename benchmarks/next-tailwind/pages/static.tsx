import React from 'react';
import Head from 'next/head';

export default function StaticPage() {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <Head>
        <title>Static Page</title>
      </Head>
      <h1 className="text-3xl font-bold text-gray-900 mb-4">Static Page</h1>
      <p className="text-gray-600">This page has no data fetching — pure static render.</p>
    </div>
  );
}
