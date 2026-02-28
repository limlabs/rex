import React from 'react';
import Image from 'rex/image';
import Head from 'rex/head';

export default function Gallery() {
  return (
    <div>
      <Head>
        <title>Gallery</title>
      </Head>
      <h1>Gallery</h1>
      <Image src="/images/hero.jpg" width={1920} height={1080} alt="Hero" priority />
      <Image src="/images/thumbnail.jpg" width={400} height={300} alt="Thumb" />
      <Image src="/images/avatar.png" width={64} height={64} alt="Avatar" />
    </div>
  );
}
