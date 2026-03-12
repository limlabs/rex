// next/image → Rex Image stub for server bundles.
// Returns a plain <img> element descriptor for SSR.

/* eslint-disable @typescript-eslint/no-explicit-any */

function Image(props: any) {
    const { src, alt, width, height, ...rest } = props;
    const srcStr = typeof src === 'string' ? src
        : (src && typeof src === 'object' && src.src) ? src.src
        : '';
    return { type: 'img', props: { src: srcStr, alt, width, height, ...rest } };
}

Image.displayName = 'Image';

export default Image;
export { Image };
