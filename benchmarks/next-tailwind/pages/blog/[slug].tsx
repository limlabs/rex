import React from 'react';
import type { GetServerSidePropsContext } from 'next';

interface Props {
  slug: string;
  title: string;
}

export default function BlogPost({ slug, title }: Props) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <h1 className="text-3xl font-bold text-gray-900 mb-4">Blog Post: {slug}</h1>
      <p className="text-gray-600">{title}</p>
    </div>
  );
}

export async function getServerSideProps(ctx: GetServerSidePropsContext) {
  const slug = ctx.params?.slug as string;
  return {
    props: {
      slug,
      title: `Post about ${slug}`,
    },
  };
}
