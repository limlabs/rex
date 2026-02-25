import React from 'react';
import Link from 'rex/link';

interface Props {
  slug: string;
  title: string;
}

export default function BlogPost({ slug, title }: Props) {
  return (
    <div>
      <h1>{title}</h1>
      <p>Slug: {slug}</p>
      <Link href="/">Back to home</Link>
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
