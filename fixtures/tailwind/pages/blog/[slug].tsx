import React from 'react';
import Link from 'rex/link';

interface Props {
  slug: string;
  title: string;
}

export default function BlogPost({ slug, title }: Props) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <h1 className="text-3xl font-bold text-gray-900 mb-4">{title}</h1>
      <p className="text-gray-500 mb-6">Slug: {slug}</p>
      <article className="prose prose-gray">
        <p className="text-gray-700 leading-relaxed">
          This is a blog post about {slug}. It demonstrates dynamic routing
          with Tailwind CSS styling in Rex.
        </p>
      </article>
      <Link href="/">
        <span className="mt-8 inline-block text-blue-600 hover:text-blue-800 underline">
          Back to home
        </span>
      </Link>
    </div>
  );
}

export async function getServerSideProps(context: { params: { slug: string } }) {
  return {
    props: {
      slug: context.params.slug,
      title: `Blog Post: ${context.params.slug}`,
    },
  };
}
