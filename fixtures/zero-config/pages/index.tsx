import React from "react";

interface Props {
  message: string;
  timestamp: number;
}

export default function Home({ message, timestamp }: Props) {
  return (
    <div>
      <h1>Zero-Config Rex</h1>
      <p>{message}</p>
      <p>Rendered at: {new Date(timestamp).toISOString()}</p>
    </div>
  );
}

export async function getServerSideProps() {
  return {
    props: {
      message: "No package.json needed — Rex provides React!",
      timestamp: Date.now(),
    },
  };
}
