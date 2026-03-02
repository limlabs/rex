import React from 'react';

// Server component with dynamic params
export default function BlogPost({ params }: { params: { slug: string } }) {
  const { slug } = params;

  return (
    <div>
      <h1>Blog Post: {slug}</h1>
      <p>This is the blog post about {slug}.</p>
      <a href="/">Back to home</a>
    </div>
  );
}
