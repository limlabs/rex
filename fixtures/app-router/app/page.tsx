import React from 'react';
import ActionCounter from '../components/ActionCounter';

// Server component (default) — no "use client" directive
export default function Home() {
  const message = "Hello from Rex RSC!";
  const timestamp = new Date().toISOString();

  return (
    <div>
      <h1>Rex App Router</h1>
      <p>{message}</p>
      <p>Rendered at: {timestamp}</p>
      <ActionCounter />
    </div>
  );
}
