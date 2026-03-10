import React from "react";

interface Props {
  app: string;
  timestamp: number;
}

export default function Home({ app, timestamp }: Props) {
  return (
    <div>
      <h1>Live App A</h1>
      <p>Running: {app}</p>
      <p>Rendered at: {new Date(timestamp).toISOString()}</p>
    </div>
  );
}

export async function getServerSideProps() {
  return {
    props: {
      app: "live-a",
      timestamp: Date.now(),
    },
  };
}
