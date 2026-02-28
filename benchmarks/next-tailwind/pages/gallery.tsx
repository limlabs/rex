import React from 'react';
import Image from 'next/image';
import Head from 'next/head';

export default function Gallery() {
  return (
    <div className="max-w-4xl mx-auto p-8">
      <Head>
        <title>Gallery</title>
      </Head>
      <h1 className="text-3xl font-bold text-gray-900 mb-6">Gallery</h1>
      <div className="grid grid-cols-1 gap-6">
        <Image src="/images/hero.jpg" width={1920} height={1080} alt="Hero" priority />
        <Image src="/images/thumbnail.jpg" width={400} height={300} alt="Thumb" />
        <Image src="/images/avatar.png" width={64} height={64} alt="Avatar" />
      </div>
    </div>
  );
}
