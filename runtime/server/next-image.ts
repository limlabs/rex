// next/image → Rex Image stub for server bundles.
// Renders a plain <img> React element for SSR.
import { createElement, type ReactElement } from "react";

/* eslint-disable @typescript-eslint/no-explicit-any */

function Image(props: any): ReactElement {
    const src = props.src;
    const srcStr = typeof src === 'string' ? src
        : (src && typeof src === 'object' && src.src) ? src.src
        : (src && typeof src === 'object' && src.default && typeof src.default === 'string') ? src.default
        : '';

    const imgProps: Record<string, any> = {
        alt: props.alt || '',
        loading: props.priority ? 'eager' : 'lazy',
        decoding: 'async',
    };

    // Only set src if we have a non-empty URL
    if (srcStr) imgProps.src = srcStr;

    if (props.fill) {
        imgProps.style = {
            position: 'absolute',
            top: 0,
            left: 0,
            width: '100%',
            height: '100%',
            objectFit: props.objectFit || props.style?.objectFit || 'cover',
            ...props.style,
        };
    } else {
        if (props.width != null) imgProps.width = props.width;
        if (props.height != null) imgProps.height = props.height;
        imgProps.style = props.style;
    }

    if (props.sizes) imgProps.sizes = props.sizes;
    if (props.className) imgProps.className = props.className;
    if (props.id) imgProps.id = props.id;
    if (props.priority) imgProps.fetchPriority = 'high';

    return createElement('img', imgProps);
}

Image.displayName = 'Image';

export default Image;
export { Image };
