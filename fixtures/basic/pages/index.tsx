import React from "react";
import { useRouter } from "rex/router";

interface Props {
  message: string;
  timestamp: number;
}

export default function Home({ message, timestamp }: Props) {
  const router = useRouter();
  return (
    <div>
      <h1>Rex!!</h1>
      <p>{message}</p>
      <p>Rendered at: {new Date(timestamp).toISOString()}</p>
      <p>Route: {router.pathname}</p>
    </div>
  );
}

export async function getServerSideProps() {
  return {
    props: {
      message: "Hello from Rex!",
      timestamp: Date.now(),
    },
  };
}
