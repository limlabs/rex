import React from 'react';

interface Props {
  slug: string;
  title: string;
}

export default function BlogPost({ slug, title }: Props) {
  return (
    <div>
      <h1>{title}</h1>
      <p>Slug: {slug}</p>
      <a href="/">Back to home</a>
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
