import React from 'react';

export default function Post({ id, title }: { id: string; title: string }) {
  return (
    <div>
      <h1>{title}</h1>
      <p>Post ID: {id}</p>
    </div>
  );
}

export function getStaticPaths() {
  return {
    paths: [
      { params: { id: 'first' } },
      { params: { id: 'second' } },
      { params: { id: 'third' } },
    ],
    fallback: false,
  };
}

export function getStaticProps({ params }: { params: { id: string } }) {
  return {
    props: {
      id: params.id,
      title: `Post ${params.id}`,
    },
  };
}
