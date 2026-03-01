import React from 'react';
function buildSrc(src, w, q) {
    return '/_rex/image?url=' + encodeURIComponent(src) + '&w=' + w + '&q=' + q;
}
export default function Image(props) {
    const { src, width, height, alt, quality = 75, priority = false, className } = props;
    const imgProps = {
        alt: alt ?? '',
        src: buildSrc(src, width ?? 1920, quality),
        loading: priority ? 'eager' : 'lazy',
        decoding: 'async',
        style: { display: 'block', maxWidth: '100%', height: 'auto' },
    };
    if (width !== undefined)
        imgProps.width = width;
    if (height !== undefined)
        imgProps.height = height;
    if (className)
        imgProps.className = className;
    return React.createElement('img', imgProps);
}
//# sourceMappingURL=image.js.map