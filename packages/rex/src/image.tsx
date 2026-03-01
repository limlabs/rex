import React from 'react';

interface ImageProps {
  src: string;
  width?: number;
  height?: number;
  alt?: string;
  quality?: number;
  priority?: boolean;
  className?: string;
}

function buildSrc(src: string, w: number, q: number): string {
  return '/_rex/image?url=' + encodeURIComponent(src) + '&w=' + w + '&q=' + q;
}

export default function Image(props: ImageProps): React.ReactElement {
  const { src, width, height, alt, quality = 75, priority = false, className } = props;

  const imgProps: React.ImgHTMLAttributes<HTMLImageElement> = {
    alt: alt ?? '',
    src: buildSrc(src, width ?? 1920, quality),
    loading: priority ? 'eager' : 'lazy',
    decoding: 'async',
    style: { display: 'block', maxWidth: '100%', height: 'auto' },
  };

  if (width !== undefined) imgProps.width = width;
  if (height !== undefined) imgProps.height = height;
  if (className) imgProps.className = className;

  return React.createElement('img', imgProps);
}
