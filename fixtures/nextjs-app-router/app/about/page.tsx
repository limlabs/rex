import React from 'react';

// Server component
export default function About() {
  const description = "Next.js is a React framework with RSC support.";

  return (
    <div>
      <h1>About</h1>
      <p>{description}</p>
      <a href="/">Back to home</a>
    </div>
  );
}
