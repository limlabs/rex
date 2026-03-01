import React from "react";

interface Props {
  greeting: string;
  runtime: string;
}

export default function Home({ greeting, runtime }: Props) {
  return (
    <div>
      <h1>{greeting}</h1>
      <p>Served by: {runtime}</p>
    </div>
  );
}

export async function getServerSideProps() {
  return {
    props: {
      greeting: "Hello from Rex + Bun!",
      runtime: typeof Bun !== "undefined" ? "Bun" : "Node.js",
    },
  };
}
