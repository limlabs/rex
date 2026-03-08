import React from 'react';

interface Props {
  slug: string;
}

export default function BlogPost({ slug }: Props) {
  return (
    <div>
      <h1>Blog Post: {slug}</h1>
      <p>Post about {slug}</p>
    </div>
  );
}

export async function getServerSideProps(context: { params: { slug: string } }) {
  return {
    props: {
      slug: context.params.slug,
    },
  };
}
