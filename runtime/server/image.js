// Server-side Image component for rex/image
// Renders an optimized <img> tag pointing to /_rex/image during SSR.
import { createElement } from 'react';

// Device-scale widths matching next/image breakpoints
var DEVICE_SIZES = [640, 750, 828, 1080, 1200, 1920];
var ICON_SIZES = [16, 32, 48, 64, 96, 128, 256, 384];

function buildSrc(src, w, q) {
    return '/_rex/image?url=' + encodeURIComponent(src) + '&w=' + w + '&q=' + (q || 75);
}

function buildSrcSet(src, width, quality) {
    // Pick widths relevant to this image's display size.
    // For small images (<=384), use icon sizes up to 2x.
    // For larger images, use device sizes up to 2x.
    var all = ICON_SIZES.concat(DEVICE_SIZES);
    var limit = width * 2;
    var parts = [];
    for (var i = 0; i < all.length; i++) {
        var w = all[i];
        // Skip widths smaller than 1/4 of display size (useless for this image)
        if (w < Math.floor(width / 4)) continue;
        if (w <= limit) {
            parts.push(buildSrc(src, w, quality) + ' ' + w + 'w');
        }
    }
    // Always include the exact requested width if not already covered
    if (parts.length === 0) {
        parts.push(buildSrc(src, width, quality) + ' ' + width + 'w');
    }
    return parts.join(', ');
}

export default function Image(props) {
    var src = props.src;
    var width = props.width;
    var height = props.height;
    var alt = props.alt;
    var quality = props.quality || 75;
    var priority = props.priority || false;
    var placeholder = props.placeholder;
    var blurDataURL = props.blurDataURL;
    var fill = props.fill || false;
    var sizes = props.sizes;

    // Base style: block display, prevent overflow, maintain aspect ratio (like next/image)
    var style = {
        display: 'block',
        maxWidth: '100%',
        height: 'auto',
    };

    var imgProps = {
        alt: alt || '',
        loading: priority ? 'eager' : 'lazy',
        decoding: 'async',
    };

    if (fill) {
        style = {
            position: 'absolute',
            top: 0,
            left: 0,
            width: '100%',
            height: '100%',
            objectFit: props.objectFit || 'cover',
        };
    } else {
        imgProps.width = width;
        imgProps.height = height;
    }

    if (priority) {
        imgProps.fetchPriority = 'high';
    }

    // Set src and srcSet
    imgProps.src = buildSrc(src, width || 1920, quality);
    if (width) {
        imgProps.srcSet = buildSrcSet(src, width, quality);
    }

    if (sizes) {
        imgProps.sizes = sizes;
    } else if (width) {
        imgProps.sizes = '(max-width: ' + width + 'px) 100vw, ' + width + 'px';
    }

    // Blur placeholder as background
    if (placeholder === 'blur' && blurDataURL) {
        style.backgroundImage = 'url(' + blurDataURL + ')';
        style.backgroundSize = 'cover';
        style.backgroundRepeat = 'no-repeat';
    }

    imgProps.style = style;

    if (props.className) imgProps.className = props.className;
    if (props.id) imgProps.id = props.id;
    if (props.onLoad) imgProps.onLoad = props.onLoad;

    return createElement('img', imgProps);
}
