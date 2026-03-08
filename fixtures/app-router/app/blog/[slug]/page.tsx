import React from 'react';

export function generateMetadata({ params }: { params: { slug: string } }) {
  return {
    title: `Blog: ${params.slug}`,
    description: `Read about ${params.slug}`,
    openGraph: {
      title: `Blog: ${params.slug}`,
      type: 'article',
    },
  };
}

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
