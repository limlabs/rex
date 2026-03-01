// rex/image - Image component stub
// During bundling, rex/image is aliased to the full runtime implementation.
// This file exists for direct imports outside the bundler (e.g., type checking).
import React from 'react';

function buildSrc(src, w, q) {
    return '/_rex/image?url=' + encodeURIComponent(src) + '&w=' + w + '&q=' + (q || 75);
}

export default function Image(props) {
    var src = props.src;
    var width = props.width;
    var height = props.height;
    var alt = props.alt;
    var quality = props.quality || 75;
    var priority = props.priority || false;

    var imgProps = {
        alt: alt || '',
        src: buildSrc(src, width || 1920, quality),
        loading: priority ? 'eager' : 'lazy',
        decoding: 'async',
        style: { display: 'block', maxWidth: '100%', height: 'auto' },
    };

    if (width) imgProps.width = width;
    if (height) imgProps.height = height;
    if (props.className) imgProps.className = props.className;

    return React.createElement('img', imgProps);
}
