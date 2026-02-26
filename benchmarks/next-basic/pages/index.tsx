import React from 'react';
import type { GetServerSidePropsContext } from 'next';

interface Props {
  message: string;
  timestamp: string;
}

export default function Home({ message, timestamp }: Props) {
  return (
    <div>
      <h1>Rex!</h1>
      <p>{message}</p>
      <p>Rendered at: {timestamp}</p>
    </div>
  );
}

export async function getServerSideProps(ctx: GetServerSidePropsContext) {
  return {
    props: {
      message: "Hello from Rex!",
      timestamp: new Date().toISOString(),
    },
  };
}
