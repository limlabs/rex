import React from 'react';

// Server component with dynamic params
export default async function BlogPost({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;

  return (
    <div>
      <h1>Blog Post: {slug}</h1>
      <p>This is the blog post about {slug}.</p>
      <a href="/">Back to home</a>
    </div>
  );
}
