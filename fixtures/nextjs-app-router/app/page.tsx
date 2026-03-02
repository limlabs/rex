import React from 'react';

// Server component (default) — no "use client" directive
export default function Home() {
  const message = "Hello from Next.js RSC!";
  const timestamp = new Date().toISOString();

  return (
    <div>
      <h1>Next.js App Router</h1>
      <p>{message}</p>
      <p>Rendered at: {timestamp}</p>
    </div>
  );
}
