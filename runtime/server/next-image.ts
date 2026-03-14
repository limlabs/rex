// next/image → Rex Image stub for server bundles.
// Renders a plain <img> React element for SSR.
import { createElement, type ReactElement } from "react";

/* eslint-disable @typescript-eslint/no-explicit-any */

function Image(props: any): ReactElement {
    const { src, alt, width, height, fill, loader: _loader, ...rest } = props;
    const srcStr = typeof src === 'string' ? src
        : (src && typeof src === 'object' && src.src) ? src.src
        : '';

    const imgProps: Record<string, any> = { src: srcStr, alt, ...rest };

    if (fill) {
        // fill mode: absolute positioned, no explicit width/height
        imgProps.style = {
            position: 'absolute',
            top: 0,
            left: 0,
            width: '100%',
            height: '100%',
            objectFit: rest.objectFit || 'cover',
            ...rest.style,
        };
    } else {
        if (width != null) imgProps.width = width;
        if (height != null) imgProps.height = height;
    }

    imgProps.loading = props.priority ? 'eager' : 'lazy';
    imgProps.decoding = 'async';

    return createElement('img', imgProps);
}

Image.displayName = 'Image';

export default Image;
export { Image };
