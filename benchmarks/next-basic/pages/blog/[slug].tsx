import React from 'react';
import type { GetServerSidePropsContext } from 'next';

interface Props {
  slug: string;
  title: string;
}

export default function BlogPost({ slug, title }: Props) {
  return (
    <div>
      <h1>Blog Post: {slug}</h1>
      <p>{title}</p>
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
