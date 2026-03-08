import React from "react";

interface Props {
  timestamp: number;
  region: string;
}

export default function Home({ timestamp, region }: Props) {
  return (
    <div>
      <h1>Rex on Railway</h1>
      <p>Server-rendered at: {new Date(timestamp).toISOString()}</p>
      <p>Region: {region}</p>
    </div>
  );
}

export async function getServerSideProps() {
  return {
    props: {
      timestamp: Date.now(),
      region: process.env.RAILWAY_REGION || "local",
    },
  };
}
