import React from 'react';

// Server component
export default function About() {
  const description = "Rex is a Next.js-compatible framework in Rust with RSC support.";

  return (
    <div>
      <h1>About</h1>
      <p>{description}</p>
      <a href="/">Back to home</a>
    </div>
  );
}
