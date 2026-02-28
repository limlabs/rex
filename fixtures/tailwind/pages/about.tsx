import React from 'react';
import Head from 'rex/head';
import Link from 'rex/link';

interface Props {
  description: string;
  builtAt: string;
}

export default function About({ description, builtAt }: Props) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <Head>
        <title>About - Rex</title>
        <meta name="description" content="Learn about Rex, a Next.js Pages Router in Rust." />
      </Head>
      <h1 className="text-3xl font-bold text-gray-900 mb-4">About</h1>
      <p className="text-lg text-gray-600 mb-2">{description}</p>
      <p className="text-sm text-gray-400">Built at: {builtAt}</p>
      <Link href="/">
        <span className="mt-4 inline-block text-blue-600 hover:text-blue-800 underline">
          Back to home
        </span>
      </Link>
    </div>
  );
}

export async function getStaticProps() {
  return {
    props: {
      description: 'Rex is a Next.js Pages Router reimplemented in Rust.',
      builtAt: new Date().toISOString(),
    },
  };
}
