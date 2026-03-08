import React from 'react';
import ActionCounter from '../components/ActionCounter';

export const metadata = {
  title: 'Home',
  description: 'Welcome to Rex App Router',
};

// Server component (default) — no "use client" directive
export default function Home() {
  const message = "Hello from Rex!";
  const timestamp = new Date().toISOString();

  return (
    <div>
      <h1>Rex!</h1>
      <p>{message}</p>
      <p>Rendered at: {timestamp}</p>
      <ActionCounter />
    </div>
  );
}
