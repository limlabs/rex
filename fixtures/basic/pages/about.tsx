import React from 'react';
import Head from 'rex/head';
import Link from 'rex/link';

interface Props {
  description: string;
  builtAt: string;
}

export default function About({ description, builtAt }: Props) {
  return (
    <div>
      <Head>
        <title>About - Rex</title>
        <meta name="description" content="Learn about Rex, a Next.js Pages Router in Rust." />
      </Head>
      <h1>About</h1>
      <p>{description}</p>
      <p>Built at: {builtAt}</p>
      <Link href="/">Back to home</Link>
    </div>
  );
}

export async function getStaticProps() {
  return {
    props: {
      description: "Rex is a Next.js Pages Router reimplemented in Rust.",
      builtAt: new Date().toISOString(),
    },
  };
}
