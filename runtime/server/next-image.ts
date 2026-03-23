// next/image → Rex Image stub for server bundles.
// Renders a plain <img> React element for SSR.
import { createElement, type ReactElement } from "react";

/* eslint-disable @typescript-eslint/no-explicit-any */

function resolveUrl(src: string, w: number, q: number): string {
    if (!src) return '';
    // SVGs are vector — serve directly, skip raster optimizer
    if (src.endsWith('.svg')) return src;
    return '/_rex/image?url=' + encodeURIComponent(src) + '&w=' + w + '&q=' + q;
}

function Image(props: any): ReactElement {
    const rawSrc = props.src;
    const srcStr = typeof rawSrc === 'string' ? rawSrc
        : (rawSrc && typeof rawSrc === 'object' && rawSrc.src) ? rawSrc.src
        : (rawSrc && typeof rawSrc === 'object' && rawSrc.default && typeof rawSrc.default === 'string') ? rawSrc.default
        : '';

    const quality = props.quality || 75;
    const width = props.width || (props.fill ? 1920 : 0);
    const resolvedSrc = resolveUrl(srcStr, width || 1920, quality);

    const imgProps: Record<string, any> = {
        alt: props.alt || '',
        loading: props.priority ? 'eager' : 'lazy',
        decoding: 'async',
    };

    if (resolvedSrc) imgProps.src = resolvedSrc;

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
